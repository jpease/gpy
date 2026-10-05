//! Theme data model: the typed structures loaded from
//! `~/.config/gpy/themes/{theme}.toml`.
//!
//! This module owns the theme schema only — file I/O, hot-reload, and
//! shell-export serialization live in [`crate::theme::manager`] and
//! [`crate::theme::export`] respectively.

use crate::config::defaults::{
    default_character_error_color, default_character_error_symbol, default_character_success_color,
    default_character_success_symbol, default_color_black, default_color_red, default_color_white,
    default_duration_threshold, default_hostname_trim_at, default_status_fail_bg,
    default_status_fail_text, default_status_ok_bg, default_status_ok_text, default_true,
    default_ui_prompt, default_ui_prompt_close, default_ui_prompt_open, default_ui_root_prompt,
    default_ui_root_prompt_color, default_ui_segment_close_delimiter,
    default_ui_segment_open_delimiter,
};
use crate::config::language_icons::LanguageTheme;
use crate::config::types;
use crate::config::{
    DEFAULT_PROMPT_CLOSE, DEFAULT_PROMPT_OPEN, DEFAULT_SEGMENT_CLOSE, DEFAULT_SEGMENT_OPEN,
    DelimiterConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Hostname segment theme configuration
    #[serde(default)]
    pub hostname: HostnameTheme,
    /// Username segment theme configuration (Starship `username` module parity)
    #[serde(default)]
    pub username: UsernameTheme,
    /// Generic theme configuration for community/plugin segments.
    ///
    /// This map captures unknown `[segments.<id>]` tables while preserving
    /// strongly-typed built-in segment fields above.
    #[serde(flatten)]
    pub plugin: HashMap<String, PluginSegmentTheme>,
}

/// Generic style configuration for non-built-in/community segments.
///
/// Fallback semantics:
/// - If `bg_color` is unset, consumers should fall back to their segment default.
/// - If `text_color` is unset, consumers should fall back to their segment default.
/// - If `open`/`close` are unset, consumers should use UI-level delimiter defaults.
/// - Unknown string properties are carried in `properties` for plugin-specific styling.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginSegmentTheme {
    /// Optional icon for the segment.
    #[serde(default)]
    pub icon: Option<types::Icon>,
    /// Optional open delimiter override.
    #[serde(default)]
    pub open: Option<DelimiterConfig>,
    /// Optional close delimiter override.
    #[serde(default)]
    pub close: Option<DelimiterConfig>,
    /// Optional segment background color.
    #[serde(default)]
    pub bg_color: Option<types::ColorSpec>,
    /// Optional segment text color.
    #[serde(default)]
    pub text_color: Option<types::ColorSpec>,
    /// Additional plugin-specific string style properties.
    ///
    /// Common patterns include keys such as `context_color`,
    /// `namespace_color`, and `secondary_icon`.
    #[serde(flatten)]
    pub properties: HashMap<String, String>,
}

