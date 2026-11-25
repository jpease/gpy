//! How many colours the terminal shows, and fitting 24-bit SGR to it.
//!
//! The encoder always writes 24-bit colour, so the agent's instant cache and
//! its replies never depend on the terminal that started it (#1265). What
//! prints to a terminal fits its whole output to that terminal once, with
//! [`downgrade_sgr`]: `gpy prompt` and `gpy-agent oneshot`. The wizard's
//! preview fits its colours with the same maths. The conversions match
//! `tests/fixtures/color_depth_vectors.tsv`.

use super::style_encoder::{push_separator, push_u8};
use std::borrow::Cow;
use std::sync::LazyLock;

/// How many colors the terminal can show. Colors beyond it are approximated;
/// the default is 24-bit, so output only changes on an override or on positive
/// evidence of a limited terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorDepth {
    /// 24-bit color: hex colors are emitted as `38;2;r;g;b`.
    #[default]
    TrueColor,
    /// The xterm 256-color palette (`38;5;n`).
    Ansi256,
    /// The 16 ANSI colors (`30-37`, `90-97`).
    Ansi16,
    /// No color codes at all; text attributes (bold, ...) are kept.
    Mono,
}

impl ColorDepth {
    /// The depth for this process, read from the environment once.
    #[must_use]
    pub fn detect() -> Self {
        static DEPTH: LazyLock<ColorDepth> = LazyLock::new(|| {
            ColorDepth::resolve(
                crate::env_config::COLOR_DEPTH.color_depth(),
                std::env::var("TERM_PROGRAM").ok().as_deref(),
                std::env::var("TERM").ok().as_deref(),
            )
        });
        // Unit tests render against the default, whatever terminal runs them.
        if cfg!(test) {
            return Self::TrueColor;
        }
        *DEPTH
    }

    /// `depth` (`GPY_COLOR_DEPTH`) when set, else 256 colors on a terminal
    /// known to lack 24-bit color (`TERM_PROGRAM=Apple_Terminal`,
    /// `TERM=linux`), else 24-bit. Absence of `COLORTERM` proves nothing.
    fn resolve(depth: Option<Self>, term_program: Option<&str>, term: Option<&str>) -> Self {
        depth.unwrap_or(
            if term_program == Some("Apple_Terminal") || term == Some("linux") {
                Self::Ansi256
            } else {
                Self::TrueColor
            },
        )
    }

    /// A depth name (`truecolor`/`24bit`, `256`, `16`, `none`), case-insensitive.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" => Some(Self::TrueColor),
            "256" => Some(Self::Ansi256),
            "16" => Some(Self::Ansi16),
            "none" => Some(Self::Mono),
            _ => None,
        }
    }

    /// The name [`Self::parse`] reads back.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::TrueColor => "truecolor",
            Self::Ansi256 => "256",
            Self::Ansi16 => "16",
            Self::Mono => "none",
        }
    }
}

