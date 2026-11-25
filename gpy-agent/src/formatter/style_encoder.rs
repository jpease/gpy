//! Encode engine [`Span`]s into shell-specific escape sequences. One SGR encoder
//! serves three [`PromptDialect`]s that differ only in text escaping and
//! zero-width markers. Unresolved palette/prev colors are no-ops, never panics.

use crate::template::{Attr, Color, Span, SpanKind, Style, canonical_ansi_name};
use std::sync::OnceLock;

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
    /// Environment override: `truecolor`, `256`, `16` or `none`.
    pub const ENV_OVERRIDE: &'static str = "GPY_COLOR_DEPTH";

    /// The depth for this process, read from the environment once.
    #[must_use]
    pub fn detect() -> Self {
        static DEPTH: OnceLock<ColorDepth> = OnceLock::new();
        // Unit tests render against the default, whatever terminal runs them.
        if cfg!(test) {
            return Self::TrueColor;
        }
        *DEPTH.get_or_init(|| Self::from_env(|key| std::env::var(key).ok()))
    }

    /// [`Self::ENV_OVERRIDE`] when it names a depth, else 256 colors on a
    /// terminal known to lack 24-bit color (`TERM_PROGRAM=Apple_Terminal`,
    /// `TERM=linux`), else 24-bit. Absence of `COLORTERM` proves nothing.
    fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        if let Some(depth) = get(Self::ENV_OVERRIDE).and_then(|value| Self::parse(&value)) {
            return depth;
        }
        let limited = get("TERM_PROGRAM").as_deref() == Some("Apple_Terminal")
            || get("TERM").as_deref() == Some("linux");
        if limited {
            Self::Ansi256
        } else {
            Self::TrueColor
        }
    }

    /// A depth name (`truecolor`/`24bit`, `256`, `16`, `none`), case-insensitive.
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" => Some(Self::TrueColor),
            "256" => Some(Self::Ansi256),
            "16" => Some(Self::Ansi16),
            "none" => Some(Self::Mono),
            _ => None,
        }
    }
}

/// Output dialect. Bash/zsh expand `PS1`/`PROMPT` as code on every draw, so
/// their dialects escape text; [`Self::Ansi`] (fish) emits it verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptDialect {
    /// Raw ANSI SGR with text verbatim (fish and generic terminals).
    #[default]
    Ansi,
    /// Bash `PS1` source (assumes `shopt -s promptvars`).
    BashPrompt,
    /// Zsh `PROMPT` source (assumes `prompt_subst` and `prompt_percent`; the shell doubles `!` under `prompt_bang`).
    ZshPrompt,
}

impl PromptDialect {
    /// Every dialect, in a fixed order (the instant cache renders all three).
    pub const ALL: [Self; 3] = [Self::Ansi, Self::BashPrompt, Self::ZshPrompt];

    /// Encode `spans` in this dialect with 24-bit colors.
    #[must_use]
    pub fn encode(self, spans: &[Span]) -> String {
        self.encode_depth(spans, ColorDepth::TrueColor)
    }

    /// Encode `spans` in this dialect, approximating colors to `depth`.
    #[must_use]
    pub fn encode_depth(self, spans: &[Span], depth: ColorDepth) -> String {
        match self {
            Self::Ansi => encode_sgr(spans, ("\x1b[", "m"), |_| None, depth),
            Self::BashPrompt => encode_sgr(spans, ("\\[\x1b[", "m\\]"), bash_escape, depth),
            Self::ZshPrompt => encode_sgr(spans, ("%{\x1b[", "m%}"), zsh_escape, depth),
        }
    }

    /// File extension of this dialect's instant-cache entries.
    #[must_use]
    pub const fn cache_ext(self) -> &'static str {
        match self {
            Self::Ansi => "ansi",
            Self::BashPrompt => "bash",
            Self::ZshPrompt => "zsh",
        }
    }
}

/// Encode `spans` as raw ANSI SGR. Resets at each styled→default boundary
/// (Starship's convention) so trailing literal text renders uncolored.
#[must_use]
pub fn encode_ansi(spans: &[Span]) -> String {
    PromptDialect::Ansi.encode(spans)
}

