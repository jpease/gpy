//! The pills `gpy prompt` draws itself rather than asking the agent: the
//! clock and status fallbacks, script segment pills and the
//! root/no-character prompt tail. Colours are 24-bit, as the agent's;
//! [`super::render`] fits the whole prompt to the terminal's colour depth.
//! `tests/bash/prompt_golden.test.bash` pins the output.

use super::dialect::Dialect;
use super::vars::ShellVars;

/// `ESC`, as the shell's `__GPY_SGR_ESC`.
const ESC: char = '\u{1b}';

/// The zero-width SGR reset that ends every pill.
fn reset(dialect: Dialect) -> String {
    dialect.invisible(&format!("{ESC}[0m"))
}

/// `#rrggbb` as three channels.
fn hex_rgb(color: &str) -> Option<[u8; 3]> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |range| hex.get(range).and_then(|h| u8::from_str_radix(h, 16).ok());
    Some([channel(0..2)?, channel(2..4)?, channel(4..6)?])
}

/// SGR parameter for `color` as foreground (`bg == false`) or background:
/// a name, `#rrggbb` (24-bit) or a 256-colour index; unknown names,
/// `transparent`, `normal` and empty are the default (39/49).
fn sgr_param(bg: bool, color: &str) -> String {
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
            if let Some([r, g, b]) = hex_rgb(color) {
                return format!("{};2;{r};{g};{b}", base.saturating_add(8));
            }
            if (1..=3).contains(&color.len())
                && color.bytes().all(|b| b.is_ascii_digit())
                && let Ok(index) = color.parse::<u8>()
            {
                return format!("{};5;{index}", base.saturating_add(8));
            }
            named(9)
        }
    }
}

/// One zero-width `ESC[fg;bgm` sequence.
fn sgr_pair(fg: &str, bg: &str, dialect: Dialect) -> String {
    let f = sgr_param(false, fg);
    let b = sgr_param(true, bg);
    dialect.invisible(&format!("{ESC}[{f};{b}m"))
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

    fn draw(&self, dialect: Dialect) -> String {
        let mut out = sgr_pair(self.color, self.back, dialect);
        out.push_str(&dialect.escape(self.glyph));
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
    dialect: Dialect,
    bg: &str,
    fg: &str,
    content: &str,
    place: Place,
) -> String {
    section(vars, dialect, [bg, fg], content, place, None)
}

/// `__gpy_section_standalone` for a script segment pill (CAPS `o`, `c`, `d`).
pub(crate) fn section_with_caps(
    vars: &ShellVars,
    dialect: Dialect,
    [bg, fg]: [&str; 2],
    content: &str,
    place: Place,
    caps: &Caps<'_>,
) -> String {
    section(vars, dialect, [bg, fg], content, place, Some(caps))
}

fn section<'a>(
    vars: &'a ShellVars,
    dialect: Dialect,
    [bg, fg]: [&'a str; 2],
    content: &str,
    place: Place,
    own: Option<&Caps<'a>>,
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
    // An own cap replaces the glyph; its colours replace the theme's in Bash
    // and Zsh, but only when non-empty in Fish (`__gpy_section_pill`).
    let own_cap =
        |values: [&'a str; 3], names: [(&str, &'a str); 3], back_default: Option<&'a str>| {
            if dialect != Dialect::Fish {
                return Cap::resolve(values, bg, fg);
            }
            let [glyph, color, back] = values;
            let [
                _,
                (color_name, color_default),
                (back_name, back_default_value),
            ] = names;
            let theme_back = back_default.unwrap_or_else(|| vars.or(back_name, back_default_value));
            Cap::resolve(
                [
                    glyph,
                    if color.is_empty() {
                        vars.or(color_name, color_default)
                    } else {
                        color
                    },
                    if back.is_empty() { theme_back } else { back },
                ],
                bg,
                fg,
            )
        };
    let open = own.and_then(|caps| caps.open).map_or_else(
        || Cap::new(vars, open_names, bg, fg),
        |values| own_cap(values, open_names, None),
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
            ("__segment_delimiter_close_color", ""),
            ("__segment_delimiter_close_bg", ""),
        ]
    };
    // `d`: a script pill's theme close cap sits on the default background.
    let plugin_last = place.is_last && own.is_some();
    let close = own.and_then(|caps| caps.close).map_or_else(
        || {
            let mut cap = Cap::new(vars, close_names, bg, fg);
            if plugin_last {
                cap.back = "";
            }
            cap
        },
        |values| own_cap(values, close_names, plugin_last.then_some("")),
    );
    let mut out = open.draw(dialect);
    out.push_str(&sgr_pair(fg, bg, dialect));
    out.push_str(content);
    if !place.is_last {
        out.push_str(&sgr_pair("normal", bg, dialect));
        out.push(' ');
    }
    out.push_str(&close.draw(dialect));
    out.push_str(&reset(dialect));
    out
}