/// UI-level colors (prompt, delimiters)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiTheme {
    // Note: delimiter fields keep the `*_delimiter` suffix because they represent literal separators, not icons
    /// Prompt icon (default: "❯")
    #[serde(default = "default_ui_prompt")]
    pub prompt_icon: types::Icon,

    /// Root prompt icon shown when running as root (default: "!❯!")
    #[serde(default = "default_ui_root_prompt")]
    pub root_prompt_icon: types::Icon,

    /// Prompt open delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub prompt_open: Option<DelimiterConfig>,
    /// Prompt close delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub prompt_close: Option<DelimiterConfig>,
    /// Default segment open delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub segment_open: Option<DelimiterConfig>,
    /// Default segment close delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub segment_close: Option<DelimiterConfig>,

    /// Color for the prompt symbol (default: "white")
    #[serde(default)]
    pub prompt_color: Option<types::ColorSpec>,

    /// Color for the root prompt symbol (default: "red")
    #[serde(default)]
    pub root_prompt_color: Option<types::ColorSpec>,

    /// Opt-in two-line layout for bash/zsh: render the enabled segments on one
    /// line and the prompt character on the next (Starship-style).
    ///
    /// Default `false` keeps bash/zsh single-line (no regression). Fish already
    /// renders two-line and ignores this flag. Exported to shells as
    /// `__gpy_two_line` (`0`/`1`).
    #[serde(default)]
    pub two_line: bool,

    /// Blank line before each prompt, for visual separation between commands.
    ///
    /// Defaults to `true`, which is what Fish has always done unconditionally
    /// (`fish/functions/fish_prompt.fish`). Bash and zsh had no equivalent at
    /// all, so the same theme produced visibly tighter prompts there; they now
    /// read this flag too. Exported to shells as `__gpy_add_newline` (`0`/`1`).
    #[serde(default = "default_true")]
    pub add_newline: bool,

    /// Optional `config.ui` settings a theme/preset recommends for the closest
    /// match to its intended look (e.g. the Starship preset's segment order,
    /// abbreviated directory, and language icons).
    ///
    /// Purely advisory metadata under `[ui.recommended]`: it is *not* exported to
    /// shells and does not change how any theme resolves the active layout — that
    /// always comes from `config.ui`. A user opts into a theme's recommendation
    /// with `gpy theme use <name> --force`, which copies the set fields into
    /// `config.ui` while preserving any the user explicitly configured. `None`
    /// means the theme leaves all of `config.ui` to the user's config (the
    /// default for `text`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<RecommendedUi>,
}

/// Advisory `config.ui` settings a theme recommends.
///
/// Applied only via `gpy theme use <name> --force`. Each field is independently
/// optional: `None` leaves that `config.ui` setting untouched even when applying.
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
    ///
    /// Lives under `[ui.recommended]` for consistency with the rest of the
    /// recommended-layout block even though it applies to `config.language`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_detection: Option<types::DetectionMode>,
    /// Recommended color palette → `config.ui.palette`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<types::PaletteName>,
}

/// Advisory `config.ui.directory` settings a theme recommends.
///
/// Each field is independently optional: `None` leaves that `config.ui.directory`
/// setting untouched even when applying via `--force`.
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
            prompt_icon: default_ui_prompt(),
            root_prompt_icon: default_ui_root_prompt(),
            prompt_open: Some(DelimiterConfig::simple(default_ui_prompt_open())),
            prompt_close: Some(DelimiterConfig::simple(default_ui_prompt_close())),
            segment_open: Some(DelimiterConfig::simple(default_ui_segment_open_delimiter())),
            segment_close: Some(DelimiterConfig::simple(default_ui_segment_close_delimiter())),
            prompt_color: None,
            root_prompt_color: Some(default_ui_root_prompt_color()),
            two_line: false,
            add_newline: default_true(),
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

/// Character segment configuration — the final `❯` prompt symbol, colored by exit status.
///
/// Starship-compatible: variables available in `format` are `symbol` and `style`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterTheme {
    /// Optional Starship-compatible format template.
    ///
    /// When `Some`, the agent renders the character symbol via the template engine and
    /// returns pre-formatted ANSI. `None` makes the agent emit nothing, so the shell
    /// falls back to its local (`set_color`-style) character rendering.
    #[serde(default)]
    pub format: Option<String>,
    /// Symbol shown when the last command succeeded (default: `❯`)
    #[serde(default = "default_character_success_symbol")]
    pub success_symbol: String,
    /// Symbol shown when the last command failed (default: `❯`, but in error color)
    #[serde(default = "default_character_error_symbol")]
    pub error_symbol: String,
    /// Foreground color when the last command succeeded (default: green)
    #[serde(default = "default_character_success_color")]
    pub success_color: types::ColorSpec,
    /// Foreground color when the last command failed (default: red)
    #[serde(default = "default_character_error_color")]
    pub error_color: types::ColorSpec,
}

impl Default for CharacterTheme {
    fn default() -> Self {
        Self {
            format: None,
            success_symbol: default_character_success_symbol(),
            error_symbol: default_character_error_symbol(),
            success_color: default_character_success_color(),
            error_color: default_character_error_color(),
        }
    }
}

