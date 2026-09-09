//! Output formatting layer
//!
//! This module provides a unified interface for formatting agent responses
//! in different output formats (JSON, Fish, etc.) without requiring changes
//! to core agent logic.
//!
//! # Architecture
//!
//! The formatter layer consists of:
//! - [`crate::formatter::Format`] enum: Supported output formats
//! - [`crate::formatter::Formatter`] trait: Common interface for all formatters
//! - [`crate::formatter::RenderContext`]: Shared context (theme, config, delimiters)
//! - Format-specific implementations: [`crate::formatter::JsonFormatter`],
//!   [`crate::formatter::AnsiFormatter`], etc.
//!
//! # Adding New Formats
//!
//! To add support for a new shell (e.g., Bash, Zsh):
//!
//! 1. Add variant to [`crate::formatter::Format`] enum
//! 2. Implement [`crate::formatter::Formatter`] trait for your format
//! 3. Add factory case in [`crate::formatter::create_formatter()`]
//! 4. Write tests covering the new formatter
//!
//! # Example
//!
//! ```text
//! use gpy_agent::formatter::{Format, RenderContext, create_formatter};
//! use gpy_agent::ipc::Response;
//!
//! let ctx = RenderContext::new(config, theme, SegmentPosition::MIDDLE);
//! let formatter = create_formatter(Format::Ansi);
//! let output = formatter.render(&response, &ctx)?;
//! ```

pub(crate) mod character_resolver;
pub(crate) mod clock_resolver;
pub(crate) mod directory_resolver;
pub(crate) mod duration_resolver;
mod fish;
mod fish_ansi;
mod fish_source;
pub(crate) mod git_resolver;
pub(crate) mod hostname_resolver;
mod json;
pub(crate) mod language_resolver;
pub(crate) mod separator;
mod style_encoder;
pub(crate) mod username_resolver;

// The ANSI format is shell-agnostic and was previously a thin delegating
// wrapper around `FishAnsiFormatter`; re-export it directly instead of
// keeping a separate module whose only job was forwarding `render()`.
pub use fish::FishFormatter;
pub use fish_ansi::FishAnsiFormatter as AnsiFormatter;
// Zsh's shell-argument rendering is byte-identical to Fish's (both just emit
// space-separated `--flag value` pairs, which Zsh's own arg-parsing side
// consumes the same way Fish's does), so `ZshFormatter` is a re-export of the
// same implementation rather than a separate duplicated one.
pub use fish::FishFormatter as ZshFormatter;
pub use fish_ansi::FishAnsiFormatter;
pub(crate) use fish_ansi::{get_language_display, select_languages};
pub use fish_source::FishSourceFormatter;
pub use json::JsonFormatter;
pub use style_encoder::{encode_ansi, encode_zsh};

use crate::Result;
use crate::config::Config;
use crate::ipc::Response;
use crate::theme::ThemeConfig;
use clap::{ValueEnum, builder::PossibleValue};
use serde::{Deserialize, Serialize};

/// Supported output formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// JSON format (default)
    #[default]
    Json,
    /// Shell-agnostic ANSI escape codes (works with Fish, Zsh, Bash)
    /// This is the recommended format for shell integration.
    Ansi,
    /// Fish shell arguments (space-separated flags)
    Fish,
    /// Fish shell source code (variable assignments)
    FishSource,
    /// Bash source code (future)
    BashSource,
    /// Zsh arguments (space-separated flags)
    Zsh,
    /// Zsh source code (variable assignments)
    ZshSource,
}

