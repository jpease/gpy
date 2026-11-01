//! Theme data model loaded from `~/.config/gpy/themes/{theme}.toml`.

use crate::config::defaults::literal;
use crate::config::language_icons::LanguageTheme;
use crate::config::types;
use crate::config::{
    DEFAULT_PROMPT_CLOSE, DEFAULT_PROMPT_OPEN, DEFAULT_SEGMENT_CLOSE, DEFAULT_SEGMENT_OPEN,
    DelimiterConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod normalise;

pub use normalise::{normalise, normalised_table};

/// Theme configuration loaded from ~/.config/gpy/themes/{theme}.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThemeConfig {
    /// UI delimiters and prompt icons
    #[serde(default)]
    pub ui: UiTheme,
    /// Segment configuration grouped under `[segments.*]`
    #[serde(default)]
    pub segments: SegmentThemes,
}

/// Grouped segment configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SegmentThemes {
    /// Clock segment theme configuration
    #[serde(default)]
    pub clock: ClockTheme,
    /// Character segment theme configuration (the final `❯` prompt symbol)
    #[serde(default)]
    pub character: CharacterTheme,
    /// Directory segment theme configuration
    #[serde(default)]
    pub directory: DirectoryTheme,
    /// Duration segment theme configuration
    #[serde(default)]
    pub duration: DurationTheme,
    /// Status segment theme configuration
    #[serde(default)]
    pub status: StatusTheme,
    /// Git segment theme configuration
    #[serde(default)]
    pub git: GitTheme,
    /// Language segment theme configuration
    #[serde(default)]
    pub language: LanguageTheme,
    /// Unknown `[segments.<id>]` tables for community/plugin segments.
    #[serde(flatten)]
    pub plugin: HashMap<String, PluginSegmentTheme>,
}

/// Style for non-built-in segments; unset fields fall back to segment/UI defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(try_from = "RawPluginSegmentTheme", into = "RawPluginSegmentTheme")]
pub struct PluginSegmentTheme {
    /// Optional icon for the segment.
    pub icon: Option<types::Icon>,
    /// Optional open delimiter override.
    pub open: Option<DelimiterConfig>,
    /// Optional close delimiter override.
    pub close: Option<DelimiterConfig>,
    /// Optional segment background color.
    pub bg_color: Option<types::ColorSpec>,
    /// Optional segment text color.
    pub text_color: Option<types::ColorSpec>,
    /// Colours per output variant: `<variant>_bg_color` / `<variant>_text_color`.
    pub variant_colors:
        std::collections::BTreeMap<crate::plugin::VariantColorKey, types::ColorSpec>,
    /// Icons per output variant (`<variant>_icon`), replacing `icon` while it is shown.
    pub variant_icons: std::collections::BTreeMap<crate::plugin::VariantName, types::Icon>,
}

/// [`PluginSegmentTheme`] as written in TOML: variant styles are flat keys.
#[derive(Serialize, Deserialize)]
struct RawPluginSegmentTheme {
    #[serde(default)]
    icon: Option<types::Icon>,
    #[serde(default)]
    open: Option<DelimiterConfig>,
    #[serde(default)]
    close: Option<DelimiterConfig>,
    #[serde(default)]
    bg_color: Option<types::ColorSpec>,
    #[serde(default)]
    text_color: Option<types::ColorSpec>,
    #[serde(flatten)]
    variants: std::collections::BTreeMap<crate::plugin::VariantStyleKey, String>,
}

impl TryFrom<RawPluginSegmentTheme> for PluginSegmentTheme {
    type Error = crate::Error;

    fn try_from(raw: RawPluginSegmentTheme) -> crate::Result<Self> {
        use crate::plugin::VariantStyleKey;
        let mut theme = Self {
            icon: raw.icon,
            open: raw.open,
            close: raw.close,
            bg_color: raw.bg_color,
            text_color: raw.text_color,
            variant_colors: std::collections::BTreeMap::new(),
            variant_icons: std::collections::BTreeMap::new(),
        };
        for (key, value) in raw.variants {
            let named = |err: crate::Error| crate::Error::invalid(format!("{key}: {err}"));
            match &key {
                VariantStyleKey::Color(color_key) => {
                    let color = types::ColorSpec::new(&value).map_err(named)?;
                    theme.variant_colors.insert(color_key.clone(), color);
                }
                VariantStyleKey::Icon(variant) => {
                    let icon = types::Icon::new(value).map_err(named)?;
                    theme.variant_icons.insert(variant.clone(), icon);
                }
            }
        }
        Ok(theme)
    }
}

