//! The variables every segment resolver draws around its own content:
//! `$style`, `$bg` and the powerline caps `$sep_open`, `$sep_gap`, `$sep_close`.

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, separator_var};

/// One segment's position, glyph set and colors, resolved to its chrome variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SegmentChrome<'a> {
    pos: SegmentPosition,
    glyphs: Glyphs,
    separators: SeparatorStyle,
    fg: &'a str,
    /// `None` for a foreground-only segment, whose `$bg` stays unresolved.
    bg: Option<&'a str>,
    /// Style attributes (e.g. `bold`) leading `$style`; empty for none.
    attrs: &'a str,
}

impl<'a> SegmentChrome<'a> {
    /// A pill in the segment chain: `fg` on `bg`, with chained caps.
    #[must_use]
    pub(crate) const fn pill(
        pos: SegmentPosition,
        glyphs: Glyphs,
        fg: &'a str,
        bg: &'a str,
    ) -> Self {
        Self {
            pos,
            glyphs,
            separators: SeparatorStyle::Chained,
            fg,
            bg: Some(bg),
            attrs: "",
        }
    }

    /// The prompt symbol: foreground only, with a closing cap only when last.
    #[must_use]
    pub(crate) const fn terminal(pos: SegmentPosition, glyphs: Glyphs, fg: &'a str) -> Self {
        Self {
            pos,
            glyphs,
            separators: SeparatorStyle::Terminal,
            fg,
            bg: None,
            attrs: "",
        }
    }

    /// Lead `$style` with the style attributes `attrs`.
    #[must_use]
    pub(crate) const fn with_attrs(self, attrs: &'a str) -> Self {
        Self { attrs, ..self }
    }

    /// The glyph set this segment draws with, for its own icons.
    #[must_use]
    pub(crate) const fn glyphs(&self) -> Glyphs {
        self.glyphs
    }

    /// `$style`, `$bg` and `$sep_*`; `None` for every other name.
    #[must_use]
    pub(crate) fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "style" => Some(self.style()),
            "bg" => self.bg.map(str::to_owned),
            "sep_gap" | "sep_open" | "sep_close" => {
                separator_var(name, self.pos, self.separators, self.glyphs)
            }
            _ => None,
        }
    }

    /// `[<attrs> ]fg:<fg>[ bg:<bg>]`.
    fn style(&self) -> String {
        let attrs_gap = if self.attrs.is_empty() { "" } else { " " };
        self.bg.map_or_else(
            || format!("{}{attrs_gap}fg:{}", self.attrs, self.fg),
            |bg| format!("{}{attrs_gap}fg:{} bg:{bg}", self.attrs, self.fg),
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a failed unwrap is the test failure")]

    use super::SegmentChrome;
    use crate::formatter::SegmentPosition;
    use crate::formatter::separator::Glyphs;

    #[test]
    fn pill_resolves_style_bg_and_chained_caps() {
        let chrome = SegmentChrome::pill(SegmentPosition::MIDDLE, Glyphs::Nerd, "white", "blue");
        assert_eq!(chrome.resolve("style").unwrap(), "fg:white bg:blue");
        assert_eq!(chrome.resolve("bg").unwrap(), "blue");
        assert_eq!(chrome.resolve("sep_open").unwrap(), "\u{e0ba}");
        assert_eq!(chrome.resolve("sep_gap").unwrap(), " ");
        assert_eq!(chrome.resolve("sep_close").unwrap(), "\u{e0bc}");
        assert_eq!(chrome.resolve("symbol"), None);
    }

    #[test]
    fn terminal_has_no_bg_and_closes_only_when_last() {
        let middle = SegmentChrome::terminal(SegmentPosition::MIDDLE, Glyphs::Nerd, "green");
        assert_eq!(middle.resolve("style").unwrap(), "fg:green");
        assert_eq!(middle.resolve("bg"), None);
        assert_eq!(middle.resolve("sep_close"), None);
        let last = SegmentChrome::terminal(SegmentPosition::LAST, Glyphs::Nerd, "green");
        assert_eq!(last.resolve("sep_close").unwrap(), "\u{e0b4}");
    }

    #[test]
    fn attrs_lead_the_style() {
        let chrome = SegmentChrome::pill(SegmentPosition::MIDDLE, Glyphs::Nerd, "white", "blue")
            .with_attrs("bold");
        assert_eq!(chrome.resolve("style").unwrap(), "bold fg:white bg:blue");
    }

    #[test]
    fn plain_glyphs_drop_the_caps_and_keep_the_gap() {
        let chrome = SegmentChrome::pill(SegmentPosition::MIDDLE, Glyphs::Plain, "white", "blue");
        assert_eq!(chrome.resolve("sep_open"), None);
        assert_eq!(chrome.resolve("sep_close"), None);
        assert_eq!(chrome.resolve("sep_gap").unwrap(), " ");
    }
}
