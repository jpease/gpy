//! Maps the clock segment's configured time format to template variable names.
//!
//! Variable names: `time`, `style`, `bg`, `sep_open`, `sep_gap`, `sep_close`.
//!
//! Unlike every other segment resolver, `time` does not resolve to a rendered
//! value. The clock has to keep ticking between prompt draws, and a literal
//! timestamp baked into the response would freeze until the next `precmd`. So
//! `time` resolves to the *shell's own* live-time prompt token wrapping a
//! `strftime(3)` spec built from the theme's clock configuration:
//!
//! - Zsh emits `%D{…}`, expanded by zsh every time the prompt is drawn.
//! - Bash emits `\D{…}`, expanded by bash the same way.
//! - Fish has no prompt-level time token, so it gets the bare spec, and
//!   `segment_clock_render` (`fish/segments/clock.fish`) replaces that exact
//!   text with the time it formatted through `__gpy_clock_date_format`, whose
//!   spec must therefore match `ClockResolver::time_spec` character for
//!   character.
//!
//! This is what lets the agent own the caps, colors and delimiters — the same
//! template pipeline as every other segment — without an IPC round-trip per
//! second.

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, resolve_separator};
use crate::shell::Shell;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves the clock segment's template variables.
pub struct ClockResolver<'a> {
    shell: Shell,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    glyphs: Glyphs,
}

impl<'a> ClockResolver<'a> {
    /// Build a resolver for one clock segment render.
    #[must_use]
    pub const fn new(shell: Shell, theme: &'a ThemeConfig, pos: SegmentPosition) -> Self {
        Self {
            shell,
            theme,
            pos,
            glyphs: Glyphs::Nerd,
        }
    }

    /// Draw this segment with `glyphs` instead of the default [`Glyphs::Nerd`];
    /// callers holding a `Config` pass `Glyphs::from(&config.ui)` (#695).
    #[must_use]
    pub const fn with_glyphs(self, glyphs: Glyphs) -> Self {
        Self { glyphs, ..self }
    }

    /// Build the `strftime(3)` spec for the configured clock format.
    ///
    /// Identical, character for character, to Fish's `__gpy_clock_date_format`
    /// (`fish/segments/clock.fish`) and the Bash/Zsh copies: Fish finds this
    /// text in the agent's response and swaps in the formatted time. The
    /// no-pad `%-I`/`%-H` forms need no trimming step, which a
    /// prompt-expansion token could not perform.
    fn time_spec(&self) -> String {
        let clock = &self.theme.segments.clock;
        let is_24h = clock.time_format.as_deref() == Some("24");
        let leading_zero = clock.show_leading_zero.unwrap_or(false);
        let show_seconds = clock.show_seconds.unwrap_or(false);

        let hour = if is_24h {
            if leading_zero { "%H" } else { "%-H" }
        } else if leading_zero {
            "%I"
        } else {
            "%-I"
        };

        let mut spec = String::with_capacity(16_usize);
        spec.push_str(hour);
        spec.push_str(":%M");
        if show_seconds {
            spec.push_str(":%S");
        }
        if !is_24h {
            spec.push_str(" %p");
        }
        spec
    }

    /// Wrap the spec in the shell's live-time prompt token.
    fn time_token(&self) -> String {
        let spec = self.time_spec();
        match self.shell {
            Shell::Zsh => format!("%D{{{spec}}}"),
            Shell::Bash => format!("\\D{{{spec}}}"),
            Shell::Fish => spec,
        }
    }

    /// Style string: `fg:<text_color> bg:<bg_color>`.
    fn style(&self) -> String {
        let fg = self.theme.segments.clock.text_color.as_str();
        let bg = self.theme.segments.clock.bg_color.as_str();
        format!("fg:{fg} bg:{bg}")
    }
}

