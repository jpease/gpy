//! Registry and parsers for dot-notation config keys (`config list/get/set`).

use crate::config::validation::{ValidationError, ValidationErrorKind};
use crate::config::{Config, GitIconKind, types};
use crate::{Error, Result};
use std::str::FromStr;

/// Build an `Error::Config` from a [`ValidationErrorKind::InvalidValue`] error.
fn invalid_value(field_path: &str, expected: &'static str, value: impl Into<String>) -> Error {
    ValidationError {
        field_path: field_path.to_owned(),
        kind: ValidationErrorKind::InvalidValue {
            expected,
            value: value.into(),
        },
    }
    .into()
}

/// Type alias for the set function in `ConfigKeyMeta` to reduce type complexity.
type SetFn = fn(&mut Config, &str) -> Result<()>;

/// Metadata for a configuration key
pub struct ConfigKeyMeta {
    /// The dot-notation key (e.g. "agent.enabled")
    pub key: &'static str,
    /// Function to retrieve the value as a string
    pub get: fn(&Config) -> String,
    /// Function to set the value from a string (optional, some keys might be read-only)
    pub set: Option<SetFn>,
    /// Description for help text
    pub description: &'static str,
}

/// One `git.icons.<name>` entry: `get` is the effective glyph for the current
/// `git.icon_set`, `set` stores an explicit override.
macro_rules! git_icon_key {
    ($field:ident, $kind:ident, $description:expr) => {
        ConfigKeyMeta {
            key: concat!("git.icons.", stringify!($field)),
            get: |c| {
                c.git
                    .icons
                    .glyph(GitIconKind::$kind, c.git.icon_set)
                    .to_owned()
            },
            set: Some(|c, v| {
                c.git.icons.$field = Some(types::Icon::new(v).ok().ok_or_else(|| {
                    invalid_value(concat!("git.icons.", stringify!($field)), "icon", v)
                })?);
                Ok(())
            }),
            description: $description,
        }
    };
}

