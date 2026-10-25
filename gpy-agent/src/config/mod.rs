//! TOML configuration: schema, loading, validation, and hot-reload.
//! Lookup order: `$GPY_CONFIG_PATH`, `$XDG_CONFIG_HOME/gpy/config.toml`,
//! `~/.config/gpy/config.toml`, then built-in defaults.

pub mod defaults;
pub mod discovery;
pub mod language_icons;
pub mod loader;
pub mod manager;
/// Configuration metadata for keys and descriptions.
pub mod metadata;
/// `[plugin.<segment>]` overrides for script plugin segments.
pub mod plugin_segments;
/// Recommended-layout reconciliation engine for `gpy theme use --force`.
pub mod recommended_layout;
pub mod schema;
/// Shared matcher for `git.skip_paths`.
pub mod skip_paths;
/// Type-safe configuration primitives.
pub mod types;
/// Configuration validation logic.
pub mod validation;

use serde::{Deserialize, Serialize};

pub use defaults::{
    DEFAULT_PROMPT_CLOSE, DEFAULT_PROMPT_OPEN, DEFAULT_SEGMENT_CLOSE, DEFAULT_SEGMENT_OPEN,
    DEFAULT_UI_PROMPT_COLOR, DEFAULT_UI_ROOT_PROMPT_COLOR,
};
pub use language_icons::{LanguageIcons, LanguageTheme};

use defaults::{
    default_color_white, default_delimiter_bg_transparent, default_enabled_segments,
    default_git_icon_ahead, default_git_icon_behind, default_git_icon_conflicts,
    default_git_icon_detached, default_git_icon_in_progress, default_git_icon_staged,
    default_git_icon_stash, default_git_icon_unstaged, default_git_icon_untracked,
    default_language_filter, default_language_show, default_max_ahead_behind, default_true,
};

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Agent-specific configuration
    #[serde(default)]
    pub agent: AgentSettings,
    /// Git-related configuration
    #[serde(default)]
    pub git: GitSettings,
    /// Language detection configuration
    #[serde(default)]
    pub language: LanguageSettings,
    /// UI/display configuration
    #[serde(default)]
    pub ui: UiSettings,
    /// Per-segment overrides for script plugin segments (`[plugin.<segment>]`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub plugin: std::collections::BTreeMap<
        crate::plugin::SegmentName,
        plugin_segments::PluginSegmentConfig,
    >,
}

/// Agent process configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    /// Enable background agent mode
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Socket timeout in seconds
    #[serde(default)]
    pub timeout_seconds: types::AgentTimeout,
    /// Enable live updates via signals
    #[serde(default = "default_true")]
    pub live_updates: bool,
    /// Supervisor (auto-restart) configuration
    #[serde(default)]
    pub supervisor: SupervisorSettings,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            timeout_seconds: types::AgentTimeout::default(),
            live_updates: default_true(),
            supervisor: SupervisorSettings::default(),
        }
    }
}

/// Supervisor configuration for automatic agent restart
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorSettings {
    /// Enable supervisor for auto-restart
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Supervisor check interval in seconds (5..=3600)
    #[serde(default)]
    pub check_interval_seconds: types::SupervisorCheckInterval,
    /// Maximum restart attempts before giving up (1..=100)
    #[serde(default)]
    pub max_restart_attempts: types::SupervisorMaxRestartAttempts,
}

impl Default for SupervisorSettings {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            check_interval_seconds: types::SupervisorCheckInterval::default(),
            max_restart_attempts: types::SupervisorMaxRestartAttempts::default(),
        }
    }
}

/// Git-related configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent toggles, not a state machine"
)]
pub struct GitSettings {
    /// Enable git status detection
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Show ahead/behind counts
    #[serde(default = "default_true")]
    pub show_upstream: bool,
    /// Git subprocess timeout (seconds); also bounds the IPC wait for fresh status.
    #[serde(default)]
    pub timeout_seconds: types::GitTimeout,
    /// Directories where git detection is skipped (`~` and symlinks resolved).
    #[serde(default)]
    pub skip_paths: Vec<String>,
    /// Maximum branch name length to display (0 = unlimited)
    #[serde(default)]
    pub max_branch_length: types::MaxBranchLength,
    /// Max ahead/behind count (0 = unlimited); larger counts render as `N+`.
    #[serde(default = "default_max_ahead_behind")]
    pub max_ahead_behind: usize,
    /// Watch the working tree, not just `.git` (`GPY_WATCH_WORKTREE` overrides).
    /// When off, worktree-only edits surface on the next `.git` write or reconcile scan.
    #[serde(default = "default_true")]
    pub watch_worktree: bool,
    /// Git icons configuration
    #[serde(default)]
    pub icons: GitIcons,
    /// Glyph set for the stash/detached/in-progress icons only.
    #[serde(default)]
    pub icon_set: GitIconSet,
    /// Enable the `git stash list` capture (and `$stash` rendering).
    #[serde(default = "default_true")]
    pub stash_enabled: bool,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            show_upstream: default_true(),
            timeout_seconds: types::GitTimeout::default(),
            skip_paths: Vec::new(),
            max_branch_length: types::MaxBranchLength::default(),
            max_ahead_behind: default_max_ahead_behind(),
            watch_worktree: default_true(),
            icons: GitIcons::default(),
            icon_set: GitIconSet::default(),
            stash_enabled: default_true(),
        }
    }
}

