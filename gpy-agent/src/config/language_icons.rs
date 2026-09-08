//! Language icon mappings and theme configuration
//!
//! This module handles language-specific icons (Nerd Fonts) and per-language
//! color theme customization.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use super::DelimiterConfig;
use super::defaults::{default_color_black, default_color_white};

// ============================================================================
// Language Icons Configuration
// ============================================================================

/// Language icons configuration (Nerd Fonts icons)
///
/// This struct allows users to customize the icons displayed for different
/// programming languages in the prompt. All fields are optional and will
/// fall back to built-in defaults if not specified.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LanguageIcons {
    /// Dynamic map of language icons
    #[serde(flatten)]
    pub icons: HashMap<String, super::types::Icon>,
}

// ============================================================================
// Language Theme Configuration
// ============================================================================

/// Language segment color configuration with per-language overrides
///
/// Provides fine-grained control over the appearance of the language segment.
/// Each language can have its own background and text colors, with fallback
/// to the segment-level defaults for unknown languages.
#[derive(Debug, Clone, Serialize)]
pub struct LanguageTheme {
    /// Optional Starship-style template controlling how this segment renders.
    /// `None` selects the legacy formatter path (byte-identical output).
    #[serde(default)]
    pub format: Option<String>,
    // Delimiter configurations
    /// Open delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub open: Option<DelimiterConfig>,
    /// Close delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub close: Option<DelimiterConfig>,
    /// Background color (fallback for unknown languages)
    #[serde(default = "default_color_black")]
    pub bg_color: super::types::ColorSpec,
    /// Text color (fallback for unknown languages)
    #[serde(default = "default_color_white")]
    pub text_color: super::types::ColorSpec,

    // Per-language background/text color overrides
    // e.g., "rust_bg_color", "rust_text_color"
    /// Per-language background/text color overrides (e.g., "`rust_bg_color`", "`rust_text_color`")
    pub overrides: HashMap<String, super::types::ColorSpec>,
    /// Per-language symbol overrides (e.g., `rust_symbol = "🦀"`).
    /// Takes precedence over config icons when `show_icons` is enabled.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub symbols: HashMap<String, String>,
    /// Per-language style-attribute overrides (e.g. `java_style = "dimmed"`).
    /// Holds attribute tokens only (`bold`/`dimmed`/`italic`/…); composed with the
    /// language color in the format as `($attr fg:$color)`. Returned verbatim by
    /// `get_style`, defaulting to `"bold"` when unset.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub styles: HashMap<String, String>,

    /// Show a language's symbol even when no version resolves. `None`/`false`
    /// keeps the legacy behavior (a versionless language is hidden while
    /// `config.language.show_versions` is on). Starship shows the symbol
    /// regardless, so the starship preset sets this to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_symbol_without_version: Option<bool>,

    /// Theme-owned allow-list of languages this preset renders (canonical names,
    /// matched case-insensitively). `None` defers to
    /// `config.language.enabled_languages`. Lets a preset match another tool's
    /// module set (e.g. Starship has no fish-shell module) without editing global
    /// config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_languages: Option<Vec<String>>,
}

// Manual Deserialize to work around a toml-crate limitation: #[serde(flatten)] on
// HashMap<String, T> (where T has a custom deserializer) captures ALL fields —
// including named struct fields like `format` — and tries to validate them as T.
// The visitor pattern routes each key explicitly so `format` never reaches ColorSpec.
impl<'de> Deserialize<'de> for LanguageTheme {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(LanguageThemeVisitor)
    }
}

// Hoisted to module scope (rather than nested in `deserialize`) so each method
// is its own function for line-length lints.
struct LanguageThemeVisitor;

