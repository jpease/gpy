//! Encode engine [`Span`]s into shell-specific escape sequences. One SGR encoder
//! serves three [`PromptDialect`]s that differ only in text escaping and
//! zero-width markers. Unresolved palette/prev colors are no-ops, never panics.

use crate::template::{Attr, Color, Span, SpanKind, Style, canonical_ansi_name};
use std::fmt::Write as _;

/// Output dialect. Bash/zsh expand `PS1`/`PROMPT` as code on every draw, so
/// their dialects escape text; [`Self::Ansi`] (fish) emits it verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptDialect {
    /// Raw ANSI SGR with text verbatim (fish and generic terminals).
    #[default]
    Ansi,
    /// Bash `PS1` source (assumes `shopt -s promptvars`).
    BashPrompt,
    /// Zsh `PROMPT` source (assumes `prompt_subst`, `prompt_percent`, `no_prompt_bang`).
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
    encode_sgr(spans, push_sgr, |out, span| out.push_str(&span.text))
}

/// Encode `spans` as bash `PS1` source: SGR inside `\[ \]`, text escaped;
/// `PromptToken` spans (`\D{…}`) emitted raw.
#[must_use]
pub fn encode_bash_prompt(spans: &[Span]) -> String {
    encode_sgr(spans, push_bash_sgr, push_bash_text)
}

/// Encode `spans` as zsh `PROMPT` source: SGR inside `%{ %}`, text escaped;
/// `PromptToken` spans (`%D{…}`) emitted raw.
#[must_use]
pub fn encode_zsh_prompt(spans: &[Span]) -> String {
    encode_sgr(spans, push_zsh_sgr, push_zsh_text)
}

/// Span walk shared by every SGR encoder. A styled span that would inherit
/// from the previous one folds in a reset (`0;<codes>`); otherwise only new codes.
fn encode_sgr(
    spans: &[Span],
    sgr: impl Fn(&mut String, &str),
    text: impl Fn(&mut String, &Span),
) -> String {
    let mut out = String::with_capacity(64_usize);
    let mut open: Option<&Style> = None;
    for span in spans {
        let codes = ansi_codes(&span.style);
        if codes.is_empty() {
            if open.take().is_some() {
                sgr(&mut out, "0");
            }
        } else {
            let joined = codes.join(";");
            if open.is_some_and(|before| leaks_into(before, &span.style)) {
                sgr(&mut out, &format!("0;{joined}"));
            } else {
                sgr(&mut out, &joined);
            }
            open = Some(&span.style);
        }
        text(&mut out, span);
    }
    if open.is_some() {
        sgr(&mut out, "0");
    }
    out
}

/// Whether `next`, written after `prev` without reset, inherits from it (SGR is additive).
fn leaks_into(prev: &Style, next: &Style) -> bool {
    prev.attrs.iter().any(|attr| !next.attrs.contains(attr))
        || (sets_color(prev.fg.as_ref(), false) && !sets_color(next.fg.as_ref(), false))
        || (sets_color(prev.bg.as_ref(), true) && !sets_color(next.bg.as_ref(), true))
}

/// Whether `color` emits any SGR code (unresolved variants emit none).
fn sets_color(color: Option<&Color>, bg: bool) -> bool {
    color.is_some_and(|value| {
        let mut codes = Vec::new();
        push_color(&mut codes, value, bg);
        !codes.is_empty()
    })
}

/// Write one raw SGR sequence, `ESC [ codes m`.
fn push_sgr(out: &mut String, codes: &str) {
    let _ = write!(out, "\x1b[{codes}m");
}

/// Write one SGR sequence inside bash's non-printing markers `\[ \]`.
fn push_bash_sgr(out: &mut String, codes: &str) {
    let _ = write!(out, "\\[\x1b[{codes}m\\]");
}

/// Write one SGR sequence inside zsh's non-printing markers `%{ %}`.
fn push_zsh_sgr(out: &mut String, codes: &str) {
    let _ = write!(out, "%{{\x1b[{codes}m%}}");
}

