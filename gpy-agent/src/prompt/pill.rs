//! The pills the Bash integration draws itself (`bash/core/renderer.bash`):
//! the clock and status fallbacks and the root/no-character prompt tail.
//! Byte-for-byte ports, so `gpy prompt` and the shell path print the same
//! `PS1`; `parity` tests pin them against the shell functions.

use super::vars::ShellVars;

/// `ESC`, as the shell's `__GPY_SGR_ESC`.
const ESC: char = '\u{1b}';

/// Colour depth as `__gpy_color_depth` resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Depth {
    /// 24-bit SGR.
    TrueColor,
    /// xterm 256 palette.
    Ansi256,
    /// The 16 ANSI colours.
    Ansi16,
    /// No colour codes.
    NoColor,
}

impl Depth {
    /// `GPY_COLOR_DEPTH` (case-insensitive, surrounding blanks ignored) wins;
    /// otherwise 256 on `Apple_Terminal` or `TERM=linux`, else truecolor.
    pub(crate) fn from_env(
        depth: Option<&str>,
        term_program: Option<&str>,
        term: Option<&str>,
    ) -> Self {
        match depth.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("truecolor" | "24bit") => Self::TrueColor,
            Some("256") => Self::Ansi256,
            Some("16") => Self::Ansi16,
            Some("none") => Self::NoColor,
            _ if term_program == Some("Apple_Terminal") || term == Some("linux") => Self::Ansi256,
            _ => Self::TrueColor,
        }
    }
}

/// The 16 ANSI colours as xterm draws them, in SGR order.
const ANSI16_RGB: [[i32; 3]; 16] = [
    [0, 0, 0],
    [205, 0, 0],
    [0, 205, 0],
    [205, 205, 0],
    [0, 0, 238],
    [205, 0, 205],
    [0, 205, 205],
    [229, 229, 229],
    [127, 127, 127],
    [255, 0, 0],
    [0, 255, 0],
    [255, 255, 0],
    [92, 92, 255],
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 255],
];

/// Foreground code (30-37, 90-97) of the ANSI colour nearest to `rgb`.
fn rgb_to_16(rgb: [i32; 3]) -> i32 {
    let mut best = 0_i32;
    let mut best_distance = -1_i32;
    for (index, ansi) in (0_i32..).zip(ANSI16_RGB) {
        let distance = rgb
            .iter()
            .zip(ansi)
            .map(|(c, a)| c.saturating_sub(a).saturating_pow(2))
            .fold(0_i32, i32::saturating_add);
        if best_distance < 0_i32 || distance < best_distance {
            best_distance = distance;
            best = index;
        }
    }
    if best < 8 {
        best.saturating_add(30)
    } else {
        best.saturating_add(82)
    }
}

/// xterm cube level (0-5) nearest to `channel`.
const fn cube_level(channel: i32) -> i32 {
    match channel {
        ..48 => 0,
        48..115 => 1,
        115..155 => 2,
        155..195 => 3,
        195..235 => 4,
        _ => 5,
    }
}

/// Nearest xterm 256 index to `rgb`.
fn rgb_to_256([red, green, blue]: [i32; 3]) -> i32 {
    if red == green && green == blue {
        return match red {
            ..8 => 16,
            248.. => 231,
            _ => red
                .saturating_sub(3)
                .saturating_div(10)
                .min(23)
                .saturating_add(232),
        };
    }
    cube_level(red)
        .saturating_mul(36)
        .saturating_add(cube_level(green).saturating_mul(6))
        .saturating_add(cube_level(blue))
        .saturating_add(16)
}

/// Foreground code (30-37, 90-97) nearest to 256-colour `index`.
fn index_to_16(index: i32) -> i32 {
    match index {
        ..8 => index.saturating_add(30),
        8..16 => index.saturating_add(82),
        16..232 => {
            let cube = index.saturating_sub(16);
            let level = |l: i32| {
                if l == 0_i32 {
                    0_i32
                } else {
                    l.saturating_mul(40).saturating_add(55)
                }
            };
            rgb_to_16([
                level(cube.saturating_div(36)),
                level(cube.saturating_div(6).rem_euclid(6)),
                level(cube.rem_euclid(6)),
            ])
        }
        _ => {
            let gray = index
                .saturating_sub(232)
                .saturating_mul(10)
                .saturating_add(8);
            rgb_to_16([gray, gray, gray])
        }
    }
}