impl<'de> serde::de::Visitor<'de> for LanguageThemeVisitor {
    type Value = LanguageTheme;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("language theme table")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut map: A,
    ) -> Result<LanguageTheme, A::Error> {
        let mut format: Option<String> = None;
        let mut open: Option<DelimiterConfig> = None;
        let mut close: Option<DelimiterConfig> = None;
        let mut bg_color: Option<super::types::ColorSpec> = None;
        let mut text_color: Option<super::types::ColorSpec> = None;
        let mut overrides: HashMap<String, super::types::ColorSpec> = HashMap::new();
        let mut symbols: HashMap<String, String> = HashMap::new();
        let mut styles: HashMap<String, String> = HashMap::new();
        let mut show_symbol_without_version: Option<bool> = None;
        let mut enabled_languages: Option<Vec<String>> = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "format" => format = map.next_value()?,
                "open" => open = map.next_value()?,
                "close" => close = map.next_value()?,
                "bg_color" => bg_color = Some(map.next_value()?),
                "text_color" => text_color = Some(map.next_value()?),
                "show_symbol_without_version" => show_symbol_without_version = map.next_value()?,
                "enabled_languages" => enabled_languages = map.next_value()?,
                // Flat keys (rust_bg_color = "red") or a nested [..overrides] subtable.
                "overrides" => {
                    let nested: HashMap<String, super::types::ColorSpec> = map.next_value()?;
                    overrides.extend(nested);
                }
                // Nested [..symbols] subtable or flat keys like rust_symbol = "🦀".
                "symbols" => {
                    let nested: HashMap<String, String> = map.next_value()?;
                    symbols.extend(nested);
                }
                // Route per-language symbol overrides (e.g. rust_symbol = "🦀").
                k if k.ends_with("_symbol") => {
                    symbols.insert(key, map.next_value()?);
                }
                // Nested [..styles] subtable or flat keys like java_style = "dimmed".
                "styles" => {
                    let nested: HashMap<String, String> = map.next_value()?;
                    styles.extend(nested);
                }
                // Route per-language style-attribute overrides (e.g. java_style = "dimmed").
                k if k.ends_with("_style") => {
                    styles.insert(key, map.next_value()?);
                }
                _ => {
                    overrides.insert(key, map.next_value()?);
                }
            }
        }

        Ok(LanguageTheme {
            format,
            open,
            close,
            bg_color: bg_color.unwrap_or_else(default_color_black),
            text_color: text_color.unwrap_or_else(default_color_white),
            overrides,
            symbols,
            styles,
            show_symbol_without_version,
            enabled_languages,
        })
    }
}

impl Default for LanguageTheme {
    fn default() -> Self {
        let mut overrides = HashMap::new();

        // Populate defaults from metadata
        #[expect(
            clippy::expect_used,
            reason = "colors come from the static LANGUAGES metadata table; a color string baked into the binary is infallible to parse in practice"
        )]
        for lang in crate::language::metadata::LANGUAGES {
            if let Some(color) = lang.default_color {
                overrides.insert(
                    format!("{}_bg_color", lang.canonical),
                    super::types::ColorSpec::new(color)
                        .expect("Metadata default color must be valid"),
                );
                overrides.insert(
                    format!("{}_text_color", lang.canonical),
                    super::types::ColorSpec::new("white")
                        .expect("Metadata default color must be valid"),
                );
            }
        }

        Self {
            format: None,
            open: None,
            close: None,
            bg_color: default_color_black(),
            text_color: default_color_white(),
            overrides,
            symbols: HashMap::new(),
            styles: HashMap::new(),
            show_symbol_without_version: None,
            enabled_languages: None,
        }
    }
}

impl LanguageTheme {
    /// Get background color for a specific language with fallback to segment default
    ///
    /// This function maps language names (case-insensitive) to their configured
    /// background colors. It handles multiple aliases for the same language
    /// (e.g., "node", "nodejs", "javascript", "js" all map to Node.js colors).
    ///
    /// # Arguments
    ///
    /// * `language` - The language name (case-insensitive)
    ///
    /// The background color for the language, or the fallback `bg_color` if
    /// no specific color is configured.
    #[must_use]
    pub fn get_bg_color(&self, language: &str) -> &str {
        let canonical = crate::language::metadata::get_canonical_name(language).unwrap_or(language);
        let key = format!("{canonical}_bg_color");
        self.overrides
            .get(&key)
            .map_or_else(|| self.bg_color.as_str(), |c| c.as_str())
    }

