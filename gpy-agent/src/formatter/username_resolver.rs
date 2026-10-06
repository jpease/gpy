//! Maps the client-supplied username to Starship-exact template variable names.
//!
//! Variable names: `username`, `symbol`, `sep_close`.
//! Compatible with Starship's `username` module (whose content variable is
//! `$user`; the importer renames `$user` -> `$username` on import).
//!
//! Note: no `style` variable — the starship-preset format string carries its
//! own literal style span (`(bold red)`), the same way the hostname/directory/
//! duration presets hardcode literal colors rather than referencing `$style`.

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, resolve_separator};
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

    /// Draw this segment with `glyphs` instead of the default [`Glyphs::Nerd`];
    /// callers holding a `Config` pass `Glyphs::from(&config.ui)` (#695).
    #[must_use]
    pub fn with_glyphs(self, glyphs: Glyphs) -> Self {
        Self { glyphs, ..self }
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

    use super::UsernameResolver;
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
    fn unknown_variable_is_none() {
        let thr = theme();
        let res = UsernameResolver::new(
            "root".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("nonsuch"), None);
    }

    #[test]
    fn sep_close_some_when_is_last_true() {
        let thr = theme();
        let res = UsernameResolver::new(
            "root".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_none_when_is_last_false() {
        let thr = theme();
        let res = UsernameResolver::new(
            "root".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), None);
    }

    #[test]
    fn style_is_none_for_username_segment() {
        let thr = theme();
        let res = UsernameResolver::new(
            "root".to_owned(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(
            res.resolve("style"),
            None,
            "username segment has no $style variable — format strings hardcode literal colors"
        );
    }
}