impl From<PluginSegmentTheme> for RawPluginSegmentTheme {
    fn from(theme: PluginSegmentTheme) -> Self {
        use crate::plugin::VariantStyleKey;
        let colors = theme
            .variant_colors
            .into_iter()
            .map(|(key, color)| (VariantStyleKey::Color(key), color.to_string()));
        let icons = theme
            .variant_icons
            .into_iter()
            .map(|(variant, icon)| (VariantStyleKey::Icon(variant), icon.as_str().to_owned()));
        Self {
            icon: theme.icon,
            open: theme.open,
            close: theme.close,
            bg_color: theme.bg_color,
            text_color: theme.text_color,
            variants: colors.chain(icons).collect(),
        }
    }
}

/// UI-level colors (prompt, delimiters)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiTheme {
    /// Prompt icon (default: "❯")
    pub prompt_icon: types::Icon,

    /// Root prompt icon shown when running as root (default: "!❯!")
    pub root_prompt_icon: types::Icon,

    /// Prompt open delimiter configuration (supports fg/bg colors)
    pub prompt_open: Option<DelimiterConfig>,
    /// Prompt close delimiter configuration (supports fg/bg colors)
    pub prompt_close: Option<DelimiterConfig>,
    /// Default segment open delimiter configuration (supports fg/bg colors)
    pub segment_open: Option<DelimiterConfig>,
    /// Default segment close delimiter configuration (supports fg/bg colors)
    pub segment_close: Option<DelimiterConfig>,

    /// Color for the prompt symbol (unset: `DEFAULT_UI_PROMPT_COLOR`)
    pub prompt_color: Option<types::ColorSpec>,

    /// Color for the root prompt symbol (unset: `DEFAULT_UI_ROOT_PROMPT_COLOR`)
    pub root_prompt_color: Option<types::ColorSpec>,

    /// Opt-in two-line layout (prompt char on its own line). Exported as `__gpy_two_line`.
    pub two_line: bool,

    /// Blank line before each prompt. Exported as `__gpy_add_newline`.
    pub add_newline: bool,

    /// Advisory `config.ui` settings under `[ui.recommended]`; never exported.
    /// Applied only by `gpy theme use <name> --force`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended: Option<RecommendedUi>,
}

/// Advisory `config.ui` settings a theme recommends; `None` fields are left untouched.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RecommendedUi {
    /// Recommended segment order/enablement → `config.ui.enabled_segments`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_segments: Option<Vec<String>>,
    /// Recommended directory settings → `config.ui.directory.*`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<RecommendedDirectory>,
    /// Recommended language-icon visibility → `config.ui.show_icons`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_icons: Option<bool>,
    /// Recommended language-detection strategy → `config.language.detection_mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_detection: Option<types::DetectionMode>,
    /// Recommended color palette → `config.ui.palette`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<types::PaletteName>,
}

/// Advisory `config.ui.directory` settings; `None` fields are left untouched.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RecommendedDirectory {
    /// Recommended directory display mode → `config.ui.directory.display`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<types::DirectoryDisplay>,
    /// Recommended truncation length → `config.ui.directory.truncation_length`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation_length: Option<types::DirectoryTruncationLength>,
    /// Recommended truncation symbol → `config.ui.directory.truncation_symbol`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation_symbol: Option<types::DirectoryTruncationSymbol>,
    /// Recommended repo anchoring → `config.ui.directory.truncate_to_repo`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncate_to_repo: Option<bool>,
}

impl Default for UiTheme {
    fn default() -> Self {
        Self {
            prompt_icon: literal("❯"),
            root_prompt_icon: literal("!❯!"),
            prompt_open: None,
            prompt_close: None,
            segment_open: None,
            segment_close: None,
            prompt_color: None,
            root_prompt_color: None,
            two_line: false,
            add_newline: true,
            recommended: None,
        }
    }
}

impl UiTheme {
    /// Get the `prompt_open` delimiter text (with default fallback)
    #[must_use]
    pub fn get_prompt_open_delimiter(&self) -> &str {
        self.prompt_open
            .as_ref()
            .map_or(DEFAULT_PROMPT_OPEN, |c| c.icon.as_str())
    }

    /// Get the `prompt_close` delimiter text (with default fallback)
    #[must_use]
    pub fn get_prompt_close_delimiter(&self) -> &str {
        self.prompt_close
            .as_ref()
            .map_or(DEFAULT_PROMPT_CLOSE, |c| c.icon.as_str())
    }

    /// Get the `segment_open_delimiter` text (with default fallback)
    #[must_use]
    pub fn get_segment_open_delimiter(&self) -> &str {
        self.segment_open
            .as_ref()
            .map_or(DEFAULT_SEGMENT_OPEN, |c| c.icon.as_str())
    }