/// `#rrggbb` as three channels.
fn hex_rgb(color: &str) -> Option<[i32; 3]> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |range| hex.get(range).and_then(|h| i32::from_str_radix(h, 16).ok());
    Some([channel(0..2)?, channel(2..4)?, channel(4..6)?])
}

/// SGR parameter for `color` as foreground (`bg == false`) or background,
/// as `__gpy_sgr_param`: empty under [`Depth::NoColor`]; unknown names,
/// `transparent`, `normal` and empty are the default (39/49).
fn sgr_param(bg: bool, color: &str, depth: Depth) -> String {
    if depth == Depth::NoColor {
        return String::new();
    }
    let base: i32 = if bg { 40 } else { 30 };
    let named = |offset: i32| base.saturating_add(offset).to_string();
    match color {
        "black" => named(0),
        "red" => named(1),
        "green" => named(2),
        "yellow" => named(3),
        "blue" => named(4),
        "magenta" | "purple" => named(5),
        "cyan" => named(6),
        "white" => named(7),
        "brblack" | "bright_black" | "bright-black" | "gray" | "grey" => named(60),
        "brred" | "bright_red" | "bright-red" => named(61),
        "brgreen" | "bright_green" | "bright-green" => named(62),
        "bryellow" | "bright_yellow" | "bright-yellow" => named(63),
        "brblue" | "bright_blue" | "bright-blue" => named(64),
        "brmagenta" | "bright_magenta" | "bright-purple" => named(65),
        "brcyan" | "bright_cyan" | "bright-cyan" => named(66),
        "brwhite" | "bright_white" | "bright-white" => named(67),
        _ => {
            if let Some(rgb @ [r, g, b]) = hex_rgb(color) {
                return match depth {
                    Depth::TrueColor => format!("{};2;{r};{g};{b}", base.saturating_add(8)),
                    Depth::Ansi256 => format!("{};5;{}", base.saturating_add(8), rgb_to_256(rgb)),
                    Depth::Ansi16 | Depth::NoColor => rgb_to_16(rgb)
                        .saturating_add(base)
                        .saturating_sub(30)
                        .to_string(),
                };
            }
            if (1..=3).contains(&color.len()) && color.bytes().all(|b| b.is_ascii_digit()) {
                let index: i32 = color.parse().unwrap_or(256);
                if index <= 255 {
                    return if depth == Depth::Ansi16 {
                        index_to_16(index)
                            .saturating_add(base)
                            .saturating_sub(30)
                            .to_string()
                    } else {
                        format!("{};5;{index}", base.saturating_add(8))
                    };
                }
            }
            named(9)
        }
    }
}

/// One `\[ESC[fg;bgm\]` sequence, or nothing when both parameters are empty.
fn sgr_pair(fg: &str, bg: &str, depth: Depth) -> String {
    let f = sgr_param(false, fg, depth);
    let b = sgr_param(true, bg, depth);
    if f.is_empty() && b.is_empty() {
        return String::new();
    }
    format!("\\[{ESC}[{f};{b}m\\]")
}

/// Escape text for `PS1` under `promptvars`, as `__gpy_prompt_escape`.
pub(crate) fn prompt_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\\\\\"),
            '$' => out.push_str("\\\\$"),
            '`' => out.push_str("\\\\`"),
            _ => out.push(ch),
        }
    }
    out
}

/// Where a pill sits in the chain.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Place {
    /// Last segment before the character.
    pub(crate) is_last: bool,
    /// First segment of the prompt.
    pub(crate) is_first: bool,
}

/// One delimiter cap: its glyph and colours, after `match_bg`/`match_text`.
struct Cap<'a> {
    glyph: &'a str,
    color: &'a str,
    back: &'a str,
}