/// `text` with the colours of its SGR sequences (`ESC[…m`) fitted to `depth`:
/// `38;2;r;g;b`/`48;2;r;g;b` become the nearest 256-colour index or ANSI
/// colour, `38;5;n`/`48;5;n` the nearest ANSI colour, and under
/// [`ColorDepth::Mono`] every colour parameter goes, and with it a sequence
/// left without parameters (`ESC[m` would be a reset). Attributes, resets,
/// other escapes and the text are untouched. Borrowed, without a scan, at
/// [`ColorDepth::TrueColor`], and when `text` holds no escape.
#[must_use]
pub fn downgrade_sgr(text: &str, depth: ColorDepth) -> Cow<'_, str> {
    if depth == ColorDepth::TrueColor || !text.contains('\x1b') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("\x1b[") {
        out.push_str(before);
        let params_len = after
            .bytes()
            .take_while(|byte| byte.is_ascii_digit() || matches!(byte, b';' | b':'))
            .count();
        let sgr = after
            .split_at_checked(params_len)
            .and_then(|(params, tail)| Some((params, tail.strip_prefix('m')?)));
        if let Some((params, tail)) = sgr {
            push_fitted_sgr(&mut out, params, depth);
            rest = tail;
        } else {
            out.push_str("\x1b[");
            rest = after;
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Write `ESC[{params}m` with its colour parameters fitted to `depth`, or
/// nothing when none are left.
fn push_fitted_sgr(out: &mut String, params: &str, depth: ColorDepth) {
    let start = out.len();
    out.push_str("\x1b[");
    let mut first = true;
    let mut parts = params.split(';');
    while let Some(part) = parts.next() {
        let extended = match part {
            "38" => Some(false),
            "48" => Some(true),
            _ => None,
        };
        if let Some(bg) = extended {
            let unread = parts.clone();
            let fitted = match parts.next() {
                Some("2") => {
                    let mut channel = || parts.next().and_then(|value| value.parse::<u8>().ok());
                    match (channel(), channel(), channel()) {
                        (Some(red), Some(green), Some(blue)) => Some(Fitted::Rgb(red, green, blue)),
                        _ => None,
                    }
                }
                Some("5") => parts
                    .next()
                    .and_then(|value| value.parse::<u8>().ok())
                    .map(Fitted::Indexed),
                _ => None,
            };
            let Some(color) = fitted else {
                // Not a colour this module writes: keep it and the rest as
                // they are.
                push_separator(out, &mut first);
                out.push_str(part);
                for kept in unread {
                    out.push(';');
                    out.push_str(kept);
                }
                break;
            };
            push_fitted_color(out, &mut first, color, bg, depth);
        } else if depth != ColorDepth::Mono || !is_color_param(part) {
            push_separator(out, &mut first);
            out.push_str(part);
        }
    }
    if first {
        out.truncate(start);
    } else {
        out.push('m');
    }
}

/// An extended colour parameter: `2;r;g;b` or `5;n`.
#[derive(Clone, Copy)]
enum Fitted {
    Rgb(u8, u8, u8),
    Indexed(u8),
}

/// Write `color` as a foreground (`bg == false`) or background at `depth`.
fn push_fitted_color(
    out: &mut String,
    first: &mut bool,
    color: Fitted,
    bg: bool,
    depth: ColorDepth,
) {
    let lead = if bg { "48" } else { "38" };
    let basic = |code: u8| if bg { code.saturating_add(10) } else { code };
    match (depth, color) {
        (ColorDepth::Mono, _) => {}
        (ColorDepth::TrueColor, Fitted::Rgb(red, green, blue)) => {
            push_separator(out, first);
            out.push_str(lead);
            out.push_str(";2;");
            push_u8(out, red);
            out.push(';');
            push_u8(out, green);
            out.push(';');
            push_u8(out, blue);
        }
        (ColorDepth::Ansi256, Fitted::Rgb(red, green, blue)) => {
            push_separator(out, first);
            out.push_str(lead);
            out.push_str(";5;");
            push_u8(out, rgb_to_ansi256(red, green, blue));
        }
        (ColorDepth::TrueColor | ColorDepth::Ansi256, Fitted::Indexed(index)) => {
            push_separator(out, first);
            out.push_str(lead);
            out.push_str(";5;");
            push_u8(out, index);
        }
        (ColorDepth::Ansi16, Fitted::Rgb(red, green, blue)) => {
            push_separator(out, first);
            push_u8(out, basic(rgb_to_ansi16(red, green, blue)));
        }
        (ColorDepth::Ansi16, Fitted::Indexed(index)) => {
            push_separator(out, first);
            push_u8(out, basic(index_to_ansi16(index)));
        }
    }
}

/// Whether the SGR parameter `part` sets a colour: `30-39`, `40-49`,
/// `90-97`, `100-107`, or a colon form of `38`/`48`.
fn is_color_param(part: &str) -> bool {
    let lead = part.split(':').next().unwrap_or(part);
    lead.parse::<u8>()
        .is_ok_and(|code| matches!(code, 30..=49 | 90..=97 | 100..=107))
}

/// The xterm 6x6x6 color-cube level (0-5) nearest to `channel`.
const fn cube_level(channel: u8) -> u8 {
    match channel {
        0..=47 => 0,
        48..=114 => 1,
        115..=154 => 2,
        155..=194 => 3,
        195..=234 => 4,
        235..=255 => 5,
    }
}

/// The nearest xterm 256-color index to `r;g;b`: the grey ramp for greys, else the color cube.
pub(crate) fn rgb_to_ansi256(red: u8, green: u8, blue: u8) -> u8 {
    if red == green && green == blue {
        return match red {
            0..=7 => 16,
            248..=255 => 231,
            _ => 232_u8.saturating_add((red.saturating_sub(3) / 10).min(23)),
        };
    }
    16_u8
        .saturating_add(36_u8.saturating_mul(cube_level(red)))
        .saturating_add(6_u8.saturating_mul(cube_level(green)))
        .saturating_add(cube_level(blue))
}

/// The 16 ANSI colors as xterm draws them, in SGR order (`30-37`, then `90-97`).
const ANSI16_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The foreground code (`30-37`, `90-97`) of the ANSI color nearest to `r;g;b`.
pub(crate) fn rgb_to_ansi16(red: u8, green: u8, blue: u8) -> u8 {
    let distance = |&(pr, pg, pb): &(u8, u8, u8)| {
        let gap = |x: u8, y: u8| u32::from(x.abs_diff(y));
        [gap(red, pr), gap(green, pg), gap(blue, pb)]
            .iter()
            .map(|delta| delta.saturating_mul(*delta))
            .fold(0_u32, u32::saturating_add)
    };
    let nearest = ANSI16_RGB
        .iter()
        .enumerate()
        .min_by_key(|(_, color)| distance(color))
        .map_or(0_usize, |(index, _)| index);
    let index = u8::try_from(nearest).unwrap_or(0_u8);
    if index < 8 {
        30_u8.saturating_add(index)
    } else {
        90_u8.saturating_add(index.saturating_sub(8))
    }
}

/// The foreground code (`30-37`, `90-97`) of the ANSI color nearest to a 256-color `index`.
pub(crate) fn index_to_ansi16(index: u8) -> u8 {
    match index {
        0..=7 => 30_u8.saturating_add(index),
        8..=15 => 90_u8.saturating_add(index.saturating_sub(8)),
        16..=231 => {
            let cube = index.saturating_sub(16);
            let level = |step: u8| {
                if step == 0 {
                    0_u8
                } else {
                    55_u8.saturating_add(40_u8.saturating_mul(step))
                }
            };
            rgb_to_ansi16(level(cube / 36), level(cube / 6 % 6), level(cube % 6))
        }
        232..=255 => {
            let grey = 8_u8.saturating_add(10_u8.saturating_mul(index.saturating_sub(232)));
            rgb_to_ansi16(grey, grey, grey)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorDepth, downgrade_sgr};
    use std::borrow::Cow;

    const DEPTHS: [ColorDepth; 4] = [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Ansi16,
        ColorDepth::Mono,
    ];

    fn at(depth: ColorDepth, text: &str) -> String {
        downgrade_sgr(text, depth).into_owned()
    }

    #[test]
    fn truecolor_and_escape_free_text_are_borrowed() {
        let styled = "\x1b[38;2;1;2;3mx\x1b[0m";
        assert!(matches!(
            downgrade_sgr(styled, ColorDepth::TrueColor),
            Cow::Borrowed(text) if std::ptr::eq(text, styled)
        ));
        for depth in DEPTHS {
            assert!(matches!(
                downgrade_sgr("plain", depth),
                Cow::Borrowed("plain")
            ));
        }
    }

    #[test]
    fn rgb_fits_each_depth() {
        let fg = "\x1b[38;2;222;165;132mx\x1b[0m";
        let bg = "\x1b[48;2;222;165;132mx\x1b[0m";
        assert_eq!(at(ColorDepth::Ansi256, fg), "\x1b[38;5;180mx\x1b[0m");
        assert_eq!(at(ColorDepth::Ansi256, bg), "\x1b[48;5;180mx\x1b[0m");
        assert_eq!(at(ColorDepth::Ansi16, fg), "\x1b[90mx\x1b[0m");
        assert_eq!(at(ColorDepth::Ansi16, bg), "\x1b[100mx\x1b[0m");
        assert_eq!(at(ColorDepth::Mono, fg), "x\x1b[0m");
        assert_eq!(
            at(ColorDepth::Ansi256, "\x1b[38;2;128;128;128mx"),
            "\x1b[38;5;244mx"
        );
    }

    #[test]
    fn indexed_colors_fit_sixteen_colors_and_named_ones_stay() {
        let indexed = "\x1b[0;38;5;196;48;5;21mx";
        assert_eq!(at(ColorDepth::Ansi256, indexed), indexed);
        assert_eq!(at(ColorDepth::Ansi16, indexed), "\x1b[0;91;44mx");
        let named = "\x1b[37;40mx\x1b[39;49m";
        assert_eq!(at(ColorDepth::Ansi256, named), named);
        assert_eq!(at(ColorDepth::Ansi16, named), named);
    }

    /// Mono keeps attributes and resets; a sequence of colours alone goes,
    /// with the prompt's zero-width markers around it left as they were.
    #[test]
    fn mono_drops_every_colour_and_keeps_attributes() {
        assert_eq!(
            at(ColorDepth::Mono, "\x1b[1;32;48;2;1;2;3mx\x1b[0m"),
            "\x1b[1mx\x1b[0m"
        );
        assert_eq!(
            at(ColorDepth::Mono, "\\[\x1b[37;100m\\]x%{\x1b[38;5;9m%}y"),
            "\\[\\]x%{%}y"
        );
    }

    /// Only SGR is rewritten: other CSI sequences, a parameter this module
    /// does not write and an unterminated escape pass through.
    #[test]
    fn everything_but_sgr_colours_passes_through() {
        for depth in [ColorDepth::Ansi256, ColorDepth::Ansi16] {
            for text in [
                "\x1b[2Kx",
                "\x1b[?25lx",
                "\x1b[38;2;1mx",
                "\x1b[38:2::1:2:3mx",
                "x\x1b[",
                "x\x1b[38;2;1;2;3",
            ] {
                assert_eq!(at(depth, text), text, "{depth:?}");
            }
        }
    }

    #[test]
    fn color_depth_env_resolution() {
        let resolve = |depth: Option<&str>, term_program: Option<&str>, term: Option<&str>| {
            ColorDepth::resolve(depth.and_then(ColorDepth::parse), term_program, term)
        };
        assert_eq!(resolve(None, None, None), ColorDepth::TrueColor);
        assert_eq!(
            resolve(None, None, Some("xterm-256color")),
            ColorDepth::TrueColor
        );
        assert_eq!(
            resolve(None, Some("Apple_Terminal"), None),
            ColorDepth::Ansi256
        );
        assert_eq!(resolve(None, None, Some("linux")), ColorDepth::Ansi256);
        assert_eq!(
            resolve(Some("truecolor"), Some("Apple_Terminal"), None),
            ColorDepth::TrueColor
        );
        assert_eq!(resolve(Some("16"), None, None), ColorDepth::Ansi16);
        assert_eq!(resolve(Some(" NONE "), None, None), ColorDepth::Mono);
        assert_eq!(resolve(Some("bogus"), None, None), ColorDepth::TrueColor);
        for depth in DEPTHS {
            assert_eq!(ColorDepth::parse(depth.name()), Some(depth));
        }
    }
}