impl Format {
    /// Convert to string representation
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Ansi => "ansi",
            Self::Fish => "fish",
            Self::FishSource => "fish-source",
            Self::BashSource => "bash-source",
            Self::Zsh => "zsh",
            Self::ZshSource => "zsh-source",
        }
    }
    /// Returns `true` when the provided format is JSON.
    #[must_use]
    pub const fn is_json(format: Self) -> bool {
        matches!(format, Self::Json)
    }

    /// Returns `true` when the provided format is JSON (reference version for serde).
    #[must_use]
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde skip_serializing_if signature requires &Self"
    )]
    pub const fn is_json_ref(format: &Self) -> bool {
        Self::is_json(*format)
    }

    /// Returns `true` when [`create_formatter`] can produce a real formatter for
    /// this format today.
    ///
    /// This is the single source of truth for the "not yet implemented" format
    /// set (`BashSource`/`ZshSource`). Callers that need to short-circuit before
    /// reaching [`create_formatter`] — e.g. to silently degrade to an empty
    /// string instead of propagating its error — should consult this method
    /// rather than hand-rolling a `matches!` against the not-yet-implemented
    /// variants.
    #[must_use]
    pub const fn is_renderable(self) -> bool {
        match self {
            Self::Json | Self::Ansi | Self::Fish | Self::FishSource | Self::Zsh => true,
            Self::BashSource | Self::ZshSource => false,
        }
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Format {
    type Err = crate::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "json" => Ok(Self::Json),
            "ansi" => Ok(Self::Ansi),
            "fish" => Ok(Self::Fish),
            "fish-source" => Ok(Self::FishSource),
            "bash-source" => Ok(Self::BashSource),
            "zsh" => Ok(Self::Zsh),
            "zsh-source" => Ok(Self::ZshSource),
            _ => Err(crate::Error::config(format!("Unknown format: {s}"))),
        }
    }
}

impl ValueEnum for Format {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::Json,
            Self::Ansi,
            Self::Fish,
            Self::FishSource,
            Self::Zsh,
            Self::ZshSource,
        ]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(match self {
            Self::Json => PossibleValue::new("json"),
            Self::Ansi => PossibleValue::new("ansi"),
            Self::Fish => PossibleValue::new("fish"),
            Self::FishSource => PossibleValue::new("fish-source"),
            Self::BashSource => PossibleValue::new("bash-source"),
            Self::Zsh => PossibleValue::new("zsh"),
            Self::ZshSource => PossibleValue::new("zsh-source"),
        })
    }
}

/// Whether this is the last agent-rendered segment in the prompt.
///
/// Controls whether a closing powerline cap (`[$sep_close](fg:$bg)`) is rendered.
/// Using an enum instead of `bool` makes call sites self-documenting and satisfies
/// the `fn_params_excessive_bools` pedantic lint when combined with another `bool` param.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IsLast {
    /// This is the last segment — emit the closing powerline cap.
    Yes,
    /// This is not the last segment — no closing cap needed.
    No,
}

impl IsLast {
    /// Convert to `bool`: `Yes` → `true`, `No` → `false`.
    #[must_use]
    pub(crate) const fn as_bool(self) -> bool {
        matches!(self, Self::Yes)
    }
}

impl From<bool> for IsLast {
    fn from(b: bool) -> Self {
        if b { Self::Yes } else { Self::No }
    }
}

/// Whether this is the first agent-rendered segment in the prompt.
///
/// Controls whether an opening powerline cap (`[$sep_open](fg:$bg bg:default)`)
/// is rendered. Mirrors [`IsLast`]/`$sep_close` exactly, but for the leading
/// edge of the chain instead of the trailing one: the first segment has
/// nothing before it to transition from, so it should render flush rather
/// than with a cap that transitions from the terminal's default background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum IsFirst {
    /// This is the first segment — suppress the opening powerline cap.
    Yes,
    /// This is not the first segment — render the opening cap as usual.
    #[default]
    No,
}

impl IsFirst {
    /// Convert to `bool`: `Yes` → `true`, `No` → `false`.
    #[must_use]
    pub(crate) const fn as_bool(self) -> bool {
        matches!(self, Self::Yes)
    }
}

impl From<bool> for IsFirst {
    fn from(b: bool) -> Self {
        if b { Self::Yes } else { Self::No }
    }
}

