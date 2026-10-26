//! Maps command duration to Starship-compatible `cmd_duration` variables.

use crate::config::Config;
use crate::formatter::SegmentPosition;
use crate::formatter::chrome::SegmentChrome;
use crate::formatter::separator::Glyphs;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact duration variable names.
pub struct DurationResolver<'a> {
    duration_ms: u64,
    theme: &'a ThemeConfig,
    chrome: SegmentChrome<'a>,
}

impl<'a> DurationResolver<'a> {
    /// Build a resolver for one duration segment render.
    #[must_use]
    pub fn new(
        duration_ms: u64,
        config: &Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        let colors = &theme.segments.duration;
        Self {
            duration_ms,
            theme,
            chrome: SegmentChrome::pill(
                pos,
                Glyphs::from(&config.ui),
                colors.text_color.as_str(),
                colors.bg_color.as_str(),
            ),
        }
    }

    /// Format milliseconds, e.g. `3_661_500` → `"1h 1m 1.500s"` (humanized: `"1h 1m 1s"`).
    fn format_duration(&self) -> String {
        let total = self.duration_ms;
        let hours = total / 3_600_000_u64;
        let rem = total % 3_600_000_u64;
        let minutes = rem / 60_000_u64;
        let sec_ms = rem % 60_000_u64;
        let sec_whole = sec_ms / 1_000_u64;
        let sec_frac = sec_ms % 1_000_u64;

        let mut parts: Vec<String> = Vec::with_capacity(3_usize);
        if hours > 0_u64 {
            parts.push(format!("{hours}h"));
        }
        if minutes > 0_u64 {
            parts.push(format!("{minutes}m"));
        }
        if self.theme.segments.duration.show_milliseconds {
            parts.push(format!("{sec_whole}.{sec_frac:03}s"));
        } else {
            parts.push(format!("{sec_whole}s"));
        }
        parts.join(" ")
    }
}

impl VariableResolver for DurationResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "duration" => Some(self.format_duration()),
            _ => self.chrome.resolve(name),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::DurationResolver;
    use crate::config::Config;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn theme() -> ThemeConfig {
        ThemeConfig::default()
    }

    #[test]
    fn zero_ms() {
        let thr = theme();
        let res = DurationResolver::new(
            0_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "0.000s");
    }

    #[test]
    fn sub_second_500ms() {
        let thr = theme();
        let res = DurationResolver::new(
            500_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "0.500s");
    }

    #[test]
    fn seconds_only_2500ms() {
        let thr = theme();
        let res = DurationResolver::new(
            2500_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "2.500s");
    }

    #[test]
    fn minutes_and_seconds_65000ms() {
        let thr = theme();
        let res = DurationResolver::new(
            65_000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "1m 5.000s");
    }

    #[test]
    fn hours_minutes_seconds_3661500ms() {
        let thr = theme();
        let res = DurationResolver::new(
            3_661_500_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "1h 1m 1.500s");
    }

    #[test]
    fn exact_one_minute() {
        let thr = theme();
        let res = DurationResolver::new(
            60_000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "1m 0.000s");
    }

    #[test]
    fn exact_one_hour() {
        let thr = theme();
        let res = DurationResolver::new(
            3_600_000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "1h 0.000s");
    }

    #[test]
    fn style_contains_fg_and_bg() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let style = res.resolve("style").expect("style present");
        assert!(style.starts_with("fg:"), "got {style}");
        assert!(style.contains("bg:"), "got {style}");
    }

    #[test]
    fn unknown_variable_is_none() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("nonsuch"), None);
    }

    #[test]
    fn sep_close_is_half_circle_when_last() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_is_triangle_when_not_last() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_close"), Some("\u{e0bc}".to_owned()));
    }

    #[test]
    fn sep_gap_is_space_when_not_last() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_gap"), Some(" ".to_owned()));
    }

    #[test]
    fn sep_gap_is_none_when_last() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_gap"), None);
    }

    #[test]
    fn sep_open_is_none_when_first() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::Yes),
        );
        assert_eq!(res.resolve("sep_open"), None);
    }

    #[test]
    fn sep_open_is_glyph_when_not_first() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("sep_open"), Some("\u{e0ba}".to_owned()));
    }

    #[test]
    fn bg_is_some_with_duration_bg_color() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert!(
            res.resolve("bg").is_some(),
            "bg should be non-None for duration segment"
        );
    }

    fn theme_no_ms() -> ThemeConfig {
        let mut t = ThemeConfig::default();
        t.segments.duration.show_milliseconds = false;
        t
    }

    #[test]
    fn humanized_seconds_only_2500ms() {
        let thr = theme_no_ms();
        let res = DurationResolver::new(
            2_500_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "2s");
    }

    #[test]
    fn humanized_sub_second_500ms() {
        let thr = theme_no_ms();
        let res = DurationResolver::new(
            500_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "0s");
    }

    #[test]
    fn humanized_minutes_and_seconds_65000ms() {
        let thr = theme_no_ms();
        let res = DurationResolver::new(
            65_000_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "1m 5s");
    }

    #[test]
    fn humanized_hours_minutes_seconds() {
        let thr = theme_no_ms();
        let res = DurationResolver::new(
            3_661_500_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "1h 1m 1s");
    }

    #[test]
    fn precise_mode_unchanged_by_default() {
        let thr = theme();
        let res = DurationResolver::new(
            2_500_u64,
            &Config::default(),
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "2.500s");
    }
}