    /// Get text color for a specific language with fallback to segment default
    ///
    /// This function maps language names (case-insensitive) to their configured
    /// text colors.
    ///
    /// # Arguments
    ///
    /// * `language` - The language name (case-insensitive)
    ///
    /// # Returns
    ///
    /// The text color for the language, or the fallback `text_color` if
    /// no specific color is configured.
    #[must_use]
    pub fn get_text_color(&self, language: &str) -> &str {
        let canonical = crate::language::metadata::get_canonical_name(language).unwrap_or(language);
        let key = format!("{canonical}_text_color");
        self.overrides
            .get(&key)
            .map_or_else(|| self.text_color.as_str(), |c| c.as_str())
    }

    /// Get a theme-declared symbol for a specific language, if set.
    ///
    /// Returns `None` when the theme does not declare a symbol for this language,
    /// signalling that the caller should fall back to the config-icon path.
    #[must_use]
    pub fn get_symbol(&self, language: &str) -> Option<&str> {
        let canonical = crate::language::metadata::get_canonical_name(language).unwrap_or(language);
        let key = format!("{canonical}_symbol");
        self.symbols.get(&key).map(String::as_str)
    }

    /// Get a theme-declared style-attribute string for a language.
    ///
    /// Returns the `<canonical>_style` override verbatim (so an explicit empty
    /// string suppresses the default), or `"bold"` when no override is set —
    /// matching the previously hardcoded attribute for every language.
    #[must_use]
    pub fn get_style(&self, language: &str) -> &str {
        let canonical = crate::language::metadata::get_canonical_name(language).unwrap_or(language);
        let key = format!("{canonical}_style");
        self.styles.get(&key).map_or("bold", String::as_str)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn language_theme_parses_format_from_toml() {
        let toml_str = "format = \"[ ](fg:prev_bg bg:$bg)[$symbol]($style)([ $version]($style))([$sep_close](fg:$bg))\"\ntext_color = \"black\"\nbg_color = \"white\"\nrust_bg_color = \"red\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        assert_eq!(
            theme.format.as_deref(),
            Some(
                "[ ](fg:prev_bg bg:$bg)[$symbol]($style)([ $version]($style))([$sep_close](fg:$bg))"
            )
        );
        assert_eq!(theme.bg_color.as_str(), "white");
        assert_eq!(theme.text_color.as_str(), "black");
        assert_eq!(
            theme
                .overrides
                .get("rust_bg_color")
                .map(super::super::types::ColorSpec::as_str),
            Some("red")
        );
    }

    #[test]
    fn language_theme_format_does_not_pollute_overrides() {
        let toml_str = "[ui]\nprompt_icon = \"❯\"\n\n[segments.language]\nformat = \"[ ](fg:prev_bg bg:$bg)[$symbol]($style)\"\ntext_color = \"black\"\nbg_color = \"white\"\nrust_bg_color = \"red\"\n";
        let result = crate::theme::parse(toml_str, "test");
        assert!(result.is_ok(), "theme::parse failed: {:?}", result.err());
        let theme = result.expect("already checked");
        assert_eq!(
            theme.segments.language.format.as_deref(),
            Some("[ ](fg:prev_bg bg:$bg)[$symbol]($style)")
        );
    }

    #[test]
    fn language_theme_parses_symbol_from_toml() {
        let toml_str = "rust_symbol = \"🦀\"\npython_symbol = \"🐍\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        assert_eq!(theme.get_symbol("Rust"), Some("🦀"));
        assert_eq!(theme.get_symbol("Python"), Some("🐍"));
    }

