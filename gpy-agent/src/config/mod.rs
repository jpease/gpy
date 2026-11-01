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

use defaults::{DEFAULT_ENABLED_SEGMENTS, GIT_GLYPHS_NERD_FONT, GIT_GLYPHS_UNICODE, literal};

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Agent-specific configuration
    pub agent: AgentSettings,
    /// Git-related configuration
    pub git: GitSettings,
    /// Language detection configuration
    pub language: LanguageSettings,
    /// UI/display configuration
    pub ui: UiSettings,
    /// Per-segment overrides for script plugin segments (`[plugin.<segment>]`).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub plugin: std::collections::BTreeMap<
        crate::plugin::SegmentName,
        plugin_segments::PluginSegmentConfig,
    >,
}

impl Config {
    /// The single rule for whether segment `name` renders and whether the agent
    /// does its work: it is listed in `ui.enabled_segments`. For `git` and
    /// `language` the deprecated `git.enabled` / `language.enabled` flag, when
    /// `false`, also removes the segment, so existing files render as before.
    ///
    /// Compares strings directly (no allocation, no `BuiltinSegment` parse)
    /// because [`Self::git_enabled_for`] runs on every git request.
    #[must_use]
    pub fn segment_enabled(&self, name: &str) -> bool {
        self.ui.enabled_segments.iter().any(|s| s == name)
            && match name {
                "git" => self.git.enabled,
                "language" => self.language.enabled,
                _ => true,
            }
    }

    /// The segments that render, in `ui.enabled_segments` order; every entry
    /// passes [`Self::segment_enabled`].
    pub fn effective_segments(&self) -> impl Iterator<Item = &str> {
        self.ui
            .enabled_segments
            .iter()
            .map(String::as_str)
            .filter(|name| self.segment_enabled(name))
    }

    /// Whether the agent has any background work to watch: `git` or `language`
    /// renders ([`Self::segment_enabled`]).
    #[must_use]
    pub fn any_agent_segment_enabled(&self) -> bool {
        self.segment_enabled("git") || self.segment_enabled("language")
    }

    /// Whether Git detection may run for the repository rooted at `root` (pass
    /// its canonical root): `git` renders ([`Self::segment_enabled`]) and the
    /// root is not at or under a `git.skip_paths` entry, so an entry inside a
    /// repository does not skip it. Every Git status scan and Git cache
    /// publication entry point, foreground or background, must check this
    /// against the repository root.
    #[must_use]
    pub fn git_enabled_for(&self, root: &std::path::Path) -> bool {
        self.segment_enabled("git") && !self.git.is_path_skipped(root)
    }
}

/// Agent process configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// Enable background agent mode
    pub enabled: bool,
    /// Socket timeout in seconds
    pub timeout_seconds: types::AgentTimeout,
    /// Enable live updates via signals
    pub live_updates: bool,
    /// Supervisor (auto-restart) configuration
    pub supervisor: SupervisorSettings,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_seconds: types::AgentTimeout::default(),
            live_updates: true,
            supervisor: SupervisorSettings::default(),
        }
    }
}

/// Supervisor configuration for automatic agent restart
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SupervisorSettings {
    /// Enable supervisor for auto-restart
    pub enabled: bool,
    /// Supervisor check interval in seconds (5..=3600)
    pub check_interval_seconds: types::SupervisorCheckInterval,
    /// Maximum restart attempts before giving up (1..=100)
    pub max_restart_attempts: types::SupervisorMaxRestartAttempts,
}

impl Default for SupervisorSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_seconds: types::SupervisorCheckInterval::default(),
            max_restart_attempts: types::SupervisorMaxRestartAttempts::default(),
        }
    }
}

