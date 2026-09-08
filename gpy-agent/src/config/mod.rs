//! Configuration management for GPY Agent
//!
//! Handles loading, validation, and hot-reload of TOML configuration files.
//!
//! ## Architecture
//!
//! The configuration system uses a three-layer architecture:
//!
//! - **Schema** ([`crate::config::Config`]) - Type-safe representation of all settings
//! - **Loader** ([`crate::config::loader`]) - File I/O and TOML parsing with XDG Base Directory support
//! - **Manager** ([`crate::config::manager::ConfigManager`]) - Hot-reload and change detection
//!
//! ## Configuration File Location
//!
//! The agent searches for `config.toml` in the following order:
//!
//! 1. `$GPY_CONFIG_PATH` (environment variable override for custom config location)
//! 2. `$XDG_CONFIG_HOME/gpy/config.toml` (typically `~/.config/gpy/config.toml`)
//! 3. `~/.config/gpy/config.toml` (fallback if `$XDG_CONFIG_HOME` is not set)
//! 4. `.gpy.toml` (local directory configuration for project-specific overrides)
//! 6. Falls back to built-in defaults if no file exists
//!
//! ## Hot-Reload Support
//!
//! The [`crate::config::manager::ConfigManager`] provides atomic config updates:
//!
//! ```rust,no_run
//! use gpy_agent::config::manager::ConfigManager;
//!
//! let manager = ConfigManager::new()?;
//! let config = manager.get(); // Get current snapshot
//! manager.reload_now()?;      // Reload from disk
//! # Ok::<(), gpy_agent::Error>(())
//! ```
//!
//! ## Example Configuration
//!
//! ```toml
//! [agent]
//! enabled = true
//! timeout_seconds = 2
//! live_updates = true
//!
//! [git]
//! enabled = true
//! max_branch_length = 30
//!
//! [language]
//! enabled = true
//! cache_ttl_hours = 24
//!
//! [ui]
//! theme = "default"
//! clock_format = "%H:%M"
//! ```

// Module declarations
pub mod defaults;
pub mod discovery;
pub mod language_icons;
pub mod loader;
pub mod manager;
/// Configuration metadata for keys and descriptions.
pub mod metadata;
/// Recommended-layout reconciliation engine for `gpy theme use --force`.
pub mod recommended_layout;
pub mod schema;
/// Type-safe configuration primitives.
pub mod types;
/// Configuration validation logic.
pub mod validation;

use serde::{Deserialize, Serialize};

// Re-exports from submodules
pub use defaults::{
    DEFAULT_PROMPT_CLOSE, DEFAULT_PROMPT_OPEN, DEFAULT_SEGMENT_CLOSE, DEFAULT_SEGMENT_OPEN,
    DEFAULT_UI_PROMPT_COLOR, DEFAULT_UI_ROOT_PROMPT_COLOR,
};
pub use language_icons::{LanguageIcons, LanguageTheme};

// Import default functions for use in this module
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
    /// Supervisor check interval in seconds (5..=3600; #597)
    #[serde(default)]
    pub check_interval_seconds: types::SupervisorCheckInterval,
    /// Maximum restart attempts before giving up (1..=100; #597)
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
    reason = "each flag independently toggles a distinct git-detection behavior (enabled, upstream counts, worktree watching, stash), not a state machine"
)]
pub struct GitSettings {
    /// Enable git status detection
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Show ahead/behind counts
    #[serde(default = "default_true")]
    pub show_upstream: bool,
    /// Timeout (seconds) for the underlying git subprocess. The IPC handler
    /// also uses this value (plus a small margin) as the ceiling it waits for
    /// a fresh status before falling back to stale/cached data.
    #[serde(default)]
    pub timeout_seconds: types::GitTimeout,
    /// Paths to skip git detection (e.g., `["/tmp", "/var", "/usr"]`)
    #[serde(default)]
    pub skip_paths: Vec<String>,
    /// Maximum branch name length to display (0 = unlimited)
    #[serde(default)]
    pub max_branch_length: types::MaxBranchLength,
    /// Maximum ahead/behind commit count to report (0 = unlimited). Counts above
    /// this value are clamped to it and rendered with a trailing `+`.
    #[serde(default = "default_max_ahead_behind")]
    pub max_ahead_behind: usize,
    /// Watch the working tree (not just `.git`) so editing a tracked file
    /// updates the prompt instantly. Ignored paths (`.gitignore`, build dirs)
    /// are filtered out. The `GPY_WATCH_WORKTREE` env var overrides this when set.
    ///
    /// When disabled, only `.git/` metadata is watched, so pure working-tree
    /// changes that touch nothing under `.git` — creating an untracked file,
    /// editing or removing a tracked file — fire no watcher event. Such
    /// dirty/untracked state is then reflected only on the next `.git` write
    /// (commit, stage, checkout) or the periodic reconcile scan (default 45s,
    /// `RECONCILE_INTERVAL_SECS`), not instantly. This is the intended tradeoff
    /// for turning off the expensive per-file watching; lower
    /// `GPY_RECONCILE_INTERVAL_SECS` to bound that staleness more tightly at the
    /// cost of more frequent full rescans.
    #[serde(default = "default_true")]
    pub watch_worktree: bool,
    /// Git icons configuration
    #[serde(default)]
    pub icons: GitIcons,
    /// Glyph set for the new state/stash/detached-HEAD icons only (does not
    /// affect the four existing staged/unstaged/untracked/conflicts icons).
    #[serde(default)]
    pub icon_set: GitIconSet,
    /// Enable the `git stash list` capture (and `$stash` rendering).
    /// Disabling skips the extra subprocess call entirely.
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
    /// Stash indicator (default: "≡"). The Unicode glyph; `[git] icon_set =
    /// "nerd_font"` swaps in a Nerd Font alternate at render time.
    #[serde(default = "default_git_icon_stash")]
    pub stash: types::Icon,
    /// Detached-HEAD indicator (default: "➦"). The Unicode glyph; `[git]
    /// icon_set = "nerd_font"` swaps in a Nerd Font alternate at render time.
    #[serde(default = "default_git_icon_detached")]
    pub detached: types::Icon,
    /// In-progress-operation indicator (default: "↻"). The Unicode glyph;
    /// `[git] icon_set = "nerd_font"` swaps in a Nerd Font alternate at
    /// render time.
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
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            show_icons: default_true(),
            theme: types::ThemeName::default(),
            palette: types::PaletteName::default(),
            directory: DirectorySettings::default(),
            enabled_segments: default_enabled_segments(),
        }
    }
}

/// Delimiter configuration with text, foreground, and background colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelimiterConfig {
    /// The icon/glyph text (can be plain text or Nerd Font glyph)
    pub icon: types::Icon,
    /// Foreground (text) color - can be a color name or magic keyword (`match_text`, `match_bg`, transparent)
    #[serde(default = "default_color_white")]
    pub icon_color: types::ColorSpec,
    /// Background color - can be a color name or magic keyword (`match_text`, `match_bg`, transparent)
    #[serde(default = "default_delimiter_bg_transparent")]
    pub bg_color: types::ColorSpec,
}

impl DelimiterConfig {
    /// Create a simple delimiter with default colors
    ///
    /// # Panics
    /// Panics if the default colors or icon are invalid.
    #[expect(
        clippy::expect_used,
        reason = "every call site today passes a compile-time-constant default icon string (or an empty string); Icon::new only fails on control characters, none of which appear in those literals"
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
