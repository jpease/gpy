//! Theme TOML parsing and validation.

use super::ThemeConfig;
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

    let theme: ThemeConfig = match value.try_into() {
        Ok(theme) => theme,
        Err(_) => {
            // Re-parse the string: `toml::Value` carries no source spans.
            toml::from_str(contents).map_err(|e| {
                Error::config(format!(
                    "Invalid theme schema or value in {source}\n  Diagnostic: {e}\n  Remediation: Check the key/value at the reported location. Ensure colors are valid (hex, named, or 'transparent') and icons contain no control characters."
                ))
            })?
        }
    };

    validation::validate_theme_config(&theme, source)?;

    Ok(theme)
}