    /// Get the `segment_close_delimiter` text (with default fallback)
    #[must_use]
    pub fn get_segment_close_delimiter(&self) -> &str {
        self.segment_close
            .as_ref()
            .map_or(DEFAULT_SEGMENT_CLOSE, |c| c.icon.as_str())
    }
}

/// Character segment (final `❯` symbol); `format` variables are `symbol` and `style`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterTheme {
    /// Starship format template; `None` lets the shell render the character locally.
    pub format: Option<String>,
    /// Symbol shown when the last command succeeded (default: `❯`)
    pub success_symbol: String,
    /// Symbol shown when the last command failed (default: `❯`, but in error color)
    pub error_symbol: String,
    /// Foreground color when the last command succeeded (default: green)
    pub success_color: types::ColorSpec,
    /// Foreground color when the last command failed (default: red)
    pub error_color: types::ColorSpec,
}

impl Default for CharacterTheme {
    fn default() -> Self {
        Self {
            format: None,
            success_symbol: "❯".to_owned(),
            error_symbol: "❯".to_owned(),
            success_color: literal("green"),
            error_color: literal("red"),
        }
    }
}

/// Clock segment colors
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockTheme {
    /// Agent template; the reply embeds each shell's live time token. Unset: every shell draws it locally.
    pub format: Option<String>,
    /// Open delimiter configuration (supports fg/bg colors)
    pub open: Option<DelimiterConfig>,
    /// Close delimiter configuration (supports fg/bg colors)
    pub close: Option<DelimiterConfig>,
    /// Background color
    pub bg_color: types::ColorSpec,
    /// Text color
    pub text_color: types::ColorSpec,
    /// Time format: "12" for 12-hour (HH:MM AM/PM), "24" for 24-hour (HH:MM) (default: "12")
    pub time_format: Option<String>,
    /// Show leading zero for hours before 10 (e.g., "02:15" vs "2:15") (default: false)
    pub show_leading_zero: Option<bool>,
    /// Show seconds in time display (e.g., "12:34:56" vs "12:34") (default: false)
    pub show_seconds: Option<bool>,
}

impl Default for ClockTheme {
    fn default() -> Self {
        Self {
            format: None,
            open: None,
            close: None,
            bg_color: literal("black"),
            text_color: literal("white"),
            time_format: None,
            show_leading_zero: None,
            show_seconds: None,
        }
    }
}

impl ClockTheme {
    /// `time_format`, defaulting to `"12"`.
    #[must_use]
    pub fn time_format(&self) -> &str {
        self.time_format.as_deref().unwrap_or("12")
    }

    /// `show_leading_zero`, defaulting to off.
    #[must_use]
    pub fn show_leading_zero(&self) -> bool {
        self.show_leading_zero.unwrap_or(false)
    }

    /// `show_seconds`, defaulting to off.
    #[must_use]
    pub fn show_seconds(&self) -> bool {
        self.show_seconds.unwrap_or(false)
    }
}

/// Directory segment colors
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectoryTheme {
    /// Starship-style template controlling how this segment renders.
    /// The directory is always agent-rendered, so `None` renders nothing.
    pub format: Option<String>,
    /// Open delimiter configuration (supports fg/bg colors)
    pub open: Option<DelimiterConfig>,
    /// Close delimiter configuration (supports fg/bg colors)
    pub close: Option<DelimiterConfig>,
    /// Background color
    pub bg_color: types::ColorSpec,
    /// Text color
    pub text_color: types::ColorSpec,
    /// `$read_only`, shown when the directory is not writable (default: 🔒, or
    /// `ro` with `ui.show_icons = false`).
    pub read_only_icon: Option<types::Icon>,
}

impl Default for DirectoryTheme {
    fn default() -> Self {
        Self {
            format: None,
            open: None,
            close: None,
            bg_color: literal("black"),
            text_color: literal("white"),
            read_only_icon: None,
        }
    }
}

/// Duration segment colors
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DurationTheme {
    /// Starship template; always agent-rendered, so `None` renders nothing.
    pub format: Option<String>,
    /// Optional icon shown before command duration.
    pub icon: Option<types::Icon>,
    /// Background color
    pub bg_color: types::ColorSpec,
    /// Text color
    pub text_color: types::ColorSpec,
    /// Show duration segment only if command execution exceeds this threshold (in milliseconds)
    pub show_if_exceeds_ms: u64,
    /// Include millisecond precision (default true); false = whole seconds.
    pub show_milliseconds: bool,
}

impl Default for DurationTheme {
    fn default() -> Self {
        Self {
            format: None,
            icon: None,
            bg_color: literal("black"),
            text_color: literal("white"),
            show_if_exceeds_ms: 100,
            show_milliseconds: true,
        }
    }
}

