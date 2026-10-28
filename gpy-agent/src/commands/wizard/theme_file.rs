//! Writes the wizard's theme edits (ADR-0012 section 5, "Writers"): only the
//! keys whose effective value changed, compared as the theme reads them
//! (through the field or its defaulting accessor), never as serialized
//! fields, so a key toggled twice writes nothing.
//!
//! - **No user file** (a builtin or plugin theme is selected): a self-named
//!   overlay holding the changed keys.
//! - **An overlay:** each changed key is compared with the overlay's parent
//!   after the overlay's `unset` entries; an entry for exactly that key is
//!   dropped first. The key is set when it differs and removed from the file
//!   when it is equal. Edits are made in place, keeping comments.
//! - **A full copy:** the changed keys are set in place; the file is never
//!   converted, and the `gpy theme rebase` hint is returned instead.
//!
//! Every result is loaded as the file it will become before it is written.

use super::state::ThemeEdits;
use crate::commands::theme_write::{check_write, plugin_dependencies, write_checked};
use crate::config::discovery::validate_config_name;
use crate::theme::overlay::{self, split_path};
use crate::theme::{ChainLink, ThemeConfig, user_copy};
use crate::{Error, Result};
use std::path::PathBuf;
use toml_edit::{DocumentMut, Item, TableLike};

/// A theme key the wizard edits: its path and how the theme reads it.
struct WizardKey {
    /// `[section, segment, field]`.
    path: [&'static str; 3],
    /// The value the theme uses, through the accessor when there is one.
    read: fn(&ThemeConfig) -> KeyValue,
}

/// An effective value of a [`WizardKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyValue {
    Str(String),
    Bool(bool),
}

impl KeyValue {
    /// The value as a theme file item.
    fn item(&self) -> Item {
        match self {
            Self::Str(text) => toml_edit::value(text.as_str()),
            Self::Bool(flag) => toml_edit::value(*flag),
        }
    }
}

fn clock_time_format(theme: &ThemeConfig) -> KeyValue {
    KeyValue::Str(theme.segments.clock.time_format().to_owned())
}

fn clock_show_seconds(theme: &ThemeConfig) -> KeyValue {
    KeyValue::Bool(theme.segments.clock.show_seconds())
}

const fn duration_show_milliseconds(theme: &ThemeConfig) -> KeyValue {
    KeyValue::Bool(theme.segments.duration.show_milliseconds)
}

fn git_show_branch(theme: &ThemeConfig) -> KeyValue {
    KeyValue::Bool(theme.segments.git.show_branch())
}

fn git_show_ahead_behind(theme: &ThemeConfig) -> KeyValue {
    KeyValue::Bool(theme.segments.git.show_ahead_behind())
}

fn git_show_stash(theme: &ThemeConfig) -> KeyValue {
    KeyValue::Bool(theme.segments.git.show_stash())
}

/// Every theme key a [`SegmentEdit`](super::state::SegmentEdit) changes.
static WIZARD_KEYS: [WizardKey; 6] = [
    WizardKey {
        path: ["segments", "clock", "time_format"],
        read: clock_time_format,
    },
    WizardKey {
        path: ["segments", "clock", "show_seconds"],
        read: clock_show_seconds,
    },
    WizardKey {
        path: ["segments", "duration", "show_milliseconds"],
        read: duration_show_milliseconds,
    },
    WizardKey {
        path: ["segments", "git", "show_branch"],
        read: git_show_branch,
    },
    WizardKey {
        path: ["segments", "git", "show_ahead_behind"],
        read: git_show_ahead_behind,
    },
    WizardKey {
        path: ["segments", "git", "show_stash"],
        read: git_show_stash,
    },
];

/// A theme file the wizard wrote.
#[derive(Debug)]
pub struct ThemeWrite {
    /// The written file.
    pub path: PathBuf,
    /// Lines to print after the save: the `rebase` hint for a full copy, the
    /// plugins an overlay depends on.
    pub notes: Vec<String>,
}

