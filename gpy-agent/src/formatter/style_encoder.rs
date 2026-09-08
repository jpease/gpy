//! Encode engine [`Span`]s into shell-specific escape sequences.
//!
//! One grammar, N encoders. `Palette`/`PrevFg`/`PrevBg` colors are resolved
//! away by the evaluator before reaching here; the encoder treats any residual
//! unresolved variant as a no-op so it can never panic.

use crate::template::{Attr, Color, Span, Style};
use std::fmt::Write as _;

/// Encode `spans` as raw ANSI SGR sequences (the live fish-ansi / ansi path).
///
/// A reset (`\x1b[0m`) is emitted whenever a styled span is immediately
/// followed by a default-style (no-code) span, not only at the very end. This
/// matches Starship's own convention of resetting exactly at a template
/// group's closing boundary, so literal text following a styled group (e.g.
/// the hostname preset's trailing ` in `) renders uncolored rather than
/// inheriting the preceding span's still-active SGR state. Themes whose
/// segments always carry a background color (so every span has non-empty
/// codes, e.g. the default/text pill themes) never hit the no-code branch and
/// are unaffected.
#[must_use]
pub fn encode_ansi(spans: &[Span]) -> String {
    let mut out = String::with_capacity(64_usize);
    let mut active = false;
    for span in spans {
        let codes = ansi_codes(&span.style);
        if codes.is_empty() {
            if active {
                out.push_str("\x1b[0m");
                active = false;
            }
            out.push_str(&span.text);
        } else {
            let _ = write!(out, "\x1b[{}m{}", codes.join(";"), span.text);
            active = true;
        }
    }
    if active {
        out.push_str("\x1b[0m");
    }
    out
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
    let base: u16 = match name {
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

/// Which per-group zsh resets a styled run still owes.
///
/// The reset is emitted at a group boundary and then cleared for the next group.
/// `active` marks whether any styled span is currently open (fg/bg alone counts).
#[derive(Default)]
struct ZshResetState {
    active: bool,
    /// Bitset over [`Attr`] categories needing a trailing reset (see [`Self::note_attr`]).
    attr_resets: u8,
}

impl ZshResetState {
    const UNDERLINE: u8 = 0b001;
    const STANDOUT: u8 = 0b010;
    const RAW_ATTR: u8 = 0b100;

    /// Record which reset `attr` will require when its group closes.
    const fn note_attr(&mut self, attr: Attr) {
        match attr {
            Attr::Underline => self.attr_resets |= Self::UNDERLINE,
            Attr::Inverted => self.attr_resets |= Self::STANDOUT,
            Attr::Dimmed | Attr::Italic | Attr::Blink | Attr::Hidden | Attr::Strikethrough => {
                self.attr_resets |= Self::RAW_ATTR;
            }
            Attr::Bold => {}
        }
    }

    /// Emit the accumulated reset (if a styled group is open) and clear state.
    fn flush(&mut self, out: &mut String) {
        if !self.active {
            return;
        }
        // Reset zsh-native fg/bg/bold unconditionally (matches the historical
        // baseline), then clear any extra attributes that were actually used.
        out.push_str("%f%k%b");
        if self.attr_resets & Self::UNDERLINE != 0 {
            out.push_str("%u");
        }
        if self.attr_resets & Self::STANDOUT != 0 {
            out.push_str("%s");
        }
        if self.attr_resets & Self::RAW_ATTR != 0 {
            // No zsh prompt escape resets dimmed/italic/blink/hidden/strikethrough,
            // so emit the SGR-off codes (2→22, 3→23, 5→25, 8→28, 9→29) raw.
            out.push_str("%{\x1b[22;23;25;28;29m%}");
        }
        *self = Self::default();
    }
}

/// Encode `spans` as zsh prompt escapes. Not wired into live output yet (#185);
/// shipped tested so the preset work can adopt it without re-deriving the mapping.
///
/// Mirrors [`encode_ansi`]'s group-boundary reset (#288): the reset is emitted
/// whenever a styled span is immediately followed by a default-style (empty
/// prelude) span, not only once at the very end. Otherwise literal default-style
/// text after a styled group would render with the prior group's style still
/// active.
#[must_use]
pub fn encode_zsh(spans: &[Span]) -> String {
    let mut out = String::with_capacity(64_usize);
    let mut state = ZshResetState::default();
    for span in spans {
        let prelude = zsh_prelude(&span.style);
        if prelude.is_empty() {
            state.flush(&mut out);
        } else {
            out.push_str(&prelude);
            state.active = true;
            for attr in &span.style.attrs {
                state.note_attr(*attr);
            }
        }
        out.push_str(&span.text);
    }
    state.flush(&mut out);
    out
}

/// Build the zsh escape prelude for one style.
fn zsh_prelude(style: &Style) -> String {
    let mut prelude = String::new();
    for attr in &style.attrs {
        prelude.push_str(zsh_attr_escape(*attr));
    }
    if let Some(fg) = &style.fg {
        zsh_push_color(&mut prelude, fg, false);
    }
    if let Some(bg) = &style.bg {
        zsh_push_color(&mut prelude, bg, true);
    }
    prelude
}

/// Map an engine attribute to its zsh prelude escape. Bold/underline/standout
/// use native prompt escapes; the rest emit raw SGR wrapped in `%{…%}` since
/// zsh prompt expansion has no escape for them.
const fn zsh_attr_escape(attr: Attr) -> &'static str {
    match attr {
        Attr::Bold => "%B",
        Attr::Underline => "%U",
        Attr::Inverted => "%S",
        Attr::Dimmed => "%{\x1b[2m%}",
        Attr::Italic => "%{\x1b[3m%}",
        Attr::Blink => "%{\x1b[5m%}",
        Attr::Hidden => "%{\x1b[8m%}",
        Attr::Strikethrough => "%{\x1b[9m%}",
    }
}

/// Append a zsh color escape for `color`. `%F{}`/`%K{}` for named/index;
/// raw truecolor SGR wrapped in `%{…%}` for rgb. Unresolved variants skipped.
fn zsh_push_color(prelude: &mut String, color: &Color, bg: bool) {
    let key = if bg { 'K' } else { 'F' };
    match color {
        Color::Named(name) => {
            let _ = write!(prelude, "%{key}{{{name}}}");
        }
        Color::Ansi256(index) => {
            let _ = write!(prelude, "%{key}{{{index}}}");
        }
        Color::Rgb { r, g, b } => {
            let lead = if bg { "48" } else { "38" };
            let _ = write!(prelude, "%{{\x1b[{lead};2;{r};{g};{b}m%}}");
        }
        Color::Palette(_) | Color::PrevFg | Color::PrevBg => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::encode_ansi;
    use crate::template::{Attr, Color, Span, Style};

    fn span(text: &str, style: Style) -> Span {
        Span {
            text: text.to_owned(),
            style,
        }
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
        // Regression for #265 (hostname preset parity): a styled group followed
        // by literal default-style text (e.g. `[$symbol$hostname](style) in `)
        // must reset immediately after the styled text, not only at the very
        // end — otherwise the trailing literal text visually inherits the
        // preceding span's still-active SGR state.
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
        // Two adjacent styled spans (e.g. `$symbol` then `$hostname` inside the
        // same group) each re-emit their own escape; the reset still only
        // appears once, after the last styled span.
        let style = Style {
            fg: Some(Color::Named("green".to_owned())),
            bg: None,
            attrs: vec![],
        };
        let spans = [span("a", style.clone()), span("b", style)];
        assert_eq!(encode_ansi(&spans), "\x1b[32ma\x1b[32mb\x1b[0m");
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
    fn zsh_named_fg_bg_bold() {
        use super::encode_zsh;
        let spans = [span(
            "main",
            Style {
                fg: Some(Color::Named("green".to_owned())),
                bg: Some(Color::Named("black".to_owned())),
                attrs: vec![Attr::Bold],
            },
        )];
        assert_eq!(encode_zsh(&spans), "%B%F{green}%K{black}main%f%k%b");
    }

    #[test]
    fn zsh_default_color_keywords() {
        use super::encode_zsh;
        let spans = [span(
            "x",
            Style {
                fg: Some(Color::Named("default".to_owned())),
                bg: Some(Color::Named("default".to_owned())),
                attrs: vec![],
            },
        )];
        assert_eq!(encode_zsh(&spans), "%F{default}%K{default}x%f%k%b");
    }

    #[test]
    fn zsh_rgb_wraps_raw_escape() {
        use super::encode_zsh;
        let spans = [span(
            "y",
            Style {
                fg: Some(Color::Rgb { r: 1, g: 2, b: 3 }),
                bg: None,
                attrs: vec![],
            },
        )];
        assert_eq!(encode_zsh(&spans), "%{\x1b[38;2;1;2;3m%}y%f%k%b");
    }

    #[test]
    fn zsh_plain_span_has_no_escapes() {
        use super::encode_zsh;
        let spans = [span("plain", Style::default())];
        assert_eq!(encode_zsh(&spans), "plain");
    }

    /// Underline uses the native `%U`/`%u` prompt escapes.
    #[test]
    fn zsh_underline_uses_native_escape() {
        use super::encode_zsh;
        let spans = [span(
            "u",
            Style {
                fg: None,
                bg: None,
                attrs: vec![Attr::Underline],
            },
        )];
        assert_eq!(encode_zsh(&spans), "%Uu%f%k%b%u");
    }

    /// Inverted maps to standout (`%S`/`%s`).
    #[test]
    fn zsh_inverted_uses_standout() {
        use super::encode_zsh;
        let spans = [span(
            "s",
            Style {
                fg: None,
                bg: None,
                attrs: vec![Attr::Inverted],
            },
        )];
        assert_eq!(encode_zsh(&spans), "%Ss%f%k%b%s");
    }

    /// Attributes without a zsh escape emit raw SGR and a raw SGR-off reset.
    #[test]
    fn zsh_dimmed_emits_raw_sgr_with_off_reset() {
        use super::encode_zsh;
        let spans = [span(
            "d",
            Style {
                fg: None,
                bg: None,
                attrs: vec![Attr::Dimmed],
            },
        )];
        assert_eq!(
            encode_zsh(&spans),
            "%{\x1b[2m%}d%f%k%b%{\x1b[22;23;25;28;29m%}"
        );
    }

    #[test]
    fn zsh_italic_blink_hidden_strikethrough_emit_raw_sgr() {
        use super::encode_zsh;
        for (attr, code) in [
            (Attr::Italic, "3"),
            (Attr::Blink, "5"),
            (Attr::Hidden, "8"),
            (Attr::Strikethrough, "9"),
        ] {
            let spans = [span(
                "x",
                Style {
                    fg: None,
                    bg: None,
                    attrs: vec![attr],
                },
            )];
            assert_eq!(
                encode_zsh(&spans),
                format!("%{{\x1b[{code}m%}}x%f%k%b%{{\x1b[22;23;25;28;29m%}}")
            );
        }
    }

    /// Regression for #232: the starship preset renders java as `red dimmed`.
    /// The old encoder dropped `dimmed`, so non-bold java rendered plain in zsh.
    #[test]
    fn zsh_red_dimmed_java_survives() {
        use super::encode_zsh;
        let spans = [span(
            "java",
            Style {
                fg: Some(Color::Named("red".to_owned())),
                bg: None,
                attrs: vec![Attr::Dimmed],
            },
        )];
        assert_eq!(
            encode_zsh(&spans),
            "%{\x1b[2m%}%F{red}java%f%k%b%{\x1b[22;23;25;28;29m%}"
        );
    }

    /// All eight attributes together: native escapes precede raw SGR, and the
    /// reset clears every category exactly once.
    #[test]
    fn zsh_all_attributes_round_trip() {
        use super::encode_zsh;
        let spans = [span(
            "all",
            Style {
                fg: None,
                bg: None,
                attrs: vec![
                    Attr::Bold,
                    Attr::Underline,
                    Attr::Inverted,
                    Attr::Dimmed,
                    Attr::Italic,
                    Attr::Blink,
                    Attr::Hidden,
                    Attr::Strikethrough,
                ],
            },
        )];
        assert_eq!(
            encode_zsh(&spans),
            "%B%U%S%{\x1b[2m%}%{\x1b[3m%}%{\x1b[5m%}%{\x1b[8m%}%{\x1b[9m%}\
             all%f%k%b%u%s%{\x1b[22;23;25;28;29m%}"
        );
    }

    /// Regression for #288: a styled group followed by literal default-style
    /// text must reset at the boundary.
    ///
    /// Otherwise the trailing text inherits the preceding span's still-active
    /// zsh style, the same bug the ansi encoder fixed.
    #[test]
    fn zsh_styled_span_followed_by_default_span_resets_at_the_boundary() {
        use super::encode_zsh;
        let spans = [
            span(
                "studio",
                Style {
                    fg: Some(Color::Named("green".to_owned())),
                    bg: None,
                    attrs: vec![Attr::Bold],
                },
            ),
            span(" in ", Style::default()),
        ];
        assert_eq!(encode_zsh(&spans), "%B%F{green}studio%f%k%b in ");
    }

    /// Two adjacent styled spans emit their own preludes but share a single
    /// trailing reset (mirrors the ansi consecutive-styled behavior).
    #[test]
    fn zsh_consecutive_styled_spans_defer_reset_to_the_end() {
        use super::encode_zsh;
        let style = Style {
            fg: Some(Color::Named("green".to_owned())),
            bg: None,
            attrs: vec![],
        };
        let spans = [span("a", style.clone()), span("b", style)];
        assert_eq!(encode_zsh(&spans), "%F{green}a%F{green}b%f%k%b");
    }

    /// Default-only spans never open a group, so no reset is emitted.
    #[test]
    fn zsh_default_only_spans_emit_no_reset() {
        use super::encode_zsh;
        let spans = [
            span("just ", Style::default()),
            span("text", Style::default()),
        ];
        assert_eq!(encode_zsh(&spans), "just text");
    }

    /// styled → default → styled: reset at the first boundary, and again at the
    /// end. Each styled group carries only its own attribute resets.
    #[test]
    fn zsh_styled_default_styled_resets_each_group() {
        use super::encode_zsh;
        let spans = [
            span(
                "a",
                Style {
                    fg: Some(Color::Named("green".to_owned())),
                    bg: None,
                    attrs: vec![Attr::Underline],
                },
            ),
            span(" x ", Style::default()),
            span(
                "b",
                Style {
                    fg: Some(Color::Named("blue".to_owned())),
                    bg: None,
                    attrs: vec![],
                },
            ),
        ];
        // First group used underline → its reset includes %u; the boundary reset
        // fires before " x ". The second group carries no extra-attr reset.
        assert_eq!(encode_zsh(&spans), "%U%F{green}a%f%k%b%u x %F{blue}b%f%k%b");
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
}
