//! Output formatting layer: renders agent responses as JSON or ANSI prompt text.

pub(crate) mod character_resolver;
pub(crate) mod clock_resolver;
pub(crate) mod directory_resolver;
pub(crate) mod duration_resolver;
mod fish_ansi;
pub(crate) mod git_resolver;
pub(crate) mod hostname_resolver;
mod json;
pub(crate) mod language_resolver;
pub(crate) mod separator;
mod style_encoder;
pub(crate) mod username_resolver;

// The ANSI format is shell-agnostic, so it is a re-export of `FishAnsiFormatter`.
pub use fish_ansi::FishAnsiFormatter as AnsiFormatter;
pub use fish_ansi::FishAnsiFormatter;
pub(crate) use fish_ansi::{get_language_display, select_languages};
pub use json::JsonFormatter;
pub use style_encoder::{PromptDialect, encode_ansi, encode_bash_prompt, encode_zsh_prompt};

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
    /// ANSI escape codes with text verbatim (Fish prints the prompt literally).
    Ansi,
    /// ANSI colors with text escaped for bash `PS1`.
    BashPrompt,
    /// ANSI colors with text escaped for zsh `PROMPT`.
    ZshPrompt,
}

impl Format {
    /// Convert to string representation
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Ansi => "ansi",
            Self::BashPrompt => "bash-prompt",
            Self::ZshPrompt => "zsh-prompt",
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
            "bash-prompt" => Ok(Self::BashPrompt),
            "zsh-prompt" => Ok(Self::ZshPrompt),
            _ => Err(crate::Error::config(format!("Unknown format: {s}"))),
        }
    }
}

impl ValueEnum for Format {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Json, Self::Ansi, Self::BashPrompt, Self::ZshPrompt]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(match self {
            Self::Json => PossibleValue::new("json"),
            Self::Ansi => PossibleValue::new("ansi"),
            Self::BashPrompt => PossibleValue::new("bash-prompt"),
            Self::ZshPrompt => PossibleValue::new("zsh-prompt"),
        })
    }
}

/// Whether this is the last segment; controls the closing powerline cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IsLast {
    /// Emit the closing cap.
    Yes,
    /// No closing cap.
    No,
}

impl IsLast {
    /// `Yes` → `true`.
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

/// Whether this is the first segment; the first renders flush, without an opening cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum IsFirst {
    /// Suppress the opening cap.
    Yes,
    /// Render the opening cap.
    #[default]
    No,
}

impl IsFirst {
    /// `Yes` → `true`.
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

/// A segment's position (first/last) within the rendered chain.
/// No `from_bools` constructor: `max-fn-params-bools = 1` forbids it.
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

    /// Build a position.
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
    /// Where this segment sits in the chain.
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
    /// Render a response in this format.
    ///
    /// # Errors
    /// Returns an error when serialization or UTF-8 conversion fails.
    fn render(&self, response: &Response, ctx: &RenderContext<'_>) -> Result<String>;
}

/// Create the formatter for `format`.
#[must_use]
pub fn create_formatter(format: Format) -> Box<dyn Formatter> {
    match format {
        Format::Json => Box::new(JsonFormatter),
        Format::Ansi => Box::new(AnsiFormatter::new(PromptDialect::Ansi)),
        Format::BashPrompt => Box::new(AnsiFormatter::new(PromptDialect::BashPrompt)),
        Format::ZshPrompt => Box::new(AnsiFormatter::new(PromptDialect::ZshPrompt)),
    }
}

/// Truncate to `max_length` chars (0 = unlimited), ending with `…`.
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
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::missing_panics_doc
    )]

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