/// Glyph set selector for the stash/detached-HEAD/in-progress icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitIconSet {
    /// Plain Unicode glyphs (default; no special font required).
    #[default]
    Unicode,
    /// Nerd Font glyphs (requires a patched font).
    NerdFont,
}
impl std::fmt::Display for GitIconSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unicode => "unicode",
            Self::NerdFont => "nerd_font",
        })
    }
}

impl std::str::FromStr for GitIconSet {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "unicode" => Ok(Self::Unicode),
            "nerd_font" => Ok(Self::NerdFont),
            other => Err(format!(
                "Invalid git.icon_set '{other}'. Use 'unicode' or 'nerd_font'"
            )),
        }
    }
}

/// Git icons configuration (semantic icons)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIcons {
    /// Ahead indicator (default: "↑")
    #[serde(default = "default_git_icon_ahead")]
    pub ahead: types::Icon,
    /// Behind indicator (default: "↓")
    #[serde(default = "default_git_icon_behind")]
    pub behind: types::Icon,
    /// Staged files indicator (default: "✚")
    #[serde(default = "default_git_icon_staged")]
    pub staged: types::Icon,
    /// Unstaged files indicator (default: "✱")
    #[serde(default = "default_git_icon_unstaged")]
    pub unstaged: types::Icon,
    /// Untracked files indicator (default: "?")
    #[serde(default = "default_git_icon_untracked")]
    pub untracked: types::Icon,
    /// Conflicts indicator (default: "✖")
    #[serde(default = "default_git_icon_conflicts")]
    pub conflicts: types::Icon,
    /// Stash indicator (default: "≡"); `icon_set` may swap in a Nerd Font glyph.
    #[serde(default = "default_git_icon_stash")]
    pub stash: types::Icon,
    /// Detached-HEAD indicator (default: "➦"); `icon_set` may swap it.
    #[serde(default = "default_git_icon_detached")]
    pub detached: types::Icon,
    /// In-progress-operation indicator (default: "↻"); `icon_set` may swap it.
    #[serde(default = "default_git_icon_in_progress")]
    pub in_progress: types::Icon,
}

impl Default for GitIcons {
    fn default() -> Self {
        Self {
            ahead: default_git_icon_ahead(),
            behind: default_git_icon_behind(),
            staged: default_git_icon_staged(),
            unstaged: default_git_icon_unstaged(),
            untracked: default_git_icon_untracked(),
            conflicts: default_git_icon_conflicts(),
            stash: default_git_icon_stash(),
            detached: default_git_icon_detached(),
            in_progress: default_git_icon_in_progress(),
        }
    }
}

/// Language detection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageSettings {
    /// Enable language detection
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Enable version detection
    #[serde(default = "default_true")]
    pub show_versions: bool,
    /// Version cache TTL in hours
    #[serde(default)]
    pub cache_ttl_hours: types::CacheTtlHours,
    /// Languages to detect (empty = all)
    #[serde(default)]
    pub enabled_languages: Vec<String>,
    /// Display mode: icons or plain text
    #[serde(default = "default_language_show")]
    pub display: types::LanguageDisplay,
    /// Filter mode: all, primary, or top N
    #[serde(default = "default_language_filter")]
    pub filter: types::LanguageFilter,
    /// Detection strategy: `content` scan (default) or project `markers`
    #[serde(default)]
    pub detection_mode: types::DetectionMode,
    /// Minimum confidence threshold (0.0-1.0)
    #[serde(default)]
    pub confidence_threshold: types::ConfidenceThreshold,
    /// Language icons configuration
    #[serde(default)]
    pub icons: LanguageIcons,
}

