//! Maps a detected language to Starship-exact template variables (`symbol`, `version`, `style`)
//! so imported Starship `format` strings work verbatim, and picks which detected languages
//! the language segment draws.

use crate::config::Config;
use crate::config::defaults::DEFAULT_LANGUAGE_ICONS;
use crate::config::types::LanguageDisplay;
use crate::formatter::SegmentPosition;
use crate::formatter::chrome::SegmentChrome;
use crate::formatter::separator::Glyphs;
use crate::ipc::LanguageInfo;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Most language pills one prompt draws.
pub(crate) const MAX_LANGUAGE_PILLS: usize = 3;

/// The languages the segment draws, in order, at most [`MAX_LANGUAGE_PILLS`]. With
/// `show_versions`, versionless ones are dropped unless `show_symbol_without_version`.
pub(crate) fn select_languages<'a>(
    config: &Config,
    theme: &ThemeConfig,
    languages: &'a [LanguageInfo],
) -> Vec<&'a LanguageInfo> {
    let versions_required = config.language.show_versions
        && !theme
            .segments
            .language
            .show_symbol_without_version
            .unwrap_or(false);
    languages
        .iter()
        .filter(|lang| !versions_required || lang.version.is_some())
        .take(MAX_LANGUAGE_PILLS)
        .collect()
}

/// Resolves Starship-exact language variable names from one detected language.
pub struct LanguageResolver<'a> {
    lang: &'a LanguageInfo,
    config: &'a Config,
    theme: &'a ThemeConfig,
    chrome: SegmentChrome<'a>,
}

impl<'a> LanguageResolver<'a> {
    /// Build a resolver for one language's segment render.
    #[must_use]
    pub fn new(
        lang: &'a LanguageInfo,
        config: &'a Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        Self {
            lang,
            config,
            theme,
            chrome: SegmentChrome::pill(
                pos,
                Glyphs::from(&config.ui),
                theme.segments.language.get_text_color(&lang.name),
                lang.color.as_str(),
            ),
        }
    }

    /// `$symbol`: with icons on, the theme symbol, else the `[language.icons]`
    /// entry, else the built-in icon, else the name; with icons off, the name.
    /// An explicitly empty configured icon leaves it unresolved.
    fn symbol(&self) -> Option<String> {
        let name = self.lang.name.as_str();
        let icons_on =
            self.config.ui.show_icons && self.config.language.display == LanguageDisplay::Icon;
        if !icons_on {
            return (!name.is_empty()).then(|| name.to_owned());
        }
        if let Some(symbol) = self.theme.segments.language.get_symbol(name) {
            return Some(symbol.to_owned());
        }
        let icon_key = crate::language::metadata::get_icon_key(name).unwrap_or(name);
        let display = self
            .config
            .language
            .icons
            .icons
            .get(icon_key)
            .map(crate::config::types::Icon::as_str)
            .or_else(|| {
                DEFAULT_LANGUAGE_ICONS
                    .iter()
                    .find_map(|(key, glyph)| (*key == icon_key).then_some(*glyph))
            })
            .unwrap_or(name);
        (!display.is_empty()).then(|| display.to_owned())
    }
}

impl VariableResolver for LanguageResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "symbol" => self.symbol(),
            "version" => self
                .config
                .language
                .show_versions
                .then(|| self.lang.version.clone())
                .flatten(),
            // `$color`: the language's own color, which is also its `$bg`.
            "color" => Some(self.lang.color.as_str().to_owned()),
            // `$attr`: style attributes, defaulting to `bold`.
            "attr" => Some(
                self.theme
                    .segments
                    .language
                    .get_style(&self.lang.name)
                    .to_owned(),
            ),
            _ => self.chrome.resolve(name),
        }
    }
}

#[cfg(test)]
mod tests {

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
        assert!(resolver.resolve("symbol").is_some());
    }

    fn symbol_of(name: &str, config: &Config) -> Option<String> {
        let li = LanguageInfo {
            name: name.to_owned(),
            version: None,
            color: ColorSpec::new("#dea584").unwrap(),
        };
        let theme = ThemeConfig::default();
        LanguageResolver::new(
            &li,
            config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        )
        .resolve("symbol")
    }

    #[test]
    fn builtin_icon_used_when_neither_theme_nor_config_sets_one() {
        let config = Config::default();
        assert_eq!(symbol_of("Rust", &config), Some("\u{f1617}".to_owned()));
    }

    #[test]
    fn config_icon_beats_builtin() {
        let mut config = Config::default();
        config.language.icons.icons.insert(
            "rust".to_owned(),
            crate::config::types::Icon::new("R").unwrap(),
        );
        assert_eq!(symbol_of("Rust", &config), Some("R".to_owned()));
    }

    #[test]
    fn language_without_builtin_icon_shows_name() {
        let config = Config::default();
        assert_eq!(symbol_of("Zig", &config), Some("Zig".to_owned()));
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
