//! Maps the clock segment's time format to template variables.
//!
//! `time` resolves to the shell's live-time token (zsh `%D{…}`, bash `\D{…}`) so the
//! clock ticks between redraws; Fish gets the bare spec, replaced by `clock.fish`.

use crate::config::Config;
use crate::formatter::SegmentPosition;
use crate::formatter::chrome::SegmentChrome;
use crate::formatter::separator::Glyphs;
use crate::shell::Shell;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves the clock segment's template variables.
pub struct ClockResolver<'a> {
    shell: Shell,
    theme: &'a ThemeConfig,
    chrome: SegmentChrome<'a>,
}

impl<'a> ClockResolver<'a> {
    /// Build a resolver for one clock segment render.
    #[must_use]
    pub fn new(
        shell: Shell,
        config: &Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        let colors = &theme.segments.clock;
        Self {
            shell,
            theme,
            chrome: SegmentChrome::pill(
                pos,
                Glyphs::from(&config.ui),
                colors.text_color.as_str(),
                colors.bg_color.as_str(),
            ),
        }
    }

    /// Build the `strftime(3)` spec for the configured clock format.
    /// Must match Fish's `__gpy_clock_date_format` exactly; Fish substitutes this text.
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
}

impl VariableResolver for ClockResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "time" => Some(self.time_token()),
            _ => self.chrome.resolve(name),
        }
    }

    /// `time` is a live shell token, so prompt encoders must leave it unescaped.
    fn is_prompt_token(&self, name: &str) -> bool {
        name == "time"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::ClockResolver;
    use crate::config::Config;
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

    /// Paired enum flags avoid pedantic `fn_params_excessive_bools`.
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
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%-I:%M %p}");
    }

    #[test]
    fn bash_gets_prompt_expansion_token() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Bash, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "\\D{%-I:%M %p}");
    }

    #[test]
    fn fish_gets_bare_spec() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Fish, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%-I:%M %p");
    }

    #[test]
    fn twelve_hour_no_leading_zero_is_the_default() {
        // zsh must honor the configured 12-hour format.
        let thr = theme_with("12", LeadingZero::No, Seconds::No);
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%-I:%M %p}");
    }

    #[test]
    fn twelve_hour_leading_zero() {
        let thr = theme_with("12", LeadingZero::Yes, Seconds::No);
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%I:%M %p}");
    }

    #[test]
    fn twenty_four_hour_drops_the_meridiem() {
        let thr = theme_with("24", LeadingZero::No, Seconds::No);
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%-H:%M}");
    }

    #[test]
    fn twenty_four_hour_leading_zero() {
        let thr = theme_with("24", LeadingZero::Yes, Seconds::No);
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%H:%M}");
    }

    #[test]
    fn seconds_are_appended_before_the_meridiem() {
        let thr = theme_with("12", LeadingZero::No, Seconds::Yes);
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "%D{%-I:%M:%S %p}");
    }

    #[test]
    fn seconds_in_twenty_four_hour() {
        let thr = theme_with("24", LeadingZero::Yes, Seconds::Yes);
        let res = ClockResolver::new(Shell::Bash, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("time").unwrap(), "\\D{%H:%M:%S}");
    }

    #[test]
    fn style_contains_fg_and_bg() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        let style = res.resolve("style").expect("style present");
        assert!(style.starts_with("fg:"), "got {style}");
        assert!(style.contains("bg:"), "got {style}");
    }

    #[test]
    fn bg_is_the_clock_background() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
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
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_is_triangle_when_not_last() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("sep_close"), Some("\u{e0bc}".to_owned()));
    }

    #[test]
    fn sep_open_is_none_when_first() {
        let thr = theme();
        let res = ClockResolver::new(
            Shell::Zsh,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::Yes),
        );
        assert_eq!(res.resolve("sep_open"), None);
    }

    #[test]
    fn sep_gap_is_space_when_not_last() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("sep_gap"), Some(" ".to_owned()));
    }

    #[test]
    fn unknown_variable_is_none() {
        let thr = theme();
        let res = ClockResolver::new(Shell::Zsh, &Config::default(), &thr, mid());
        assert_eq!(res.resolve("nonsuch"), None);
    }
}