/// Write text escaped for bash `PS1` with `promptvars`: prompt decoding then
/// double-quote expansion, so `\` needs four backslashes and `$`/`` ` `` need `\\`.
/// `\$` is avoided: it decodes to `#` when euid is 0.
fn push_bash_text(out: &mut String, span: &Span) {
    if span.kind == SpanKind::PromptToken {
        out.push_str(&span.text);
        return;
    }
    for ch in span.text.chars() {
        match ch {
            '\\' => out.push_str("\\\\\\\\"),
            '$' => out.push_str("\\\\$"),
            '`' => out.push_str("\\\\`"),
            _ => out.push(ch),
        }
    }
}

/// Write a span's text escaped for zsh `PROMPT` with `prompt_subst` and
/// `prompt_percent` on: backslash-quote `\`, `$` and `` ` ``, and double `%`.
fn push_zsh_text(out: &mut String, span: &Span) {
    if span.kind == SpanKind::PromptToken {
        out.push_str(&span.text);
        return;
    }
    for ch in span.text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '$' => out.push_str("\\$"),
            '`' => out.push_str("\\`"),
            '%' => out.push_str("%%"),
            _ => out.push(ch),
        }
    }
}

/// Collect SGR numeric codes (as strings) for one style, in attr→fg→bg order.
fn ansi_codes(style: &Style) -> Vec<String> {
    let mut codes: Vec<String> = Vec::new();
    for attr in &style.attrs {
        codes.push(attr_code(*attr).to_owned());
    }
    if let Some(fg) = &style.fg {
        push_color(&mut codes, fg, false);
    }
    if let Some(bg) = &style.bg {
        push_color(&mut codes, bg, true);
    }
    codes
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

/// Push SGR codes for `color` onto `codes`. `bg` selects background variants.
/// Unresolved palette/prev variants are skipped (defensive: eval resolves them).
fn push_color(codes: &mut Vec<String>, color: &Color, bg: bool) {
    match color {
        Color::Named(name) => {
            if let Some(code) = named_code(name, bg) {
                codes.push(code);
            }
        }
        Color::Rgb { r, g, b } => {
            let lead = if bg { "48" } else { "38" };
            codes.push(format!("{lead};2;{r};{g};{b}"));
        }
        Color::Ansi256(index) => {
            let lead = if bg { "48" } else { "38" };
            codes.push(format!("{lead};5;{index}"));
        }
        Color::Palette(_) | Color::PrevFg | Color::PrevBg => {}
    }
}

/// Map an engine named color to its SGR code string, or `None` if unrecognized.
fn named_code(name: &str, bg: bool) -> Option<String> {
    let canonical = if name == "default" {
        name
    } else {
        canonical_ansi_name(name)?
    };
    let base: u16 = match canonical {
        "black" => 30,
        "red" => 31,
        "green" => 32,
        "yellow" => 33,
        "blue" => 34,
        "purple" => 35,
        "cyan" => 36,
        "white" => 37,
        "default" => 39,
        "bright-black" => 90,
        "bright-red" => 91,
        "bright-green" => 92,
        "bright-yellow" => 93,
        "bright-blue" => 94,
        "bright-purple" => 95,
        "bright-cyan" => 96,
        "bright-white" => 97,
        _ => return None,
    };
    let code = if bg { base.checked_add(10)? } else { base };
    Some(code.to_string())
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
        use crate::formatter::directory_resolver::DirectoryResolver;
        use crate::template::{RenderContext as TemplateContext, render};

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
            &config,
            &theme,
            SegmentPosition::MIDDLE,
        );
        let ctx = TemplateContext::new(&resolver)
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
        use crate::template::{Color, MapResolver, RenderContext as TemplateContext, render};

        // Segment A renders with bg:blue. Its bg becomes the "previous bg" fed to B.
        let a_resolver = MapResolver::from_pairs([("x", "A")]);
        let a_ctx = TemplateContext::new(&a_resolver);
        let a_spans = render("[$x](bg:blue)", &a_ctx).unwrap();
        assert_eq!(encode_ansi(&a_spans), "\x1b[44mA\x1b[0m");

        // Segment B uses bg:prev_bg, fed Segment A's bg (blue).
        let b_resolver = MapResolver::from_pairs([("y", "B")]);
        let b_ctx = TemplateContext::new(&b_resolver)
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
