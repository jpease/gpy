//! Mechanical theme rewrites for keys removed by breaking changes.
//!
//! Rules are grouped in [`RULE_SETS`], one per release that removed theme keys,
//! applied in order. To cover a later release, append a [`RuleSet`]; never edit
//! an old one, because a copy may come from any earlier version.

use crate::plugin::SettingValue;
use toml_edit::{DocumentMut, TableLike};

/// The kind of value a theme key must hold to be moved to `config.toml`.
#[derive(Clone, Copy)]
enum ValueKind {
    Bool,
    Text,
}

/// One rewrite of a `[segments.<segment>]` table.
enum Rule {
    /// Remove a key the theme no longer reads.
    Drop {
        segment: &'static str,
        key: &'static str,
        why: &'static str,
    },
    /// Remove a key that is now a `config.toml` `[plugin.<segment>]` setting.
    MoveToConfig {
        segment: &'static str,
        key: &'static str,
        kind: ValueKind,
    },
}

/// The rewrites for the keys one release removed.
struct RuleSet {
    /// Release that removed the keys.
    since: &'static str,
    rules: &'static [Rule],
}

const PILL_DRAWN_BY_FRAMEWORK: &str = "the plugin framework draws the pill";

/// Every rule set, oldest first.
const RULE_SETS: &[RuleSet] = &[RuleSet {
    since: "0.7",
    // `username` and `hostname` became bundled script plugins.
    rules: &[
        Rule::Drop {
            segment: "username",
            key: "format",
            why: PILL_DRAWN_BY_FRAMEWORK,
        },
        Rule::Drop {
            segment: "username",
            key: "user_format",
            why: PILL_DRAWN_BY_FRAMEWORK,
        },
        Rule::MoveToConfig {
            segment: "username",
            key: "show_always",
            kind: ValueKind::Bool,
        },
        Rule::Drop {
            segment: "hostname",
            key: "format",
            why: PILL_DRAWN_BY_FRAMEWORK,
        },
        Rule::MoveToConfig {
            segment: "hostname",
            key: "show_always",
            kind: ValueKind::Bool,
        },
        Rule::MoveToConfig {
            segment: "hostname",
            key: "trim_at",
            kind: ValueKind::Text,
        },
    ],
}];

/// A theme key that moved to `config.toml` `[plugin.<segment>]`.
pub struct SettingMove {
    /// Release that moved the setting.
    pub since: &'static str,
    /// Segment whose `[plugin.<segment>]` table takes the setting.
    pub segment: &'static str,
    /// Setting name.
    pub key: &'static str,
    /// The value the theme held.
    pub value: SettingValue,
}

/// What [`apply`] did to a theme.
#[derive(Default)]
pub struct Migration {
    /// One line per rewrite or skipped rewrite, in the order applied.
    pub notes: Vec<String>,
    /// Keys removed from the theme that belong in `config.toml`.
    pub moves: Vec<SettingMove>,
}

/// Apply every rule set to `theme`, rewriting it in place with its comments and
/// layout intact. An overlay's `unset` entry for a removed or moved key is
/// dropped with the key.
pub fn apply(theme: &mut DocumentMut) -> Migration {
    let mut migration = Migration::default();
    for set in RULE_SETS {
        for rule in set.rules {
            apply_rule(theme, set.since, rule, &mut migration);
            rewrite_unset(theme, set.since, rule, &mut migration);
        }
    }
    migration
}

/// What a rule does to the `unset` entry for `segments.<segment>.<key>`: the
/// entry of a removed or moved key is dropped, since the old path would be
/// ignored at load and the user's intent lost.
fn rewrite_unset(theme: &mut DocumentMut, since: &str, rule: &Rule, out: &mut Migration) {
    let (Rule::Drop { segment, key, .. } | Rule::MoveToConfig { segment, key, .. }) = *rule;
    let Some(entries) = theme.get_mut("unset").and_then(|item| item.as_array_mut()) else {
        return;
    };
    let old_path = format!("segments.{segment}.{key}");
    let had_entries = !entries.is_empty();
    let mut index = 0;
    while index < entries.len() {
        if entries.get(index).and_then(|entry| entry.as_str()) == Some(old_path.as_str()) {
            entries.remove(index);
            out.notes.push(format!(
                "{since}: dropped `{old_path}` from `unset` (the key no longer exists)"
            ));
        } else {
            index = index.saturating_add(1);
        }
    }
    if had_entries && entries.is_empty() {
        theme.remove("unset");
    }
}