/// Status segment colors
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusTheme {
    /// Starship-style template for the exit-status pill. `None` lets the shell
    /// draw the pill locally; `Some` makes the agent render it.
    pub format: Option<String>,
    /// Optional icon for successful command (exit code 0). Falls back by glyph set when `None`.
    pub ok_icon: Option<types::Icon>,
    /// Background color when command succeeded
    pub ok_bg_color: types::ColorSpec,
    /// Text color when command succeeded
    pub ok_text_color: types::ColorSpec,
    /// Optional icon for failed command (non-zero exit). Falls back by glyph set when `None`.
    pub fail_icon: Option<types::Icon>,
    /// Background color when command failed
    pub fail_bg_color: types::ColorSpec,
    /// Text color when command failed
    pub fail_text_color: types::ColorSpec,
}

impl Default for StatusTheme {
    fn default() -> Self {
        Self {
            format: None,
            ok_icon: None,
            ok_bg_color: literal("green"),
            ok_text_color: literal("black"),
            fail_icon: None,
            fail_bg_color: literal("red"),
            fail_text_color: literal("black"),
        }
    }
}

/// Git segment colors. Every per-state color is optional and falls back to the
/// segment `bg_color`/`text_color` (see [`GitTheme::get_bg_color`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GitTheme {
    /// Starship-style template controlling how this segment renders.
    /// The git segment is always agent-rendered, so `None` renders nothing.
    pub format: Option<String>,
    /// Open delimiter configuration (supports fg/bg colors)
    pub open: Option<DelimiterConfig>,
    /// Close delimiter configuration (supports fg/bg colors)
    pub close: Option<DelimiterConfig>,
    /// Background color (fallback for all git states)
    pub bg_color: types::ColorSpec,
    /// Text color (fallback for all git text)
    pub text_color: types::ColorSpec,

    /// Background color for clean state
    pub clean_bg_color: Option<types::ColorSpec>,
    /// Text color for clean state
    pub clean_text_color: Option<types::ColorSpec>,
    /// Background for the dirty state (`GitState::Modified`; TOML key `dirty_bg_color`).
    pub dirty_bg_color: Option<types::ColorSpec>,
    /// Text color for the dirty state (`GitState::Modified`; TOML key `dirty_text_color`).
    pub dirty_text_color: Option<types::ColorSpec>,
    /// Background color for ahead/behind state
    pub ahead_behind_bg_color: Option<types::ColorSpec>,
    /// Text color for ahead/behind state
    pub ahead_behind_text_color: Option<types::ColorSpec>,
    /// Background color for conflicts state
    pub conflicts_bg_color: Option<types::ColorSpec>,
    /// Text color for conflicts state
    pub conflicts_text_color: Option<types::ColorSpec>,
    /// Background for the untracked-only state; falls back to `dirty_bg_color`.
    pub untracked_only_bg_color: Option<types::ColorSpec>,
    /// Text color for the untracked-only state; falls back to `dirty_text_color`.
    pub untracked_only_text_color: Option<types::ColorSpec>,
    /// Background while a git operation is in progress; falls back to `dirty_bg_color`.
    pub in_progress_bg_color: Option<types::ColorSpec>,
    /// Text color while a git operation is in progress; falls back to `dirty_text_color`.
    pub in_progress_text_color: Option<types::ColorSpec>,

    /// Text color for branch name
    pub branch_text_color: Option<types::ColorSpec>,
    /// Text color for ahead/behind arrows
    pub arrows_text_color: Option<types::ColorSpec>,
    /// Text color for staged indicator
    pub staged_text_color: Option<types::ColorSpec>,
    /// Text color for unstaged indicator
    pub unstaged_text_color: Option<types::ColorSpec>,
    /// Text color for untracked indicator
    pub untracked_text_color: Option<types::ColorSpec>,
    /// Text color for conflicts indicator
    pub conflicts_indicator_text_color: Option<types::ColorSpec>,
    /// Text color for state indicator (rebase/merge/etc)
    pub state_text_color: Option<types::ColorSpec>,

    // Status overrides below fall back to `config.git.icons` when `None`.
    /// Override for the staged-files indicator (default: `config.git.icons.staged`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staged_icon: Option<String>,
    /// Override for the unstaged-files indicator (default: `config.git.icons.unstaged`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unstaged_icon: Option<String>,
    /// Override for the untracked-files indicator (default: `config.git.icons.untracked`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untracked_icon: Option<String>,
    /// Override for the conflicts indicator (default: `config.git.icons.conflicts`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflicts_icon: Option<String>,
    /// Override for the stash indicator (default: `config.git.icons.stash`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash_icon: Option<String>,
    /// Append per-category counts after status icons (`None` = shown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_counts: Option<bool>,
    /// Render the branch name (`None` = shown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_branch: Option<bool>,
    /// Render ahead/behind arrows (`None` = shown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_ahead_behind: Option<bool>,
    /// Render the stash indicator (`None` = shown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_stash: Option<bool>,
    /// `[git_style]` attributes by `GitState` name; `modified` is the dirty state (`dirty_*`).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub git_style: HashMap<String, String>,
}

