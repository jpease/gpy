//! Maps the previous command's exit status to the status pill's variables:
//! `symbol` (ok/fail icon), `style`, `bg` and the powerline `sep_*` caps.

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, separator_var};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Icon drawn for a successful command when the theme sets none and glyphs are available.
const OK_ICON_NERD: &str = "\u{2714}";
/// Icon drawn for a failed command when the theme sets none and glyphs are available.
const FAIL_ICON_NERD: &str = "\u{2716}";
/// Text drawn for a successful command when the theme sets none and glyphs are plain.
const OK_ICON_PLAIN: &str = "ok";
/// Text drawn for a failed command when the theme sets none and glyphs are plain.
const FAIL_ICON_PLAIN: &str = "err";

/// Resolves the status segment's template variables.
pub struct StatusResolver<'a> {
    success: bool,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    glyphs: Glyphs,
}

impl<'a> StatusResolver<'a> {
    /// Build a resolver for one status pill render.
    #[must_use]
    pub const fn new(success: bool, theme: &'a ThemeConfig, pos: SegmentPosition) -> Self {
        Self {
            success,
            theme,
            pos,
            glyphs: Glyphs::Nerd,
        }
    }

    /// Draw this segment with `glyphs` instead of the default [`Glyphs::Nerd`].
    #[must_use]
    pub const fn with_glyphs(self, glyphs: Glyphs) -> Self {
        Self { glyphs, ..self }
    }

    /// The theme icon for the outcome; an unset icon, or a Nerd Font icon that
    /// [`Glyphs::Plain`] drops, falls back by glyph set, while an icon explicitly
    /// set to the empty string stays empty.
    fn symbol(&self) -> String {
        let status = &self.theme.segments.status;
        let (icon, nerd, plain) = if self.success {
            (&status.ok_icon, OK_ICON_NERD, OK_ICON_PLAIN)
        } else {
            (&status.fail_icon, FAIL_ICON_NERD, FAIL_ICON_PLAIN)
        };
        icon.as_deref()
            .and_then(|configured| self.glyphs.icon(configured))
            .map_or_else(
                || match self.glyphs {
                    Glyphs::Nerd => nerd.to_owned(),
                    Glyphs::Plain => plain.to_owned(),
                },
                str::to_owned,
            )
    }

    /// Background color for the outcome.
    fn bg(&self) -> &str {
        let status = &self.theme.segments.status;
        if self.success {
            status.ok_bg_color.as_str()
        } else {
            status.fail_bg_color.as_str()
        }
    }

    /// Style string: `fg:<text_color> bg:<bg_color>` for the outcome.
    fn style(&self) -> String {
        let status = &self.theme.segments.status;
        let fg = if self.success {
            status.ok_text_color.as_str()
        } else {
            status.fail_text_color.as_str()
        };
        format!("fg:{fg} bg:{}", self.bg())
    }
}

impl VariableResolver for StatusResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "symbol" => Some(self.symbol()),
            "style" => Some(self.style()),
            "bg" => Some(self.bg().to_owned()),
            "sep_gap" | "sep_open" | "sep_close" => {
                separator_var(name, self.pos, SeparatorStyle::Chained, self.glyphs)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "a failed unwrap or assertion is the test failure"
    )]
    #![allow(
        clippy::missing_panics_doc,
        clippy::missing_errors_doc,
        reason = "test functions are not API"
    )]

    use super::StatusResolver;
    use crate::config::types::{ColorSpec, Icon};
    use crate::formatter::separator::Glyphs;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn mid() -> SegmentPosition {
        SegmentPosition::new(IsLast::No, IsFirst::No)
    }

    fn themed() -> ThemeConfig {
        let mut theme = ThemeConfig::default();
        let status = &mut theme.segments.status;
        status.ok_icon = Some(Icon::new("OK").unwrap());
        status.fail_icon = Some(Icon::new("NO").unwrap());
        status.ok_bg_color = ColorSpec::new("cyan").unwrap();
        status.ok_text_color = ColorSpec::new("black").unwrap();
        status.fail_bg_color = ColorSpec::new("magenta").unwrap();
        status.fail_text_color = ColorSpec::new("white").unwrap();
        theme
    }

    #[test]
    fn ok_and_fail_pick_their_own_icon_bg_and_style() {
        let theme = themed();
        let ok = StatusResolver::new(true, &theme, mid());
        assert_eq!(ok.resolve("symbol").unwrap(), "OK");
        assert_eq!(ok.resolve("bg").unwrap(), "cyan");
        assert_eq!(ok.resolve("style").unwrap(), "fg:black bg:cyan");

        let fail = StatusResolver::new(false, &theme, mid());
        assert_eq!(fail.resolve("symbol").unwrap(), "NO");
        assert_eq!(fail.resolve("bg").unwrap(), "magenta");
        assert_eq!(fail.resolve("style").unwrap(), "fg:white bg:magenta");
    }

    #[test]
    fn unset_icon_falls_back_by_glyph_set() {
        let theme = ThemeConfig::default();
        assert!(theme.segments.status.ok_icon.is_none());
        let resolve = |success, glyphs| {
            StatusResolver::new(success, &theme, mid())
                .with_glyphs(glyphs)
                .resolve("symbol")
                .unwrap()
        };
        assert_eq!(resolve(true, Glyphs::Nerd), "✔");
        assert_eq!(resolve(false, Glyphs::Nerd), "✖");
        assert_eq!(resolve(true, Glyphs::Plain), "ok");
        assert_eq!(resolve(false, Glyphs::Plain), "err");
    }

    #[test]
    fn explicitly_empty_icon_stays_empty() {
        let mut theme = ThemeConfig::default();
        theme.segments.status.ok_icon = Some(Icon::new("").unwrap());
        let res = StatusResolver::new(true, &theme, mid());
        assert_eq!(res.resolve("symbol").unwrap(), "");
    }

    #[test]
    fn open_cap_is_absent_only_when_first() {
        let theme = ThemeConfig::default();
        let at = |is_first| {
            StatusResolver::new(true, &theme, SegmentPosition::new(IsLast::No, is_first))
                .resolve("sep_open")
        };
        assert_eq!(at(IsFirst::Yes), None);
        assert_eq!(at(IsFirst::No), Some("\u{e0ba}".to_owned()));
    }

    #[test]
    fn close_cap_and_gap_follow_is_last() {
        let theme = ThemeConfig::default();
        let at = |is_last, name| {
            StatusResolver::new(true, &theme, SegmentPosition::new(is_last, IsFirst::No))
                .resolve(name)
        };
        assert_eq!(at(IsLast::Yes, "sep_close"), Some("\u{e0b4}".to_owned()));
        assert_eq!(at(IsLast::No, "sep_close"), Some("\u{e0bc}".to_owned()));
        assert_eq!(at(IsLast::No, "sep_gap"), Some(" ".to_owned()));
        assert_eq!(at(IsLast::Yes, "sep_gap"), None);
    }

    #[test]
    fn unknown_variable_is_none() {
        let theme = ThemeConfig::default();
        assert_eq!(
            StatusResolver::new(true, &theme, mid()).resolve("nonsuch"),
            None
        );
    }
}
