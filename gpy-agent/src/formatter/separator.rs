//! The one powerline-separator implementation shared by every segment
//! resolver (#586).
//!
//! Before this module each of the seven resolvers carried its own copy of the
//! `sep_gap`/`sep_open`/`sep_close` logic. Two of those copies were genuinely
//! different from each other, so collapsing them into one *identical* body
//! would have been a behavior change; [`SeparatorStyle`] names the difference
//! instead, and [`resolve_separator`] is the single place the glyphs live.
//!
//! The caps are Powerline-Extra private-use codepoints that only Nerd/Powerline
//! patched fonts carry. [`Glyphs`] says whether the terminal is assumed to have
//! them (`ui.show_icons`, #695); [`Glyphs::Plain`] resolves every cap to nothing
//! so a no-Nerd-Font install draws flat colored blocks instead of tofu boxes.

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

/// Which of the two real separator behaviors a segment uses (#586).
///
/// `Chained` segments (directory/git/language/duration) sit in the middle of
/// the segment chain and need the full open/gap/close chrome. `Terminal`
/// segments (character/hostname/username) are always adjacent to the prompt
/// symbol with no leading transition to render, so they only ever need a
/// closing cap, and only when they actually are the last segment — their
/// formats have no unconditional opening-cap literal to make conditional
/// (see `SCHEMA_EVOLUTION.md`'s `is_first` entry).
///
/// This distinction is behavior, not an accident of copy-paste: `Terminal`
/// ignores [`SegmentPosition::is_first`] entirely and yields `None` for
/// `sep_close` when not last, where `Chained` always yields *some* closing
/// glyph. `terminal_and_chained_close_differ_when_not_last` pins it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorStyle {
    /// Full chrome: leading cap, trailing cap, and inter-segment gap.
    Chained,
    /// Trailing cap only, and only on the last segment.
    Terminal,
}

/// Which glyph set a segment may draw (#695).
///
/// An enum rather than a `bool` for the same reason as [`IsLast`]: a bare
/// `bool` is unreadable at the call site and is easy to swap with the other
/// booleans resolvers already take (`CharacterResolver::new`'s `success`).
/// Derived from `ui.show_icons`.
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

    /// A theme-supplied delimiter as it should be drawn with this glyph set.
    ///
    /// [`Glyphs::Plain`] drops delimiter text containing a private-use
    /// codepoint (the default theme's Nerd caps) and keeps plain text such as
    /// a custom `[`/`]` delimiter, which renders on any font.
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

/// Whether `c` sits in one of Unicode's three private-use areas, where Nerd
/// Font glyphs live (BMP U+E000–F8FF for the powerline caps, the
/// supplementary planes for the Material Design range).
const fn is_private_use(c: char) -> bool {
    matches!(c, '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{ffffd}' | '\u{100000}'..='\u{10fffd}')
}

/// Resolved separator template variables for one segment position.
///
/// `None` means the template variable resolves to nothing at this position;
/// resolvers surface that as an unresolved `$sep_*` variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Separators {
    /// `$sep_gap` — the space that follows a non-last segment.
    pub gap: Option<&'static str>,
    /// `$sep_open` — the leading cap, suppressed on the first segment.
    pub open: Option<&'static str>,
    /// `$sep_close` — the trailing cap.
    pub close: Option<&'static str>,
}

/// Resolve `$sep_gap`/`$sep_open`/`$sep_close` for one segment position.
///
/// See [`SeparatorStyle`] for why there are two behaviors sharing this one
/// function rather than one literal behavior. With [`Glyphs::Plain`] the caps
/// (`open`/`close`) are `None` and `gap` is unchanged, so segments stay
/// separated by a space.
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
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::{
        CLOSE_LAST, CLOSE_NOT_LAST, GAP, Glyphs, OPEN_NOT_FIRST, SeparatorStyle, resolve_separator,
    };
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};

    /// Every position, in the order `SEGMENT_POSITIONS` uses.
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

    /// #586: character/hostname/username expose *only* `sep_close`. This is
    /// the decision the acceptance criteria asked to pin — a future "cleanup"
    /// that folds `Terminal` into `Chained` fails here.
    #[test]
    fn terminal_never_yields_gap_or_open() {
        for pos in all_positions() {
            let seps = resolve_separator(pos, SeparatorStyle::Terminal, Glyphs::Nerd);
            assert_eq!(seps.gap, None, "terminal has no sep_gap at {pos:?}");
            assert_eq!(seps.open, None, "terminal has no sep_open at {pos:?}");
        }
    }

    /// #586: `Terminal`'s `sep_close` is conditional where `Chained`'s is not.
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

    /// `Terminal` ignores `is_first` outright.
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

    /// #695: with plain glyphs no position of either style draws a cap, but
    /// the inter-segment gap survives so segments stay visually separated.
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

    /// #695: `Glyphs::Plain` drops private-use delimiter text in all three
    /// Unicode private-use areas and keeps everything else verbatim.
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