/// Git-related configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent toggles, not a state machine"
)]
pub struct GitSettings {
    /// Enable git status detection
    pub enabled: bool,
    /// Show ahead/behind counts
    pub show_upstream: bool,
    /// Git subprocess timeout (seconds); also bounds the IPC wait for fresh status.
    pub timeout_seconds: types::GitTimeout,
    /// Directories where git detection is skipped (`~` and symlinks resolved).
    pub skip_paths: Vec<String>,
    /// Maximum branch name length to display (0 = unlimited)
    pub max_branch_length: types::MaxBranchLength,
    /// Largest ahead/behind count displayed; larger counts render as `N+`
    /// (0 = no cap). Display only: git still counts them all.
    pub max_ahead_behind: usize,
    /// Watch the working tree, not just `.git` (`GPY_WATCH_WORKTREE` overrides).
    /// When off, worktree-only edits surface on the next `.git` write or reconcile scan.
    pub watch_worktree: bool,
    /// Git icons configuration
    pub icons: GitIcons,
    /// Default glyph set for the stash, detached-HEAD and in-progress icons only.
    pub icon_set: GitIconSet,
    /// Enable the `git stash list` capture (and `$stash` rendering).
    pub stash_enabled: bool,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            show_upstream: true,
            timeout_seconds: types::GitTimeout::default(),
            skip_paths: Vec::new(),
            max_branch_length: types::MaxBranchLength::default(),
            max_ahead_behind: 100,
            watch_worktree: true,
            icons: GitIcons::default(),
            icon_set: GitIconSet::default(),
            stash_enabled: true,
        }
    }
}

/// Default-glyph selector for the stash, detached-HEAD and in-progress icons.
///
/// It does not touch the branch glyph or the powerline caps: those follow
/// `ui.show_icons` and need a patched font in either set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitIconSet {
    /// Plain Unicode glyphs for the three icons (default).
    #[default]
    Unicode,
    /// Nerd Font glyphs for the three icons (requires a patched font).
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
    type Err = types::ParseEnumError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "unicode" => Ok(Self::Unicode),
            "nerd_font" => Ok(Self::NerdFont),
            _ => Err(types::ParseEnumError::new(
                "git.icon_set",
                s,
                "'unicode' or 'nerd_font'",
            )),
        }
    }
}

/// Git icons configuration (semantic icons).
///
/// Every field is an optional override. Icon precedence, shared with
/// [`LanguageIcons`]: `ui.show_icons` gates first; then a theme icon
/// (`[segments.git] *_icon`, `[segments.language] <lang>_symbol`); then an
/// explicit `config.toml` icon (`[git.icons]`, `[language.icons]`); then the
/// built-in default, which for git depends on `git.icon_set`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitIcons {
    /// Ahead indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<types::Icon>,
    /// Behind indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<types::Icon>,
    /// Staged files indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub staged: Option<types::Icon>,
    /// Unstaged files indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unstaged: Option<types::Icon>,
    /// Untracked files indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untracked: Option<types::Icon>,
    /// Conflicts indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflicts: Option<types::Icon>,
    /// Stash indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash: Option<types::Icon>,
    /// Detached-HEAD indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detached: Option<types::Icon>,
    /// In-progress-operation indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_progress: Option<types::Icon>,
}

/// One git status icon, for [`GitIcons::glyph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitIconKind {
    /// Ahead indicator
    Ahead,
    /// Behind indicator
    Behind,
    /// Staged files indicator
    Staged,
    /// Unstaged files indicator
    Unstaged,
    /// Untracked files indicator
    Untracked,
    /// Conflicts indicator
    Conflicts,
    /// Stash indicator
    Stash,
    /// Detached-HEAD indicator
    Detached,
    /// In-progress-operation indicator
    InProgress,
}

impl GitIcons {
    /// The icon to draw for `kind`: the explicit `[git.icons]` value, else the
    /// built-in glyph for `set`.
    #[must_use]
    pub fn glyph(&self, kind: GitIconKind, set: GitIconSet) -> &str {
        let table = match set {
            GitIconSet::Unicode => &GIT_GLYPHS_UNICODE,
            GitIconSet::NerdFont => &GIT_GLYPHS_NERD_FONT,
        };
        let (explicit, default) = match kind {
            GitIconKind::Ahead => (&self.ahead, table.ahead),
            GitIconKind::Behind => (&self.behind, table.behind),
            GitIconKind::Staged => (&self.staged, table.staged),
            GitIconKind::Unstaged => (&self.unstaged, table.unstaged),
            GitIconKind::Untracked => (&self.untracked, table.untracked),
            GitIconKind::Conflicts => (&self.conflicts, table.conflicts),
            GitIconKind::Stash => (&self.stash, table.stash),
            GitIconKind::Detached => (&self.detached, table.detached),
            GitIconKind::InProgress => (&self.in_progress, table.in_progress),
        };
        explicit.as_ref().map_or(default, types::Icon::as_str)
    }