/// Centralized definition of all configuration keys
///
/// New entries are automatically exposed via `gpy config get`/`set`; use
/// `set: None` for read-only keys and pass the entry's `key` as `field_path`
/// to the `parse_*_field` helpers.
pub const CONFIG_KEYS: &[ConfigKeyMeta] = &[
    ConfigKeyMeta {
        key: "agent.enabled",
        get: |c| c.agent.enabled.to_string(),
        set: Some(|c, v| {
            c.agent.enabled = parse_bool_field(v, "agent.enabled")?;
            Ok(())
        }),
        description: "Enable/disable the background agent",
    },
    ConfigKeyMeta {
        key: "agent.timeout_seconds",
        get: |c| c.agent.timeout_seconds.to_string(),
        set: Some(|c, v| {
            c.agent.timeout_seconds =
                types::AgentTimeout::new(parse_field(v, "agent.timeout_seconds")?)
                    .ok_or_else(|| invalid_value("agent.timeout_seconds", "agent timeout", v))?;
            Ok(())
        }),
        description: "Socket timeout in seconds",
    },
    ConfigKeyMeta {
        key: "agent.live_updates",
        get: |c| c.agent.live_updates.to_string(),
        set: Some(|c, v| {
            c.agent.live_updates = parse_bool_field(v, "agent.live_updates")?;
            Ok(())
        }),
        description: "Enable live updates via signals",
    },
    ConfigKeyMeta {
        key: "agent.supervisor.enabled",
        get: |c| c.agent.supervisor.enabled.to_string(),
        set: Some(|c, v| {
            c.agent.supervisor.enabled = parse_bool_field(v, "agent.supervisor.enabled")?;
            Ok(())
        }),
        description: "Enable supervisor for auto-restart",
    },
    ConfigKeyMeta {
        key: "agent.supervisor.check_interval_seconds",
        get: |c| c.agent.supervisor.check_interval_seconds.to_string(),
        set: Some(|c, v| {
            c.agent.supervisor.check_interval_seconds = types::SupervisorCheckInterval::new(
                parse_field(v, "agent.supervisor.check_interval_seconds")?,
            )
            .ok_or_else(|| {
                invalid_value(
                    "agent.supervisor.check_interval_seconds",
                    "supervisor check interval",
                    v,
                )
            })?;
            Ok(())
        }),
        description: "Supervisor check interval in seconds",
    },
    ConfigKeyMeta {
        key: "agent.supervisor.max_restart_attempts",
        get: |c| c.agent.supervisor.max_restart_attempts.to_string(),
        set: Some(|c, v| {
            c.agent.supervisor.max_restart_attempts = types::SupervisorMaxRestartAttempts::new(
                parse_field(v, "agent.supervisor.max_restart_attempts")?,
            )
            .ok_or_else(|| {
                invalid_value(
                    "agent.supervisor.max_restart_attempts",
                    "supervisor max restart attempts",
                    v,
                )
            })?;
            Ok(())
        }),
        description: "Maximum restart attempts before giving up",
    },
    ConfigKeyMeta {
        key: "git.enabled",
        get: |c| c.git.enabled.to_string(),
        set: Some(|c, v| {
            c.git.enabled = parse_bool_field(v, "git.enabled")?;
            Ok(())
        }),
        description: "Deprecated: use `gpy disable git` / `gpy enable git` (ui.enabled_segments)",
    },
    ConfigKeyMeta {
        key: "git.show_upstream",
        get: |c| c.git.show_upstream.to_string(),
        set: Some(|c, v| {
            c.git.show_upstream = parse_bool_field(v, "git.show_upstream")?;
            Ok(())
        }),
        description: "Show ahead/behind counts",
    },
    ConfigKeyMeta {
        key: "git.watch_worktree",
        get: |c| c.git.watch_worktree.to_string(),
        set: Some(|c, v| {
            c.git.watch_worktree = parse_bool_field(v, "git.watch_worktree")?;
            Ok(())
        }),
        description: "Watch the working tree for instant updates on file edits",
    },
    ConfigKeyMeta {
        key: "git.timeout_seconds",
        get: |c| c.git.timeout_seconds.to_string(),
        set: Some(|c, v| {
            c.git.timeout_seconds = types::GitTimeout::new(parse_field(v, "git.timeout_seconds")?)
                .ok_or_else(|| invalid_value("git.timeout_seconds", "git timeout", v))?;
            Ok(())
        }),
        description: "Git command timeout in seconds",
    },
    ConfigKeyMeta {
        key: "git.max_branch_length",
        get: |c| c.git.max_branch_length.to_string(),
        set: Some(|c, v| {
            c.git.max_branch_length =
                types::MaxBranchLength::new(parse_field(v, "git.max_branch_length")?).ok_or_else(
                    || invalid_value("git.max_branch_length", "max branch length", v),
                )?;
            Ok(())
        }),
        description: "Maximum branch name length to display",
    },
    ConfigKeyMeta {
        key: "git.skip_paths",
        get: |c| c.git.skip_paths.join(" "),
        set: None, // List-valued; read-only like ui.enabled_segments
        description: "Paths where git detection is skipped (space-separated in `config get`)",
    },
    ConfigKeyMeta {
        key: "git.max_ahead_behind",
        get: |c| c.git.max_ahead_behind.to_string(),
        set: Some(|c, v| {
            c.git.max_ahead_behind = parse_field(v, "git.max_ahead_behind")?;
            Ok(())
        }),
        description: "Largest ahead/behind count displayed; larger counts show as N+ (display only: git still counts them all; 0 = no cap)",
    },
    ConfigKeyMeta {
        key: "git.stash_enabled",
        get: |c| c.git.stash_enabled.to_string(),
        set: Some(|c, v| {
            c.git.stash_enabled = parse_bool_field(v, "git.stash_enabled")?;
            Ok(())
        }),
        description: "Enable the git stash count capture",
    },
    ConfigKeyMeta {
        key: "git.icon_set",
        get: |c| c.git.icon_set.to_string(),
        set: Some(|c, v| {
            c.git.icon_set = v.parse()?;
            Ok(())
        }),
        description: "Glyph set for stash/detached/in-progress icons: 'unicode' or 'nerd_font'",
    },
    git_icon_key!(ahead, Ahead, "Ahead indicator"),
    git_icon_key!(behind, Behind, "Behind indicator"),
    git_icon_key!(staged, Staged, "Staged files indicator"),
    git_icon_key!(unstaged, Unstaged, "Unstaged files indicator"),
    git_icon_key!(untracked, Untracked, "Untracked files indicator"),
    git_icon_key!(conflicts, Conflicts, "Conflicts indicator"),
    git_icon_key!(stash, Stash, "Stash indicator"),
    git_icon_key!(detached, Detached, "Detached-HEAD indicator"),
    git_icon_key!(in_progress, InProgress, "In-progress operation indicator"),
    ConfigKeyMeta {
        key: "language.enabled",
        get: |c| c.language.enabled.to_string(),
        set: Some(|c, v| {
            c.language.enabled = parse_bool_field(v, "language.enabled")?;
            Ok(())
        }),
        description: "Deprecated: use `gpy disable language` / `gpy enable language` (ui.enabled_segments)",
    },
    ConfigKeyMeta {
        key: "language.show_versions",
        get: |c| c.language.show_versions.to_string(),
        set: Some(|c, v| {
            c.language.show_versions = parse_bool_field(v, "language.show_versions")?;
            Ok(())
        }),
        description: "Enable version detection",
    },
    ConfigKeyMeta {
        key: "language.enabled_languages",
        get: |c| c.language.enabled_languages.join(" "),
        set: None, // List-valued; read-only like ui.enabled_segments
        description: "Languages to detect (empty = all; space-separated in `config get`)",
    },
    ConfigKeyMeta {
        key: "language.detection_mode",
        get: |c| c.language.detection_mode.to_string(),
        set: Some(|c, v| {
            c.language.detection_mode = v.parse()?;
            Ok(())
        }),
        description: "Detection strategy: 'content', 'markers', or 'hybrid'",
    },
    ConfigKeyMeta {
        key: "language.cache_ttl_hours",
        get: |c| c.language.cache_ttl_hours.to_string(),
        set: Some(|c, v| {
            c.language.cache_ttl_hours =
                types::CacheTtlHours::new(parse_field(v, "language.cache_ttl_hours")?)
                    .ok_or_else(|| invalid_value("language.cache_ttl_hours", "cache TTL", v))?;
            Ok(())
        }),
        description: "Language version, mise and virtualenv lookup cache TTL in hours",
    },
    ConfigKeyMeta {
        key: "language.display",
        get: |c| c.language.display.to_string(),
        set: Some(|c, v| {
            c.language.display = v.parse()?;
            Ok(())
        }),
        description: "Display mode: 'icon' or 'text'",
    },
    ConfigKeyMeta {
        key: "language.filter",
        get: |c| c.language.filter.to_string(),
        set: Some(|c, v| {
            c.language.filter = v.parse()?;
            Ok(())
        }),
        description: "Language filter: 'all', 'primary', or top N (e.g. '3')",
    },
    ConfigKeyMeta {
        key: "language.confidence_threshold",
        get: |c| c.language.confidence_threshold.to_string(),
        set: Some(|c, v| {
            let threshold = v
                .parse::<f32>()
                .ok()
                .ok_or_else(|| invalid_value("language.confidence_threshold", "threshold", v))?;

            c.language.confidence_threshold = types::ConfidenceThreshold::new(threshold)
                .ok_or_else(|| {
                    invalid_value("language.confidence_threshold", "confidence threshold", v)
                })?;
            Ok(())
        }),
        description: "Minimum confidence (0.0-1.0) to display a language",
    },
    ConfigKeyMeta {
        key: "ui.show_icons",
        get: |c| c.ui.show_icons.to_string(),
        set: Some(|c, v| {
            c.ui.show_icons = parse_bool_field(v, "ui.show_icons")?;
            Ok(())
        }),
        description: "Show glyphs: language icons, git icons and powerline separators",
    },
    ConfigKeyMeta {
        key: "ui.transient_prompt",
        get: |c| c.ui.transient_prompt.to_string(),
        set: Some(|c, v| {
            c.ui.transient_prompt = parse_bool_field(v, "ui.transient_prompt")?;
            Ok(())
        }),
        description: "Collapse submitted prompts to clock, prompt icon and command",
    },
    ConfigKeyMeta {
        key: "ui.retire_unmodified_themes",
        get: |c| c.ui.retire_unmodified_themes.to_string(),
        set: Some(|c, v| {
            c.ui.retire_unmodified_themes = parse_bool_field(v, "ui.retire_unmodified_themes")?;
            Ok(())
        }),
        description: "Let gpy setup retire user copies of builtin themes that have no edits",
    },
    ConfigKeyMeta {
        key: "ui.theme",
        get: |c| c.ui.theme.to_string(),
        set: Some(|c, v| {
            c.ui.theme = types::ThemeName::new(v.to_owned())
                .ok_or_else(|| invalid_value("ui.theme", "theme name", v))?;
            Ok(())
        }),
        description: "Color theme",
    },
    ConfigKeyMeta {
        key: "ui.directory.display",
        get: |c| c.ui.directory.display.to_string(),
        set: Some(|c, v| {
            c.ui.directory.display = v.parse()?;
            Ok(())
        }),
        description: "Directory display mode: 'basename', 'abbreviated', 'truncated', 'full'",
    },
    ConfigKeyMeta {
        key: "ui.directory.truncation_length",
        get: |c| c.ui.directory.truncation_length.to_string(),
        set: Some(|c, v| {
            c.ui.directory.truncation_length = types::DirectoryTruncationLength::new(parse_field(
                v,
                "ui.directory.truncation_length",
            )?)
            .ok_or_else(|| {
                invalid_value("ui.directory.truncation_length", "truncation length", v)
            })?;
            Ok(())
        }),
        description: "Trailing path components kept when display = 'truncated'",
    },
    ConfigKeyMeta {
        key: "ui.directory.truncation_symbol",
        get: |c| c.ui.directory.truncation_symbol.to_string(),
        set: Some(|c, v| {
            c.ui.directory.truncation_symbol = types::DirectoryTruncationSymbol::new(v.to_owned())
                .ok_or_else(|| {
                    invalid_value("ui.directory.truncation_symbol", "truncation symbol", v)
                })?;
            Ok(())
        }),
        description: "Prefix shown before a truncated path (e.g. '…/')",
    },
    ConfigKeyMeta {
        key: "ui.directory.max_length",
        get: |c| c.ui.directory.max_length.to_string(),
        set: Some(|c, v| {
            c.ui.directory.max_length =
                types::MaxPathLength::new(parse_field(v, "ui.directory.max_length")?).ok_or_else(
                    || invalid_value("ui.directory.max_length", "max path length", v),
                )?;
            Ok(())
        }),
        description: "Maximum path length to display",
    },
    ConfigKeyMeta {
        key: "ui.directory.truncate_to_repo",
        get: |c| c.ui.directory.truncate_to_repo.to_string(),
        set: Some(|c, v| {
            c.ui.directory.truncate_to_repo = parse_bool_field(v, "ui.directory.truncate_to_repo")?;
            Ok(())
        }),
        description: "Anchor the path at the enclosing git repo root",
    },
    ConfigKeyMeta {
        key: "ui.palette",
        get: |c| c.ui.palette.to_string(),
        set: Some(|c, v| {
            c.ui.palette = types::PaletteName::new(v.to_owned())
                .ok_or_else(|| invalid_value("ui.palette", "palette name", v))?;
            Ok(())
        }),
        description: "Color palette name (named-color set used by templates)",
    },
    ConfigKeyMeta {
        key: "ui.enabled_segments",
        get: |c| c.ui.enabled_segments.join(" "),
        set: None, // List-valued: edited in config.toml or with `gpy enable`/`gpy disable`
        description: "Enabled segments and their order (space-separated in `config get`; see `gpy enable`/`gpy disable`)",
    },
];