impl Default for LanguageSettings {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            show_versions: default_true(),
            cache_ttl_hours: types::CacheTtlHours::default(),
            enabled_languages: Vec::new(),
            display: default_language_show(),
            filter: default_language_filter(),
            detection_mode: types::DetectionMode::default(),
            confidence_threshold: types::ConfidenceThreshold::default(),
            icons: LanguageIcons::default(),
        }
    }
}

/// Directory segment display configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DirectorySettings {
    /// Directory display mode: `basename`, `abbreviated`, `truncated`, `full`
    #[serde(default)]
    pub display: types::DirectoryDisplay,
    /// Trailing path components kept when `display = "truncated"`
    #[serde(default)]
    pub truncation_length: types::DirectoryTruncationLength,
    /// Prefix shown before a truncated path (e.g. `"…/"`)
    #[serde(default)]
    pub truncation_symbol: types::DirectoryTruncationSymbol,
    /// Maximum path length to display (character cap)
    #[serde(default)]
    pub max_length: types::MaxPathLength,
    /// Anchor the displayed path at the enclosing git repo root (Starship parity)
    #[serde(default)]
    pub truncate_to_repo: bool,
}

/// UI/display configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    /// Show icons for languages
    #[serde(default = "default_true")]
    pub show_icons: bool,
    /// Color theme
    #[serde(default)]
    pub theme: types::ThemeName,
    /// Color palette (color theme) selecting the named-color set used by templates
    #[serde(default)]
    pub palette: types::PaletteName,
    /// Directory segment display configuration
    #[serde(default)]
    pub directory: DirectorySettings,
    /// Enabled segments and their order (e.g., `["clock", "duration", "language", "directory", "git"]`)
    #[serde(default = "default_enabled_segments")]
    pub enabled_segments: Vec<String>,
    /// Collapse submitted prompts in scrollback (Fish 4.2+ and Zsh only).
    #[serde(default = "default_true")]
    pub transient_prompt: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            show_icons: default_true(),
            theme: types::ThemeName::default(),
            palette: types::PaletteName::default(),
            directory: DirectorySettings::default(),
            enabled_segments: default_enabled_segments(),
            transient_prompt: default_true(),
        }
    }
}

/// Delimiter configuration with text, foreground, and background colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelimiterConfig {
    /// The icon/glyph text (can be plain text or Nerd Font glyph)
    pub icon: types::Icon,
    /// Foreground color: name or `match_text`/`match_bg`/transparent.
    #[serde(default = "default_color_white")]
    pub icon_color: types::ColorSpec,
    /// Background color: name or `match_text`/`match_bg`/transparent.
    #[serde(default = "default_delimiter_bg_transparent")]
    pub bg_color: types::ColorSpec,
}

impl DelimiterConfig {
    /// Delimiter with default colors.
    ///
    /// # Panics
    /// Panics if `icon` contains control characters.
    #[expect(
        clippy::expect_used,
        reason = "callers pass constant icons without control characters"
    )]
    pub fn simple(icon: impl Into<String>) -> Self {
        Self {
            icon: types::Icon::new(icon).expect("Default icon must be valid"),
            icon_color: default_color_white(),
            bg_color: default_delimiter_bg_transparent(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    #[test]
    fn ui_settings_defaults_palette_to_default() {
        let cfg = crate::config::UiSettings::default();
        assert_eq!(cfg.palette.as_str(), "default");
    }

    #[test]
    fn ui_settings_parses_palette_field() {
        let toml_src = "theme = \"default\"\npalette = \"nord\"\n";
        let ui: crate::config::UiSettings = toml::from_str(toml_src).expect("parse");
        assert_eq!(ui.palette.as_str(), "nord");
    }

    #[test]
    fn git_icon_set_defaults_to_unicode() {
        let settings = crate::config::GitSettings::default();
        assert_eq!(settings.icon_set, crate::config::GitIconSet::Unicode);
    }

    #[test]
    fn git_icon_set_parses_nerd_font() {
        let settings: crate::config::GitSettings =
            toml::from_str("icon_set = \"nerd_font\"\n").expect("parse");
        assert_eq!(settings.icon_set, crate::config::GitIconSet::NerdFont);
    }

    #[test]
    fn git_stash_enabled_defaults_to_true() {
        let settings = crate::config::GitSettings::default();
        assert!(settings.stash_enabled);
    }

    #[test]
    fn git_icons_default_includes_stash_detached_in_progress() {
        let icons = crate::config::GitIcons::default();
        assert_eq!(icons.stash.as_str(), "≡");
        assert_eq!(icons.detached.as_str(), "➦");
        assert_eq!(icons.in_progress.as_str(), "↻");
    }
}