    /// Every icon filled with its effective value (explicit, else the default
    /// for `set`), for `config show` and key enumeration.
    #[must_use]
    pub fn resolved(&self, set: GitIconSet) -> Self {
        let fill = |kind: GitIconKind| types::Icon::new(self.glyph(kind, set)).ok();
        Self {
            ahead: fill(GitIconKind::Ahead),
            behind: fill(GitIconKind::Behind),
            staged: fill(GitIconKind::Staged),
            unstaged: fill(GitIconKind::Unstaged),
            untracked: fill(GitIconKind::Untracked),
            conflicts: fill(GitIconKind::Conflicts),
            stash: fill(GitIconKind::Stash),
            detached: fill(GitIconKind::Detached),
            in_progress: fill(GitIconKind::InProgress),
        }
    }
}

/// Language detection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageSettings {
    /// Enable language detection
    pub enabled: bool,
    /// Enable version detection
    pub show_versions: bool,
    /// Language version, mise and virtualenv lookup cache TTL in hours
    pub cache_ttl_hours: types::CacheTtlHours,
    /// Languages to detect (empty = all)
    pub enabled_languages: Vec<String>,
    /// Display mode: icons or plain text
    pub display: types::LanguageDisplay,
    /// Filter mode: all, primary, or top N
    pub filter: types::LanguageFilter,
    /// Detection strategy: `markers` (default), `content` scan, or `hybrid`
    pub detection_mode: types::DetectionMode,
    /// Minimum confidence threshold (0.0-1.0)
    pub confidence_threshold: types::ConfidenceThreshold,
    /// Language icons configuration
    pub icons: LanguageIcons,
}

impl Default for LanguageSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            show_versions: true,
            cache_ttl_hours: types::CacheTtlHours::default(),
            enabled_languages: Vec::new(),
            display: types::LanguageDisplay::Icon,
            filter: types::LanguageFilter::All,
            detection_mode: types::DetectionMode::default(),
            confidence_threshold: types::ConfidenceThreshold::default(),
            icons: LanguageIcons::default(),
        }
    }
}

/// Directory segment display configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct DirectorySettings {
    /// Directory display mode: `basename`, `abbreviated`, `truncated`, `full`
    pub display: types::DirectoryDisplay,
    /// Trailing path components kept when `display = "truncated"`
    pub truncation_length: types::DirectoryTruncationLength,
    /// Prefix shown before a truncated path (e.g. `"…/"`)
    pub truncation_symbol: types::DirectoryTruncationSymbol,
    /// Maximum path length to display (character cap)
    pub max_length: types::MaxPathLength,
    /// Anchor the displayed path at the enclosing git repo root (Starship parity)
    pub truncate_to_repo: bool,
}

/// UI/display configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    /// Show glyphs: language icons, git icons and powerline separators
    pub show_icons: bool,
    /// Color theme
    pub theme: types::ThemeName,
    /// Color palette (color theme) selecting the named-color set used by templates
    pub palette: types::PaletteName,
    /// Directory segment display configuration
    pub directory: DirectorySettings,
    /// Enabled segments and their order (e.g., `["clock", "duration", "language", "directory", "git"]`)
    pub enabled_segments: Vec<String>,
    /// Collapse submitted prompts in scrollback (Fish 4.2+ and Zsh only).
    pub transient_prompt: bool,
    /// Let `gpy setup` move a user theme file byte-identical to a released
    /// builtin of the same name to `<name>.toml.bak`, so the builtin applies.
    pub retire_unmodified_themes: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            show_icons: true,
            theme: types::ThemeName::default(),
            palette: types::PaletteName::default(),
            directory: DirectorySettings::default(),
            enabled_segments: DEFAULT_ENABLED_SEGMENTS.map(str::to_owned).to_vec(),
            transient_prompt: true,
            retire_unmodified_themes: true,
        }
    }
}

