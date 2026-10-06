//! Maps the agent-echoed hostname to Starship-exact template variable names.
//!
//! Variable names: `hostname`, `symbol`, `sep_close`.
//! Compatible with Starship's `hostname` module: `symbol` is Starship's
//! `ssh_symbol`, so it resolves to the theme icon only when the client reports
//! an SSH session (#826) and to an empty string otherwise.
//!
//! Note: no `style` variable — the starship-preset format string carries its
//! own literal style span, the same way the directory/duration/git presets
//! hardcode literal colors rather than referencing `$style`.

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, resolve_separator};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact hostname variable names.
pub struct HostnameResolver<'a> {
    hostname: String,
    is_ssh: bool,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    glyphs: Glyphs,
}

impl<'a> HostnameResolver<'a> {
    /// Build a resolver for one hostname segment render; `is_ssh` is the
    /// client-reported SSH session state that gates `symbol`.
    #[must_use]
    pub const fn new(
        hostname: String,
        is_ssh: bool,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        Self {
            hostname,
            is_ssh,
            theme,
            pos,
            glyphs: Glyphs::Nerd,
        }
    }

    /// Draw this segment with `glyphs` instead of the default [`Glyphs::Nerd`];
    /// callers holding a `Config` pass `Glyphs::from(&config.ui)` (#695).
    #[must_use]
    pub fn with_glyphs(self, glyphs: Glyphs) -> Self {
        Self { glyphs, ..self }
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
                    .unwrap_or("")
                    .to_owned()
            } else {
                String::new()
            }),
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

    use super::HostnameResolver;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn theme() -> ThemeConfig {
        ThemeConfig::default()
    }

    /// A local (non-SSH), middle-of-prompt resolver for `hostname`.
    fn local<'a>(hostname: &str, thr: &'a ThemeConfig) -> HostnameResolver<'a> {
        HostnameResolver::new(
            hostname.to_owned(),
            false,
            thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        )
    }

    #[test]
    fn hostname_trimmed_at_default_dot() {
        let thr = theme();
        assert_eq!(thr.segments.hostname.trim_at, ".");
        assert_eq!(
            local("host.example.com", &thr).resolve("hostname").unwrap(),
            "host"
        );
    }

    #[test]
    fn hostname_unchanged_when_trim_at_empty() {
        let mut thr = theme();
        thr.segments.hostname.trim_at = String::new();
        assert_eq!(
            local("host.example.com", &thr).resolve("hostname").unwrap(),
            "host.example.com"
        );
    }

    #[test]
    fn hostname_unchanged_when_trim_at_delimiter_absent() {
        let thr = theme();
        assert_eq!(
            local("localhost", &thr).resolve("hostname").unwrap(),
            "localhost"
        );
    }

    /// #826: `symbol` is Starship's `ssh_symbol` — the icon only over SSH,
    /// empty locally, and empty whenever no icon is configured.
    #[test]
    fn symbol_matrix_is_ssh_by_icon() {
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
                thr,
                SegmentPosition::new(IsLast::No, IsFirst::No),
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
    fn unknown_variable_is_none() {
        let thr = theme();
        assert_eq!(local("host", &thr).resolve("nonsuch"), None);
    }

    #[test]
    fn sep_close_some_when_is_last_true() {
        let thr = theme();
        let res = HostnameResolver::new(
            "host".to_owned(),
            false,
            &thr,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_none_when_is_last_false() {
        let thr = theme();
        assert_eq!(local("host", &thr).resolve("sep_close"), None);
    }

    #[test]
    fn style_is_none_for_hostname_segment() {
        let thr = theme();
        assert_eq!(
            local("host", &thr).resolve("style"),
            None,
            "hostname segment has no $style variable — format strings hardcode literal colors"
        );
    }
}