/// A segment's position within the rendered chain (#586).
///
/// Whether it is the last segment (controls the trailing separator/chevron
/// shape) and whether it is the first (controls whether a leading separator
/// is drawn).
///
/// Bundles [`IsLast`] and [`IsFirst`] into one value so callers that already
/// hold both — the common case — pass a single parameter, and so
/// [`RenderContext`] can store one field instead of two loose bools that every
/// consumer then has to convert back into `IsLast`/`IsFirst`.
///
/// The fields are crate-internal because `IsLast`/`IsFirst` are; outside the
/// crate the four named constants below name every position there is. There
/// is deliberately no `from_bools(bool, bool)` constructor: two bool
/// parameters are exactly what `.clippy.toml`'s `max-fn-params-bools = 1`
/// forbids, and dodging it here would reintroduce the bool plumbing this type
/// exists to remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentPosition {
    /// Whether this is the last segment in the prompt.
    pub(crate) is_last: IsLast,
    /// Whether this is the first segment in the prompt.
    pub(crate) is_first: IsFirst,
}

impl SegmentPosition {
    /// Neither first nor last: a segment with chain neighbours on both sides.
    pub const MIDDLE: Self = Self::new(IsLast::No, IsFirst::No);
    /// First in the chain, with segments still to come after it.
    pub const FIRST: Self = Self::new(IsLast::No, IsFirst::Yes);
    /// Last in the chain, with segments before it.
    pub const LAST: Self = Self::new(IsLast::Yes, IsFirst::No);
    /// The chain's only segment — both first and last.
    pub const ONLY: Self = Self::new(IsLast::Yes, IsFirst::Yes);

    /// Build a position from the two newtypes.
    #[must_use]
    pub(crate) const fn new(is_last: IsLast, is_first: IsFirst) -> Self {
        Self { is_last, is_first }
    }
}

/// Rendering context shared across formatters
#[derive(Clone)]
pub struct RenderContext<'a> {
    /// Theme configuration
    pub theme: &'a ThemeConfig,
    /// Agent configuration
    pub config: &'a Config,
    /// Where this segment sits in the chain (last/first), as one value rather
    /// than two bools the renderers would have to re-widen into
    /// [`IsLast`]/[`IsFirst`] (#586).
    pub position: SegmentPosition,
    /// Previous segment's foreground (feeds the engine's `prev_fg`).
    pub prev_fg: Option<crate::template::Color>,
    /// Previous segment's background (feeds the engine's `prev_bg`).
    pub prev_bg: Option<crate::template::Color>,
    /// Active color palette, supplied to the template engine for color resolution.
    pub palette: crate::template::Palette,
}

impl<'a> RenderContext<'a> {
    /// Create new render context with no previous-segment colors and an empty palette.
    #[must_use]
    pub fn new(config: &'a Config, theme: &'a ThemeConfig, position: SegmentPosition) -> Self {
        Self {
            theme,
            config,
            position,
            prev_fg: None,
            prev_bg: None,
            palette: crate::template::Palette::default(),
        }
    }

    /// Attach previous-segment colors for powerline `prev_fg`/`prev_bg` transfer.
    #[must_use]
    pub fn with_prev_colors(
        mut self,
        fg: Option<crate::template::Color>,
        bg: Option<crate::template::Color>,
    ) -> Self {
        self.prev_fg = fg;
        self.prev_bg = bg;
        self
    }

    /// Attach the active palette used for template color resolution.
    #[must_use]
    pub fn with_palette(mut self, palette: crate::template::Palette) -> Self {
        self.palette = palette;
        self
    }
}