/// Hostname segment configuration.
///
/// `None` format = pure-fish fast path (no IPC). `Some` format =
/// agent-rendered via the Starship-compatible template engine, for
/// theme-exact parity (e.g. the starship preset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostnameTheme {
    /// Optional Starship-compatible format template.
    ///
    /// When `Some`, the agent renders the hostname via the template engine and
    /// returns pre-formatted ANSI. `None` makes the agent emit nothing; the shell
    /// renders the hostname itself (GPY pill colors).
    #[serde(default)]
    pub format: Option<String>,
    /// Icon/symbol shown before the hostname (default: none, matching Starship)
    #[serde(default)]
    pub icon: Option<types::Icon>,
    /// Pill background color, pure-fish path only (ignored when `format` is set)
    #[serde(default = "default_color_black")]
    pub bg_color: types::ColorSpec,
    /// Pill text color, pure-fish path only (ignored when `format` is set)
    #[serde(default = "default_color_white")]
    pub text_color: types::ColorSpec,
    /// Trim the hostname at the first occurrence of this delimiter (default: ".",
    /// FQDN -> short name). Empty string disables trimming. Applies to both paths.
    #[serde(default = "default_hostname_trim_at")]
    pub trim_at: String,
    /// Show even on local (non-SSH) sessions (default: false — SSH-only, matching
    /// Starship's `ssh_only = true` default).
    #[serde(default)]
    pub show_always: bool,
}

impl Default for HostnameTheme {
    fn default() -> Self {
        Self {
            format: None,
            icon: None,
            bg_color: default_color_black(),
            text_color: default_color_white(),
            trim_at: default_hostname_trim_at(),
            show_always: false,
        }
    }
}

/// Username segment configuration — Starship `username` module parity.
///
/// Shows the effective username (e.g. `root`) as a styled prefix when running
/// as root or under sudo. Visibility is gated shell-side by `__enabled_segments`
/// membership plus `segment_username_detect` (root/sudo, or `show_always`), so
/// there is no separate `enabled` flag here — matching every other zero-backend
/// segment (`hostname`/`clock`/`status`).
///
/// `None` format = pure-shell fast path (no IPC), rendering gpy's pill aesthetic.
/// `Some` format = agent-rendered via the Starship-compatible template engine,
/// for theme-exact parity (e.g. the starship preset's `[$user](bold red)`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsernameTheme {
    /// Optional Starship-compatible format template.
    ///
    /// When `Some`, the agent renders the username via the template engine and
    /// returns pre-formatted ANSI. `None` makes the agent emit nothing; the shell
    /// renders the username itself (GPY pill colors).
    #[serde(default)]
    pub format: Option<String>,
    /// Icon/symbol shown before the username (default: none, matching Starship)
    #[serde(default)]
    pub icon: Option<types::Icon>,
    /// Pill background color, pure-shell path only (ignored when `format` is set).
    /// Default red — a strong root/sudo warning, mirroring Starship's bold-red.
    #[serde(default = "default_color_red")]
    pub bg_color: types::ColorSpec,
    /// Pill text color, pure-shell path only (ignored when `format` is set).
    #[serde(default = "default_color_white")]
    pub text_color: types::ColorSpec,
    /// Show even for normal (non-root, non-sudo) users (default: false — root/sudo
    /// only, matching Starship's `show_always = false` default).
    #[serde(default)]
    pub show_always: bool,
}

impl Default for UsernameTheme {
    fn default() -> Self {
        Self {
            format: None,
            icon: None,
            bg_color: default_color_red(),
            text_color: default_color_white(),
            show_always: false,
        }
    }
}

