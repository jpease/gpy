//! Language icon mappings and per-language theme colors.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use super::DelimiterConfig;
use super::defaults::literal;

/// Language icons from `[language.icons]`; a language without an entry uses the
/// built-in Nerd Font icon when there is one, else its name.
///
/// Icon precedence, shared with [`GitIcons`](super::GitIcons): `ui.show_icons`
/// gates first; then a theme icon (`[segments.language] <lang>_symbol`); then
/// an explicit `config.toml` entry here; then the built-in default
/// ([`DEFAULT_LANGUAGE_ICONS`](super::defaults::DEFAULT_LANGUAGE_ICONS)).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LanguageIcons {
    /// Dynamic map of language icons
    #[serde(flatten)]
    pub icons: HashMap<String, super::types::Icon>,
}

/// Language segment colors with per-language overrides.
#[derive(Debug, Clone, Serialize)]
pub struct LanguageTheme {
    /// Starship-style template controlling how this segment renders.
    /// The language segment is always agent-rendered, so `None` renders nothing.
    pub format: Option<String>,
    /// Open delimiter configuration (supports fg/bg colors)
    pub open: Option<DelimiterConfig>,
    /// Close delimiter configuration (supports fg/bg colors)
    pub close: Option<DelimiterConfig>,
    /// Background color (fallback for languages without a `<lang>_bg_color`)
    pub bg_color: super::types::ColorSpec,
    /// Text color (fallback for languages without a `<lang>_text_color`)
    pub text_color: super::types::ColorSpec,

    /// Per-language color overrides (e.g. `rust_bg_color`, `rust_text_color`).
    pub overrides: HashMap<String, super::types::ColorSpec>,
    /// Per-language symbol overrides (e.g., `rust_symbol = "🦀"`).
    /// Takes precedence over `[language.icons]` and the built-in icons when
    /// `show_icons` is enabled.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub symbols: HashMap<String, String>,
    /// Per-language style attributes (e.g. `java_style = "dimmed"`); default `"bold"`.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub styles: HashMap<String, String>,

    /// Show a language's symbol even when no version resolves (Starship behavior).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_symbol_without_version: Option<bool>,

    /// Theme-owned allow-list of languages (case-insensitive); `None` defers to
    /// `config.language.enabled_languages`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_languages: Option<Vec<String>>,
}

/// Keys of `[segments.language]` that [`LanguageTheme`] reads as named fields,
/// including its three nested per-language maps. Every other key is a flat
/// per-language key, read into the map [`flat_language_map`] names.
pub(crate) const LANGUAGE_FIELDS: &[&str] = &[
    "format",
    "open",
    "close",
    "bg_color",
    "text_color",
    "show_symbol_without_version",
    "enabled_languages",
    "overrides",
    "symbols",
    "styles",
];

/// One of [`LanguageTheme`]'s per-language maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanguageMap {
    /// `overrides`: per-language colours (`rust_bg_color`).
    Overrides,
    /// `symbols`: per-language symbols (`rust_symbol`).
    Symbols,
    /// `styles`: per-language style attributes (`java_style`).
    Styles,
}

impl LanguageMap {
    /// The map's key in `[segments.language]`, e.g. `symbols`.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Overrides => "overrides",
            Self::Symbols => "symbols",
            Self::Styles => "styles",
        }
    }
}

/// The map a flat `[segments.language]` key (one not in [`LANGUAGE_FIELDS`])
/// is read into: `*_symbol` → `symbols`, `*_style` → `styles`, anything else →
/// `overrides`. The deserializer and theme overlay canonicalisation share this rule.
pub(crate) fn flat_language_map(key: &str) -> LanguageMap {
    if key.ends_with("_symbol") {
        LanguageMap::Symbols
    } else if key.ends_with("_style") {
        LanguageMap::Styles
    } else {
        LanguageMap::Overrides
    }
}