/// Prefixes of dynamic (map-backed) keys addressed as `<prefix><name>`.
///
/// Not in [`CONFIG_KEYS`] because the name part is user-chosen.
pub const DYNAMIC_KEY_PREFIXES: &[&str] = &[
    LANGUAGE_ICONS_PREFIX,
    super::plugin_segments::PLUGIN_KEY_PREFIX,
];

/// Prefix of the `[language.icons]` map keys (`language.icons.<lang>`).
pub(crate) const LANGUAGE_ICONS_PREFIX: &str = "language.icons.";

/// What `gpy config list` says about the `language.icons.<lang>` family.
pub(crate) const LANGUAGE_ICONS_DESCRIPTION: &str =
    "Icon for one language, e.g. language.icons.rust";

/// Canonical `[language.icons]` name for a `language.icons.<lang>` key.
///
/// Normalized like the formatter (`rs` == `rust`); empty or dotted names are rejected.
fn language_icon_name(key: &str) -> Option<String> {
    let lang = key.strip_prefix(LANGUAGE_ICONS_PREFIX)?;
    if lang.is_empty() || lang.contains('.') {
        return None;
    }
    Some(
        crate::language::metadata::get_icon_key(lang)
            .unwrap_or(lang)
            .to_owned(),
    )
}

/// Key as serialized in config, with dynamic name parts normalized
/// (`language.icons.rs` → `language.icons.rust`).
#[must_use]
pub fn canonical_config_key(key: &str) -> String {
    language_icon_name(key).map_or_else(
        || key.to_owned(),
        |name| format!("{LANGUAGE_ICONS_PREFIX}{name}"),
    )
}