/// Common interface for all output formatters
pub trait Formatter {
    /// Render a response in this format
    ///
    /// # Errors
    ///
    /// Returns an error when serialization or UTF-8 conversion fails for the requested format.
    fn render(&self, response: &Response, ctx: &RenderContext<'_>) -> Result<String>;
}

/// Create a formatter for the given format
///
/// # Errors
///
/// Returns an error if the requested format is not yet implemented.
pub fn create_formatter(format: Format) -> Result<Box<dyn Formatter>> {
    match format {
        Format::Json => Ok(Box::new(JsonFormatter)),
        Format::Ansi => Ok(Box::new(AnsiFormatter)),
        Format::Fish => Ok(Box::new(FishFormatter)),
        Format::FishSource => Ok(Box::new(FishSourceFormatter)),
        Format::Zsh => Ok(Box::new(ZshFormatter)),
        Format::BashSource | Format::ZshSource => Err(crate::Error::config(format!(
            "Format '{format}' not yet implemented. Use 'ansi' (works with all shells) or 'json'.",
        ))),
    }
}

/// Truncate a branch name if it exceeds `max_length` Unicode characters.
///
/// If `max_length` is 0, no truncation is performed. Appends `…` as the final
/// character when truncation occurs, so the result is exactly `max_length` chars.
///
/// Measured in Unicode characters (not bytes) to correctly handle emoji, CJK, etc.
#[must_use]
pub(crate) fn truncate_branch(branch: &str, max_length: usize) -> String {
    if max_length == 0 {
        return branch.to_owned();
    }

    let char_count = branch.chars().count();
    if char_count <= max_length {
        return branch.to_owned();
    }

    let truncated_len = max_length.saturating_sub(1);
    let mut result: String = branch.chars().take(truncated_len).collect();
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{RenderContext, SegmentPosition, truncate_branch};
    use crate::config::Config;
    use crate::theme::ThemeConfig;

    #[test]
    fn render_context_defaults_prev_colors_to_none() {
        let config = Config::default();
        let theme = ThemeConfig::default();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        assert!(ctx.prev_fg.is_none());
        assert!(ctx.prev_bg.is_none());
    }

    #[test]
    fn with_prev_colors_sets_fields() {
        use crate::template::Color;
        let config = Config::default();
        let theme = ThemeConfig::default();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE)
            .with_prev_colors(None, Some(Color::Named("blue".to_owned())));
        assert_eq!(ctx.prev_fg, None);
        assert_eq!(ctx.prev_bg, Some(Color::Named("blue".to_owned())));
    }

    #[test]
    fn truncate_branch_handles_unicode_correctly() {
        let res1 = truncate_branch("feature/very-long-branch", 15);
        assert_eq!(res1, "feature/very-l…");
        assert_eq!(res1.chars().count(), 15);

        let emoji_branch = "feature/🚀-rocket-launch";
        assert_eq!(emoji_branch.len(), 26);
        assert_eq!(emoji_branch.chars().count(), 23);
        let res2 = truncate_branch(emoji_branch, 15);
        assert_eq!(res2, "feature/🚀-rock…");
        assert_eq!(res2.chars().count(), 15);

        let cjk_branch = "功能/新功能-开发";
        assert_eq!(cjk_branch.chars().count(), 9);
        let res3 = truncate_branch(cjk_branch, 6);
        assert_eq!(res3, "功能/新功…");
        assert_eq!(res3.chars().count(), 6);

        let res4 = truncate_branch("short", 10);
        assert_eq!(res4, "short");

        let res5 = truncate_branch("very-long-branch-name-here", 0);
        assert_eq!(res5, "very-long-branch-name-here");
    }

    #[test]
    fn render_context_defaults_palette_to_empty() {
        use crate::template::Palette;
        let config = Config::default();
        let theme = ThemeConfig::default();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        // Empty palette: any name lookup misses.
        assert!(ctx.palette.get("anything").is_none());
        let _ = Palette::default(); // type is reachable
    }

    #[test]
    fn with_palette_sets_palette() {
        use crate::template::{Color, Palette};
        use std::collections::HashMap;
        let config = Config::default();
        let theme = ThemeConfig::default();
        let mut map = HashMap::new();
        map.insert("accent".to_owned(), Color::Named("blue".to_owned()));
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE)
            .with_palette(Palette::new(map));
        assert_eq!(
            ctx.palette.get("accent"),
            Some(Color::Named("blue".to_owned()))
        );
    }
}