impl Default for GitTheme {
    fn default() -> Self {
        Self {
            format: None,
            open: None,
            close: None,
            bg_color: literal("black"),
            text_color: literal("white"),
            clean_bg_color: None,
            clean_text_color: None,
            dirty_bg_color: None,
            dirty_text_color: None,
            ahead_behind_bg_color: None,
            ahead_behind_text_color: None,
            conflicts_bg_color: None,
            conflicts_text_color: None,
            untracked_only_bg_color: None,
            untracked_only_text_color: None,
            in_progress_bg_color: None,
            in_progress_text_color: None,
            branch_text_color: None,
            arrows_text_color: None,
            staged_text_color: None,
            unstaged_text_color: None,
            untracked_text_color: None,
            conflicts_indicator_text_color: None,
            state_text_color: None,
            staged_icon: None,
            unstaged_icon: None,
            untracked_icon: None,
            conflicts_icon: None,
            stash_icon: None,
            show_counts: None,
            show_branch: None,
            show_ahead_behind: None,
            show_stash: None,
            git_style: HashMap::new(),
        }
    }
}

/// Git state for color determination, in precedence order (highest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitState {
    /// Merge/rebase/etc. conflicts
    Conflicts,
    /// A special git operation (merge/rebase/cherry-pick/etc.) is in progress
    InProgress,
    /// Staged or unstaged changes (TOML `dirty_*` keys).
    Modified,
    /// Only untracked files present (no staged/unstaged changes)
    UntrackedOnly,
    /// Ahead or behind remote
    AheadBehind,
    /// Clean working directory
    Clean,
}

impl GitState {
    /// Determine git state from status, in precedence order:
    /// `Conflicts > InProgress > Modified > UntrackedOnly > AheadBehind > Clean`.
    #[must_use]
    pub(crate) const fn from_status(status: &crate::git::RepositoryStatus) -> Self {
        if status.conflicts > 0 {
            Self::Conflicts
        } else if is_in_progress(status.state) {
            Self::InProgress
        } else if status.staged > 0 || status.unstaged > 0 {
            Self::Modified
        } else if status.untracked > 0 {
            Self::UntrackedOnly
        } else if status.ahead > 0 || status.behind > 0 {
            Self::AheadBehind
        } else {
            Self::Clean
        }
    }
}

/// Whether a special git operation (merge/rebase/cherry-pick/etc.) is in progress.
const fn is_in_progress(state: crate::git::RepositoryState) -> bool {
    matches!(
        state,
        crate::git::RepositoryState::Merging
            | crate::git::RepositoryState::Rebasing
            | crate::git::RepositoryState::CherryPicking
            | crate::git::RepositoryState::Reverting
            | crate::git::RepositoryState::Bisecting
            | crate::git::RepositoryState::Applying
            | crate::git::RepositoryState::InProgress
    )
}

impl GitTheme {
    /// `show_counts`, defaulting to shown.
    #[must_use]
    pub fn show_counts(&self) -> bool {
        self.show_counts.unwrap_or(true)
    }

    /// `show_branch`, defaulting to shown.
    #[must_use]
    pub fn show_branch(&self) -> bool {
        self.show_branch.unwrap_or(true)
    }

    /// `show_ahead_behind`, defaulting to shown.
    #[must_use]
    pub fn show_ahead_behind(&self) -> bool {
        self.show_ahead_behind.unwrap_or(true)
    }

    /// `show_stash`, defaulting to shown.
    #[must_use]
    pub fn show_stash(&self) -> bool {
        self.show_stash.unwrap_or(true)
    }

    /// Get background color for a git state with fallback.
    #[must_use]
    pub fn get_bg_color(&self, state: GitState) -> &str {
        match state {
            GitState::Conflicts => self.conflicts_bg_color.as_deref().unwrap_or(&self.bg_color),
            GitState::InProgress => self
                .in_progress_bg_color
                .as_deref()
                .or(self.dirty_bg_color.as_deref())
                .unwrap_or(&self.bg_color),
            GitState::Modified => self.dirty_bg_color.as_deref().unwrap_or(&self.bg_color),
            GitState::UntrackedOnly => self
                .untracked_only_bg_color
                .as_deref()
                .or(self.dirty_bg_color.as_deref())
                .unwrap_or(&self.bg_color),
            GitState::AheadBehind => self
                .ahead_behind_bg_color
                .as_deref()
                .unwrap_or(&self.bg_color),
            GitState::Clean => self.clean_bg_color.as_deref().unwrap_or(&self.bg_color),
        }
    }

