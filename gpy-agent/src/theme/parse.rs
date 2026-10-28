//! Theme TOML parsing and validation, in two steps: [`parse_table`] checks the
//! syntax and [`from_table`] runs the typed checks on a table, which may be a
//! single file's or a merged overlay chain's.

use super::ThemeConfig;
use super::locate::plugin_key_error;
use crate::config::validation::{self, ValidationError};
use crate::{Error, Result};

/// A top-level table every theme needs after any overlay merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredTable {
    /// `[segments]`.
    Segments,
    /// `[ui]`.
    Ui,
}

impl RequiredTable {
    /// The table's key, `segments` or `ui`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Segments => "segments",
            Self::Ui => "ui",
        }
    }
}

/// Why [`from_table`] rejected a theme table. Every variant keeps its typed
/// cause, so overlay error attribution can find the failing key structurally.
#[derive(Debug)]
pub enum FromTableError {
    /// `[segments]` or `[ui]` is absent or not a table.
    Missing(RequiredTable),
    /// Typed deserialization failed. A table has no source text, so the error
    /// has no span.
    Deserialize(toml::de::Error),
    /// The typed theme failed `validate_theme_config`.
    Validation(ValidationError),
}

impl std::fmt::Display for FromTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(table) => write!(f, "missing required [{}] table", table.key()),
            Self::Deserialize(error) => f.write_str(error.message().trim()),
            Self::Validation(error) => std::fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for FromTableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Missing(_) => None,
            Self::Deserialize(error) => Some(error),
            Self::Validation(error) => Some(error),
        }
    }
}

/// Parses and validates theme TOML `contents` into a [`ThemeConfig`]:
/// [`parse_table`] followed by [`from_table`]. `source` labels diagnostics.
/// Re-parses from the string only on failure, to get span info.
///
/// # Errors
/// Returns a config error on bad syntax, missing `[segments]`/`[ui]`, or invalid schema.
pub fn parse(contents: &str, source: &str) -> Result<ThemeConfig> {
    let table = parse_table(contents, source)?;
    from_table(table, source).or_else(|error| full_theme_result(contents, source, error))
}

/// Parses theme TOML `contents` into a table, checking only the syntax.
/// `source` labels the error.
///
/// # Errors
/// Returns a config error naming the line of a TOML syntax error.
pub fn parse_table(contents: &str, source: &str) -> Result<toml::Table> {
    toml::from_str(contents).map_err(|e| {
        Error::config(format!(
            "Failed to parse theme TOML syntax in {source}:\n  {e}\n  Remediation: Fix the TOML syntax error at the reported line."
        ))
    })
}

/// Runs the typed theme checks on `table`: `[segments]` and `[ui]` present,
/// typed deserialization, and `validate_theme_config`. `source` labels the
/// colours a validation error names.
///
/// # Errors
/// Returns the first failing check as a [`FromTableError`].
pub fn from_table(
    table: toml::Table,
    source: &str,
) -> std::result::Result<ThemeConfig, FromTableError> {
    for required in [RequiredTable::Segments, RequiredTable::Ui] {
        if !table.get(required.key()).is_some_and(toml::Value::is_table) {
            return Err(FromTableError::Missing(required));
        }
    }
    let theme: ThemeConfig = toml::Value::Table(table)
        .try_into()
        .map_err(FromTableError::Deserialize)?;
    validation::validate_theme_config(&theme, source).map_err(FromTableError::Validation)?;
    Ok(theme)
}

/// The result of a full theme (one file, no overlay) whose [`from_table`]
/// failed, with the diagnostics [`parse`] has always given: a typed failure is
/// re-parsed from `contents` for its span.
///
/// # Errors
/// Returns the config error describing `error`.
pub(super) fn full_theme_result(
    contents: &str,
    source: &str,
    error: FromTableError,
) -> Result<ThemeConfig> {
    match error {
        FromTableError::Missing(RequiredTable::Segments) => Err(Error::config(format!(
            "Invalid theme in {source}\n  Diagnostic: Missing required [segments] table\n  Remediation: Add a [segments] block to define segment colors."
        ))),
        FromTableError::Missing(RequiredTable::Ui) => Err(Error::config(format!(
            "Invalid theme in {source}\n  Diagnostic: Missing required [ui] table\n  Remediation: Add a [ui] block to define prompt icons and delimiters."
        ))),
        FromTableError::Deserialize(_) => {
            let theme = reparse_for_diagnostic(contents, source)?;
            validation::validate_theme_config(&theme, source)?;
            Ok(theme)
        }
        FromTableError::Validation(invalid) => Err(invalid.into()),
    }
}

/// Re-parses `contents` after `toml::Value` (which has no spans) failed to deserialize.
///
/// Serde also drops spans of flattened plugin tables, so those keys are located separately.
fn reparse_for_diagnostic(contents: &str, source: &str) -> Result<ThemeConfig> {
    let located = toml::from_str::<toml::Value>(contents)
        .ok()
        .and_then(|reparsed| plugin_key_error(contents, &reparsed));
    if let Some(diagnostic) = located {
        return Err(schema_error(source, &diagnostic));
    }
    toml::from_str(contents).map_err(|e| schema_error(source, &e))
}

fn schema_error(source: &str, diagnostic: &dyn std::fmt::Display) -> Error {
    Error::config(format!(
        "Invalid theme schema or value in {source}\n  Diagnostic: {diagnostic}\n  Remediation: Check the key/value at the reported location. Ensure colors are valid (hex, named, or 'transparent') and icons contain no control characters."
    ))
}

#[cfg(test)]
mod tests {
    use super::parse;

    const THEME: &str = r#"[ui]
prompt_icon = "❯"

[segments.clock]
bg_color = "blue"

[segments.directory]
bg_color = "green"

[segments.username]
bg_color = "yellow"
format = "$user"

[segments.hostname]
show_always = true
"#;

    #[test]
    fn unknown_plugin_key_in_later_table_names_key_and_line() {
        let message = parse(THEME, "stale").unwrap_err().to_string();
        assert!(
            message.contains("Invalid theme schema or value in stale"),
            "{message}"
        );
        assert!(
            message.contains("segments.username.format (line 12, column 1): unknown plugin segment style key \"format\""),
            "{message}"
        );
        assert!(!message.contains("[segments.clock]"), "{message}");
        assert!(
            message.contains("Remediation: Check the key/value"),
            "{message}"
        );
    }

    #[test]
    fn invalid_plugin_value_after_valid_plugin_table_reports_its_own_key() {
        let theme = "[ui]\nprompt_icon = \"x\"\n\n[segments.aws]\nbg_color = \"blue\"\n\n[segments.hostname]\nicon = \"h\"\n  text_color = \"not-a-colour\"\n";
        let message = parse(theme, "t").unwrap_err().to_string();
        assert!(
            message.contains("segments.hostname.text_color (line 9, column 3)"),
            "{message}"
        );
    }

    #[test]
    fn invalid_builtin_value_keeps_serde_location() {
        let theme = "[ui]\nprompt_icon = \"x\"\n\n[segments.username]\nbg_color = \"blue\"\n\n[segments.clock]\nbg_color = \"not-a-colour\"\n";
        let message = parse(theme, "t").unwrap_err().to_string();
        assert!(message.contains("line 8, column 12"), "{message}");
    }
}
