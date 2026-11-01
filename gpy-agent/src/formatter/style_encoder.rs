//! Encode engine [`Span`]s into shell-specific escape sequences. One SGR encoder
//! serves three [`PromptDialect`]s that differ only in text escaping and
//! zero-width markers. Unresolved palette/prev colors are no-ops, never panics.

use crate::template::{Attr, Color, Span, SpanKind, Style, canonical_ansi_name};

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

    /// Encode `spans` in this dialect.
    #[must_use]
    pub fn encode(self, spans: &[Span]) -> String {
        match self {
            Self::Ansi => encode_ansi(spans),
            Self::BashPrompt => encode_bash_prompt(spans),
            Self::ZshPrompt => encode_zsh_prompt(spans),
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
    encode_sgr(spans, ("\x1b[", "m"), |_| None)
}

/// Encode `spans` as bash `PS1` source: SGR inside `\[ \]`, text escaped;
/// `PromptToken` spans (`\D{…}`) emitted raw.
#[must_use]
pub fn encode_bash_prompt(spans: &[Span]) -> String {
    encode_sgr(spans, ("\\[\x1b[", "m\\]"), bash_escape)
}

/// Encode `spans` as zsh `PROMPT` source: SGR inside `%{ %}`, text escaped;
/// `PromptToken` spans (`%D{…}`) emitted raw.
#[must_use]
pub fn encode_zsh_prompt(spans: &[Span]) -> String {
    encode_sgr(spans, ("%{\x1b[", "m%}"), zsh_escape)
}

/// Span walk shared by every SGR encoder. `wrap` is the (open, close) text around
/// each SGR sequence; `escape` maps a text character to its replacement, if any.
/// A styled span that would inherit from the previous one folds in a reset
/// (`0;<codes>`); otherwise only new codes.
fn encode_sgr(
    spans: &[Span],
    wrap: (&str, &str),
    escape: fn(char) -> Option<&'static str>,
) -> String {
    let (open_seq, close_seq) = wrap;
    let mut out = String::with_capacity(64_usize);
    let mut open: Option<&Style> = None;
    for span in spans {
        if emits_codes(&span.style) {
            out.push_str(open_seq);
            if open.is_some_and(|before| leaks_into(before, &span.style)) {
                out.push_str("0;");
            }
            push_codes(&mut out, &span.style);
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
fn leaks_into(prev: &Style, next: &Style) -> bool {
    prev.attrs.iter().any(|attr| !next.attrs.contains(attr))
        || (sets_color(prev.fg.as_ref()) && !sets_color(next.fg.as_ref()))
        || (sets_color(prev.bg.as_ref()) && !sets_color(next.bg.as_ref()))
}

/// Whether `color` emits any SGR code (unresolved variants emit none).
fn sets_color(color: Option<&Color>) -> bool {
    match color {
        Some(Color::Named(name)) => named_base(name).is_some(),
        Some(Color::Rgb { .. } | Color::Ansi256(_)) => true,
        Some(Color::Palette(_) | Color::PrevFg | Color::PrevBg) | None => false,
    }
}

/// Whether `style` emits at least one SGR code.
fn emits_codes(style: &Style) -> bool {
    !style.attrs.is_empty() || sets_color(style.fg.as_ref()) || sets_color(style.bg.as_ref())
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
fn push_codes(out: &mut String, style: &Style) {
    let mut first = true;
    for attr in &style.attrs {
        push_separator(out, &mut first);
        out.push_str(attr_code(*attr));
    }
    if let Some(fg) = &style.fg {
        push_color(out, &mut first, fg, false);
    }
    if let Some(bg) = &style.bg {
        push_color(out, &mut first, bg, true);
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
/// background variants. Unresolved palette/prev variants write nothing
/// (defensive: eval resolves them).
fn push_color(out: &mut String, first: &mut bool, color: &Color, bg: bool) {
    let lead = if bg { "48" } else { "38" };
    match color {
        Color::Named(name) => {
            if let Some(base) = named_base(name) {
                push_separator(out, first);
                push_u8(out, if bg { base.saturating_add(10) } else { base });
            }
        }
        Color::Rgb { r, g, b } => {
            push_separator(out, first);
            out.push_str(lead);
            out.push_str(";2;");
            push_u8(out, *r);
            out.push(';');
            push_u8(out, *g);
            out.push(';');
            push_u8(out, *b);
        }
        Color::Ansi256(index) => {
            push_separator(out, first);
            out.push_str(lead);
            out.push_str(";5;");
            push_u8(out, *index);
        }
        Color::Palette(_) | Color::PrevFg | Color::PrevBg => {}
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

    use super::{encode_ansi, encode_bash_prompt, encode_zsh_prompt};
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
}