/// Write `edits` to the theme `name`'s user file. `None` when no key's
/// effective value changed: nothing is written.
///
/// # Errors
/// Returns an error on an unsafe name, a theme file that can't be read or
/// edited, a result that would not load, or a failed write.
pub fn save_theme(name: &str, edits: &ThemeEdits) -> Result<Option<ThemeWrite>> {
    let changed = changed_keys(edits);
    if changed.is_empty() {
        return Ok(None);
    }
    validate_config_name("theme", name)?;

    let mut notes = Vec::new();
    let text = if user_copy::user_copy_path(name).exists() {
        let link = overlay::own_user_link(name)?;
        let label = link.path.display().to_string();
        let source = std::fs::read_to_string(&link.path)
            .map_err(|e| Error::io_context(format_args!("read {label}"), &e))?;
        let mut doc: DocumentMut = source
            .parse()
            .map_err(|e| Error::config(format!("Failed to parse {label} as TOML:\n  {e}")))?;
        if let Some(base) = doc.get("extends").and_then(Item::as_str).map(str::to_owned) {
            edit_overlay(&mut doc, &changed, &edits.edited, |unset| {
                parent_after(&link, &base, unset)
            })?;
        } else {
            set_keys(&mut doc, &changed, &edits.edited)?;
            notes.push(format!(
                "{label} is a full copy, so later changes to its base do not reach it; \
                 `gpy theme rebase {}` rewrites it to keep only your changes.",
                link.name
            ));
        }
        doc.to_string()
    } else {
        new_overlay(name, &changed, &edits.edited)?
    };

    let loaded = check_write(name, text.clone())?;
    write_checked(&loaded, &text, false)?;
    notes.extend(plugin_dependencies(&loaded.chain));
    Ok(Some(ThemeWrite {
        path: loaded.chain.top().path.clone(),
        notes,
    }))
}

/// The keys whose effective value differs between the seed and the edits.
fn changed_keys(edits: &ThemeEdits) -> Vec<&'static WizardKey> {
    WIZARD_KEYS
        .iter()
        .filter(|key| (key.read)(&edits.seed) != (key.read)(&edits.edited))
        .collect()
}

/// A self-named overlay of `name` holding `changed` as `edited` reads them.
///
/// # Errors
/// As [`set_keys`].
fn new_overlay(name: &str, changed: &[&WizardKey], edited: &ThemeConfig) -> Result<String> {
    let mut doc = DocumentMut::new();
    doc.insert("extends", toml_edit::value(name));
    set_keys(&mut doc, changed, edited)?;
    Ok(doc.to_string())
}

/// Set each of `changed` in `doc` to its value in `edited`.
///
/// # Errors
/// As [`set_key`].
fn set_keys(doc: &mut DocumentMut, changed: &[&WizardKey], edited: &ThemeConfig) -> Result<()> {
    for key in changed {
        set_key(doc, key.path, &(key.read)(edited))?;
    }
    Ok(())
}

/// Apply `changed` to the overlay `doc`. `parent` loads the overlay's parent
/// after the given `unset` entries; `None` when that does not load, and the
/// key is then set.
///
/// # Errors
/// As [`set_key`].
fn edit_overlay(
    doc: &mut DocumentMut,
    changed: &[&WizardKey],
    edited: &ThemeConfig,
    mut parent: impl FnMut(&[String]) -> Option<ThemeConfig>,
) -> Result<()> {
    for key in changed {
        let value = (key.read)(edited);
        drop_unset_entry(doc, key.path);
        let inherited = parent(&unset_entries(doc)).map(|theme| (key.read)(&theme));
        if inherited.as_ref() == Some(&value) {
            remove_key(doc, key.path);
        } else {
            set_key(doc, key.path, &value)?;
        }
    }
    Ok(())
}

/// The overlay `link`'s parent `base`, merged after the `unset` entries.
fn parent_after(link: &ChainLink, base: &str, unset: &[String]) -> Option<ThemeConfig> {
    let mut doc = DocumentMut::new();
    doc.insert("extends", toml_edit::value(base));
    if !unset.is_empty() {
        let entries: toml_edit::Array = unset.iter().map(String::as_str).collect();
        doc.insert("unset", toml_edit::value(entries));
    }
    overlay::load_text(link.clone(), doc.to_string())
        .ok()
        .map(|loaded| loaded.theme)
}

