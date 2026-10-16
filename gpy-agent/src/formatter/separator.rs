//! Powerline separators shared by every segment resolver. Caps are Nerd Font
//! private-use glyphs; [`Glyphs::Plain`] drops them so plain fonts show no tofu.

use crate::config::UiSettings;
use crate::formatter::{IsFirst, IsLast, SegmentPosition};

/// Closing cap drawn by the last segment: a filled half circle.
const CLOSE_LAST: &str = "\u{e0b4}";
/// Closing cap drawn by a non-last segment: a right-pointing triangle.
const CLOSE_NOT_LAST: &str = "\u{e0bc}";
/// Opening cap drawn by a non-first segment: a left half circle.
const OPEN_NOT_FIRST: &str = "\u{e0ba}";
/// Gap emitted after a non-last segment.
const GAP: &str = " ";

/// Separator behavior. `Chained` gets open/gap/close; `Terminal` (adjacent to
/// the prompt symbol) ignores `is_first` and only closes when last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorStyle {
    /// Full chrome: leading cap, trailing cap, and inter-segment gap.
    Chained,
    /// Trailing cap only, and only on the last segment.
    Terminal,
}

/// Which glyph set a segment may draw, derived from `ui.show_icons`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    /// Nerd/Powerline-patched font available: draw the powerline caps.
    Nerd,
    /// Any terminal font: draw no private-use glyphs.
    Plain,
}

impl Glyphs {
    /// The glyph set for a `ui.show_icons` setting.
    #[must_use]
    pub const fn from_show_icons(show_icons: bool) -> Self {
        if show_icons { Self::Nerd } else { Self::Plain }
    }

    /// A theme delimiter as drawn with this glyph set; `Plain` drops private-use text.
    #[must_use]
    pub fn delimiter(self, text: &str) -> &str {
        if self == Self::Plain && text.chars().any(is_private_use) {
            ""
        } else {
            text
        }
    }
}

impl From<&UiSettings> for Glyphs {
    fn from(ui: &UiSettings) -> Self {
        Self::from_show_icons(ui.show_icons)
    }
}

/// Whether `c` is in a Unicode private-use area (where Nerd Font glyphs live).
const fn is_private_use(c: char) -> bool {
    matches!(c, '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{ffffd}' | '\u{100000}'..='\u{10fffd}')
}

/// Resolved separator variables; `None` leaves the `$sep_*` variable unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Separators {
    /// `$sep_gap` — the space that follows a non-last segment.
    pub gap: Option<&'static str>,
    /// `$sep_open` — the leading cap, suppressed on the first segment.
    pub open: Option<&'static str>,
    /// `$sep_close` — the trailing cap.
    pub close: Option<&'static str>,
}

