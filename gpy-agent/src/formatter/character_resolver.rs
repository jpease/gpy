//! Maps exit status to Starship-exact `character` variables: `symbol`, `style`, `sep_close`.
//! Foreground-only: `$bg` resolves to `None`.

use crate::config::Config;
use crate::formatter::SegmentPosition;
use crate::formatter::chrome::SegmentChrome;
use crate::formatter::separator::Glyphs;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact character variable names.
pub struct CharacterResolver<'a> {
    success: bool,
    theme: &'a ThemeConfig,
    chrome: SegmentChrome<'a>,
}

impl<'a> CharacterResolver<'a> {
    /// Build a resolver for one character segment render.
    #[must_use]
    pub fn new(
        success: bool,
        config: &Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        let character = &theme.segments.character;
        let fg = if success {
            character.success_color.as_str()
        } else {
            character.error_color.as_str()
        };
        Self {
            success,
            theme,
            chrome: SegmentChrome::terminal(pos, Glyphs::from(&config.ui), fg),
        }
    }
}

impl VariableResolver for CharacterResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "symbol" => {
                let character = &self.theme.segments.character;
                Some(if self.success {
                    character.success_symbol.clone()
                } else {
                    character.error_symbol.clone()
                })
            }
            _ => self.chrome.resolve(name),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::CharacterResolver;
    use crate::config::Config;
    use crate::formatter::SegmentPosition;
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn resolve(success: bool, pos: SegmentPosition, name: &str) -> Option<String> {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        CharacterResolver::new(success, &config, &theme, pos).resolve(name)
    }

    #[test]
    fn success_symbol_resolved() {
        assert_eq!(
            resolve(true, SegmentPosition::MIDDLE, "symbol").unwrap(),
            "❯"
        );
    }

    #[test]
    fn error_symbol_resolved() {
        assert_eq!(
            resolve(false, SegmentPosition::MIDDLE, "symbol").unwrap(),
            "❯"
        );
    }

    #[test]
    fn success_style_contains_fg_green() {
        let style = resolve(true, SegmentPosition::MIDDLE, "style").expect("style present");
        assert!(style.starts_with("fg:"), "got {style}");
        assert!(style.contains("green"), "expected green, got {style}");
    }

    #[test]
    fn error_style_contains_fg_red() {
        let style = resolve(false, SegmentPosition::MIDDLE, "style").expect("style present");
        assert!(style.starts_with("fg:"), "got {style}");
        assert!(style.contains("red"), "expected red, got {style}");
    }

    #[test]
    fn unknown_variable_is_none() {
        assert_eq!(resolve(true, SegmentPosition::MIDDLE, "nonsuch"), None);
    }

    #[test]
    fn sep_close_some_when_is_last_true() {
        assert_eq!(
            resolve(true, SegmentPosition::LAST, "sep_close"),
            Some("\u{e0b4}".to_owned())
        );
    }

    #[test]
    fn sep_close_none_when_is_last_false() {
        assert_eq!(resolve(true, SegmentPosition::MIDDLE, "sep_close"), None);
    }

    #[test]
    fn bg_is_none_for_character_segment() {
        assert_eq!(
            resolve(true, SegmentPosition::MIDDLE, "bg"),
            None,
            "character segment has no background color"
        );
    }
}