/// Encode `spans` as bash `PS1` source: SGR inside `\[ \]`, text escaped;
/// `PromptToken` spans (`\D{…}`) emitted raw.
#[must_use]
pub fn encode_bash_prompt(spans: &[Span]) -> String {
    PromptDialect::BashPrompt.encode(spans)
}

/// Encode `spans` as zsh `PROMPT` source: SGR inside `%{ %}`, text escaped;
/// `PromptToken` spans (`%D{…}`) emitted raw.
#[must_use]
pub fn encode_zsh_prompt(spans: &[Span]) -> String {
    PromptDialect::ZshPrompt.encode(spans)
}

/// Span walk shared by every SGR encoder. `wrap` is the (open, close) text around
/// each SGR sequence; `escape` maps a text character to its replacement, if any.
/// A styled span that would inherit from the previous one folds in a reset
/// (`0;<codes>`); otherwise only new codes.
fn encode_sgr(
    spans: &[Span],
    wrap: (&str, &str),
    escape: fn(char) -> Option<&'static str>,
    depth: ColorDepth,
) -> String {
    let (open_seq, close_seq) = wrap;
    let mut out = String::with_capacity(64_usize);
    let mut open: Option<&Style> = None;
    for span in spans {
        if emits_codes(&span.style, depth) {
            out.push_str(open_seq);
            if open.is_some_and(|before| leaks_into(before, &span.style, depth)) {
                out.push_str("0;");
            }
            push_codes(&mut out, &span.style, depth);
            out.push_str(close_seq);
            open = Some(&span.style);
        } else if open.take().is_some() {
            out.push_str(open_seq);
            out.push('0');
            out.push_str(close_seq);
        }
        push_text(&mut out, span, escape);
    }
    if open.is_some() {
        out.push_str(open_seq);
        out.push('0');
        out.push_str(close_seq);
    }
    out
}

/// Whether `next`, written after `prev` without reset, inherits from it (SGR is additive).
fn leaks_into(prev: &Style, next: &Style, depth: ColorDepth) -> bool {
    prev.attrs.iter().any(|attr| !next.attrs.contains(attr))
        || (sets_color(prev.fg.as_ref(), depth) && !sets_color(next.fg.as_ref(), depth))
        || (sets_color(prev.bg.as_ref(), depth) && !sets_color(next.bg.as_ref(), depth))
}

/// Whether `color` emits any SGR code (unresolved variants emit none, and
/// [`ColorDepth::Mono`] emits no color at all).
fn sets_color(color: Option<&Color>, depth: ColorDepth) -> bool {
    if depth == ColorDepth::Mono {
        return false;
    }
    match color {
        Some(Color::Named(name)) => named_base(name).is_some(),
        Some(Color::Rgb { .. } | Color::Ansi256(_)) => true,
        Some(Color::Palette(_) | Color::PrevFg | Color::PrevBg) | None => false,
    }
}

/// Whether `style` emits at least one SGR code.
fn emits_codes(style: &Style, depth: ColorDepth) -> bool {
    !style.attrs.is_empty()
        || sets_color(style.fg.as_ref(), depth)
        || sets_color(style.bg.as_ref(), depth)
}

/// Write `span`'s text, escaping each character through `escape` unless the
/// span is a `PromptToken`, which the shell must interpret and so stays raw.
///
/// Bash with `promptvars` decodes prompt escapes then double-quote expands, so
/// `\` needs four backslashes and `$`/`` ` `` need `\\`; `\$` is avoided since
/// it decodes to `#` when euid is 0. Zsh with `prompt_subst` and
/// `prompt_percent` needs `\`, `$` and `` ` `` backslash-quoted and `%` doubled.
fn push_text(out: &mut String, span: &Span, escape: fn(char) -> Option<&'static str>) {
    if span.kind == SpanKind::PromptToken {
        out.push_str(&span.text);
        return;
    }
    for ch in span.text.chars() {
        match escape(ch) {
            Some(replacement) => out.push_str(replacement),
            None => out.push(ch),
        }
    }
}