/// Rewrite a raw `[segments.language]` table to the nested spelling: each flat
/// per-language key moves into the table [`flat_language_map`] names.
///
/// Keys are visited in table order, and a later key of the same language
/// replaces an earlier one, as the deserializer reads them; so the nested
/// table deserializes to the same [`LanguageTheme`]. A flat key whose map key
/// holds a non-table value stays flat (the deserializer rejects that value).
pub(crate) fn nest_language_table(table: &mut toml::Table) {
    if table
        .keys()
        .all(|key| LANGUAGE_FIELDS.contains(&key.as_str()))
    {
        return;
    }
    for (key, value) in std::mem::take(table) {
        if LANGUAGE_FIELDS.contains(&key.as_str()) {
            let meets_table = matches!(table.get(&key), Some(toml::Value::Table(_)));
            match value {
                // A nested map after flat keys of the same map: its entries win.
                toml::Value::Table(nested) if meets_table => {
                    if let Some(toml::Value::Table(existing)) = table.get_mut(&key) {
                        existing.extend(nested);
                    }
                }
                other => {
                    table.insert(key, other);
                }
            }
            continue;
        }
        let map = flat_language_map(&key).key();
        if table.get(map).is_none_or(toml::Value::is_table) {
            if let toml::Value::Table(nested) = table
                .entry(map)
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            {
                nested.insert(key, value);
            }
        } else {
            table.insert(key, value);
        }
    }
}

// Manual Deserialize: toml's `#[serde(flatten)]` on a map would capture named
// fields like `format` and validate them as ColorSpec.
impl<'de> Deserialize<'de> for LanguageTheme {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(LanguageThemeVisitor)
    }
}

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
        let mut theme = LanguageTheme::default();

        // The named arms are exactly `LANGUAGE_FIELDS`.
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "format" => theme.format = map.next_value()?,
                "open" => theme.open = map.next_value()?,
                "close" => theme.close = map.next_value()?,
                "bg_color" => theme.bg_color = map.next_value()?,
                "text_color" => theme.text_color = map.next_value()?,
                "show_symbol_without_version" => {
                    theme.show_symbol_without_version = map.next_value()?;
                }
                "enabled_languages" => theme.enabled_languages = map.next_value()?,
                // Nested [..overrides] subtable or flat keys like rust_bg_color = "red".
                "overrides" => {
                    let nested: HashMap<String, super::types::ColorSpec> = map.next_value()?;
                    theme.overrides.extend(nested);
                }
                // Nested [..symbols] subtable or flat keys like rust_symbol = "🦀".
                "symbols" => {
                    let nested: HashMap<String, String> = map.next_value()?;
                    theme.symbols.extend(nested);
                }
                // Nested [..styles] subtable or flat keys like java_style = "dimmed".
                "styles" => {
                    let nested: HashMap<String, String> = map.next_value()?;
                    theme.styles.extend(nested);
                }
                _ => match flat_language_map(&key) {
                    LanguageMap::Symbols => {
                        theme.symbols.insert(key, map.next_value()?);
                    }
                    LanguageMap::Styles => {
                        theme.styles.insert(key, map.next_value()?);
                    }
                    LanguageMap::Overrides => {
                        theme.overrides.insert(key, map.next_value()?);
                    }
                },
            }
        }

        Ok(theme)
    }
}

impl Default for LanguageTheme {
    fn default() -> Self {
        Self {
            format: None,
            open: None,
            close: None,
            bg_color: literal("black"),
            text_color: literal("white"),
            overrides: HashMap::new(),
            symbols: HashMap::new(),
            styles: HashMap::new(),
            show_symbol_without_version: None,
            enabled_languages: None,
        }
    }
}

impl LanguageTheme {
    /// Background color for `language` (case-insensitive, aliases resolved), else `bg_color`.
    #[must_use]
    pub fn get_bg_color(&self, language: &str) -> &str {
        let canonical = crate::language::metadata::get_canonical_name(language).unwrap_or(language);
        lookup(&self.overrides, canonical, "_bg_color")
            .map_or_else(|| self.bg_color.as_str(), |c| c.as_str())
    }

    /// Text color for `language` (case-insensitive), else `text_color`.
    #[must_use]
    pub fn get_text_color(&self, language: &str) -> &str {
        let canonical = crate::language::metadata::get_canonical_name(language).unwrap_or(language);
        lookup(&self.overrides, canonical, "_text_color")
            .map_or_else(|| self.text_color.as_str(), |c| c.as_str())
    }

    /// Get a theme-declared symbol for a specific language, if set.
    ///
    /// `None` means fall back to config icons.
    #[must_use]
    pub fn get_symbol(&self, language: &str) -> Option<&str> {
        let canonical = crate::language::metadata::get_canonical_name(language).unwrap_or(language);
        lookup(&self.symbols, canonical, "_symbol").map(String::as_str)
    }