    /// `[git_style]` override for a state, or `""` when unset.
    #[must_use]
    pub fn get_style(&self, state: GitState) -> &str {
        let key = match state {
            GitState::Clean => "clean",
            GitState::AheadBehind => "ahead_behind",
            GitState::UntrackedOnly => "untracked_only",
            GitState::Modified => "modified",
            GitState::InProgress => "in_progress",
            GitState::Conflicts => "conflicts",
        };
        self.git_style.get(key).map_or("", String::as_str)
    }

    /// Get text color with fallback: element → state → segment.
    #[must_use]
    pub fn get_text_color(&self, element: GitTextElement, state: GitState) -> &str {
        let element_color = match element {
            GitTextElement::Branch => self.branch_text_color.as_deref(),
            GitTextElement::Arrows => self.arrows_text_color.as_deref(),
            GitTextElement::Staged => self.staged_text_color.as_deref(),
            GitTextElement::Unstaged => self.unstaged_text_color.as_deref(),
            GitTextElement::Untracked => self.untracked_text_color.as_deref(),
            GitTextElement::ConflictsIndicator => self.conflicts_indicator_text_color.as_deref(),
            GitTextElement::State => self.state_text_color.as_deref(),
        };

        if let Some(color) = element_color {
            return color;
        }

        let state_color = match state {
            GitState::Clean => self.clean_text_color.as_deref(),
            GitState::Modified => self.dirty_text_color.as_deref(),
            GitState::InProgress => self
                .in_progress_text_color
                .as_deref()
                .or(self.dirty_text_color.as_deref()),
            GitState::UntrackedOnly => self
                .untracked_only_text_color
                .as_deref()
                .or(self.dirty_text_color.as_deref()),
            GitState::AheadBehind => self.ahead_behind_text_color.as_deref(),
            GitState::Conflicts => self.conflicts_text_color.as_deref(),
        };

        if let Some(color) = state_color {
            return color;
        }

        &self.text_color
    }
}

/// Git text elements for specific color control
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitTextElement {
    /// Branch name
    Branch,
    /// Ahead/behind arrows
    Arrows,
    /// Staged indicator
    Staged,
    /// Unstaged indicator
    Unstaged,
    /// Untracked indicator
    Untracked,
    /// Conflicts indicator
    ConflictsIndicator,
    /// State indicator (rebase/merge/etc)
    State,
}

#[cfg(test)]
mod tests {

    use super::{GitState, GitTextElement, GitTheme, SegmentThemes, ThemeConfig};
    use crate::git::{RepositoryState, RepositoryStatus};

    #[test]
    fn git_theme_format_defaults_to_none() {
        let theme = GitTheme::default();
        assert!(theme.format.is_none());
    }

    #[test]
    fn git_theme_parses_format_from_toml() {
        let toml_str =
            "format = \"on [$branch](bold green)\"\nbg_color = \"black\"\ntext_color = \"white\"\n";
        let theme: GitTheme = toml::from_str(toml_str).unwrap();
        assert_eq!(theme.format.as_deref(), Some("on [$branch](bold green)"));
    }

    const ALL_STATES: [GitState; 6] = [
        GitState::Conflicts,
        GitState::InProgress,
        GitState::Modified,
        GitState::UntrackedOnly,
        GitState::AheadBehind,
        GitState::Clean,
    ];

    #[test]
    fn minimal_theme_toml_deserializes_to_default() {
        let parsed: ThemeConfig = toml::from_str("[ui]\n[segments]\n").expect("parse");
        assert_eq!(
            toml::Value::try_from(&parsed).expect("serialize parsed"),
            toml::Value::try_from(ThemeConfig::default()).expect("serialize default"),
        );
    }

    #[test]
    fn empty_segment_tables_deserialize_to_default() {
        let parsed: ThemeConfig = toml::from_str(
            "[ui]\n[segments.git]\n[segments.directory]\n[segments.language]\n[segments.clock]\n",
        )
        .expect("parse");
        assert_eq!(
            toml::Value::try_from(&parsed).expect("serialize parsed"),
            toml::Value::try_from(ThemeConfig::default()).expect("serialize default"),
        );
    }

