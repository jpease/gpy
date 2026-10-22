//! Theme colors resolved to the spelling each shell's own renderer draws.

use crate::config::validation::colors::{THEME_ONLY_COLORS, canonical_color};
use crate::shell::Shell;
use crate::template::{Color, Palette, parse_color};
use std::borrow::Cow;

/// Fish `set_color` names of ANSI indices 0–15; names keep the terminal's own palette.
const FISH_ANSI16_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brblack",
    "brred",
    "brgreen",
    "bryellow",
    "brblue",
    "brmagenta",
    "brcyan",
    "brwhite",
];

/// An ANSI index as Fish's `set_color`, which rejects bare indices, accepts it:
/// 0–15 by name, 16–255 as the xterm-256 hex.
#[allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "the match arms bound every step: 0..=15 indexes the 16 names, cube < 216 keeps each CUBE_LEVELS index < 6, and 232..=255 keeps the gray level at most 238"
)]
fn fish_ansi256_color(index: u8) -> String {
    const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => FISH_ANSI16_NAMES[usize::from(index)].to_owned(),
        16..=231 => {
            let cube = usize::from(index - 16);
            let (red, green, blue) = (
                CUBE_LEVELS[cube / 36],
                CUBE_LEVELS[cube / 6 % 6],
                CUBE_LEVELS[cube % 6],
            );
            format!("#{red:02x}{green:02x}{blue:02x}")
        }
        232..=255 => {
            let level = 8 + 10 * (index - 232);
            format!("#{level:02x}{level:02x}{level:02x}")
        }
    }
}

/// Theme colors in the spelling the shell's own renderer draws. Names resolve
/// through the active palette by [`Palette::lookup`], the rule agent-rendered
/// segments use; ANSI indices keep the terminal's palette, and for Fish become
/// `set_color` values.
#[derive(Clone, Copy)]
pub(super) struct ShellColors<'a> {
    pub(super) shell: Shell,
    pub(super) palette: &'a Palette,
}

impl ShellColors<'_> {
    /// `value` is a canonical theme color (see [`canonical_color`]).
    pub(super) fn resolve(self, value: &str) -> Cow<'_, str> {
        // `match_bg`/`match_text` are shell keywords, never palette names.
        let looked_up = if THEME_ONLY_COLORS.contains(&value) {
            None
        } else {
            parse_color(value)
                .ok()
                .and_then(|color| self.palette.lookup(&color))
                .and_then(|color| palette_color_value(&color))
        };
        let resolved = looked_up.map_or(Cow::Borrowed(value), Cow::Owned);
        match resolved.parse::<u8>() {
            Ok(index) if self.shell == Shell::Fish => Cow::Owned(fish_ansi256_color(index)),
            _ => resolved,
        }
    }
}

/// The canonical theme-color spelling of a resolved palette entry.
fn palette_color_value(color: &Color) -> Option<String> {
    match color {
        Color::Named(name) => canonical_color(name),
        Color::Rgb { r, g, b } => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Ansi256(index) => Some(index.to_string()),
        Color::Palette(_) | Color::PrevFg | Color::PrevBg => None,
    }
}