fn unknown_key(key: &str) -> Error {
    ValidationError {
        field_path: key.to_owned(),
        kind: ValidationErrorKind::UnknownKey,
    }
    .into()
}

/// Get a config value by key
///
/// # Errors
/// Returns an error if the config key is unknown, or names a
/// `language.icons.<lang>` entry that is not set. That lookup reads user
/// configuration only, so it errors even though built-in language icons apply
/// at prompt render time.
pub fn get_config_value(config: &Config, key: &str) -> Result<String> {
    for meta in CONFIG_KEYS {
        if meta.key == key {
            return Ok((meta.get)(config));
        }
    }
    if let Some(name) = language_icon_name(key) {
        return config
            .language
            .icons
            .icons
            .get(&name)
            .map(ToString::to_string)
            .ok_or_else(|| Error::config(format!("Config key {key} is not set")));
    }
    Err(unknown_key(key))
}

/// Set a config value by key
///
/// # Errors
/// Returns an error if the config key is unknown, read-only, or the value is invalid.
pub fn set_config_value(config: &mut Config, key: &str, value: &str) -> Result<()> {
    for meta in CONFIG_KEYS {
        if meta.key == key {
            if let Some(set_fn) = meta.set {
                return set_fn(config, value);
            }
            return Err(ValidationError {
                field_path: key.to_owned(),
                kind: ValidationErrorKind::ReadOnlyKey,
            }
            .into());
        }
    }
    if let Some(name) = language_icon_name(key) {
        let icon = types::Icon::new(value)
            .ok()
            .ok_or_else(|| invalid_value(key, "icon", value))?;
        config.language.icons.icons.insert(name, icon);
        return Ok(());
    }
    Err(unknown_key(key))
}

