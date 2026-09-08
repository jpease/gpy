//! Metadata and parsers for dot-notation configuration keys.
//!
//! The CLI uses this registry to implement `config list`, `config get`, and
//! `config set` without hard-coding key behavior in command handlers. Each
//! [`crate::config::metadata::ConfigKeyMeta`] entry ties a public key name to typed getters, optional
//! setters, and help text while delegating invariant checks to config newtypes.

use crate::config::validation::{ValidationError, ValidationErrorKind};
use crate::config::{Config, types};
use crate::{Error, Result};

/// Build an `Error::Config` from a structured [`ValidationError`] with kind
/// [`ValidationErrorKind::InvalidValue`].
///
/// `Display` is `"Invalid {expected}: {value}"`, matching every pre-#628
/// `Error::config(format!("Invalid ...: {v}"))` call this file used to build
/// by hand.
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

/// Centralized definition of all configuration keys
///
/// ## Adding a New Config Key
///
/// To add a new configuration key, add a new entry to this array:
///
/// ```text
/// ConfigKeyMeta {
///     key: "section.field_name",           // Dot notation matching Config struct
///     get: |c| c.section.field_name.to_string(),  // Getter closure
///     set: Some(|c, v| {                   // Setter closure (or None for read-only)
///         c.section.field_name = parse_type(v)?;
///         Ok(())
///     }),
///     description: "Human-readable description for help text",
/// }
/// ```
///
/// The key will automatically be available in:
/// - `gpy-agent config get <key>`
/// - `gpy-agent config set <key> <value>`
/// - `gpy config get <key>` (in the main binary)
///
/// ## Read-Only Keys
///
/// For read-only keys (computed values, arrays that need special parsing), use `set: None`
/// or return an error with a helpful message.
///
/// ## Type Parsing
///
/// Use the helper functions at the bottom of this file, passing this entry's
/// own `key` as `field_path` so a parse failure's structured [`crate::config::validation::ValidationError`]
/// names the right field:
/// - `parse_bool_field()` - for boolean values (accepts: true/false, 1/0, yes/no, on/off)
/// - `parse_u64_field()` - for unsigned 64-bit integers
/// - `parse_u32_field()` - for unsigned 32-bit integers
/// - `parse_usize_field()` - for size values
///
/// For string values, use `v.to_string()` directly.
pub const CONFIG_KEYS: &[ConfigKeyMeta] = &[
    // Agent Settings
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
                types::AgentTimeout::new(parse_u64_field(v, "agent.timeout_seconds")?)
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
                parse_u64_field(v, "agent.supervisor.check_interval_seconds")?,
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
                parse_u32_field(v, "agent.supervisor.max_restart_attempts")?,
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
    // Git Settings
    ConfigKeyMeta {
        key: "git.enabled",
        get: |c| c.git.enabled.to_string(),
        set: Some(|c, v| {
            c.git.enabled = parse_bool_field(v, "git.enabled")?;
            Ok(())
        }),
        description: "Enable git status detection",
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
            c.git.timeout_seconds =
                types::GitTimeout::new(parse_u64_field(v, "git.timeout_seconds")?)
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
                types::MaxBranchLength::new(parse_usize_field(v, "git.max_branch_length")?)
                    .ok_or_else(|| {
                        invalid_value("git.max_branch_length", "max branch length", v)
                    })?;
            Ok(())
        }),
        description: "Maximum branch name length to display",
    },
    // Language Settings
    ConfigKeyMeta {
        key: "language.enabled",
        get: |c| c.language.enabled.to_string(),
        set: Some(|c, v| {
            c.language.enabled = parse_bool_field(v, "language.enabled")?;
            Ok(())
        }),
        description: "Enable language detection",
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
        key: "language.cache_ttl_hours",
        get: |c| c.language.cache_ttl_hours.to_string(),
        set: Some(|c, v| {
            c.language.cache_ttl_hours =
                types::CacheTtlHours::new(parse_u64_field(v, "language.cache_ttl_hours")?)
                    .ok_or_else(|| invalid_value("language.cache_ttl_hours", "cache TTL", v))?;
            Ok(())
        }),
        description: "Version cache TTL in hours",
    },
    ConfigKeyMeta {
        key: "language.display",
        get: |c| c.language.display.to_string(),
        set: Some(|c, v| {
            // Not converted to `ValidationError`: the message embeds the
            // allowed-values hint ("Use 'icon' or 'text'") in a shape that
            // doesn't fit `InvalidValue`'s generic "Invalid {expected}:
            // {value}" template without either duplicating this one-off
            // wording as a bespoke `Display` arm (defeating the point of a
            // shared, reusable kind) or losing the hint. Single call site,
            // low structural payoff — left as plain `Error::config` text
            // (#628).
            c.language.display = v.parse().map_err(|_| {
                Error::config(format!(
                    "Invalid language.display '{v}'. Use 'icon' or 'text'"
                ))
            })?;
            Ok(())
        }),
        description: "Display mode: 'icon' or 'text'",
    },
    ConfigKeyMeta {
        key: "language.filter",
        get: |c| c.language.filter.to_string(),
        set: Some(|c, v| {
            // Not converted to `ValidationError`: the message is fully
            // composed by `LanguageFilter::from_str` (`config/types.rs`), not
            // by this call site — wrapping it in a structured shape here
            // would mean either re-deriving that wording as a bespoke
            // `Display` arm (duplicating logic that already lives in
            // `types.rs`) or losing it. Left as plain `Error::config` text
            // (#628).
            c.language.filter = v.parse().map_err(Error::config)?;
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
                .map_err(|_| invalid_value("language.confidence_threshold", "threshold", v))?;

            c.language.confidence_threshold = types::ConfidenceThreshold::new(threshold)
                .ok_or_else(|| {
                    invalid_value("language.confidence_threshold", "confidence threshold", v)
                })?;
            Ok(())
        }),
        description: "Minimum confidence (0.0-1.0) to display a language",
    },
    // UI Settings
    ConfigKeyMeta {
        key: "ui.show_icons",
        get: |c| c.ui.show_icons.to_string(),
        set: Some(|c, v| {
            c.ui.show_icons = parse_bool_field(v, "ui.show_icons")?;
            Ok(())
        }),
        description: "Show icons for languages",
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
            c.ui.directory.display = parse_directory_display(v)?;
            Ok(())
        }),
        description: "Directory display mode: 'basename', 'abbreviated', 'truncated', 'full'",
    },
    ConfigKeyMeta {
        key: "ui.directory.truncation_length",
        get: |c| c.ui.directory.truncation_length.to_string(),
        set: Some(|c, v| {
            c.ui.directory.truncation_length = types::DirectoryTruncationLength::new(
                parse_usize_field(v, "ui.directory.truncation_length")?,
            )
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
                types::MaxPathLength::new(parse_usize_field(v, "ui.directory.max_length")?)
                    .ok_or_else(|| {
                        invalid_value("ui.directory.max_length", "max path length", v)
                    })?;
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
        key: "ui.enabled_segments",
        get: |c| c.ui.enabled_segments.join(" "),
        set: None, // Setting requires Vec<String> parsing, not yet implemented
        description: "Enabled segments and their order",
    },
];

/// Get a config value by key
///
/// # Errors
///
/// Returns an error if the config key is unknown.
pub fn get_config_value(config: &Config, key: &str) -> Result<String> {
    for meta in CONFIG_KEYS {
        if meta.key == key {
            return Ok((meta.get)(config));
        }
    }
    Err(ValidationError {
        field_path: key.to_owned(),
        kind: ValidationErrorKind::UnknownKey,
    }
    .into())
}

/// Set a config value by key
///
/// # Errors
///
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
    Err(ValidationError {
        field_path: key.to_owned(),
        kind: ValidationErrorKind::UnknownKey,
    }
    .into())
}