/// Resolve `$sep_gap`/`$sep_open`/`$sep_close` for one position.
/// [`Glyphs::Plain`] nulls the caps but keeps the gap.
#[must_use]
pub const fn resolve_separator(
    pos: SegmentPosition,
    style: SeparatorStyle,
    glyphs: Glyphs,
) -> Separators {
    let nerd = match style {
        SeparatorStyle::Chained => Separators {
            gap: match pos.is_last {
                IsLast::Yes => None,
                IsLast::No => Some(GAP),
            },
            open: match pos.is_first {
                IsFirst::Yes => None,
                IsFirst::No => Some(OPEN_NOT_FIRST),
            },
            close: Some(match pos.is_last {
                IsLast::Yes => CLOSE_LAST,
                IsLast::No => CLOSE_NOT_LAST,
            }),
        },
        SeparatorStyle::Terminal => Separators {
            gap: None,
            open: None,
            close: match pos.is_last {
                IsLast::Yes => Some(CLOSE_LAST),
                IsLast::No => None,
            },
        },
    };
    match glyphs {
        Glyphs::Nerd => nerd,
        Glyphs::Plain => Separators {
            gap: nerd.gap,
            open: None,
            close: None,
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::missing_panics_doc
    )]
    #![allow(clippy::missing_errors_doc)]

    use super::{
        CLOSE_LAST, CLOSE_NOT_LAST, GAP, Glyphs, OPEN_NOT_FIRST, SeparatorStyle, resolve_separator,
    };
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};

    /// Every position.
    fn all_positions() -> [SegmentPosition; 4] {
        [
            SegmentPosition::new(IsLast::No, IsFirst::No),
            SegmentPosition::new(IsLast::No, IsFirst::Yes),
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
            SegmentPosition::new(IsLast::Yes, IsFirst::Yes),
        ]
    }

    #[test]
    fn chained_close_is_half_circle_when_last_and_triangle_otherwise() {
        let last = resolve_separator(
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
            SeparatorStyle::Chained,
            Glyphs::Nerd,
        );
        let not_last = resolve_separator(
            SegmentPosition::new(IsLast::No, IsFirst::No),
            SeparatorStyle::Chained,
            Glyphs::Nerd,
        );
        assert_eq!(last.close, Some(CLOSE_LAST));
        assert_eq!(not_last.close, Some(CLOSE_NOT_LAST));
    }

    #[test]
    fn chained_gap_only_when_not_last() {
        assert_eq!(
            resolve_separator(
                SegmentPosition::new(IsLast::No, IsFirst::No),
                SeparatorStyle::Chained,
                Glyphs::Nerd
            )
            .gap,
            Some(GAP)
        );
        assert_eq!(
            resolve_separator(
                SegmentPosition::new(IsLast::Yes, IsFirst::No),
                SeparatorStyle::Chained,
                Glyphs::Nerd
            )
            .gap,
            None
        );
    }

    #[test]
    fn chained_open_only_when_not_first() {
        assert_eq!(
            resolve_separator(
                SegmentPosition::new(IsLast::No, IsFirst::No),
                SeparatorStyle::Chained,
                Glyphs::Nerd
            )
            .open,
            Some(OPEN_NOT_FIRST)
        );
        assert_eq!(
            resolve_separator(
                SegmentPosition::new(IsLast::No, IsFirst::Yes),
                SeparatorStyle::Chained,
                Glyphs::Nerd
            )
            .open,
            None
        );
    }

    #[test]
    fn terminal_never_yields_gap_or_open() {
        for pos in all_positions() {
            let seps = resolve_separator(pos, SeparatorStyle::Terminal, Glyphs::Nerd);
            assert_eq!(seps.gap, None, "terminal has no sep_gap at {pos:?}");
            assert_eq!(seps.open, None, "terminal has no sep_open at {pos:?}");
        }
    }

    #[test]
    fn terminal_and_chained_close_differ_when_not_last() {
        for pos in all_positions() {
            let terminal = resolve_separator(pos, SeparatorStyle::Terminal, Glyphs::Nerd);
            let chained = resolve_separator(pos, SeparatorStyle::Chained, Glyphs::Nerd);
            match pos.is_last {
                IsLast::Yes => {
                    assert_eq!(terminal.close, Some(CLOSE_LAST));
                    assert_eq!(chained.close, Some(CLOSE_LAST));
                }
                IsLast::No => {
                    assert_eq!(
                        terminal.close, None,
                        "terminal draws no closing cap when not last, at {pos:?}"
                    );
                    assert_eq!(
                        chained.close,
                        Some(CLOSE_NOT_LAST),
                        "chained always draws a closing cap, at {pos:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn terminal_ignores_is_first() {
        for is_last in [IsLast::No, IsLast::Yes] {
            let first = resolve_separator(
                SegmentPosition::new(is_last, IsFirst::Yes),
                SeparatorStyle::Terminal,
                Glyphs::Nerd,
            );
            let not_first = resolve_separator(
                SegmentPosition::new(is_last, IsFirst::No),
                SeparatorStyle::Terminal,
                Glyphs::Nerd,
            );
            assert_eq!(first, not_first);
        }
    }

    #[test]
    fn plain_glyphs_suppress_caps_but_keep_gap() {
        for style in [SeparatorStyle::Chained, SeparatorStyle::Terminal] {
            for pos in all_positions() {
                let plain = resolve_separator(pos, style, Glyphs::Plain);
                let nerd = resolve_separator(pos, style, Glyphs::Nerd);
                assert_eq!(
                    plain.open, None,
                    "plain draws no open cap, {style:?} at {pos:?}"
                );
                assert_eq!(
                    plain.close, None,
                    "plain draws no close cap, {style:?} at {pos:?}"
                );
                assert_eq!(
                    plain.gap, nerd.gap,
                    "gap must not depend on glyphs, {style:?} at {pos:?}"
                );
            }
        }
    }

    #[test]
    fn plain_delimiter_drops_only_private_use_text() {
        for private in [
            "\u{e0b4}",
            "\u{e0ba}",
            "\u{f0467}",
            "\u{100000}",
            " \u{e0bc} ",
        ] {
            assert_eq!(Glyphs::Plain.delimiter(private), "", "{private:?}");
            assert_eq!(Glyphs::Nerd.delimiter(private), private, "{private:?}");
        }
        for kept in ["", " ", "[", "|", "❯", ">", "\u{2588}"] {
            assert_eq!(Glyphs::Plain.delimiter(kept), kept, "{kept:?}");
            assert_eq!(Glyphs::Nerd.delimiter(kept), kept, "{kept:?}");
        }
    }

    #[test]
    fn glyphs_follow_show_icons() {
        let mut ui = crate::config::UiSettings::default();
        assert_eq!(Glyphs::from(&ui), Glyphs::Nerd);
        ui.show_icons = false;
        assert_eq!(Glyphs::from(&ui), Glyphs::Plain);
    }
}
