//! Maps the hostname to Starship-exact `hostname` variables (`hostname`, `symbol`)
//! plus the powerline variables `style`, `bg`, `sep_open`, `sep_gap` and `sep_close`.
//!
//! `symbol` is Starship's `ssh_symbol`: the theme icon only over SSH.

use crate::config::Config;
use crate::formatter::SegmentPosition;
use crate::formatter::chrome::SegmentChrome;
use crate::formatter::separator::Glyphs;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact hostname variable names.
pub struct HostnameResolver<'a> {
    hostname: String,
    is_ssh: bool,
    theme: &'a ThemeConfig,
    chrome: SegmentChrome<'a>,
}

impl<'a> HostnameResolver<'a> {
    /// Build a resolver for one hostname render; `is_ssh` gates `symbol`.
    #[must_use]
    pub fn new(
        hostname: String,
        is_ssh: bool,
        config: &Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        let colors = &theme.segments.hostname;
        Self {
            hostname,
            is_ssh,
            theme,
            chrome: SegmentChrome::pill(
                pos,
                Glyphs::from(&config.ui),
                colors.text_color.as_str(),
                colors.bg_color.as_str(),
            ),
        }
    }

    /// The hostname trimmed at the first occurrence of `trim_at`; unchanged
    /// when `trim_at` is empty or not found in the hostname.
    fn trimmed_hostname(&self) -> String {
        let trim_at = self.theme.segments.hostname.trim_at.as_str();
        if trim_at.is_empty() {
            return self.hostname.clone();
        }
        self.hostname
            .split_once(trim_at)
            .map_or_else(|| self.hostname.clone(), |(short, _rest)| short.to_owned())
    }
}

impl VariableResolver for HostnameResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "hostname" => Some(self.trimmed_hostname()),
            "symbol" => Some(if self.is_ssh {
                self.theme
                    .segments
                    .hostname
                    .icon
                    .as_deref()
                    .and_then(|icon| self.chrome.glyphs().icon(icon))
                    .unwrap_or("")
                    .to_owned()
            } else {
                String::new()
            }),
            _ => self.chrome.resolve(name),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::HostnameResolver;
    use crate::config::Config;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn theme() -> ThemeConfig {
        ThemeConfig::default()
    }

    fn icons(show_icons: bool) -> Config {
        let mut config = Config::default();
        config.ui.show_icons = show_icons;
        config
    }

    /// A local (non-SSH), middle-of-prompt resolver for `hostname`.
    fn local<'a>(hostname: &str, config: &Config, thr: &'a ThemeConfig) -> HostnameResolver<'a> {
        HostnameResolver::new(
            hostname.to_owned(),
            false,
            config,
            thr,
            SegmentPosition::MIDDLE,
        )
    }

    #[test]
    fn hostname_trimmed_at_default_dot() {
        let (config, thr) = (Config::default(), theme());
        assert_eq!(thr.segments.hostname.trim_at, ".");
        assert_eq!(
            local("host.example.com", &config, &thr)
                .resolve("hostname")
                .unwrap(),
            "host"
        );
    }

    #[test]
    fn hostname_unchanged_when_trim_at_empty() {
        let (config, mut thr) = (Config::default(), theme());
        thr.segments.hostname.trim_at = String::new();
        assert_eq!(
            local("host.example.com", &config, &thr)
                .resolve("hostname")
                .unwrap(),
            "host.example.com"
        );
    }

    #[test]
    fn hostname_unchanged_when_trim_at_delimiter_absent() {
        let (config, thr) = (Config::default(), theme());
        assert_eq!(
            local("localhost", &config, &thr)
                .resolve("hostname")
                .unwrap(),
            "localhost"
        );
    }

    #[test]
    fn symbol_matrix_is_ssh_by_icon() {
        let config = Config::default();
        let no_icon = theme();
        assert!(no_icon.segments.hostname.icon.is_none());
        let mut with_icon = theme();
        with_icon.segments.hostname.icon = Some(crate::config::types::Icon::new("🌐 ").unwrap());

        let cases = [
            (true, &with_icon, "🌐 "),
            (false, &with_icon, ""),
            (true, &no_icon, ""),
            (false, &no_icon, ""),
        ];
        for (is_ssh, thr, expected) in cases {
            let res = HostnameResolver::new(
                "host".to_owned(),
                is_ssh,
                &config,
                thr,
                SegmentPosition::MIDDLE,
            );
            assert_eq!(
                res.resolve("symbol").unwrap(),
                expected,
                "is_ssh={is_ssh}, icon={:?}",
                thr.segments.hostname.icon
            );
        }
    }

    #[test]
    fn symbol_drops_private_use_icon_with_plain_glyphs() {
        let mut thr = theme();
        thr.segments.hostname.icon = Some(crate::config::types::Icon::new("\u{f108}").unwrap());
        let res = |show_icons| {
            HostnameResolver::new(
                "host".to_owned(),
                true,
                &icons(show_icons),
                &thr,
                SegmentPosition::MIDDLE,
            )
            .resolve("symbol")
            .unwrap()
        };
        assert_eq!(res(true), "\u{f108}");
        assert_eq!(res(false), "");
    }

    #[test]
    fn unknown_variable_is_none() {
        let (config, thr) = (Config::default(), theme());
        assert_eq!(local("host", &config, &thr).resolve("nonsuch"), None);
    }

    fn at(is_last: IsLast, is_first: IsFirst, name: &str) -> Option<String> {
        let (config, thr) = (Config::default(), theme());
        HostnameResolver::new(
            "host".to_owned(),
            false,
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
    fn plain_glyphs_drop_the_caps() {
        let (config, thr) = (icons(false), theme());
        let res = local("host", &config, &thr);
        assert_eq!(res.resolve("sep_open"), None);
        assert_eq!(res.resolve("sep_close"), None);
    }

    #[test]
    fn style_and_bg_come_from_the_hostname_theme_colors() {
        let (config, mut thr) = (Config::default(), theme());
        thr.segments.hostname.bg_color = crate::config::types::ColorSpec::new("blue").unwrap();
        thr.segments.hostname.text_color = crate::config::types::ColorSpec::new("yellow").unwrap();
        let res = local("host", &config, &thr);
        assert_eq!(res.resolve("style").unwrap(), "fg:yellow bg:blue");
        assert_eq!(res.resolve("bg").unwrap(), "blue");
    }
}