/// Clock segment colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockTheme {
    /// Template used to render the segment agent-side.
    ///
    /// When unset the agent emits nothing and the shell renders the clock
    /// locally, which is what Fish does. Zsh and Bash have no local cap
    /// renderer, so a theme that leaves this unset gets an uncapped clock
    /// there; the shipped themes set it.
    #[serde(default)]
    pub format: Option<String>,
    /// Open delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub open: Option<DelimiterConfig>,
    /// Close delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub close: Option<DelimiterConfig>,
    /// Background color
    #[serde(default = "default_color_black")]
    pub bg_color: types::ColorSpec,
    /// Text color
    #[serde(default = "default_color_white")]
    pub text_color: types::ColorSpec,
    /// Time format: "12" for 12-hour (HH:MM AM/PM), "24" for 24-hour (HH:MM) (default: "12")
    #[serde(default)]
    pub time_format: Option<String>,
    /// Show leading zero for hours before 10 (e.g., "02:15" vs "2:15") (default: false)
    #[serde(default)]
    pub show_leading_zero: Option<bool>,
    /// Show seconds in time display (e.g., "12:34:56" vs "12:34") (default: false)
    #[serde(default)]
    pub show_seconds: Option<bool>,
}

impl Default for ClockTheme {
    fn default() -> Self {
        Self {
            format: None,
            open: None,
            close: None,
            bg_color: default_color_black(),
            text_color: default_color_white(),
            time_format: None,
            show_leading_zero: None,
            show_seconds: None,
        }
    }
}

/// Directory segment colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryTheme {
    /// Starship-style template controlling how this segment renders.
    /// The directory is always agent-rendered (#199), so `None` renders nothing.
    #[serde(default)]
    pub format: Option<String>,
    /// Open delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub open: Option<DelimiterConfig>,
    /// Close delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub close: Option<DelimiterConfig>,
    /// Background color
    #[serde(default = "default_color_black")]
    pub bg_color: types::ColorSpec,
    /// Text color
    #[serde(default = "default_color_white")]
    pub text_color: types::ColorSpec,
}

impl Default for DirectoryTheme {
    fn default() -> Self {
        #[expect(
            clippy::expect_used,
            reason = "\"blue\" is a compile-time-constant color name; parsing it is infallible in practice"
        )]
        Self {
            format: None,
            open: None,
            close: None,
            bg_color: types::ColorSpec::new("blue").expect("Hardcoded default color must be valid"),
            text_color: default_color_white(),
        }
    }
}

/// Duration segment colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurationTheme {
    /// Starship-compatible template controlling how this segment renders.
    ///
    /// The duration is always agent-rendered (#199): `Some` returns
    /// pre-formatted ANSI via the template engine, `None` renders nothing.
    #[serde(default)]
    pub format: Option<String>,
    /// Icon shown before command duration (default: "󰑧")
    #[serde(default)]
    pub icon: Option<types::Icon>,
    /// Background color
    #[serde(default = "default_color_black")]
    pub bg_color: types::ColorSpec,
    /// Text color
    #[serde(default = "default_color_white")]
    pub text_color: types::ColorSpec,
    /// Show duration segment only if command execution exceeds this threshold (in milliseconds)
    #[serde(default = "default_duration_threshold")]
    pub show_if_exceeds_ms: u64,
    /// When false, format duration as whole seconds only (`2s`, `1m 5s`).
    /// When true (default), include millisecond precision (`2.500s`, `1m 5.000s`).
    #[serde(default = "default_true")]
    pub show_milliseconds: bool,
}

impl Default for DurationTheme {
    fn default() -> Self {
        Self {
            format: None,
            icon: None,
            bg_color: default_color_black(),
            text_color: default_color_white(),
            show_if_exceeds_ms: default_duration_threshold(),
            show_milliseconds: true,
        }
    }
}

/// Status segment colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusTheme {
    /// Icon for successful command (exit code 0) (default: "✔")
    #[serde(default)]
    pub ok_icon: Option<types::Icon>,
    /// Background color when command succeeded
    #[serde(default = "default_status_ok_bg")]
    pub ok_bg_color: types::ColorSpec,
    /// Text color when command succeeded
    #[serde(default = "default_status_ok_text")]
    pub ok_text_color: types::ColorSpec,
    /// Icon for failed command (non-zero exit) (default: "✖")
    #[serde(default)]
    pub fail_icon: Option<types::Icon>,
    /// Background color when command failed
    #[serde(default = "default_status_fail_bg")]
    pub fail_bg_color: types::ColorSpec,
    /// Text color when command failed
    #[serde(default = "default_status_fail_text")]
    pub fail_text_color: types::ColorSpec,
}