/// Delimiter configuration with text, foreground, and background colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelimiterConfig {
    /// The icon/glyph text (can be plain text or Nerd Font glyph)
    pub icon: types::Icon,
    /// Foreground color: name or `match_text`/`match_bg`/transparent.
    #[serde(default = "DelimiterConfig::default_icon_color")]
    pub icon_color: types::ColorSpec,
    /// Background color: name or `match_text`/`match_bg`/transparent.
    #[serde(default = "DelimiterConfig::default_bg_color")]
    pub bg_color: types::ColorSpec,
}

impl DelimiterConfig {
    fn default_icon_color() -> types::ColorSpec {
        literal("white")
    }

    fn default_bg_color() -> types::ColorSpec {
        literal("transparent")
    }
}

#[cfg(test)]
mod tests {

    use super::Config;

    fn as_toml(config: &Config) -> toml::Value {
        toml::Value::try_from(config).expect("serialize config")
    }

    #[test]
    fn empty_config_deserializes_to_default() {
        let parsed: Config = toml::from_str("").expect("parse");
        assert_eq!(as_toml(&parsed), as_toml(&Config::default()));
    }

    #[test]
    fn empty_config_tables_deserialize_to_default() {
        let parsed: Config = toml::from_str(
            "[agent]\n[agent.supervisor]\n[git]\n[git.icons]\n[language]\n[ui]\n[ui.directory]\n",
        )
        .expect("parse");
        assert_eq!(as_toml(&parsed), as_toml(&Config::default()));
    }

    #[test]
    fn default_config_round_trips_through_toml() {
        let text = toml::to_string(&Config::default()).expect("serialize");
        let parsed: Config = toml::from_str(&text).expect("parse");
        assert_eq!(as_toml(&parsed), as_toml(&Config::default()));
    }

    #[test]
    fn ui_settings_parses_palette_field() {
        let toml_src = "theme = \"default\"\npalette = \"nord\"\n";
        let ui: crate::config::UiSettings = toml::from_str(toml_src).expect("parse");
        assert_eq!(ui.palette.as_str(), "nord");
    }

    #[test]
    fn git_icon_set_parses_nerd_font() {
        let settings: crate::config::GitSettings =
            toml::from_str("icon_set = \"nerd_font\"\n").expect("parse");
        assert_eq!(settings.icon_set, crate::config::GitIconSet::NerdFont);
    }

    fn listing(segments: &[&str]) -> Config {
        let mut config = Config::default();
        config.ui.enabled_segments = segments.iter().map(|s| (*s).to_owned()).collect();
        config
    }

    #[test]
    fn segment_enabled_requires_listing() {
        let config = listing(&["directory"]);
        assert!(config.segment_enabled("directory"));
        assert!(!config.segment_enabled("git"), "flag true but unlisted");
        assert!(!config.segment_enabled("language"));
        assert!(!config.segment_enabled("clock"));
    }

    #[test]
    fn deprecated_flag_false_removes_listed_segment() {
        let mut config = listing(&["language", "directory", "git"]);
        config.git.enabled = false;
        config.language.enabled = false;
        assert!(!config.segment_enabled("git"));
        assert!(!config.segment_enabled("language"));
        assert!(config.segment_enabled("directory"));
    }

    #[test]
    fn effective_segments_keeps_order() {
        let mut config = listing(&["git", "clock", "language", "directory"]);
        config.language.enabled = false;
        let effective: Vec<&str> = config.effective_segments().collect();
        assert_eq!(effective, ["git", "clock", "directory"]);
    }

    #[test]
    fn any_agent_segment_enabled_needs_git_or_language() {
        assert!(!listing(&["clock", "directory"]).any_agent_segment_enabled());
        assert!(listing(&["git"]).any_agent_segment_enabled());
        assert!(listing(&["language"]).any_agent_segment_enabled());
        let mut flags_off = listing(&["git", "language"]);
        flags_off.git.enabled = false;
        flags_off.language.enabled = false;
        assert!(!flags_off.any_agent_segment_enabled());
    }

    #[test]
    fn git_enabled_for_requires_listing_and_honours_skip_paths() {
        let root = std::env::temp_dir().join("gpy-git-enabled-for-root");
        let unlisted = listing(&["directory"]);
        assert!(!unlisted.git_enabled_for(&root));

        let mut listed = listing(&["git"]);
        assert!(listed.git_enabled_for(&root));
        listed.git.skip_paths = vec![root.to_string_lossy().into_owned()];
        assert!(!listed.git_enabled_for(&root));
    }
}