/// The `unset` entries of `doc`, as written.
fn unset_entries(doc: &DocumentMut) -> Vec<String> {
    doc.get("unset")
        .and_then(Item::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Remove the `unset` entry for exactly `path`, and `unset` itself when that
/// was its last entry.
fn drop_unset_entry(doc: &mut DocumentMut, path: [&str; 3]) {
    let Some(entries) = doc.get_mut("unset").and_then(Item::as_array_mut) else {
        return;
    };
    let before = entries.len();
    entries.retain(|entry| {
        !entry
            .as_str()
            .and_then(split_path)
            .is_some_and(|parts| parts.iter().map(String::as_str).eq(path))
    });
    if entries.is_empty() && entries.len() < before {
        doc.remove("unset");
    }
}

/// Set `path` in `doc` to `value`, adding the tables it needs. An existing
/// key keeps its comments and spacing.
///
/// # Errors
/// Returns an error when a table on the way is some other value.
fn set_key(doc: &mut DocumentMut, path: [&str; 3], value: &KeyValue) -> Result<()> {
    let [section, segment, field] = path;
    let sections = child_table(doc.as_table_mut(), section, true)?;
    let table = child_table(sections, segment, false)?;
    if !table.contains_key(field) {
        table.insert(field, value.item());
        return Ok(());
    }
    let Some(item) = table.get_mut(field) else {
        return Ok(());
    };
    let old_decor = item.as_value().map(|old| old.decor().clone());
    *item = value.item();
    if let (Some(decor), Some(new)) = (old_decor, item.as_value_mut()) {
        *new.decor_mut() = decor;
    }
    Ok(())
}

/// The table `key` of `parent`, added (implicit when `implicit`) if missing.
///
/// # Errors
/// Returns an error when `key` holds a value other than a table.
fn child_table<'a>(
    parent: &'a mut dyn TableLike,
    key: &str,
    implicit: bool,
) -> Result<&'a mut dyn TableLike> {
    if !parent.contains_key(key) {
        let mut table = toml_edit::Table::new();
        table.set_implicit(implicit);
        parent.insert(key, Item::Table(table));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            Error::config(format!(
                "`{key}` in the theme file is not a table, so the wizard cannot save its edits there"
            ))
        })
}

