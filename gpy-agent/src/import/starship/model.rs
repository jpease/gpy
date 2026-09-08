//! Permissive deserialization of a `starship.toml` document.

use crate::import::starship::{ImportError, Result};
use std::collections::HashMap;

/// A permissively-parsed Starship configuration.
///
/// Only the top-level keys GPY understands are hard-typed; all module tables and
/// any other top-level keys are captured in [`StarshipConfig::modules`] so they
/// survive for translation or warning.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct StarshipConfig {
    /// Top-level prompt format string (`$module` ordering).
    #[serde(default)]
    pub format: Option<String>,
    /// The selected palette name (`palette = "x"` → `[palettes.x]`).
    #[serde(default)]
    pub palette: Option<String>,
    /// Whether Starship prepends a blank line before the prompt.
    #[serde(default)]
    pub add_newline: Option<bool>,
    /// All `[palettes.*]` definitions (`name → value`, values are color strings).
    #[serde(default)]
    pub palettes: HashMap<String, HashMap<String, String>>,
    /// Every other top-level key (module tables + unknown scalars).
    #[serde(flatten)]
    pub modules: HashMap<String, toml::Value>,
}

/// Parse a `starship.toml` document.
///
/// # Errors
///
/// Returns [`ImportError::Parse`] when the input is not valid Starship TOML.
pub fn parse(input: &str) -> Result<StarshipConfig> {
    toml::from_str(input).map_err(|error| ImportError::Parse {
        message: error.to_string(),
    })
}

impl StarshipConfig {
    /// Borrow a module's table if the captured value is a table.
    #[must_use]
    pub fn module_table<'a>(&'a self, name: &str) -> Option<&'a toml::value::Table> {
        self.modules.get(name).and_then(toml::Value::as_table)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::parse;

    const SAMPLE: &str = r##"
format = "$directory$git_branch$git_status$cmd_duration$character"
palette = "nord"
add_newline = false

[palettes.nord]
polar0 = "#2e3440"
frost = "#88c0d0"

[directory]
format = "[$path]($style)[$read_only]($read_only_style) "
style = "bold cyan"

[git_branch]
symbol = " "
style = "bold purple"

[aws]
disabled = false
"##;

    #[test]
    fn parses_typed_top_level_keys() {
        let config = parse(SAMPLE).unwrap();
        assert_eq!(
            config.format.as_deref(),
            Some("$directory$git_branch$git_status$cmd_duration$character")
        );
        assert_eq!(config.palette.as_deref(), Some("nord"));
        assert_eq!(config.add_newline, Some(false));
    }

    #[test]
    fn captures_palettes_as_string_maps() {
        let config = parse(SAMPLE).unwrap();
        let nord = config.palettes.get("nord").expect("nord palette present");
        assert_eq!(nord.get("polar0").map(String::as_str), Some("#2e3440"));
        assert_eq!(nord.get("frost").map(String::as_str), Some("#88c0d0"));
    }

    #[test]
    fn captures_unknown_and_known_modules_in_modules_map() {
        let config = parse(SAMPLE).unwrap();
        let directory = config.module_table("directory").expect("directory table");
        assert_eq!(
            directory.get("format").and_then(toml::Value::as_str),
            Some("[$path]($style)[$read_only]($read_only_style) ")
        );
        // Unknown module survives for later warning.
        assert!(config.module_table("aws").is_some());
    }

    #[test]
    fn rejects_invalid_toml() {
        let error = parse("this is = = not toml").unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("failed to parse starship.toml"),
            "got: {message}"
        );
    }
}