impl Default for StatusTheme {
    fn default() -> Self {
        Self {
            ok_icon: None,
            ok_bg_color: default_status_ok_bg(),
            ok_text_color: default_status_ok_text(),
            fail_icon: None,
            fail_bg_color: default_status_fail_bg(),
            fail_text_color: default_status_fail_text(),
        }
    }
}

/// Git segment color configuration with hierarchical fallbacks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitTheme {
    /// Starship-style template controlling how this segment renders.
    /// The git segment is always agent-rendered (#199), so `None` renders nothing.
    #[serde(default)]
    pub format: Option<String>,
    // Delimiter configurations
    /// Open delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub open: Option<DelimiterConfig>,
    /// Close delimiter configuration (supports fg/bg colors)
    #[serde(default)]
    pub close: Option<DelimiterConfig>,
    /// Background color (fallback for all git states)
    #[serde(default = "default_color_black")]
    pub bg_color: types::ColorSpec,
    /// Text color (fallback for all git text)
    #[serde(default = "default_color_white")]
    pub text_color: types::ColorSpec,

    // State-based background overrides
    /// Background color for clean state
    #[serde(default)]
    pub clean_bg_color: Option<types::ColorSpec>,
    /// Text color for clean state
    #[serde(default)]
    pub clean_text_color: Option<types::ColorSpec>,
    /// Background color for the modified state (staged/unstaged changes).
    /// TOML key kept as `dirty_bg_color` for backward compatibility.
    #[serde(default)]
    pub dirty_bg_color: Option<types::ColorSpec>,
    /// Text color for the modified state (staged/unstaged changes).
    /// TOML key kept as `dirty_text_color` for backward compatibility.
    #[serde(default)]
    pub dirty_text_color: Option<types::ColorSpec>,
    /// Background color for ahead/behind state
    #[serde(default)]
    pub ahead_behind_bg_color: Option<types::ColorSpec>,
    /// Text color for ahead/behind state
    #[serde(default)]
    pub ahead_behind_text_color: Option<types::ColorSpec>,
    /// Background color for conflicts state
    #[serde(default)]
    pub conflicts_bg_color: Option<types::ColorSpec>,
    /// Text color for conflicts state
    #[serde(default)]
    pub conflicts_text_color: Option<types::ColorSpec>,
    /// Background color for the untracked-only state. Falls back to
    /// `dirty_bg_color`, then the segment base color, when unset.
    #[serde(default)]
    pub untracked_only_bg_color: Option<types::ColorSpec>,
    /// Text color for the untracked-only state. Falls back to
    /// `dirty_text_color`, then the segment base text color, when unset.
    #[serde(default)]
    pub untracked_only_text_color: Option<types::ColorSpec>,
    /// Background color while a git operation (merge/rebase/etc.) is in
    /// progress. Falls back to `dirty_bg_color`, then the segment base
    /// color, when unset.
    #[serde(default)]
    pub in_progress_bg_color: Option<types::ColorSpec>,
    /// Text color while a git operation (merge/rebase/etc.) is in progress.
    /// Falls back to `dirty_text_color`, then the segment base text color,
    /// when unset.
    #[serde(default)]
    pub in_progress_text_color: Option<types::ColorSpec>,

    // Element-specific text overrides (very specific control)
    /// Text color for branch name
    #[serde(default)]
    pub branch_text_color: Option<types::ColorSpec>,
    /// Text color for ahead/behind arrows
    #[serde(default)]
    pub arrows_text_color: Option<types::ColorSpec>,
    /// Text color for staged indicator
    #[serde(default)]
    pub staged_text_color: Option<types::ColorSpec>,
    /// Text color for unstaged indicator
    #[serde(default)]
    pub unstaged_text_color: Option<types::ColorSpec>,
    /// Text color for untracked indicator
    #[serde(default)]
    pub untracked_text_color: Option<types::ColorSpec>,
    /// Text color for conflicts indicator
    #[serde(default)]
    pub conflicts_indicator_text_color: Option<types::ColorSpec>,
    /// Text color for state indicator (rebase/merge/etc)
    #[serde(default)]
    pub state_text_color: Option<types::ColorSpec>,

    // Theme-owned status rendering (Starship parity). Each falls back to
    // `config.git.icons` / legacy behavior when `None`, so existing themes are
    // unchanged. Read directly by `GitResolver::status_text`.
    /// Override for the staged-files indicator (default: `config.git.icons.staged`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_icon: Option<String>,
    /// Override for the unstaged-files indicator (default: `config.git.icons.unstaged`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unstaged_icon: Option<String>,
    /// Override for the untracked-files indicator (default: `config.git.icons.untracked`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untracked_icon: Option<String>,
    /// Override for the conflicts indicator (default: `config.git.icons.conflicts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicts_icon: Option<String>,
    /// Override for the stash indicator (default: `config.git.icons.stash`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stash_icon: Option<String>,
    /// Whether to append per-category counts after each status icon. `None` keeps
    /// the legacy behavior (counts shown). Starship's `git_status` shows symbols
    /// only (e.g. `[!?]`), so the starship preset sets this to `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_counts: Option<bool>,
    /// Whether to render the branch name. `None` keeps the legacy behavior
    /// (branch shown). Read by `GitResolver`'s `branch` variable resolver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_branch: Option<bool>,
    /// Whether to render the ahead/behind arrows (e.g. `↑2↓1`). `None` keeps
    /// the legacy behavior (arrows shown). Read by `GitResolver`'s
    /// `ahead_behind` variable resolver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_ahead_behind: Option<bool>,
    /// Whether to render the stash indicator. `None` keeps the legacy behavior
    /// (stash shown). Read by `GitResolver`'s `stash` variable resolver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_stash: Option<bool>,
    /// Per-state style attributes (e.g. `conflicts = "bold"`), composed with
    /// `$style`'s existing fg/bg. Keys are the lower-snake-case `GitState`
    /// variant names (`clean`/`ahead_behind`/`untracked_only`/`modified`/
    /// `in_progress`/`conflicts`). Populated from a `[git_style]` TOML subtable.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub git_style: HashMap<String, String>,
}