/// Check if a config key is valid
#[must_use]
pub fn validate_config_key(key: &str) -> bool {
    CONFIG_KEYS.iter().any(|m| m.key == key)
}

// Helper functions
//
// The four generic parsers below (`parse_bool_field`/`parse_u64_field`/
// `parse_u32_field`/`parse_usize_field`) don't know their own key -- unlike
// the `ok_or_else` sites above (each customizes its own message inline), a
// generic parser is shared across many `ConfigKeyMeta` entries. #628 threads
// the key through as a `field_path` parameter instead, since every call site
// already has its own key literal at hand (each closure is written once per
// `ConfigKeyMeta` entry); `Display` text is unchanged (`"Invalid {expected}:
// {value}"` reproduces the old `"Invalid boolean/u64/u32/usize value: {s}"}`
// wording byte-for-byte), so this is pure structure, not a wording change.

///
/// # Errors
///
/// Returns an error if the value is not a valid boolean.
fn parse_bool_field(s: &str, field_path: &'static str) -> Result<bool> {
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(invalid_value(field_path, "boolean value", s)),
    }
}

///
/// # Errors
///
/// Returns an error if the value is not a valid u64.
fn parse_u64_field(s: &str, field_path: &'static str) -> Result<u64> {
    s.parse()
        .map_err(|_| invalid_value(field_path, "u64 value", s))
}