    #[test]
    fn omitted_git_state_colors_inherit_segment_colors() {
        let parsed: GitTheme =
            toml::from_str("bg_color = \"blue\"\ntext_color = \"yellow\"\n").expect("parse");
        for theme in [GitTheme::default(), parsed] {
            for state in ALL_STATES {
                assert_eq!(theme.get_bg_color(state), theme.bg_color.as_str());
                assert_eq!(
                    theme.get_text_color(GitTextElement::Branch, state),
                    theme.text_color.as_str()
                );
            }
        }
    }

    #[test]
    fn get_style_defaults_to_empty_when_unset() {
        let theme = GitTheme::default();
        assert_eq!(theme.get_style(GitState::Conflicts), "");
    }

    #[test]
    fn git_style_parses_subtable() {
        let theme: GitTheme = toml::from_str("[git_style]\nconflicts = \"bold\"\n").expect("parse");
        assert_eq!(theme.get_style(GitState::Conflicts), "bold");
    }

    #[test]
    fn default_theme_format_references_state_and_stash() {
        let theme = crate::theme::parse(crate::config::defaults::DEFAULT_THEME_CONTENT, "default")
            .expect("default theme parses");
        let format = theme
            .segments
            .git
            .format
            .as_deref()
            .expect("git format present");
        assert!(format.contains("$state"), "format must reference $state");
        assert!(format.contains("$stash"), "format must reference $stash");
    }

    #[test]
    fn default_theme_sets_distinct_untracked_only_and_in_progress_colors() {
        let theme = crate::theme::parse(crate::config::defaults::DEFAULT_THEME_CONTENT, "default")
            .expect("default theme parses");
        let git = &theme.segments.git;
        assert_ne!(
            git.get_bg_color(GitState::UntrackedOnly),
            git.get_bg_color(GitState::Modified),
            "untracked-only must be visually distinct from modified out of the box"
        );
        assert_ne!(
            git.get_bg_color(GitState::InProgress),
            git.get_bg_color(GitState::Conflicts),
            "in-progress must be visually distinct from conflicts out of the box"
        );
    }

    #[test]
    fn starship_and_text_themes_still_parse_with_additive_git_schema() {
        for (content, name) in [
            (crate::config::defaults::STARSHIP_THEME_CONTENT, "starship"),
            (crate::config::defaults::TEXT_THEME_CONTENT, "text"),
        ] {
            let theme = crate::theme::parse(content, name)
                .unwrap_or_else(|e| panic!("{name} theme must still parse: {e}"));
            assert!(
                theme.segments.git.format.is_some(),
                "{name} theme's git format should still be set"
            );
        }
    }

    #[test]
    fn git_content_visibility_flags_default_none() {
        // A theme that sets none of the visibility flags must deserialize them
        // as `None` (always-on behavior).
        let git = GitTheme::default();
        assert_eq!(git.show_branch, None);
        assert_eq!(git.show_ahead_behind, None);
        assert_eq!(git.show_stash, None);
    }

    #[test]
    fn git_content_visibility_flags_parse_from_toml() {
        let theme = crate::theme::parse(
            r#"
[ui]
prompt_icon = "::"

[segments.git]
show_branch = false
show_ahead_behind = false
show_stash = true
"#,
            "git-visibility",
        )
        .expect("partial git theme parses");
        let git = &theme.segments.git;
        assert_eq!(git.show_branch, Some(false));
        assert_eq!(git.show_ahead_behind, Some(false));
        assert_eq!(git.show_stash, Some(true));
    }

    /// A plugin section takes `<variant>_icon` beside the variant colours, rejects a
    /// template `format`, and serializes back to the same flat keys.
    #[test]
    fn plugin_section_parses_variant_icons_and_round_trips() {
        let text = "[hostname]\nssh_icon = \"@ \"\nssh_bg_color = \"red\"\n";
        let segments: SegmentThemes = toml::from_str(text).expect("parse");
        let hostname = segments.plugin.get("hostname").expect("hostname");
        let ssh = crate::plugin::VariantName::new("ssh").expect("variant");
        assert_eq!(
            hostname
                .variant_icons
                .get(&ssh)
                .map(crate::config::types::Icon::as_str),
            Some("@ ")
        );
        assert_eq!(hostname.variant_colors.len(), 1);

        let round_trip: SegmentThemes =
            toml::from_str(&toml::to_string(&segments).expect("serialize")).expect("reparse");
        assert_eq!(
            round_trip
                .plugin
                .get("hostname")
                .expect("hostname")
                .variant_icons,
            hostname.variant_icons
        );

        let err = toml::from_str::<SegmentThemes>("[hostname]\nformat = \"[$hostname]\"\n")
            .expect_err("templates are not plugin styles")
            .to_string();
        assert!(err.contains("format"), "{err}");
    }

