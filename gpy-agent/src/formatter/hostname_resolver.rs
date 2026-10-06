//! Maps the agent-echoed hostname to Starship-exact template variable names.
//!
//! Variable names: `hostname`, `symbol`, `sep_close`.
//! Compatible with Starship's `hostname` module.
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
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    glyphs: Glyphs,
}

impl<'a> HostnameResolver<'a> {
    /// Build a resolver for one hostname segment render.
    #[must_use]
    pub const fn new(hostname: String, theme: &'a ThemeConfig, pos: SegmentPosition) -> Self {
        Self {
            hostname,
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
            "symbol" => Some(
                self.theme
                    .segments
                    .hostname
                    .icon
                    .as_deref()
                    .unwrap_or("")
                    .to_owned(),
            ),
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

    #[test]
    fn hostname_trimmed_at_default_dot() {
        let thr = theme();
        assert_eq!(thr.segments.hostname.trim_at, ".");
        let res = HostnameResolver::new(
            "host.example.com".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("hostname").unwrap(), "host");
    }

    #[test]
    fn hostname_unchanged_when_trim_at_empty() {
        let mut thr = theme();
        thr.segments.hostname.trim_at = String::new();
        let res = HostnameResolver::new(
            "host.example.com".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("hostname").unwrap(), "host.example.com");
    }

    #[test]
    fn hostname_unchanged_when_trim_at_delimiter_absent() {
        let thr = theme();
        let res = HostnameResolver::new(
            "localhost".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("hostname").unwrap(), "localhost");
    }

    #[test]
    fn symbol_resolved_when_icon_set() {
        let mut thr = theme();
        thr.segments.hostname.icon = Some(crate::config::types::Icon::new("🌐").unwrap());
        let res = HostnameResolver::new(
            "host".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("symbol").unwrap(), "🌐");
    }

    #[test]
    fn symbol_empty_when_icon_unset() {
        let thr = theme();
        assert!(thr.segments.hostname.icon.is_none());
        let res = HostnameResolver::new(
            "host".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("symbol").unwrap(), "");
    }

    #[test]
    fn unknown_variable_is_none() {
        let thr = theme();
        let res = HostnameResolver::new(
            "host".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("nonsuch"), None);
    }

    #[test]
    fn sep_close_some_when_is_last_true() {
        let thr = theme();
        let res = HostnameResolver::new(
            "host".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_none_when_is_last_false() {
        let thr = theme();
        let res = HostnameResolver::new(
            "host".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), None);
    }

    #[test]
    fn style_is_none_for_hostname_segment() {
        let thr = theme();
        let res = HostnameResolver::new(
            "host".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(
            res.resolve("style"),
            None,
            "hostname segment has no $style variable — format strings hardcode literal colors"
        );
    }
}
