//! Maps command duration milliseconds to Starship-exact template variable names.
//!
//! Variable names: `duration`, `style`.
//! Compatible with Starship's `cmd_duration` module.

use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, resolve_separator};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

/// Resolves Starship-exact duration variable names.
pub struct DurationResolver<'a> {
    duration_ms: u64,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    glyphs: Glyphs,
}

impl<'a> DurationResolver<'a> {
    /// Build a resolver for one duration segment render.
    #[must_use]
    pub const fn new(duration_ms: u64, theme: &'a ThemeConfig, pos: SegmentPosition) -> Self {
        Self {
            duration_ms,
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

    /// Format milliseconds as a human-readable duration string.
    ///
    /// Algorithm (integer math only, no floats):
    /// - `hours   = ms / 3_600_000`
    /// - `rem     = ms % 3_600_000`
    /// - `minutes = rem / 60_000`
    /// - `sec_ms  = rem % 60_000`
    ///
    /// Output: space-joined components.  Hours prepended iff `hours > 0`,
    /// minutes prepended iff `minutes > 0`, seconds always present as
    /// `"{sec_ms/1000}.{sec_ms%1000:03}s"`.
    ///
    /// Examples (precise / humanized):
    /// - `0`          → `"0.000s"` / `"0s"`
    /// - `500`        → `"0.500s"` / `"0s"`
    /// - `2500`       → `"2.500s"` / `"2s"`
    /// - `65000`      → `"1m 5.000s"` / `"1m 5s"`
    /// - `3_661_500`  → `"1h 1m 1.500s"` / `"1h 1m 1s"`
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

    /// Style string: `fg:<text_color> bg:<bg_color>`.
    fn style(&self) -> String {
        let fg = self.theme.segments.duration.text_color.as_str();
        let bg = self.theme.segments.duration.bg_color.as_str();
        format!("fg:{fg} bg:{bg}")
    }
}

impl VariableResolver for DurationResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "duration" => Some(self.format_duration()),
            "style" => Some(self.style()),
            "bg" => Some(self.theme.segments.duration.bg_color.as_str().to_owned()),
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::DurationResolver;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    fn theme() -> ThemeConfig {
        ThemeConfig::default()
    }

    #[test]
    fn zero_ms() {
        let thr = theme();
        let res = DurationResolver::new(0_u64, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        assert_eq!(res.resolve("duration").unwrap(), "0.000s");
    }

    #[test]
    fn sub_second_500ms() {
        let thr = theme();
        let res =
            DurationResolver::new(500_u64, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        assert_eq!(res.resolve("duration").unwrap(), "0.500s");
    }

    #[test]
    fn seconds_only_2500ms() {
        let thr = theme();
        let res = DurationResolver::new(
            2500_u64,
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
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        // No minutes component when minutes == 0.
        assert_eq!(res.resolve("duration").unwrap(), "1h 0.000s");
    }

    #[test]
    fn style_contains_fg_and_bg() {
        let thr = theme();
        let res = DurationResolver::new(
            1000_u64,
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
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "2s");
    }

    #[test]
    fn humanized_sub_second_500ms() {
        let thr = theme_no_ms();
        let res =
            DurationResolver::new(500_u64, &thr, SegmentPosition::new(IsLast::No, IsFirst::No));
        assert_eq!(res.resolve("duration").unwrap(), "0s");
    }

    #[test]
    fn humanized_minutes_and_seconds_65000ms() {
        let thr = theme_no_ms();
        let res = DurationResolver::new(
            65_000_u64,
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
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "1h 1m 1s");
    }

    #[test]
    fn precise_mode_unchanged_by_default() {
        // Default theme keeps millisecond precision — no regression.
        let thr = theme();
        let res = DurationResolver::new(
            2_500_u64,
            &thr,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(res.resolve("duration").unwrap(), "2.500s");
    }
}