/// Parse a boolean (`true/false`, `1/0`, `yes/no`, `on/off`).
///
/// # Errors
/// Returns an error if the value is not a valid boolean.
fn parse_bool_field(s: &str, field_path: &'static str) -> Result<bool> {
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(invalid_value(field_path, "boolean value", s)),
    }
}

/// Parse a whole number into any integer type; the type's range is the bound.
///
/// # Errors
/// Returns an error if the value is not a whole number that fits `T`.
fn parse_field<T: FromStr>(s: &str, field_path: &str) -> Result<T> {
    s.parse()
        .ok()
        .ok_or_else(|| invalid_value(field_path, "whole number", s))
}

#[cfg(test)]
mod tests {

    use super::{
        CONFIG_KEYS, DYNAMIC_KEY_PREFIXES, canonical_config_key, get_config_value, set_config_value,
    };
    use crate::Error;
    use crate::config::validation::ValidationError;
    use crate::config::{Config, types};

    /// Exact user-facing `gpy config set` error messages.
    #[test]
    fn set_config_value_error_messages() {
        let cases: &[(&str, &str, &str)] = &[
            ("agent.timeout_seconds", "abc", "Invalid whole number: abc"),
            ("agent.timeout_seconds", "0", "Invalid agent timeout: 0"),
            ("agent.enabled", "maybe", "Invalid boolean value: maybe"),
            (
                "agent.supervisor.max_restart_attempts",
                "-1",
                "Invalid whole number: -1",
            ),
            ("git.max_branch_length", "abc", "Invalid whole number: abc"),
            ("ui.theme", "", "Invalid theme name: "),
            (
                "ui.directory.display",
                "sideways",
                "Invalid ui.directory.display 'sideways'. Use 'basename', 'abbreviated', 'truncated', or 'full'",
            ),
            (
                "language.confidence_threshold",
                "abc",
                "Invalid threshold: abc",
            ),
            (
                "language.confidence_threshold",
                "2.0",
                "Invalid confidence threshold: 2.0",
            ),
            (
                "language.display",
                "sideways",
                "Invalid language.display 'sideways'. Use 'icon' or 'text'",
            ),
            (
                "ui.enabled_segments",
                "clock duration",
                "Config key ui.enabled_segments is read-only; use gpy enable <segment> / gpy disable <segment>",
            ),
            ("not.a.real.key", "1", "Unknown config key: not.a.real.key"),
        ];

        for (key, value, expected_message) in cases {
            let mut config = Config::default();
            let err = set_config_value(&mut config, key, value)
                .expect_err("expected an error for a deliberately invalid value");
            assert_eq!(
                err.to_string(),
                format!("Configuration error: {expected_message}"),
                "key={key} value={value}"
            );
        }
    }