    #[test]
    fn symbol_not_in_overrides() {
        let toml_str = "rust_symbol = \"🦀\"\nrust_bg_color = \"red\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        // Symbol must not pollute the color overrides map.
        assert!(!theme.overrides.contains_key("rust_symbol"));
        assert_eq!(
            theme
                .overrides
                .get("rust_bg_color")
                .map(super::super::types::ColorSpec::as_str),
            Some("red")
        );
    }

    #[test]
    fn get_symbol_returns_none_when_unset() {
        let theme = LanguageTheme::default();
        assert_eq!(theme.get_symbol("Rust"), None);
    }

    #[test]
    fn get_symbol_uses_canonical_name() {
        let toml_str = "rust_symbol = \"🦀\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        // "rust" and "Rust" both canonicalize to "rust".
        assert_eq!(theme.get_symbol("rust"), Some("🦀"));
        assert_eq!(theme.get_symbol("Rust"), Some("🦀"));
    }

    #[test]
    fn starship_theme_uses_role_names_for_off_ansi_languages() {
        let content = crate::config::defaults::STARSHIP_THEME_CONTENT;
        let theme = crate::theme::parse(content, "starship").expect("starship theme parses");
        let lang = &theme.segments.language.overrides;
        assert_eq!(
            lang.get("swift_bg_color").expect("swift").as_str(),
            "orange"
        );
        assert_eq!(
            lang.get("php_bg_color").expect("php").as_str(),
            "bright_magenta"
        );
        assert_eq!(
            lang.get("cpp_bg_color").expect("cpp").as_str(),
            "bright_green"
        );
        assert_eq!(lang.get("c_bg_color").expect("c").as_str(), "bright_green");
    }

    #[test]
    fn language_theme_parses_style_from_toml() {
        let toml_str = "java_style = \"dimmed\"\nrust_style = \"bold italic\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        assert_eq!(theme.get_style("java"), "dimmed");
        assert_eq!(theme.get_style("rust"), "bold italic");
    }

    #[test]
    fn language_theme_parses_styles_subtable() {
        let toml_str = "[styles]\njava_style = \"dimmed\"\nrust_style = \"bold italic\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        assert_eq!(theme.get_style("java"), "dimmed");
        assert_eq!(theme.get_style("rust"), "bold italic");
    }

    #[test]
    fn get_style_defaults_to_bold_when_unset() {
        let theme = LanguageTheme::default();
        assert_eq!(theme.get_style("python"), "bold");
    }

    #[test]
    fn get_style_uses_canonical_name() {
        let toml_str = "java_style = \"dimmed\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        assert_eq!(theme.get_style("Java"), "dimmed");
        assert_eq!(theme.get_style("java"), "dimmed");
    }

    #[test]
    fn get_style_returns_empty_override_verbatim() {
        // An explicit empty style suppresses the bold default (Starship `style = "red"`).
        let toml_str = "go_style = \"\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        assert_eq!(theme.get_style("go"), "");
    }

    #[test]
    fn style_not_in_overrides() {
        let toml_str = "java_style = \"dimmed\"\njava_bg_color = \"red\"\n";
        let theme: LanguageTheme = toml::from_str(toml_str).expect("parse");
        assert!(!theme.overrides.contains_key("java_style"));
        assert_eq!(
            theme
                .overrides
                .get("java_bg_color")
                .map(super::super::types::ColorSpec::as_str),
            Some("red")
        );
    }

    #[test]
    fn starship_preset_sets_java_dimmed_and_uses_attr_var() {
        let content = crate::config::defaults::STARSHIP_THEME_CONTENT;
        let theme = crate::theme::parse(content, "starship").expect("starship theme parses");
        let lang = &theme.segments.language;
        assert_eq!(lang.get_style("java"), "dimmed");
        assert_eq!(lang.get_style("rust"), "bold"); // default, unchanged
        assert!(
            lang.format
                .as_deref()
                .expect("format present")
                .contains("$attr"),
            "language format must reference $attr"
        );
    }
}
