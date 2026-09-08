//! Maps a detected language to Starship-exact template variable names.
//!
//! Variable names MUST match Starship's language modules (`rust`, `nodejs`, …)
//! so the #186 importer can drop Starship `format` strings in verbatim. Those
//! modules render `[$symbol($version )]($style)`, so PR exposes `symbol`,
//! `version`, and `style`.

use crate::config::Config;
use crate::formatter::separator::{SeparatorStyle, resolve_separator};
use crate::formatter::{SegmentPosition, get_language_display};
use crate::ipc::LanguageInfo;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact language variable names from one detected language.
pub struct LanguageResolver<'a> {
    lang: &'a LanguageInfo,
    config: &'a Config,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
}

impl<'a> LanguageResolver<'a> {
    /// Build a resolver for one language's segment render.
    #[must_use]
    pub const fn new(
        lang: &'a LanguageInfo,
        config: &'a Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        Self {
            lang,
            config,
            theme,
            pos,
        }
    }

    /// Style string for `($style)` indirection: the language's own color as bg,
    /// the theme's text color for that language as fg (mirrors the legacy path).
    fn style(&self) -> String {
        let bg = self.lang.color.as_str();
        let fg = self.theme.segments.language.get_text_color(&self.lang.name);
        format!("fg:{fg} bg:{bg}")
    }
}

impl VariableResolver for LanguageResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            // The displayed glyph (icon) or the language name when icons are off —
            // the same text the legacy renderer emits before the version.
            // Prefer a theme-declared symbol (e.g. `rust_symbol = "🦀"`) when icons
            // are enabled; fall back to the config-icon / language-name path.
            "symbol" => {
                if self.config.ui.show_icons
                    && self.config.language.display == crate::config::types::LanguageDisplay::Icon
                    && let Some(sym) = self.theme.segments.language.get_symbol(&self.lang.name)
                {
                    return Some(sym.to_owned());
                }
                let display = get_language_display(self.config, &self.lang.name);
                (!display.is_empty()).then_some(display)
            }
            "version" => self
                .config
                .language
                .show_versions
                .then(|| self.lang.version.clone())
                .flatten(),
            "style" => Some(self.style()),
            // `$color`: bare token for flat foreground styling, e.g. `(fg:$color)`.
            // `$bg`: segment background used by powerline format strings, e.g. `(fg:$bg)`.
            // Both resolve to the language's own color (language color doubles as bg).
            "color" | "bg" => Some(self.lang.color.as_str().to_owned()),
            // `$attr`: the language's style attributes (bold/dimmed/italic/…),
            // composed in the format as `($attr fg:$color)`. Defaults to `bold`.
            "attr" => Some(
                self.theme
                    .segments
                    .language
                    .get_style(&self.lang.name)
                    .to_owned(),
            ),
            "sep_gap" => resolve_separator(self.pos, SeparatorStyle::Chained)
                .gap
                .map(str::to_owned),
            "sep_close" => resolve_separator(self.pos, SeparatorStyle::Chained)
                .close
                .map(str::to_owned),
            "sep_open" => resolve_separator(self.pos, SeparatorStyle::Chained)
                .open
                .map(str::to_owned),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::LanguageResolver;
    use crate::config::Config;
    use crate::config::types::ColorSpec;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::ipc::LanguageInfo;
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn lang(version: Option<&str>) -> LanguageInfo {
        LanguageInfo {
            name: "Rust".to_owned(),
            version: version.map(str::to_owned),
            color: ColorSpec::new("#dea584").unwrap(),
        }
    }

    #[test]
    fn exposes_symbol_version_and_style() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert!(resolver.resolve("symbol").is_some());
        assert_eq!(resolver.resolve("version"), Some("1.75.0".to_owned()));
        assert!(resolver.resolve("style").is_some());
    }

    #[test]
    fn version_is_none_when_absent() {
        let (config, theme, li) = (Config::default(), ThemeConfig::default(), lang(None));
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("version"), None);
    }

    #[test]
    fn version_is_none_when_show_versions_disabled() {
        let mut config = Config::default();
        config.language.show_versions = false;
        let theme = ThemeConfig::default();
        let li = lang(Some("1.75.0"));
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(
            resolver.resolve("version"),
            None,
            "show_versions=false must hide version text even when a version was detected"
        );
    }

    #[test]
    fn style_uses_language_color_as_background() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let style = resolver.resolve("style").expect("style present");
        assert!(style.contains("bg:#dea584"), "got {style}");
    }

    #[test]
    fn color_exposes_language_color_as_bare_token() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        // Bare color token (no fg:/bg: prefix) so it can be composed as `fg:$color`.
        assert_eq!(resolver.resolve("color"), Some("#dea584".to_owned()));
    }

    #[test]
    fn unknown_variable_is_none() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("nonsuch"), None);
    }

    #[test]
    fn sep_close_is_half_circle_when_last() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_is_triangle_when_not_last() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_close"), Some("\u{e0bc}".to_owned()));
    }

    #[test]
    fn sep_gap_is_space_when_not_last() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_gap"), Some(" ".to_owned()));
    }

    #[test]
    fn sep_gap_is_none_when_last() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_gap"), None);
    }

    #[test]
    fn bg_is_language_color() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("bg"), Some("#dea584".to_owned()));
    }

    #[test]
    fn sep_open_is_none_when_first() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::Yes),
        );
        assert_eq!(resolver.resolve("sep_open"), None);
    }

    #[test]
    fn sep_open_is_glyph_when_not_first() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_open"), Some("\u{e0ba}".to_owned()));
    }

    fn theme_with_rust_symbol(symbol: &str) -> ThemeConfig {
        let mut theme = ThemeConfig::default();
        theme
            .segments
            .language
            .symbols
            .insert("rust_symbol".to_owned(), symbol.to_owned());
        theme
    }

    #[test]
    fn theme_symbol_overrides_config_icon_when_icons_enabled() {
        let config = Config::default(); // show_icons = true, display = Icon
        let theme = theme_with_rust_symbol("🦀");
        let li = lang(Some("1.75.0"));
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("symbol"), Some("🦀".to_owned()));
    }

    #[test]
    fn fallback_to_config_when_no_theme_symbol() {
        let config = Config::default();
        let theme = ThemeConfig::default(); // no symbols set
        let li = lang(Some("1.75.0"));
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        // Falls back to get_language_display — returns icon or name, not None.
        assert!(resolver.resolve("symbol").is_some());
    }

    #[test]
    fn icons_off_ignores_theme_symbol_returns_language_name() {
        let mut config = Config::default();
        config.ui.show_icons = false;
        let theme = theme_with_rust_symbol("🦀");
        let li = lang(Some("1.75.0"));
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        // Icons off: must return language name, not emoji.
        let sym = resolver.resolve("symbol").expect("should be non-None");
        assert_eq!(
            sym, "Rust",
            "icons off should return language name, got {sym}"
        );
    }

    #[test]
    fn attr_defaults_to_bold() {
        let (config, theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("attr"), Some("bold".to_owned()));
    }

    #[test]
    fn attr_uses_theme_override() {
        let (config, mut theme, li) = (
            Config::default(),
            ThemeConfig::default(),
            lang(Some("1.75.0")),
        );
        theme
            .segments
            .language
            .styles
            .insert("rust_style".to_owned(), "dimmed".to_owned());
        let resolver = LanguageResolver::new(
            &li,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("attr"), Some("dimmed".to_owned()));
    }
}