/// Remove `path` from `doc`, and each table on the way it leaves empty.
fn remove_key(doc: &mut DocumentMut, path: [&str; 3]) {
    let [section, segment, field] = path;
    let Some(sections) = doc.get_mut(section).and_then(Item::as_table_like_mut) else {
        return;
    };
    let Some(table) = sections.get_mut(segment).and_then(Item::as_table_like_mut) else {
        return;
    };
    table.remove(field);
    if table.is_empty() {
        sections.remove(segment);
    }
    if sections.is_empty() {
        doc.remove(section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::wizard::state::{SegmentEdit, WizardState};
    use crate::config::Config;
    use crate::config::defaults::STARSHIP_THEME_CONTENT;
    use crate::config::discovery::Source;

    fn starship() -> ThemeConfig {
        crate::theme::parse(STARSHIP_THEME_CONTENT, "starship").expect("starship parses")
    }

    fn state() -> WizardState {
        WizardState::from_parts(
            Config::default(),
            vec!["starship".to_owned()],
            vec!["default".to_owned()],
            vec!["clock".to_owned(), "duration".to_owned(), "git".to_owned()],
        )
    }

    /// The user file `starship` before it exists: a self-named overlay
    /// resolves to the embedded builtin.
    fn new_starship_link() -> ChainLink {
        ChainLink {
            name: "starship".to_owned(),
            source: Source::User,
            path: user_copy::user_copy_path("starship"),
            on_disk: false,
        }
    }

    fn doc(text: &str) -> DocumentMut {
        text.parse().expect("test doc parses")
    }

    #[test]
    fn an_unsafe_theme_name_is_rejected_before_any_write() {
        let base = starship();
        let mut state = state();
        state.apply_edit(SegmentEdit::ToggleClockShowSeconds, &base);
        let edits = state.theme_edits().expect("edits pending");

        let result = save_theme("../escape", edits);

        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn a_key_toggled_twice_writes_nothing() {
        let base = starship();
        let mut state = state();
        state.apply_edit(SegmentEdit::ToggleClockShowSeconds, &base);
        state.apply_edit(SegmentEdit::ToggleClockShowSeconds, &base);
        let edits = state.theme_edits().expect("edits pending");
        assert_eq!(
            edits.edited.segments.clock.show_seconds,
            Some(false),
            "the field is set where the builtin leaves it unset"
        );

        assert!(changed_keys(edits).is_empty());
        let written = save_theme("starship", edits).expect("nothing to write is no error");
        assert!(written.is_none(), "{written:?}");
    }

    #[test]
    fn a_wizard_overlay_of_builtin_starship_is_a_few_lines_and_loads() {
        let base = starship();
        let mut state = state();
        state.apply_edit(SegmentEdit::ToggleClockShowSeconds, &base);
        state.apply_edit(SegmentEdit::ToggleClockTimeFormat, &base);
        let edits = state.theme_edits().expect("edits pending");

        let text =
            new_overlay("starship", &changed_keys(edits), &edits.edited).expect("overlay text");

        assert_eq!(
            text,
            "extends = \"starship\"\n\n[segments.clock]\ntime_format = \"24\"\nshow_seconds = true\n"
        );
        let loaded = overlay::load_text(new_starship_link(), text).expect("overlay loads");
        assert_eq!(loaded.theme.segments.clock.time_format(), "24");
        assert!(loaded.theme.segments.clock.show_seconds());
        assert_eq!(
            loaded.chain.iter().nth(1).map(ToString::to_string),
            Some("starship (builtin)".to_owned())
        );
        let mut expected = base;
        expected.segments.clock.time_format = Some("24".to_owned());
        expected.segments.clock.show_seconds = Some(true);
        assert_eq!(
            crate::theme::normalised_table(&loaded.theme).expect("normalise loaded"),
            crate::theme::normalised_table(&expected).expect("normalise expected")
        );
    }

    #[test]
    fn toggling_an_unset_key_back_removes_the_unset_entry() {
        let text = "# mine\nextends = \"starship\"\nunset = [\"segments.duration.show_milliseconds\"]\n\n\
                    [segments.clock]\n# 24-hour\ntime_format = \"24\"\n";
        let link = new_starship_link();
        let seed = overlay::load_text(link.clone(), text.to_owned())
            .expect("overlay loads")
            .theme;
        assert!(
            seed.segments.duration.show_milliseconds,
            "the unset key falls back to its default"
        );
        let mut edits = ThemeEdits {
            edited: seed.clone(),
            seed,
        };
        edits.edited.segments.duration.show_milliseconds = false;
        let changed = changed_keys(&edits);

        let mut overlay_doc = doc(text);
        edit_overlay(&mut overlay_doc, &changed, &edits.edited, |unset| {
            parent_after(&link, "starship", unset)
        })
        .expect("edit applies");

        let written = overlay_doc.to_string();
        assert_eq!(
            written,
            "# mine\nextends = \"starship\"\n\n[segments.clock]\n# 24-hour\ntime_format = \"24\"\n"
        );
        let loaded = overlay::load_text(link, written).expect("edited overlay loads");
        assert!(!loaded.theme.segments.duration.show_milliseconds);
    }

    #[test]
    fn a_key_toggled_back_to_the_parent_value_leaves_the_overlay() {
        let text = "extends = \"starship\"\n\n[segments.clock]\nshow_seconds = true\n";
        let link = new_starship_link();
        let seed = overlay::load_text(link.clone(), text.to_owned())
            .expect("overlay loads")
            .theme;
        let mut edits = ThemeEdits {
            edited: seed.clone(),
            seed,
        };
        edits.edited.segments.clock.show_seconds = Some(false);

        let mut overlay_doc = doc(text);
        edit_overlay(
            &mut overlay_doc,
            &changed_keys(&edits),
            &edits.edited,
            |unset| parent_after(&link, "starship", unset),
        )
        .expect("edit applies");

        assert_eq!(overlay_doc.to_string(), "extends = \"starship\"\n");
    }

    #[test]
    fn a_full_copy_is_edited_in_place() {
        let mut copy = doc("# my theme\n[segments.git]\n# keep\nshow_branch = true\n");
        let mut edited = ThemeConfig::default();
        edited.segments.git.show_branch = Some(false);
        edited.segments.git.show_stash = Some(false);
        let changed: Vec<&WizardKey> = WIZARD_KEYS
            .iter()
            .filter(|key| {
                key.path.ends_with(&["show_branch"]) || key.path.ends_with(&["show_stash"])
            })
            .collect();

        set_keys(&mut copy, &changed, &edited).expect("keys set");

        assert_eq!(
            copy.to_string(),
            "# my theme\n[segments.git]\n# keep\nshow_branch = false\nshow_stash = false\n"
        );
    }
}