const fn bash_escape(ch: char) -> Option<&'static str> {
    match ch {
        '\\' => Some("\\\\\\\\"),
        '$' => Some("\\\\$"),
        '`' => Some("\\\\`"),
        _ => None,
    }
}

const fn zsh_escape(ch: char) -> Option<&'static str> {
    match ch {
        '\\' => Some("\\\\"),
        '$' => Some("\\$"),
        '`' => Some("\\`"),
        '%' => Some("%%"),
        _ => None,
    }
}

/// Write the `;`-joined SGR codes for one style in attr→fg→bg order.
fn push_codes(out: &mut String, style: &Style, depth: ColorDepth) {
    let mut first = true;
    for attr in &style.attrs {
        push_separator(out, &mut first);
        out.push_str(attr_code(*attr));
    }
    if let Some(fg) = &style.fg {
        push_color(out, &mut first, fg, false, depth);
    }
    if let Some(bg) = &style.bg {
        push_color(out, &mut first, bg, true, depth);
    }
}

const fn attr_code(attr: Attr) -> &'static str {
    match attr {
        Attr::Bold => "1",
        Attr::Dimmed => "2",
        Attr::Italic => "3",
        Attr::Underline => "4",
        Attr::Blink => "5",
        Attr::Inverted => "7",
        Attr::Hidden => "8",
        Attr::Strikethrough => "9",
    }
}

/// Write a `;` before every code but the first of a style.
fn push_separator(out: &mut String, first: &mut bool) {
    if !*first {
        out.push(';');
    }
    *first = false;
}

/// Write SGR codes for `color` after a separator each. `bg` selects
/// background variants; `depth` approximates colors the terminal cannot show.
/// Unresolved palette/prev variants write nothing (defensive: eval resolves them).
fn push_color(out: &mut String, first: &mut bool, color: &Color, bg: bool, depth: ColorDepth) {
    if depth == ColorDepth::Mono {
        return;
    }
    let lead = if bg { "48" } else { "38" };
    match color {
        Color::Named(name) => {
            if let Some(base) = named_base(name) {
                push_separator(out, first);
                push_u8(out, if bg { base.saturating_add(10) } else { base });
            }
        }
        Color::Rgb { r, g, b } => match depth {
            ColorDepth::TrueColor | ColorDepth::Mono => {
                push_separator(out, first);
                out.push_str(lead);
                out.push_str(";2;");
                push_u8(out, *r);
                out.push(';');
                push_u8(out, *g);
                out.push(';');
                push_u8(out, *b);
            }
            ColorDepth::Ansi256 => push_indexed(out, first, lead, rgb_to_ansi256(*r, *g, *b)),
            ColorDepth::Ansi16 => push_basic(out, first, bg, rgb_to_ansi16(*r, *g, *b)),
        },
        Color::Ansi256(index) => match depth {
            ColorDepth::Ansi16 => push_basic(out, first, bg, index_to_ansi16(*index)),
            ColorDepth::TrueColor | ColorDepth::Ansi256 | ColorDepth::Mono => {
                push_indexed(out, first, lead, *index);
            }
        },
        Color::Palette(_) | Color::PrevFg | Color::PrevBg => {}
    }
}

/// Write a `38;5;n` / `48;5;n` color.
fn push_indexed(out: &mut String, first: &mut bool, lead: &str, index: u8) {
    push_separator(out, first);
    out.push_str(lead);
    out.push_str(";5;");
    push_u8(out, index);
}

