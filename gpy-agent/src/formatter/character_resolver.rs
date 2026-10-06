//! Maps exit-status success/failure to Starship-exact template variable names.
//!
//! Variable names: `symbol`, `style`, `sep_close`.
//! Compatible with Starship's `character` module.
//!
//! Note: the character segment is fg-only — `$bg` resolves to `None` (falls
//! through to the wildcard arm) because there is no segment background color.

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, resolve_separator};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact character variable names.
pub struct CharacterResolver<'a> {
    success: bool,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    glyphs: Glyphs,
}

impl<'a> CharacterResolver<'a> {
    /// Build a resolver for one character segment render.
    #[must_use]
    pub const fn new(success: bool, theme: &'a ThemeConfig, pos: SegmentPosition) -> Self {
        Self {
            success,
            theme,
            pos,
            glyphs: Glyphs::Nerd,
        }
    }

    /// Draw this segment with `glyphs` instead of the default [`Glyphs::Nerd`];
    /// callers holding a `Config` pass `Glyphs::from(&config.ui)` (#695).
    #[must_use]
    pub const fn with_glyphs(self, glyphs: Glyphs) -> Self {
        Self { glyphs, ..self }
    }

    /// Style string: `fg:<color>` (foreground only — no background for the prompt symbol).
    fn style(&self) -> String {
        let fg = if self.success {
            self.theme.segments.character.success_color.as_str()
        } else {
            self.theme.segments.character.error_color.as_str()
        };
        format!("fg:{fg}")
    }
}

impl VariableResolver for CharacterResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "symbol" => {
                if self.success {
                    Some(self.theme.segments.character.success_symbol.clone())
                } else {
                    Some(self.theme.segments.character.error_symbol.clone())
                }
            }
            "style" => Some(self.style()),
            "sep_close" => resolve_separator(self.pos, SeparatorStyle::Terminal, self.glyphs)
                .close
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
    #![allow(clippy::missing_errors_doc)]

    use super::CharacterResolver;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn theme() -> ThemeConfig {
        ThemeConfig::default()
    }

    #[test]
    fn success_symbol_resolved() {
        let thr = theme();
        let res = CharacterResolver::new(true, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        assert_eq!(res.resolve("symbol").unwrap(), "❯");
    }

    #[test]
    fn error_symbol_resolved() {
        let thr = theme();
        let res =
            CharacterResolver::new(false, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        assert_eq!(res.resolve("symbol").unwrap(), "❯");
    }

    #[test]
    fn success_style_contains_fg_green() {
        let thr = theme();
        let res = CharacterResolver::new(true, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        let style = res.resolve("style").expect("style present");
        assert!(style.starts_with("fg:"), "got {style}");
        assert!(style.contains("green"), "expected green, got {style}");
    }

    #[test]
    fn error_style_contains_fg_red() {
        let thr = theme();
        let res =
            CharacterResolver::new(false, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        let style = res.resolve("style").expect("style present");
        assert!(style.starts_with("fg:"), "got {style}");
        assert!(style.contains("red"), "expected red, got {style}");
    }

    #[test]
    fn unknown_variable_is_none() {
        let thr = theme();
        let res = CharacterResolver::new(true, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        assert_eq!(res.resolve("nonsuch"), None);
    }

    #[test]
    fn sep_close_some_when_is_last_true() {
        let thr = theme();
        let res =
            CharacterResolver::new(true, &thr, SegmentPosition::new(IsLast::Yes, IsFirst::No));
        assert_eq!(res.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_none_when_is_last_false() {
        let thr = theme();
        let res = CharacterResolver::new(true, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        assert_eq!(res.resolve("sep_close"), None);
    }

    #[test]
    fn bg_is_none_for_character_segment() {
        let thr = theme();
        let res = CharacterResolver::new(true, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        assert_eq!(
            res.resolve("bg"),
            None,
            "character segment has no background color"
        );
    }
}
