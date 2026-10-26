//! Maps the username to Starship-exact `username` variables (`username`, `symbol`) plus the
//! powerline variables `style`, `bg`, `sep_open`, `sep_gap` and `sep_close`.
//! (The importer renames Starship's `$user` to `$username`.) `style` and `bg` take the
//! root colors for a root session and the user colors otherwise, as Starship's
//! `style_root`/`style_user`.

use crate::config::Config;
use crate::formatter::SegmentPosition;
use crate::formatter::chrome::SegmentChrome;
use crate::formatter::separator::Glyphs;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact username variable names.
pub struct UsernameResolver<'a> {
    username: String,
    theme: &'a ThemeConfig,
    chrome: SegmentChrome<'a>,
}

impl<'a> UsernameResolver<'a> {
    /// Build a resolver for one username segment render; `is_root` picks the colors.
    #[must_use]
    pub fn new(
        username: String,
        is_root: bool,
        config: &Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        let colors = &theme.segments.username;
        let (fg, bg) = if is_root {
            (colors.text_color.as_str(), colors.bg_color.as_str())
        } else {
            (
                colors.user_text_color.as_str(),
                colors.user_bg_color.as_str(),
            )
        };
        Self {
            username,
            theme,
            chrome: SegmentChrome::pill(pos, Glyphs::from(&config.ui), fg, bg),
        }
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
                    .and_then(|icon| self.chrome.glyphs().icon(icon))
                    .unwrap_or("")
                    .to_owned(),
            ),
            _ => self.chrome.resolve(name),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::UsernameResolver;
    use crate::config::Config;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn theme() -> ThemeConfig {
        ThemeConfig::default()
    }

    fn root<'a>(config: &Config, thr: &'a ThemeConfig) -> UsernameResolver<'a> {
        UsernameResolver::new(
            "root".to_owned(),
            true,
            config,
            thr,
            SegmentPosition::MIDDLE,
        )
    }

    #[test]
    fn username_resolved_verbatim() {
        let (config, thr) = (Config::default(), theme());
        assert_eq!(root(&config, &thr).resolve("username").unwrap(), "root");
    }

    #[test]
    fn symbol_resolved_when_icon_set() {
        let (config, mut thr) = (Config::default(), theme());
        thr.segments.username.icon = Some(crate::config::types::Icon::new("⚡").unwrap());
        assert_eq!(root(&config, &thr).resolve("symbol").unwrap(), "⚡");
    }

    #[test]
    fn symbol_empty_when_icon_unset() {
        let (config, thr) = (Config::default(), theme());
        assert!(thr.segments.username.icon.is_none());
        assert_eq!(root(&config, &thr).resolve("symbol").unwrap(), "");
    }

    #[test]
    fn symbol_drops_private_use_icon_with_plain_glyphs() {
        let mut thr = theme();
        thr.segments.username.icon = Some(crate::config::types::Icon::new("\u{f007}").unwrap());
        let res = |show_icons| {
            let mut config = Config::default();
            config.ui.show_icons = show_icons;
            root(&config, &thr).resolve("symbol").unwrap()
        };
        assert_eq!(res(true), "\u{f007}");
        assert_eq!(res(false), "");
    }

    #[test]
    fn unknown_variable_is_none() {
        let (config, thr) = (Config::default(), theme());
        assert_eq!(root(&config, &thr).resolve("nonsuch"), None);
    }

    fn at(is_last: IsLast, is_first: IsFirst, name: &str) -> Option<String> {
        let (config, thr) = (Config::default(), theme());
        UsernameResolver::new(
            "root".to_owned(),
            true,
            &config,
            &thr,
            SegmentPosition::new(is_last, is_first),
        )
        .resolve(name)
    }

    #[test]
    fn sep_close_is_half_circle_when_last_and_triangle_otherwise() {
        assert_eq!(
            at(IsLast::Yes, IsFirst::No, "sep_close"),
            Some("\u{e0b4}".to_owned())
        );
        assert_eq!(
            at(IsLast::No, IsFirst::No, "sep_close"),
            Some("\u{e0bc}".to_owned())
        );
    }

    #[test]
    fn sep_open_absent_when_first_present_otherwise() {
        assert_eq!(at(IsLast::No, IsFirst::Yes, "sep_open"), None);
        assert_eq!(
            at(IsLast::No, IsFirst::No, "sep_open"),
            Some("\u{e0ba}".to_owned())
        );
    }

    #[test]
    fn sep_gap_only_when_not_last() {
        assert_eq!(at(IsLast::No, IsFirst::No, "sep_gap"), Some(" ".to_owned()));
        assert_eq!(at(IsLast::Yes, IsFirst::No, "sep_gap"), None);
    }

    #[test]
    fn style_and_bg_follow_root_or_user_colors() {
        let (config, mut thr) = (Config::default(), theme());
        let color = |name| crate::config::types::ColorSpec::new(name).unwrap();
        thr.segments.username.bg_color = color("blue");
        thr.segments.username.text_color = color("yellow");
        thr.segments.username.user_bg_color = color("green");
        thr.segments.username.user_text_color = color("black");
        let res = |is_root| {
            UsernameResolver::new(
                "ada".to_owned(),
                is_root,
                &config,
                &thr,
                SegmentPosition::MIDDLE,
            )
        };
        assert_eq!(res(true).resolve("style").unwrap(), "fg:yellow bg:blue");
        assert_eq!(res(true).resolve("bg").unwrap(), "blue");
        assert_eq!(res(false).resolve("style").unwrap(), "fg:black bg:green");
        assert_eq!(res(false).resolve("bg").unwrap(), "green");
    }
}