impl<'a> Cap<'a> {
    /// A cap from its glyph, colour and background; `match_bg`/`match_text`
    /// take the segment's own colours.
    fn resolve(values: [&'a str; 3], segment_bg: &'a str, segment_text: &'a str) -> Self {
        let [glyph, color, back] = values.map(|value| match value {
            "match_bg" => segment_bg,
            "match_text" => segment_text,
            other => other,
        });
        Self { glyph, color, back }
    }

    /// The cap the export's variables `names` describe (glyph, colour,
    /// background), each `${name-default}`.
    fn new(
        vars: &'a ShellVars,
        names: [(&str, &'a str); 3],
        segment_bg: &'a str,
        segment_text: &'a str,
    ) -> Self {
        Self::resolve(
            names.map(|(name, default)| vars.or(name, default)),
            segment_bg,
            segment_text,
        )
    }

    fn draw(&self, depth: Depth) -> String {
        let mut out = sgr_pair(self.color, self.back, depth);
        out.push_str(&prompt_escape(self.glyph));
        out
    }
}

/// A script segment's own delimiters (`__gpy_section_standalone`'s CAPS):
/// an open or close cap (glyph, colour, background) replaces the global
/// delimiter on that side. Without its own close cap, a last pill draws the
/// theme's close cap on the terminal default background (`d`).
#[derive(Debug, Default)]
pub(crate) struct Caps<'a> {
    pub(crate) open: Option<[&'a str; 3]>,
    pub(crate) close: Option<[&'a str; 3]>,
}

/// `__gpy_section_standalone` without per-segment caps: `content` is `PS1`
/// source (escape data first).
pub(crate) fn section_standalone(
    vars: &ShellVars,
    depth: Depth,
    bg: &str,
    fg: &str,
    content: &str,
    place: Place,
) -> String {
    section(vars, depth, [bg, fg], content, place, None)
}

/// `__gpy_section_standalone` for a script segment pill (CAPS `o`, `c`, `d`).
pub(crate) fn section_with_caps(
    vars: &ShellVars,
    depth: Depth,
    [bg, fg]: [&str; 2],
    content: &str,
    place: Place,
    caps: &Caps<'_>,
) -> String {
    section(vars, depth, [bg, fg], content, place, Some(caps))
}

fn section(
    vars: &ShellVars,
    depth: Depth,
    [bg, fg]: [&str; 2],
    content: &str,
    place: Place,
    own: Option<&Caps<'_>>,
) -> String {
    let open_names = if place.is_first {
        [
            ("__segment_delim_first", "|"),
            ("__prompt_open_color", ""),
            ("__prompt_open_bg", ""),
        ]
    } else {
        [
            ("__segment_delim_start", "["),
            ("__segment_delimiter_color", ""),
            ("__segment_delimiter_bg", ""),
        ]
    };
    let open = own.and_then(|caps| caps.open).map_or_else(
        || Cap::new(vars, open_names, bg, fg),
        |values| Cap::resolve(values, bg, fg),
    );
    let close_names = if place.is_last {
        [
            ("__segment_delim_last", ")"),
            ("__prompt_close_color", ""),
            ("__prompt_close_bg", ""),
        ]
    } else {
        [
            ("__segment_delim_end", "]"),
            ("__segment_delimiter_color", ""),
            ("__segment_delimiter_bg", ""),
        ]
    };
    let close = own.and_then(|caps| caps.close).map_or_else(
        || {
            let mut cap = Cap::new(vars, close_names, bg, fg);
            // `d`: a script pill's theme close cap sits on the default background.
            if place.is_last && own.is_some() {
                cap.back = "";
            }
            cap
        },
        |values| Cap::resolve(values, bg, fg),
    );
    let mut out = open.draw(depth);
    out.push_str(&sgr_pair(fg, bg, depth));
    out.push_str(content);
    if !place.is_last {
        out.push_str(&sgr_pair("normal", bg, depth));
        out.push(' ');
    }
    out.push_str(&close.draw(depth));
    out.push_str("\\[\u{1b}[0m\\]");
    out
}

/// `__gpy_status_icon`: the theme's icon (an exported empty value stays
/// empty), else `✔`/`✖`, or `ok`/`err` without Nerd icons.
pub(crate) fn status_icon(vars: &ShellVars, ok: bool) -> &str {
    let nerd = vars.or_nonempty("__prompt_icons", "nerd") == "nerd";
    let (name, fallback) = match (ok, nerd) {
        (true, true) => ("__icon_status_ok", "✔"),
        (true, false) => ("__icon_status_ok", "ok"),
        (false, true) => ("__icon_status_fail", "✖"),
        (false, false) => ("__icon_status_fail", "err"),
    };
    vars.or(name, fallback)
}

/// `__gpy_prompt_tail`: status icon (unless `GPY_SHOW_STATUS` is off) and
/// the prompt or root symbol.
pub(crate) fn prompt_tail(
    vars: &ShellVars,
    depth: Depth,
    exit: i32,
    root: bool,
    show_status: bool,
) -> String {
    let reset = format!("\\[{ESC}[0m\\]");
    let mut out = String::new();
    if show_status {
        let (icon, color) = if exit == 0_i32 {
            (status_icon(vars, true), "green")
        } else {
            (status_icon(vars, false), "red")
        };
        out.push_str(&sgr_pair(color, "normal", depth));
        out.push_str(&prompt_escape(icon));
        out.push(' ');
        out.push_str(&reset);
    }
    let (icon, color) = if root {
        (
            vars.or("__icon_root_prompt", "!❯!"),
            vars.or_nonempty("__root_prompt_color", "red"),
        )
    } else {
        (
            vars.or("__icon_prompt", "❯"),
            vars.or_nonempty("__prompt_color", "green"),
        )
    };
    out.push_str(&sgr_pair(color, "normal", depth));
    out.push_str(&prompt_escape(icon));
    out.push(' ');
    out.push_str(&reset);
    out
}

/// `__gpy_clock_time_spec`: the strftime spec Bash's `\D{…}` expands.
pub(crate) fn clock_time_spec(vars: &ShellVars) -> String {
    let twenty_four = vars.or_nonempty("__time_format", "12") == "24";
    let leading_zero = vars.or_nonempty("__clock_show_leading_zero", "0") == "1";
    let seconds = vars.or_nonempty("__clock_show_seconds", "0") == "1";
    let hour = match (twenty_four, leading_zero) {
        (true, true) => "%H",
        (true, false) => "%-H",
        (false, true) => "%I",
        (false, false) => "%-I",
    };
    let mut spec = format!("{hour}:%M");
    if seconds {
        spec.push_str(":%S");
    }
    if !twenty_four {
        spec.push_str(" %p");
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The colour conversions follow the shell's tables (renderer.bash).
    #[test]
    fn sgr_params_match_the_shell_tables() {
        assert_eq!(sgr_param(false, "red", Depth::TrueColor), "31");
        assert_eq!(sgr_param(true, "bright-cyan", Depth::TrueColor), "106");
        assert_eq!(
            sgr_param(false, "#51576d", Depth::TrueColor),
            "38;2;81;87;109"
        );
        assert_eq!(sgr_param(true, "#51576d", Depth::Ansi256), "48;5;59");
        assert_eq!(sgr_param(false, "#ff0000", Depth::Ansi16), "91");
        assert_eq!(sgr_param(false, "200", Depth::Ansi256), "38;5;200");
        assert_eq!(sgr_param(false, "300", Depth::Ansi256), "39");
        assert_eq!(sgr_param(true, "transparent", Depth::TrueColor), "49");
        assert_eq!(sgr_param(true, "red", Depth::NoColor), "");
        assert_eq!(sgr_param(false, "#808080", Depth::Ansi256), "38;5;244");
    }

    #[test]
    fn prompt_escape_doubles_for_promptvars() {
        assert_eq!(prompt_escape(r"a\b$c`d"), r"a\\\\b\\$c\\`d");
    }
}