/// Write a 16-color code given as its foreground form (`30-37`, `90-97`).
fn push_basic(out: &mut String, first: &mut bool, bg: bool, foreground_code: u8) {
    push_separator(out, first);
    push_u8(
        out,
        if bg {
            foreground_code.saturating_add(10)
        } else {
            foreground_code
        },
    );
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

/// Write `value` in decimal without going through `fmt`.
fn push_u8(out: &mut String, value: u8) {
    if value >= 100 {
        out.extend(char::from_digit(u32::from(value / 100), 10));
    }
    if value >= 10 {
        out.extend(char::from_digit(u32::from(value / 10 % 10), 10));
    }
    out.extend(char::from_digit(u32::from(value % 10), 10));
}

/// Foreground SGR code of an engine named color, or `None` if unrecognized.
/// The background code is this plus 10.
fn named_base(name: &str) -> Option<u8> {
    let canonical = if name == "default" {
        name
    } else {
        canonical_ansi_name(name)?
    };
    match canonical {
        "black" => Some(30),
        "red" => Some(31),
        "green" => Some(32),
        "yellow" => Some(33),
        "blue" => Some(34),
        "purple" => Some(35),
        "cyan" => Some(36),
        "white" => Some(37),
        "default" => Some(39),
        "bright-black" => Some(90),
        "bright-red" => Some(91),
        "bright-green" => Some(92),
        "bright-yellow" => Some(93),
        "bright-blue" => Some(94),
        "bright-purple" => Some(95),
        "bright-cyan" => Some(96),
        "bright-white" => Some(97),
        _ => None,
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

    use super::{ColorDepth, PromptDialect, encode_ansi, encode_bash_prompt, encode_zsh_prompt};
    use crate::template::{Attr, Color, Span, SpanKind, Style};

    fn span(text: &str, style: Style) -> Span {
        Span {
            text: text.to_owned(),
            style,
            kind: SpanKind::Text,
        }
    }

    fn token(text: &str, style: Style) -> Span {
        Span {
            kind: SpanKind::PromptToken,
            ..span(text, style)
        }
    }

    #[test]
    fn bash_prompt_escapes_expansion_chars() {
        let spans = [span("a\\b$(x)`y`%", Style::default())];
        assert_eq!(encode_bash_prompt(&spans), "a\\\\\\\\b\\\\$(x)\\\\`y\\\\`%");
    }

    #[test]
    fn zsh_prompt_escapes_expansion_chars() {
        let spans = [span("a\\b$(x)`y`%", Style::default())];
        assert_eq!(encode_zsh_prompt(&spans), "a\\\\b\\$(x)\\`y\\`%%");
    }

    #[test]
    fn prompt_token_span_is_not_escaped() {
        let zsh = [token("%D{%H:%M}", Style::default())];
        assert_eq!(encode_zsh_prompt(&zsh), "%D{%H:%M}");
        let bash = [token("\\D{%H:%M}", Style::default())];
        assert_eq!(encode_bash_prompt(&bash), "\\D{%H:%M}");
    }

    #[test]
    fn ansi_encoder_ignores_span_kind() {
        let style = Style {
            fg: Some(Color::Named("green".to_owned())),
            bg: None,
            attrs: vec![],
        };
        let text_spans = [span("$x\\%", style.clone())];
        let token_spans = [token("$x\\%", style)];
        assert_eq!(encode_ansi(&text_spans), "\x1b[32m$x\\%\x1b[0m");
        assert_eq!(encode_ansi(&token_spans), encode_ansi(&text_spans));
    }

    #[test]
    fn prompt_dialects_emit_the_same_sgr_as_ansi() {
        let spans = [
            span(
                "a",
                Style {
                    fg: Some(Color::Named("green".to_owned())),
                    bg: Some(Color::Named("black".to_owned())),
                    attrs: vec![Attr::Bold],
                },
            ),
            span(" b", Style::default()),
        ];
        assert_eq!(encode_ansi(&spans), "\x1b[1;32;40ma\x1b[0m b");
        assert_eq!(
            encode_bash_prompt(&spans),
            "\\[\x1b[1;32;40m\\]a\\[\x1b[0m\\] b"
        );
        assert_eq!(encode_zsh_prompt(&spans), "%{\x1b[1;32;40m%}a%{\x1b[0m%} b");
    }

    /// Remove every `open`…`close` region from `encoded`.
    fn strip_regions(encoded: &str, open: &str, close: &str) -> String {
        let mut visible = String::new();
        let mut rest = encoded;
        while let Some((before, after_open)) = rest.split_once(open) {
            visible.push_str(before);
            let (_marked, after_close) = after_open
                .split_once(close)
                .unwrap_or_else(|| panic!("unclosed {open} in {encoded:?}"));
            rest = after_close;
        }
        visible.push_str(rest);
        visible
    }

    /// Default theme's directory segment spans plus displayed text.
    fn default_directory_spans() -> (Vec<Span>, String) {
        use crate::config::Config;
        use crate::formatter::SegmentPosition;
        use crate::formatter::directory_resolver::{DirectoryEnv, DirectoryResolver};
        use crate::template::{EvalContext, render};

        let config = Config::default();
        let theme_mgr =
            crate::theme::ThemeManager::builtin("default").expect("builtin default theme");
        let theme = theme_mgr.get();
        let format = theme
            .segments
            .directory
            .format
            .clone()
            .expect("default theme has a directory format");
        let resolver = DirectoryResolver::new(
            "/home/user/project",
            false,
            DirectoryEnv::unprobed(),
            &config,
            &theme,
            SegmentPosition::MIDDLE,
        );
        let ctx = EvalContext::new(&resolver)
            .with_prev_colors(None, Some(Color::Named("blue".to_owned())));
        let spans = render(&format, &ctx).expect("render default directory");
        let visible: String = spans.iter().map(|s| s.text.as_str()).collect();
        (spans, visible)
    }

    /// Readline counts bytes outside `\[ \]` as columns.
    #[test]
    fn bash_prompt_wraps_every_sgr_in_nonprinting_markers() {
        let (spans, visible) = default_directory_spans();
        let encoded = encode_bash_prompt(&spans);
        assert!(encoded.contains('\x1b'), "{encoded:?}");
        let printed = strip_regions(&encoded, "\\[", "\\]");
        assert!(!printed.contains('\x1b'), "{encoded:?}");
        assert_eq!(printed, visible);
    }

    /// ZLE counts bytes outside `%{ %}` as columns.
    #[test]
    fn zsh_prompt_wraps_every_sgr_in_nonprinting_markers() {
        let (spans, visible) = default_directory_spans();
        let encoded = encode_zsh_prompt(&spans);
        assert!(encoded.contains('\x1b'), "{encoded:?}");
        let printed = strip_regions(&encoded, "%{", "%}");
        assert!(!printed.contains('\x1b'), "{encoded:?}");
        assert_eq!(printed, visible);
    }

    #[test]
    fn named_fg_and_bg_with_attr() {
        let spans = [span(
            "main",
            Style {
                fg: Some(Color::Named("green".to_owned())),
                bg: Some(Color::Named("black".to_owned())),
                attrs: vec![Attr::Bold],
            },
        )];
        assert_eq!(encode_ansi(&spans), "\x1b[1;32;40mmain\x1b[0m");
    }

    #[test]
    fn default_color_maps_to_39_and_49() {
        let spans = [span(
            "x",
            Style {
                fg: Some(Color::Named("default".to_owned())),
                bg: Some(Color::Named("default".to_owned())),
                attrs: vec![],
            },
        )];
        assert_eq!(encode_ansi(&spans), "\x1b[39;49mx\x1b[0m");
    }

    #[test]
    fn rgb_and_ansi256() {
        let spans = [span(
            "y",
            Style {
                fg: Some(Color::Rgb { r: 1, g: 2, b: 3 }),
                bg: Some(Color::Ansi256(42)),
                attrs: vec![],
            },
        )];
        assert_eq!(encode_ansi(&spans), "\x1b[38;2;1;2;3;48;5;42my\x1b[0m");
    }

    #[test]
    fn unstyled_span_emits_plain_text_no_reset() {
        let spans = [span("plain", Style::default())];
        assert_eq!(encode_ansi(&spans), "plain");
    }

    #[test]
    fn styled_span_followed_by_default_span_resets_at_the_boundary() {
        let spans = [
            span(
                "studio",
                Style {
                    fg: Some(Color::Named("green".to_owned())),
                    bg: None,
                    attrs: vec![Attr::Bold, Attr::Dimmed],
                },
            ),
            span(" in ", Style::default()),
        ];
        assert_eq!(encode_ansi(&spans), "\x1b[1;2;32mstudio\x1b[0m in ");
    }

    #[test]
    fn consecutive_styled_spans_defer_reset_to_the_end() {
        let style = Style {
            fg: Some(Color::Named("green".to_owned())),
            bg: None,
            attrs: vec![],
        };
        let spans = [span("a", style.clone()), span("b", style)];
        assert_eq!(encode_ansi(&spans), "\x1b[32ma\x1b[32mb\x1b[0m");
    }

    #[test]
    fn adjacent_styled_spans_reset_dropped_attrs() {
        let spans = [
            span(
                "a",
                Style {
                    fg: Some(Color::Named("green".to_owned())),
                    bg: None,
                    attrs: vec![Attr::Bold],
                },
            ),
            span(
                "b",
                Style {
                    fg: Some(Color::Named("blue".to_owned())),
                    bg: None,
                    attrs: vec![],
                },
            ),
        ];
        assert_eq!(encode_ansi(&spans), "\x1b[1;32ma\x1b[0;34mb\x1b[0m");
    }

    #[test]
    fn adjacent_styled_spans_reset_dropped_background() {
        let spans = [
            span(
                "a",
                Style {
                    fg: None,
                    bg: Some(Color::Named("blue".to_owned())),
                    attrs: vec![],
                },
            ),
            span(
                "b",
                Style {
                    fg: Some(Color::Named("red".to_owned())),
                    bg: None,
                    attrs: vec![],
                },
            ),
        ];
        assert_eq!(encode_ansi(&spans), "\x1b[44ma\x1b[0;31mb\x1b[0m");
    }

    #[test]
    fn adjacent_styled_spans_without_leak_emit_no_reset() {
        let spans = [
            span(
                "a",
                Style {
                    fg: Some(Color::Named("white".to_owned())),
                    bg: Some(Color::Named("black".to_owned())),
                    attrs: vec![],
                },
            ),
            span(
                "b",
                Style {
                    fg: Some(Color::Named("black".to_owned())),
                    bg: Some(Color::Named("green".to_owned())),
                    attrs: vec![],
                },
            ),
        ];
        assert_eq!(encode_ansi(&spans), "\x1b[37;40ma\x1b[30;42mb\x1b[0m");
    }

    #[test]
    fn bright_named_uses_90s() {
        let spans = [span(
            "z",
            Style {
                fg: Some(Color::Named("bright-purple".to_owned())),
                bg: None,
                attrs: vec![],
            },
        )];
        assert_eq!(encode_ansi(&spans), "\x1b[95mz\x1b[0m");
    }

    #[test]
    fn prev_bg_transfers_across_a_two_segment_run() {
        use crate::template::{Color, EvalContext, MapResolver, render};

        // Segment A renders with bg:blue. Its bg becomes the "previous bg" fed to B.
        let a_resolver = MapResolver::from_pairs([("x", "A")]);
        let a_ctx = EvalContext::new(&a_resolver);
        let a_spans = render("[$x](bg:blue)", &a_ctx).unwrap();
        assert_eq!(encode_ansi(&a_spans), "\x1b[44mA\x1b[0m");

        // Segment B uses bg:prev_bg, fed Segment A's bg (blue).
        let b_resolver = MapResolver::from_pairs([("y", "B")]);
        let b_ctx = EvalContext::new(&b_resolver)
            .with_prev_colors(None, Some(Color::Named("blue".to_owned())));
        let b_spans = render("[$y](bg:prev_bg)", &b_ctx).unwrap();
        assert_eq!(encode_ansi(&b_spans), "\x1b[44mB\x1b[0m");
    }

    #[test]
    fn config_spelling_aliases_map_to_ansi_codes() {
        let styled = |fg: &str, bg: &str| {
            encode_ansi(&[span(
                "x",
                Style {
                    fg: Some(Color::Named(fg.to_owned())),
                    bg: Some(Color::Named(bg.to_owned())),
                    attrs: Vec::new(),
                },
            )])
        };
        assert_eq!(styled("magenta", "bright_black"), "\x1b[35;100mx\x1b[0m");
        assert_eq!(styled("gray", "grey"), "\x1b[90;100mx\x1b[0m");
        assert_eq!(styled("brred", "brblack"), "\x1b[91;100mx\x1b[0m");
        assert_eq!(
            styled("bright_magenta", "bright_red"),
            "\x1b[95;101mx\x1b[0m"
        );
    }

    fn rgb_span(red: u8, green: u8, blue: u8, bg: bool) -> Span {
        let color = Some(Color::Rgb {
            r: red,
            g: green,
            b: blue,
        });
        let style = if bg {
            Style {
                fg: None,
                bg: color,
                attrs: vec![],
            }
        } else {
            Style {
                fg: color,
                bg: None,
                attrs: vec![],
            }
        };
        span("x", style)
    }

    #[test]
    fn color_depth_downgrades_rgb() {
        let fg = [rgb_span(0xde, 0xa5, 0x84, false)];
        let bg = [rgb_span(0xde, 0xa5, 0x84, true)];
        let at = |spans: &[Span], depth| PromptDialect::Ansi.encode_depth(spans, depth);
        assert_eq!(
            at(&fg, ColorDepth::TrueColor),
            "\x1b[38;2;222;165;132mx\x1b[0m"
        );
        assert_eq!(at(&fg, ColorDepth::Ansi256), "\x1b[38;5;180mx\x1b[0m");
        assert_eq!(at(&bg, ColorDepth::Ansi256), "\x1b[48;5;180mx\x1b[0m");
        assert_eq!(at(&fg, ColorDepth::Ansi16), "\x1b[90mx\x1b[0m");
        assert_eq!(at(&bg, ColorDepth::Ansi16), "\x1b[100mx\x1b[0m");
        assert_eq!(at(&fg, ColorDepth::Mono), "x");
    }

    #[test]
    fn color_depth_maps_greys_and_indexed_colors() {
        let grey = [rgb_span(128, 128, 128, false)];
        assert_eq!(
            PromptDialect::Ansi.encode_depth(&grey, ColorDepth::Ansi256),
            "\x1b[38;5;244mx\x1b[0m"
        );
        let indexed = [span(
            "x",
            Style {
                fg: Some(Color::Ansi256(196)),
                bg: None,
                attrs: vec![],
            },
        )];
        assert_eq!(
            PromptDialect::Ansi.encode_depth(&indexed, ColorDepth::Ansi256),
            "\x1b[38;5;196mx\x1b[0m"
        );
        assert_eq!(
            PromptDialect::Ansi.encode_depth(&indexed, ColorDepth::Ansi16),
            "\x1b[91mx\x1b[0m"
        );
    }

    #[test]
    fn mono_depth_keeps_attributes_only() {
        let styled = [span(
            "x",
            Style {
                fg: Some(Color::Named("green".to_owned())),
                bg: Some(Color::Rgb { r: 1, g: 2, b: 3 }),
                attrs: vec![Attr::Bold],
            },
        )];
        assert_eq!(
            PromptDialect::Ansi.encode_depth(&styled, ColorDepth::Mono),
            "\x1b[1mx\x1b[0m"
        );
    }

    #[test]
    fn color_depth_env_resolution() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            ColorDepth::from_env(|key| {
                pairs
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| (*value).to_owned())
            })
        };
        assert_eq!(env(&[]), ColorDepth::TrueColor);
        assert_eq!(env(&[("TERM", "xterm-256color")]), ColorDepth::TrueColor);
        assert_eq!(
            env(&[("TERM_PROGRAM", "Apple_Terminal")]),
            ColorDepth::Ansi256
        );
        assert_eq!(env(&[("TERM", "linux")]), ColorDepth::Ansi256);
        assert_eq!(
            env(&[
                ("TERM_PROGRAM", "Apple_Terminal"),
                ("GPY_COLOR_DEPTH", "truecolor")
            ]),
            ColorDepth::TrueColor
        );
        assert_eq!(env(&[("GPY_COLOR_DEPTH", "16")]), ColorDepth::Ansi16);
        assert_eq!(env(&[("GPY_COLOR_DEPTH", "NONE")]), ColorDepth::Mono);
        assert_eq!(env(&[("GPY_COLOR_DEPTH", "bogus")]), ColorDepth::TrueColor);
    }
}