///
/// # Errors
///
/// Returns an error if the value is not a valid u32.
fn parse_u32_field(s: &str, field_path: &'static str) -> Result<u32> {
    s.parse()
        .map_err(|_| invalid_value(field_path, "u32 value", s))
}

///
/// # Errors
///
/// Returns an error if the value is not a valid usize.
fn parse_usize_field(s: &str, field_path: &'static str) -> Result<usize> {
    s.parse()
        .map_err(|_| invalid_value(field_path, "usize value", s))
}

///
/// # Errors
///
/// Returns an error if the value is not a valid directory display mode.
fn parse_directory_display(s: &str) -> Result<types::DirectoryDisplay> {
    s.parse()
        .map_err(|e: <types::DirectoryDisplay as std::str::FromStr>::Err| {
            invalid_value(
                "ui.directory.display",
                "directory display mode",
                e.to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{get_config_value, set_config_value};
    use crate::Error;
    use crate::config::Config;
    use crate::config::validation::ValidationError;

    /// `gpy config set agent.timeout_seconds abc` must still yield the exact
    /// pre-#628 message.
    ///
    /// Going through the public `set_config_value` entry point exactly as
    /// the CLI does. Before the structured type existed there was nothing to
    /// downcast `source()` to; this is that "no structure yet" case made
    /// concrete through the public API, not just a direct `ValidationError`
    /// construction.
    #[test]
    fn set_config_value_preserves_exact_pre_628_messages() {
        let cases: &[(&str, &str, &str)] = &[
            ("agent.timeout_seconds", "abc", "Invalid u64 value: abc"),
            ("agent.timeout_seconds", "0", "Invalid agent timeout: 0"),
            ("agent.enabled", "maybe", "Invalid boolean value: maybe"),
            (
                "agent.supervisor.max_restart_attempts",
                "-1",
                "Invalid u32 value: -1",
            ),
            ("git.max_branch_length", "abc", "Invalid usize value: abc"),
            ("ui.theme", "", "Invalid theme name: "),
            (
                "ui.directory.display",
                "sideways",
                "Invalid directory display mode: unrecognized directory display mode: sideways",
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
                "Config key ui.enabled_segments is read-only",
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

    /// A converted site's `Error::Config` must downcast to `ValidationError`.
    ///
    /// `source()` must expose the offending `field_path`, reached through
    /// the same public `set_config_value` entry point `gpy config set`
    /// uses -- not just a direct `ValidationError` construction.
    #[test]
    fn set_config_value_error_source_downcasts_with_field_path() {
        let mut config = Config::default();
        let err = set_config_value(&mut config, "agent.timeout_seconds", "abc")
            .expect_err("abc is not a valid u64");

        let Error::Config { source, .. } = &err else {
            panic!("expected Error::Config, got {err:?}");
        };
        let source_ref = source.as_ref().expect("source must be Some after #628");
        let validation_error = source_ref
            .downcast_ref::<ValidationError>()
            .expect("source must downcast to ValidationError");
        assert_eq!(validation_error.field_path, "agent.timeout_seconds");
    }
}