    #[test]
    fn get_config_value_preserves_unknown_key_message() {
        let config = Config::default();
        let err = get_config_value(&config, "not.a.real.key").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Configuration error: Unknown config key: not.a.real.key"
        );
    }

    /// Flatten a serialized value into dotted leaf paths, skipping `language.icons`.
    fn leaf_paths(prefix: &str, value: &toml::Value, out: &mut Vec<String>) {
        match value {
            toml::Value::Table(table) => {
                for (key, inner) in table {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    if path == "language.icons" {
                        continue;
                    }
                    leaf_paths(&path, inner, out);
                }
            }
            _ => out.push(prefix.to_owned()),
        }
    }

    fn default_config_leaves() -> Vec<String> {
        let mut config = Config::default();
        config.git.icons = config.git.icons.resolved(config.git.icon_set);
        let table = toml::Table::try_from(config).expect("serialize default config");
        let mut leaves = Vec::new();
        leaf_paths("", &toml::Value::Table(table), &mut leaves);
        leaves
    }

    /// Every serialized `Config` leaf has a `CONFIG_KEYS` entry.
    #[test]
    fn every_config_leaf_has_registry_entry() {
        let missing: Vec<String> = default_config_leaves()
            .into_iter()
            .filter(|leaf| !CONFIG_KEYS.iter().any(|meta| meta.key == leaf))
            .collect();
        assert!(
            missing.is_empty(),
            "Config leaves without a CONFIG_KEYS entry: {missing:?}"
        );
    }

    /// Every `CONFIG_KEYS` key names a real serialized `Config` leaf.
    #[test]
    fn every_registry_key_is_a_config_leaf() {
        let leaves = default_config_leaves();
        let stale: Vec<&str> = CONFIG_KEYS
            .iter()
            .map(|meta| meta.key)
            .filter(|key| !leaves.iter().any(|leaf| leaf == key))
            .collect();
        assert!(
            stale.is_empty(),
            "CONFIG_KEYS entries that are not Config leaves: {stale:?}"
        );
    }

    /// CLI reference "Common Keys" only names registered keys.
    #[test]
    fn cli_reference_common_keys_are_registered() {
        let doc = include_str!("../../../docs/user/cli-reference.md");
        let section = doc
            .split("**Common Keys:**")
            .nth(1)
            .expect("cli-reference.md has a Common Keys list");
        let keys: Vec<&str> = section
            .lines()
            .skip(1)
            .take_while(|line| line.starts_with("- `"))
            .filter_map(|line| line.strip_prefix("- `")?.split('`').next())
            .collect();
        assert!(!keys.is_empty(), "Common Keys list parsed empty");

        let unknown: Vec<&str> = keys
            .into_iter()
            .filter(|key| match key.split_once('<') {
                // `git.icons.<name>`: a placeholder stands for a family of
                // registered keys or a dynamic prefix.
                Some((stem, _)) => {
                    !CONFIG_KEYS.iter().any(|meta| meta.key.starts_with(stem))
                        && !DYNAMIC_KEY_PREFIXES.contains(&stem)
                }
                None => !CONFIG_KEYS.iter().any(|meta| meta.key == *key),
            })
            .collect();
        assert!(
            unknown.is_empty(),
            "cli-reference.md Common Keys not in CONFIG_KEYS: {unknown:?}"
        );
    }

    /// Every key row in `configuration-reference.md` is a registered key and
    /// every registered key has a row.
    ///
    /// A `## `/`### ` heading of the form ``[section.name]`` (with backticks)
    /// opens a section; any other heading closes it. While a section is open,
    /// a table row whose first cell is a backticked name documents
    /// `section.name`.
    #[test]
    fn reference_doc_matches_registry() {
        let doc = include_str!("../../../docs/user/configuration-reference.md");
        let mut documented: Vec<String> = Vec::new();
        let mut section: Option<&str> = None;
        for line in doc.lines() {
            if line.starts_with('#') {
                let level = line.chars().take_while(|c| *c == '#').count();
                let has_space = line[level..].starts_with(' ');
                if (1..=6).contains(&level) && has_space {
                    section = None;
                    if (2..=3).contains(&level) {
                        section = line[level + 1..]
                            .strip_prefix("`[")
                            .and_then(|rest| rest.split_once("]`"))
                            .filter(|(name, tail)| {
                                tail.is_empty()
                                    && !name.is_empty()
                                    && name
                                        .chars()
                                        .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.')
                            })
                            .map(|(name, _)| name);
                    }
                }
            } else if let (Some(active), Some(row)) = (section, line.strip_prefix("| `"))
                && let Some((key, _)) = row.split_once('`')
                && !key.is_empty()
                && key.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            {
                documented.push(format!("{active}.{key}"));
            }
        }
        assert!(
            !documented.is_empty(),
            "no key rows parsed from the reference"
        );

        let undocumented: Vec<&str> = CONFIG_KEYS
            .iter()
            .map(|meta| meta.key)
            .filter(|key| !documented.iter().any(|d| d == key))
            .collect();
        let unregistered: Vec<&String> = documented
            .iter()
            .filter(|d| !CONFIG_KEYS.iter().any(|meta| meta.key == d.as_str()))
            .collect();
        assert!(
            undocumented.is_empty() && unregistered.is_empty(),
            "configuration-reference.md drift: registered keys without a table row: \
             {undocumented:?}; table rows not in CONFIG_KEYS: {unregistered:?}"
        );
    }

    #[test]
    fn language_icon_key_round_trips() {
        let mut cfg = Config::default();
        set_config_value(&mut cfg, "language.icons.rs", "X").unwrap();
        assert_eq!(
            cfg.language
                .icons
                .icons
                .get("rust")
                .map(types::Icon::as_str),
            Some("X")
        );
        assert_eq!(get_config_value(&cfg, "language.icons.rust").unwrap(), "X");
        assert_eq!(get_config_value(&cfg, "language.icons.rs").unwrap(), "X");
        assert_eq!(
            canonical_config_key("language.icons.RS"),
            "language.icons.rust"
        );
    }

    #[test]
    fn language_icon_get_unset_names_the_key() {
        let cfg = Config::default();
        let err = get_config_value(&cfg, "language.icons.rust").unwrap_err();
        assert!(err.to_string().contains("language.icons.rust"), "{err}");
    }

    #[test]
    fn git_icon_get_reports_effective_glyph_for_icon_set() {
        let mut cfg = Config::default();
        assert_eq!(get_config_value(&cfg, "git.icons.stash").unwrap(), "≡");
        cfg.git.icon_set = crate::config::GitIconSet::NerdFont;
        assert_eq!(
            get_config_value(&cfg, "git.icons.stash").unwrap(),
            "\u{f187}"
        );
        set_config_value(&mut cfg, "git.icons.stash", "≡").unwrap();
        assert_eq!(get_config_value(&cfg, "git.icons.stash").unwrap(), "≡");
    }

    #[test]
    fn new_scalar_keys_round_trip() {
        let mut cfg = Config::default();
        for (key, value) in [
            ("ui.palette", "nord"),
            ("git.max_ahead_behind", "5"),
            ("git.stash_enabled", "false"),
            ("git.icon_set", "nerd_font"),
            ("language.detection_mode", "hybrid"),
            ("git.icons.ahead", "A"),
            ("git.icons.in_progress", "P"),
        ] {
            set_config_value(&mut cfg, key, value).unwrap();
            assert_eq!(get_config_value(&cfg, key).unwrap(), value, "{key}");
        }
        assert!(set_config_value(&mut cfg, "git.icon_set", "bogus").is_err());
        assert!(set_config_value(&mut cfg, "git.skip_paths", "/tmp").is_err());
        assert!(set_config_value(&mut cfg, "language.enabled_languages", "rust").is_err());
        assert_eq!(get_config_value(&cfg, "git.skip_paths").unwrap(), "");
    }

    /// Converted sites' `Error::Config` must expose `ValidationError` via `source()`.
    #[test]
    fn set_config_value_error_source_downcasts_with_field_path() {
        let mut config = Config::default();
        let err = set_config_value(&mut config, "agent.timeout_seconds", "abc")
            .expect_err("abc is not a valid u64");

        let Error::Config { source, .. } = &err else {
            panic!("expected Error::Config, got {err:?}");
        };
        let source_ref = source.as_ref().expect("source must be Some");
        let validation_error = source_ref
            .downcast_ref::<ValidationError>()
            .expect("source must downcast to ValidationError");
        assert_eq!(validation_error.field_path, "agent.timeout_seconds");
    }
}
