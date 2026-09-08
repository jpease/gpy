//! Theme TOML parsing and validation.
//!
//! Lives in `theme/` (not `config/loader.rs`) so theme parsing is owned by
//! the module that owns the theme model (#608). `config::loader` still does
//! the file/embedded-fallback I/O (`load_theme_from_path`), but hands the
//! resulting content here to parse.

use super::ThemeConfig;
use crate::config::validation;
use crate::{Error, Result};

/// Parses and validates theme TOML `contents` into a [`ThemeConfig`].
///
/// `source` is a human-readable label (a path or theme name) used only in
/// diagnostics. One TOML tokenization of `contents` happens in the success
/// path (`toml::from_str::<Value>`, followed by a from-`Value` typed
/// conversion, not a second string re-parse); a second, string-based parse
/// runs ONLY if the from-`Value` conversion fails, purely to recover a
/// typed error with source line/column info that a from-`Value` conversion
/// cannot provide (`toml::Value` carries no source spans) — see #608.
///
/// # Errors
///
/// Returns a configuration error if the content fails TOML syntax parsing,
/// is missing the required `[segments]`/`[ui]` tables, fails typed
/// deserialization, or fails schema validation.
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
            // Re-parse from the original string (not the already-parsed
            // Value) purely to get a typed error with source span info —
            // see the doc comment above.
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