fn apply_rule(theme: &mut DocumentMut, since: &'static str, rule: &Rule, out: &mut Migration) {
    match *rule {
        Rule::Drop { segment, key, why } => {
            if let Some(table) = segment_table(theme, segment)
                && table.remove(key).is_some()
            {
                out.notes.push(format!(
                    "{since}: removed `{key}` from [segments.{segment}] ({why})"
                ));
            }
        }
        Rule::MoveToConfig { segment, key, kind } => {
            let Some(table) = segment_table(theme, segment) else {
                return;
            };
            let Some(item) = table.get(key) else {
                return;
            };
            let held = match kind {
                ValueKind::Bool => item.as_bool().map(SettingValue::Bool),
                ValueKind::Text => item
                    .as_str()
                    .map(|text| SettingValue::String(text.to_owned())),
            };
            match held {
                Some(value) => {
                    table.remove(key);
                    out.moves.push(SettingMove {
                        since,
                        segment,
                        key,
                        value,
                    });
                }
                None => out.notes.push(format!(
                    "{since}: left `{key}` in [segments.{segment}] because it is not {}; \
                     set plugin.{segment}.{key} in config.toml by hand",
                    match kind {
                        ValueKind::Bool => "true or false",
                        ValueKind::Text => "a string",
                    }
                )),
            }
        }
    }
}

/// `[segments.<segment>]` of `theme`, whether written as a table or inline.
fn segment_table<'a>(theme: &'a mut DocumentMut, segment: &str) -> Option<&'a mut dyn TableLike> {
    theme
        .get_mut("segments")?
        .as_table_like_mut()?
        .get_mut(segment)?
        .as_table_like_mut()
}

#[cfg(test)]
mod tests {
    use super::apply;
    use crate::plugin::SettingValue;
    use toml_edit::DocumentMut;

    fn migrate(text: &str) -> (String, super::Migration) {
        let mut doc: DocumentMut = text.parse().expect("fixture parses");
        let migration = apply(&mut doc);
        (doc.to_string(), migration)
    }

    #[test]
    fn current_themes_need_no_rewrite() {
        let text = "[segments.username]\ntext_color = \"yellow\"\n\n[segments.hostname]\nssh_icon = \"@\"\n";
        let (rewritten, migration) = migrate(text);
        assert_eq!(rewritten, text);
        assert!(migration.notes.is_empty() && migration.moves.is_empty());
    }

    #[test]
    fn removed_keys_are_dropped_and_settings_are_collected() {
        let (rewritten, migration) = migrate(
            "[segments.username]\nformat = \"[$username]\"\nuser_format = \"x\"\nshow_always = true\n\
             text_color = \"red\"\n\n[segments.hostname]\nformat = \"y\"\ntrim_at = \"-\"\nshow_always = false\n",
        );
        assert_eq!(
            rewritten,
            "[segments.username]\ntext_color = \"red\"\n\n[segments.hostname]\n"
        );
        let moved: Vec<_> = migration
            .moves
            .iter()
            .map(|m| (m.segment, m.key, m.value.clone()))
            .collect();
        assert_eq!(
            moved,
            [
                ("username", "show_always", SettingValue::Bool(true)),
                ("hostname", "show_always", SettingValue::Bool(false)),
                ("hostname", "trim_at", SettingValue::String("-".to_owned())),
            ]
        );
    }

    #[test]
    fn the_current_hostname_icon_key_is_left_alone() {
        let text =
            "[segments.hostname]\n# the globe\nicon = \"G \" # trailing\nbg_color = \"red\"\n";
        let (rewritten, migration) = migrate(text);
        assert_eq!(rewritten, text);
        assert!(migration.notes.is_empty() && migration.moves.is_empty());
    }

    #[test]
    fn a_setting_of_the_wrong_type_stays_in_the_theme() {
        let text = "[segments.hostname]\nshow_always = \"yes\"\n";
        let (rewritten, migration) = migrate(text);
        assert_eq!(rewritten, text);
        assert!(migration.moves.is_empty());
        assert!(
            migration
                .notes
                .first()
                .is_some_and(|note| note.contains("not true or false")),
            "{:?}",
            migration.notes
        );
    }

    #[test]
    fn inline_segment_tables_are_rewritten() {
        let (rewritten, migration) =
            migrate("[segments]\nusername = { format = \"x\", text_color = \"red\" }\n");
        assert!(!rewritten.contains("format"), "{rewritten}");
        assert!(rewritten.contains("text_color"), "{rewritten}");
        assert_eq!(migration.notes.len(), 1);
    }

    #[test]
    fn unset_paths_of_removed_keys_are_dropped_and_current_keys_stay() {
        let (rewritten, _) = migrate(
            "extends = \"starship\"\nunset = [\"segments.hostname.icon\", \"segments.hostname.show_always\", \"segments.clock.format\"]\n",
        );
        assert_eq!(
            rewritten,
            "extends = \"starship\"\nunset = [\"segments.hostname.icon\", \"segments.clock.format\"]\n"
        );
    }
}