    #[test]
    fn get_bg_color_untracked_only_and_in_progress_fall_back_to_dirty() {
        let theme = GitTheme {
            dirty_bg_color: Some(crate::config::types::ColorSpec::new("red").expect("valid")),
            ..GitTheme::default()
        };
        assert_eq!(theme.get_bg_color(GitState::UntrackedOnly), "red");
        assert_eq!(theme.get_bg_color(GitState::InProgress), "red");
        assert_eq!(theme.get_bg_color(GitState::Clean), theme.bg_color.as_str());
    }

    #[test]
    fn get_text_color_prefers_element_override_over_state_and_segment() {
        let theme = GitTheme {
            branch_text_color: Some(
                crate::config::types::ColorSpec::new("magenta").expect("valid color"),
            ),
            dirty_text_color: Some(
                crate::config::types::ColorSpec::new("yellow").expect("valid color"),
            ),
            ..GitTheme::default()
        };

        // Element override wins even though a state override for Modified is also set.
        assert_eq!(
            theme.get_text_color(GitTextElement::Branch, GitState::Modified),
            "magenta"
        );
    }

    #[test]
    fn get_text_color_falls_back_to_state_when_no_element_override() {
        let theme = GitTheme {
            dirty_text_color: Some(
                crate::config::types::ColorSpec::new("yellow").expect("valid color"),
            ),
            ..GitTheme::default()
        };

        // No element-specific override set for Staged, so the Modified state color applies.
        assert_eq!(
            theme.get_text_color(GitTextElement::Staged, GitState::Modified),
            "yellow"
        );
    }

    #[test]
    fn get_text_color_falls_back_to_segment_default_when_state_color_unset() {
        let theme = GitTheme {
            clean_text_color: None,
            ..GitTheme::default()
        };
        assert_eq!(
            theme.get_text_color(GitTextElement::Unstaged, GitState::Clean),
            theme.text_color.as_str()
        );
    }

    #[test]
    fn get_text_color_in_progress_falls_back_to_dirty_before_segment_default() {
        let theme = GitTheme {
            in_progress_text_color: None,
            dirty_text_color: Some(
                crate::config::types::ColorSpec::new("cyan").expect("valid color"),
            ),
            ..GitTheme::default()
        };

        assert_eq!(
            theme.get_text_color(GitTextElement::State, GitState::InProgress),
            "cyan"
        );
    }

    #[test]
    fn get_text_color_in_progress_falls_back_to_segment_default_when_dirty_also_unset() {
        let theme = GitTheme {
            in_progress_text_color: None,
            dirty_text_color: None,
            ..GitTheme::default()
        };

        assert_eq!(
            theme.get_text_color(GitTextElement::State, GitState::InProgress),
            theme.text_color.as_str()
        );
    }

    /// Clean, non-diverged base fixture for `GitState::from_status` tests.
    fn precedence_status() -> RepositoryStatus {
        RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }
    }

    #[test]
    fn determine_git_state_conflicts_wins_over_everything() {
        let mut status = precedence_status();
        status.conflicts = 1;
        status.state = RepositoryState::Rebasing;
        status.staged = 1;
        assert_eq!(GitState::from_status(&status), GitState::Conflicts);
    }

    #[test]
    fn determine_git_state_in_progress_wins_over_modified() {
        let mut status = precedence_status();
        status.state = RepositoryState::Rebasing;
        status.staged = 1;
        assert_eq!(GitState::from_status(&status), GitState::InProgress);
    }

    #[test]
    fn determine_git_state_modified_wins_over_untracked_only() {
        let mut status = precedence_status();
        status.staged = 1;
        status.untracked = 1;
        assert_eq!(GitState::from_status(&status), GitState::Modified);
    }

    #[test]
    fn determine_git_state_untracked_only_when_only_untracked_set() {
        let mut status = precedence_status();
        status.untracked = 1;
        assert_eq!(GitState::from_status(&status), GitState::UntrackedOnly);
    }

    #[test]
    fn determine_git_state_ahead_behind_when_clean_but_diverged() {
        let mut status = precedence_status();
        status.ahead = 1;
        assert_eq!(GitState::from_status(&status), GitState::AheadBehind);
    }

    #[test]
    fn determine_git_state_untracked_only_wins_over_ahead_behind() {
        let mut status = precedence_status();
        status.untracked = 1;
        status.ahead = 1;
        status.behind = 1;
        assert_eq!(GitState::from_status(&status), GitState::UntrackedOnly);
    }

    #[test]
    fn determine_git_state_clean() {
        let status = precedence_status();
        assert_eq!(GitState::from_status(&status), GitState::Clean);
    }
}