    /// Get a theme-declared style-attribute string for a language.
    ///
    /// Explicit empty string suppresses the `"bold"` default.
    #[must_use]
    pub fn get_style(&self, language: &str) -> &str {
        let canonical = crate::language::metadata::get_canonical_name(language).unwrap_or(language);
        lookup(&self.styles, canonical, "_style").map_or("bold", String::as_str)
    }
}

/// Longest `<canonical><suffix>` key assembled on the stack; longer keys take the heap path.
const STACK_KEY_BYTES: usize = 64;

/// Look up `<canonical><suffix>` in a flat per-language map without allocating
/// for ordinary language names.
fn lookup<'map, V>(
    map: &'map HashMap<String, V>,
    canonical: &str,
    suffix: &str,
) -> Option<&'map V> {
    let split = canonical.len();
    let total = split.saturating_add(suffix.len());
    let mut buf = [0_u8; STACK_KEY_BYTES];
    let stack_key = buf.get_mut(..total).and_then(|key| {
        let (head, tail) = key.split_at_mut_checked(split)?;
        head.copy_from_slice(canonical.as_bytes());
        tail.copy_from_slice(suffix.as_bytes());
        std::str::from_utf8(key).ok()
    });
    stack_key.map_or_else(
        || map.get(&format!("{canonical}{suffix}")),
        |key| map.get(key),
    )
}

#[cfg(test)]
mod tests {
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
        // Stored canonically; palettes resolve `brmagenta` through their `bright_magenta` key.
        assert_eq!(lang.get("php_bg_color").expect("php").as_str(), "brmagenta");
        assert_eq!(lang.get("cpp_bg_color").expect("cpp").as_str(), "brgreen");
        assert_eq!(lang.get("c_bg_color").expect("c").as_str(), "brgreen");
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

    /// `LANGUAGE_FIELDS` is exactly the set of keys the deserializer reads as
    /// named fields: a theme with every field set serializes to those keys.
    #[test]
    fn language_fields_match_the_named_fields() {
        let theme: LanguageTheme = toml::from_str(
            "format = \"x\"\nshow_symbol_without_version = true\nenabled_languages = [\"rust\"]\n\
             rust_symbol = \"R\"\njava_style = \"dimmed\"\nrust_bg_color = \"red\"\n\
             [open]\nicon = \"<\"\n[close]\nicon = \">\"\n",
        )
        .expect("parse");
        let serialized = toml::Table::try_from(&theme).expect("serialize");
        let mut keys: Vec<&str> = serialized.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected = LANGUAGE_FIELDS.to_vec();
        expected.sort_unstable();
        assert_eq!(keys, expected);
    }

    /// Nesting a table with the shared rule deserializes to the same theme as
    /// the flat spelling, for every kind of flat key.
    #[test]
    fn nested_table_deserializes_like_the_flat_one() {
        let flat: toml::Table = toml::from_str(
            "format = \"x\"\nbg_color = \"white\"\nrust_symbol = \"R\"\njava_style = \"dimmed\"\n\
             rust_bg_color = \"red\"\ngo_text_color = \"blue\"\n[symbols]\npython_symbol = \"P\"\n",
        )
        .expect("parse");
        let mut nested = flat.clone();
        nest_language_table(&mut nested);

        let as_value = |table: toml::Table| {
            let theme: LanguageTheme = toml::Value::Table(table).try_into().expect("deserialize");
            toml::Value::try_from(&theme).expect("serialize")
        };
        assert_eq!(as_value(nested.clone()), as_value(flat));
        assert!(
            nested
                .keys()
                .all(|key| LANGUAGE_FIELDS.contains(&key.as_str())),
            "every flat key moved into a map: {nested:?}"
        );
        let symbols = nested
            .get("symbols")
            .and_then(toml::Value::as_table)
            .expect("symbols table");
        assert!(symbols.contains_key("rust_symbol") && symbols.contains_key("python_symbol"));
    }

    #[test]
    fn flat_keys_land_in_the_map_the_deserializer_uses() {
        assert_eq!(flat_language_map("rust_symbol"), LanguageMap::Symbols);
        assert_eq!(flat_language_map("java_style"), LanguageMap::Styles);
        assert_eq!(flat_language_map("rust_bg_color"), LanguageMap::Overrides);
        assert_eq!(flat_language_map("anything"), LanguageMap::Overrides);
    }
}
