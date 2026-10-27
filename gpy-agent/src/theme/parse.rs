//! Theme TOML parsing and validation.

use super::ThemeConfig;
use super::locate::plugin_key_error;
use crate::config::validation;
use crate::{Error, Result};

/// Parses and validates theme TOML `contents` into a [`ThemeConfig`].
/// `source` labels diagnostics. Re-parses from the string only on failure, to get span info.
///
/// # Errors
/// Returns a config error on bad syntax, missing `[segments]`/`[ui]`, or invalid schema.
pub fn parse(contents: &str, source: &str) -> Result<ThemeConfig> {
    let value: toml::Value = toml::from_str(contents).map_err(|e| {
        Error::config(format!(
            "Failed to parse theme TOML syntax in {source}:\n  {e}\n  Remediation: Fix the TOML syntax error at the reported line."
        ))
    })?;

    if !value.get("segments").is_some_and(toml::Value::is_table) {
        return Err(Error::config(format!(
            "Invalid theme in {source}\n  Diagnostic: Missing required [segments] table\n  Remediation: Add a [segments] block to define segment colors."
        )));
    }
    if !value.get("ui").is_some_and(toml::Value::is_table) {
        return Err(Error::config(format!(
            "Invalid theme in {source}\n  Diagnostic: Missing required [ui] table\n  Remediation: Add a [ui] block to define prompt icons and delimiters."
        )));
    }

    let theme = match value.try_into() {
        Ok(theme) => theme,
        Err(_) => reparse_for_diagnostic(contents, source)?,
    };

    validation::validate_theme_config(&theme, source)?;

    Ok(theme)
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