impl VariableResolver for ClockResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "time" => Some(self.time_token()),
            "style" => Some(self.style()),
            "bg" => Some(self.theme.segments.clock.bg_color.as_str().to_owned()),
            "sep_gap" => resolve_separator(self.pos, SeparatorStyle::Chained, self.glyphs)
                .gap
                .map(str::to_owned),
            "sep_close" => resolve_separator(self.pos, SeparatorStyle::Chained, self.glyphs)
                .close
                .map(str::to_owned),
            "sep_open" => resolve_separator(self.pos, SeparatorStyle::Chained, self.glyphs)
                .open
                .map(str::to_owned),
            _ => None,
        }
    }

    /// `time` is the shell's live-time token (`\D{…}`/`%D{…}`), which the
    /// shell must expand on every draw, so prompt encoders leave it unescaped.
    fn is_prompt_token(&self, name: &str) -> bool {
        name == "time"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::ClockResolver;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::shell::Shell;
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn mid() -> SegmentPosition {
        SegmentPosition::new(IsLast::No, IsFirst::No)
    }

    /// Default theme is 12-hour, no leading zero, no seconds.
    fn theme() -> ThemeConfig {
        ThemeConfig::default()
    }

    /// Two paired flags as enums rather than two `bool` parameters, matching
    /// the `IsLast`/`IsFirst` convention elsewhere in the formatter — the
    /// pedantic `fn_params_excessive_bools` lint is denied here.
    #[derive(Clone, Copy)]
    enum LeadingZero {
        Yes,
        No,
    }

    #[derive(Clone, Copy)]
    enum Seconds {
        Yes,
        No,
    }

    fn theme_with(time_format: &str, leading_zero: LeadingZero, seconds: Seconds) -> ThemeConfig {
        let mut t = ThemeConfig::default();
        t.segments.clock.time_format = Some(time_format.to_owned());
        t.segments.clock.show_leading_zero = Some(matches!(leading_zero, LeadingZero::Yes));
        t.segments.clock.show_seconds = Some(matches!(seconds, Seconds::Yes));
        t
    }

    #[test]
    fn zsh_gets_prompt_expansion_token() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%-I:%M %p}");
    }

    #[test]
    fn bash_gets_prompt_expansion_token() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Bash, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "\\D{%-I:%M %p}");
    }

    #[test]
    fn fish_gets_bare_spec() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Fish, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%-I:%M %p");
    }

    #[test]
    fn twelve_hour_no_leading_zero_is_the_default() {
        // The regression this whole path exists for: zsh used to hardcode
        // %H:%M:%S regardless of configuration, so a 12-hour theme rendered
        // as 17:21:42 next to Fish's 5:21 PM.
        let thr = theme_with("12", LeadingZero::No, Seconds::No);
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%-I:%M %p}");
    }

    #[test]
    fn twelve_hour_leading_zero() {
        let thr = theme_with("12", LeadingZero::Yes, Seconds::No);
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%I:%M %p}");
    }

    #[test]
    fn twenty_four_hour_drops_the_meridiem() {
        let thr = theme_with("24", LeadingZero::No, Seconds::No);
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%-H:%M}");
    }

    #[test]
    fn twenty_four_hour_leading_zero() {
        let thr = theme_with("24", LeadingZero::Yes, Seconds::No);
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%H:%M}");
    }

    #[test]
    fn seconds_are_appended_before_the_meridiem() {
        let thr = theme_with("12", LeadingZero::No, Seconds::Yes);
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%-I:%M:%S %p}");
    }

    #[test]
    fn seconds_in_twenty_four_hour() {
        let thr = theme_with("24", LeadingZero::Yes, Seconds::Yes);
        let res = ClockResolver::new(Shell::Bash, &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "\\D{%H:%M:%S}");
    }

    #[test]
    fn style_contains_fg_and_bg() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        let style = res.resolve("style").expect("style present");
        assert!(style.starts_with("fg:"), "got {style}");
        assert!(style.contains("bg:"), "got {style}");
    }

    #[test]
    fn bg_is_the_clock_background() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(
            res.resolve("bg").unwrap(),
            thr.segments.clock.bg_color.as_str()
        );
    }

    #[test]
    fn sep_close_is_half_circle_when_last() {
        let thr = theme();
        let res = ClockResolver::new(
            Shell::Zsh,
            &thr,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_is_triangle_when_not_last() {
        // The second half of the reported bug: the zsh clock emitted a bare
        // colored block with no closing cap, so the segment ended square
        // while every agent-rendered segment beside it ended in a triangle.
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("sep_close"), Some("\u{e0bc}".to_owned()));
    }

    #[test]
    fn sep_open_is_none_when_first() {
        let thr = theme();
        let res = ClockResolver::new(
            Shell::Zsh,
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::Yes),
        );
        assert_eq!(res.resolve("sep_open"), None);
    }

    #[test]
    fn sep_gap_is_space_when_not_last() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("sep_gap"), Some(" ".to_owned()));
    }

    #[test]
    fn unknown_variable_is_none() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &thr, mid());
        assert_eq!(res.resolve("nonsuch"), None);
    }
}
