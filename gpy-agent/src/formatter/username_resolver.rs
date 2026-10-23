//! Maps the username to Starship-exact `username` variables (`username`, `symbol`) plus the
//! powerline variables `style`, `bg`, `sep_open`, `sep_gap` and `sep_close`.
//! (The importer renames Starship's `$user` to `$username`.) `style` and `bg` take the
//! root colors for a root session and the user colors otherwise, as Starship's
//! `style_root`/`style_user`.

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, separator_var};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact username variable names.
pub struct UsernameResolver<'a> {
    username: String,
    is_root: bool,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    glyphs: Glyphs,
}

impl<'a> UsernameResolver<'a> {
    /// Build a resolver for one username segment render; `is_root` picks the colors.
    #[must_use]
    pub const fn new(
        username: String,
        is_root: bool,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        Self {
            username,
            is_root,
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

    /// Background color for the session.
    fn bg(&self) -> &str {
        let username = &self.theme.segments.username;
        if self.is_root {
            username.bg_color.as_str()
        } else {
            username.user_bg_color.as_str()
        }
    }

    /// Style string: `fg:<text color> bg:<bg color>` for the session.
    fn style(&self) -> String {
        let username = &self.theme.segments.username;
        let fg = if self.is_root {
            username.text_color.as_str()
        } else {
            username.user_text_color.as_str()
        };
        format!("fg:{fg} bg:{}", self.bg())
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
            true,
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
            true,
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
            true,
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
                true,
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
            true,
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("nonsuch"), None);
    }

    fn at(is_last: IsLast, is_first: IsFirst, thr: &ThemeConfig) -> UsernameResolver<'_> {
        UsernameResolver::new(
            "root".to_owned(),
            true,
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
    fn style_and_bg_follow_root_or_user_colors() {
        let mut thr = theme();
        let color = |name| crate::config::types::ColorSpec::new(name).unwrap();
        thr.segments.username.bg_color = color("blue");
        thr.segments.username.text_color = color("yellow");
        thr.segments.username.user_bg_color = color("green");
        thr.segments.username.user_text_color = color("black");
        let res = |is_root| {
            UsernameResolver::new(
                "ada".to_owned(),
                is_root,
                &thr,
                SegmentPosition::new(IsLast::No, IsFirst::No),
            )
        };
        assert_eq!(res(true).resolve("style").unwrap(), "fg:yellow bg:blue");
        assert_eq!(res(true).resolve("bg").unwrap(), "blue");
        assert_eq!(res(false).resolve("style").unwrap(), "fg:black bg:green");
        assert_eq!(res(false).resolve("bg").unwrap(), "green");
    }
}