/// Text in foreground `color` on the default background, then a reset, as
/// Fish's `__gpy_set_fg_color COLOR; printf …; set_color normal`:
/// `transparent`, `match_bg`, `match_text` and empty are the default colour.
pub(crate) fn fg_text(dialect: Dialect, color: &str, text: &str) -> String {
    let shown = match color {
        "transparent" | "match_bg" | "match_text" | "" => "normal",
        other => other,
    };
    let mut out = sgr_pair(shown, "normal", dialect);
    out.push_str(text);
    out.push_str(&reset(dialect));
    out
}

/// `text` without its `ESC[…m` sequences.
pub(crate) fn strip_sgr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == ESC {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
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
    dialect: Dialect,
    exit: i32,
    root: bool,
    show_status: bool,
) -> String {
    let end = reset(dialect);
    let mut out = String::new();
    if show_status {
        let (icon, color) = if exit == 0_i32 {
            (status_icon(vars, true), "green")
        } else {
            (status_icon(vars, false), "red")
        };
        out.push_str(&sgr_pair(color, "normal", dialect));
        out.push_str(&dialect.escape(icon));
        out.push(' ');
        out.push_str(&end);
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
    out.push_str(&sgr_pair(color, "normal", dialect));
    out.push_str(&dialect.escape(icon));
    out.push(' ');
    out.push_str(&end);
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
    use crate::formatter::{ColorDepth, downgrade_sgr};

    /// The colour conversions follow the shell's tables (renderer.bash).
    #[test]
    fn sgr_params_match_the_shell_tables() {
        assert_eq!(sgr_param(false, "red"), "31");
        assert_eq!(sgr_param(true, "bright-cyan"), "106");
        assert_eq!(sgr_param(false, "#51576d"), "38;2;81;87;109");
        assert_eq!(sgr_param(false, "200"), "38;5;200");
        assert_eq!(sgr_param(false, "300"), "39");
        assert_eq!(sgr_param(true, "transparent"), "49");
    }

    /// Fitted to a smaller colour depth, a pill's colours are the ones the
    /// shells' tables gave at that depth.
    #[test]
    fn fitted_pill_colours_match_the_shell_tables() {
        let fitted = |fg: &str, bg: &str, depth| {
            downgrade_sgr(&sgr_pair(fg, bg, Dialect::Fish), depth).into_owned()
        };
        assert_eq!(
            fitted("#808080", "#51576d", ColorDepth::Ansi256),
            "\x1b[38;5;244;48;5;59m"
        );
        assert_eq!(fitted("#ff0000", "200", ColorDepth::Ansi16), "\x1b[91;105m");
        assert_eq!(fitted("red", "black", ColorDepth::Mono), "");
    }

    /// A middle pill's start cap takes `[ui.segment_open]`'s colours and its
    /// end cap `[ui.segment_close]`'s, not the open ones twice.
    #[test]
    fn middle_pill_caps_use_their_own_delimiter_colours() {
        let vars = ShellVars::new(
            [
                ("__segment_delim_start", "<"),
                ("__segment_delimiter_color", "red"),
                ("__segment_delimiter_bg", "blue"),
                ("__segment_delim_end", ">"),
                ("__segment_delimiter_close_color", "green"),
                ("__segment_delimiter_close_bg", "yellow"),
            ]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .into(),
        );
        let dialect = Dialect::Fish;
        let place = Place {
            is_last: false,
            is_first: false,
        };

        let drawn = section_standalone(&vars, dialect, "black", "white", "x", place);

        let cap = |fg: &str, bg: &str, glyph: &str| format!("{}{glyph}", sgr_pair(fg, bg, dialect));
        assert!(drawn.starts_with(&cap("red", "blue", "<")), "{drawn:?}");
        assert!(drawn.contains(&cap("green", "yellow", ">")), "{drawn:?}");
    }
}