/// A hardcoded default git state color.
///
/// # Panics
/// Panics if `name` is not a valid color — only ever called with the
/// compile-time-constant names below, so this is infallible in practice.
#[expect(
    clippy::expect_used,
    reason = "only ever called with compile-time-constant color names below, so this is infallible in practice"
)]
fn git_default_color(name: &str) -> types::ColorSpec {
    types::ColorSpec::new(name).expect("Hardcoded default color must be valid")
}

impl Default for GitTheme {
    fn default() -> Self {
        Self {
            format: None,
            open: None,
            close: None,
            bg_color: default_color_black(),
            text_color: default_color_white(),
            clean_bg_color: Some(git_default_color("green")),
            clean_text_color: Some(git_default_color("white")),
            dirty_bg_color: Some(git_default_color("red")),
            dirty_text_color: Some(git_default_color("white")),
            ahead_behind_bg_color: Some(git_default_color("yellow")),
            ahead_behind_text_color: Some(git_default_color("black")),
            conflicts_bg_color: Some(git_default_color("red")),
            conflicts_text_color: Some(git_default_color("white")),
            untracked_only_bg_color: Some(git_default_color("cyan")),
            untracked_only_text_color: Some(git_default_color("black")),
            in_progress_bg_color: Some(git_default_color("magenta")),
            in_progress_text_color: Some(git_default_color("white")),
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
    /// Staged or unstaged changes (no conflicts, no in-progress operation).
    /// Backed by the `dirty_*` TOML keys (`GitTheme`) for backward compatibility.
    Modified,
    /// Only untracked files present (no staged/unstaged changes)
    UntrackedOnly,
    /// Ahead or behind remote
    AheadBehind,
    /// Clean working directory
    Clean,
}

impl GitState {
    /// Determine git state from repository status, in precedence order:
    /// `Conflicts > InProgress > Modified > UntrackedOnly > AheadBehind > Clean`.
    ///
    /// Promoted from `formatter::fish_ansi` (previously private to that module,
    /// `determine_git_state`) so both the real Fish-ANSI renderer and the
    /// config wizard's live preview (`commands::wizard::preview`, #373) share
    /// one precedence implementation instead of maintaining two copies. Lives
    /// here on `GitState` rather than as a free function in `crate::git`
    /// because `crate::git` has no existing dependency on `crate::theme` (and
    /// this keeps that direction unchanged), while `GitState` — the type this
    /// function produces — already lives in this module.
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
    /// Get background color for a specific git state with fallback hierarchy
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

    /// Get a theme-declared style-attribute string for a git state.
    ///
    /// Returns the `[git_style]` subtable override verbatim, keyed by the
    /// state's lower-snake-case variant name, or `""` when unset — `$style`
    /// already supplies `fg:`/`bg:`, so an empty default changes nothing.
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

    /// Get text color with fallback hierarchy: element → state → segment
    #[must_use]
    pub fn get_text_color(&self, element: GitTextElement, state: GitState) -> &str {
        // First try element-specific color
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

        // Fall back to state-specific text color
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

        // Fall back to segment default
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
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{GitState, GitTextElement, GitTheme, HostnameTheme, SegmentThemes};
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

    #[test]
    fn get_bg_color_untracked_only_and_in_progress_have_distinct_defaults() {
        let theme = GitTheme::default();
        assert_ne!(
            theme.get_bg_color(GitState::UntrackedOnly),
            theme.get_bg_color(GitState::Modified)
        );
        assert_ne!(
            theme.get_bg_color(GitState::InProgress),
            theme.get_bg_color(GitState::Conflicts)
        );
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
    fn git_content_visibility_flags_default_none_for_backward_compat() {
        // A theme that predates #410 sets none of the new flags; they must
        // deserialize as `None` (always-on behavior), an additive/MINOR change.
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

    #[test]
    fn hostname_theme_default_matches_expected_fields() {
        let theme = HostnameTheme::default();
        assert!(theme.format.is_none());
        assert!(theme.icon.is_none());
        assert_eq!(theme.bg_color, "black");
        assert_eq!(theme.text_color, "white");
        assert_eq!(theme.trim_at, ".");
        assert!(!theme.show_always);
    }

    #[test]
    fn hostname_theme_parses_partial_toml_with_defaults_for_rest() {
        let toml_str = "icon = \"@\"\nshow_always = true\n";
        let theme: HostnameTheme = toml::from_str(toml_str).expect("parse");
        assert_eq!(theme.icon.as_deref(), Some("@"));
        assert!(theme.show_always);
        // Unspecified fields fall back to defaults.
        assert!(theme.format.is_none());
        assert_eq!(theme.bg_color, "black");
        assert_eq!(theme.text_color, "white");
        assert_eq!(theme.trim_at, ".");
    }

    #[test]
    fn segment_themes_without_hostname_table_defaults_hostname() {
        let segments: SegmentThemes = toml::from_str("").expect("parse");
        assert_eq!(
            segments.hostname.bg_color,
            HostnameTheme::default().bg_color
        );
        assert!(segments.hostname.format.is_none());
        assert_eq!(segments.hostname.trim_at, ".");
    }

    #[test]
    fn get_bg_color_falls_back_to_segment_default_when_state_colors_unset() {
        let theme = GitTheme {
            clean_bg_color: None,
            ahead_behind_bg_color: None,
            ..GitTheme::default()
        };
        assert_eq!(theme.get_bg_color(GitState::Clean), theme.bg_color.as_str());
        assert_eq!(
            theme.get_bg_color(GitState::AheadBehind),
            theme.bg_color.as_str()
        );
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

    /// Base fixture for `GitState::from_status` precedence tests.
    ///
    /// A clean, non-diverged repo. Relocated from `formatter::fish_ansi`
    /// alongside the promoted `determine_git_state` logic (now
    /// `GitState::from_status`, #373).
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
