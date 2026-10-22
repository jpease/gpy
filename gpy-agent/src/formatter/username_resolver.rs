//! Maps the username to Starship-exact `username` variables (`username`, `symbol`) plus the
//! powerline variables `style`, `bg`, `sep_open`, `sep_gap` and `sep_close`.
//! (The importer renames Starship's `$user` to `$username`.)

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, separator_var};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact username variable names.
pub struct UsernameResolver<'a> {
    username: String,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    glyphs: Glyphs,
}

impl<'a> UsernameResolver<'a> {
    /// Build a resolver for one username segment render.
    #[must_use]
    pub const fn new(username: String, theme: &'a ThemeConfig, pos: SegmentPosition) -> Self {
        Self {
            username,
            theme,
            pos,
            glyphs: Glyphs::Nerd,
        }
    }

    /// Draw this segment with `glyphs` instead of the default [`Glyphs::Nerd`].
    #[must_use]
    pub fn with_glyphs(self, glyphs: Glyphs) -> Self {
        Self { glyphs, ..self }
    }

    /// Style string: `fg:<text_color> bg:<bg_color>`.
    fn style(&self) -> String {
        let fg = self.theme.segments.username.text_color.as_str();
        let bg = self.theme.segments.username.bg_color.as_str();
        format!("fg:{fg} bg:{bg}")
    }
}

impl VariableResolver for UsernameResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "username" => Some(self.username.clone()),
            "symbol" => Some(
                self.theme
                    .segments
                    .username
                    .icon
                    .as_deref()
                    .and_then(|icon| self.glyphs.icon(icon))
                    .unwrap_or("")
                    .to_owned(),
            ),
            "style" => Some(self.style()),
            "bg" => Some(self.theme.segments.username.bg_color.as_str().to_owned()),
            "sep_gap" | "sep_open" | "sep_close" => {
                separator_var(name, self.pos, SeparatorStyle::Chained, self.glyphs)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::UsernameResolver;
    use crate::formatter::separator::Glyphs;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn theme() -> ThemeConfig {
        ThemeConfig::default()
    }

    #[test]
    fn username_resolved_verbatim() {
        let thr = theme();
        let res = UsernameResolver::new(
            "root".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("username").unwrap(), "root");
    }

    #[test]
    fn symbol_resolved_when_icon_set() {
        let mut thr = theme();
        thr.segments.username.icon = Some(crate::config::types::Icon::new("⚡").unwrap());
        let res = UsernameResolver::new(
            "root".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("symbol").unwrap(), "⚡");
    }

    #[test]
    fn symbol_empty_when_icon_unset() {
        let thr = theme();
        assert!(thr.segments.username.icon.is_none());
        let res = UsernameResolver::new(
            "root".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("symbol").unwrap(), "");
    }

    #[test]
    fn symbol_drops_private_use_icon_with_plain_glyphs() {
        let mut thr = theme();
        thr.segments.username.icon = Some(crate::config::types::Icon::new("\u{f007}").unwrap());
        let res = |glyphs| {
            UsernameResolver::new(
                "root".to_owned(),
                &thr,
                SegmentPosition::new(IsLast::No, IsFirst::No),
            )
            .with_glyphs(glyphs)
            .resolve("symbol")
            .unwrap()
        };
        assert_eq!(res(Glyphs::Nerd), "\u{f007}");
        assert_eq!(res(Glyphs::Plain), "");
    }

    #[test]
    fn unknown_variable_is_none() {
        let thr = theme();
        let res = UsernameResolver::new(
            "root".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("nonsuch"), None);
    }

    fn at(is_last: IsLast, is_first: IsFirst, thr: &ThemeConfig) -> UsernameResolver<'_> {
        UsernameResolver::new(
            "root".to_owned(),
            thr,
            SegmentPosition::new(is_last, is_first),
        )
    }

    #[test]
    fn sep_close_is_half_circle_when_last_and_triangle_otherwise() {
        let thr = theme();
        assert_eq!(
            at(IsLast::Yes, IsFirst::No, &thr).resolve("sep_close"),
            Some("\u{e0b4}".to_owned())
        );
        assert_eq!(
            at(IsLast::No, IsFirst::No, &thr).resolve("sep_close"),
            Some("\u{e0bc}".to_owned())
        );
    }

    #[test]
    fn sep_open_absent_when_first_present_otherwise() {
        let thr = theme();
        assert_eq!(at(IsLast::No, IsFirst::Yes, &thr).resolve("sep_open"), None);
        assert_eq!(
            at(IsLast::No, IsFirst::No, &thr).resolve("sep_open"),
            Some("\u{e0ba}".to_owned())
        );
    }

    #[test]
    fn sep_gap_only_when_not_last() {
        let thr = theme();
        assert_eq!(
            at(IsLast::No, IsFirst::No, &thr).resolve("sep_gap"),
            Some(" ".to_owned())
        );
        assert_eq!(at(IsLast::Yes, IsFirst::No, &thr).resolve("sep_gap"), None);
    }

    #[test]
    fn style_and_bg_come_from_the_username_theme_colors() {
        let mut thr = theme();
        thr.segments.username.bg_color = crate::config::types::ColorSpec::new("blue").unwrap();
        thr.segments.username.text_color = crate::config::types::ColorSpec::new("yellow").unwrap();
        let res = at(IsLast::No, IsFirst::No, &thr);
        assert_eq!(res.resolve("style").unwrap(), "fg:yellow bg:blue");
        assert_eq!(res.resolve("bg").unwrap(), "blue");
    }
}
