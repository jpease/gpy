//! Which config changes require shells to redraw after a reload.

use super::AgentContext;
use crate::config::Config;
use crate::theme::{GitState, ThemeConfig};
use crate::watcher::{RepoFilter, WatchRegistry};
use std::path::Path;
use std::sync::Arc;

/// Repository filter for the watcher: a repository is watched only when
/// [`Config::git_enabled_for`] allows it (`git` listed, not under a
/// `git.skip_paths` entry). Holds its own copy of the config, so installing a
/// filter never borrows it.
pub(crate) fn git_repo_filter(config: &Config) -> RepoFilter {
    let owned = config.clone();
    Arc::new(move |root: &Path| owned.git_enabled_for(root))
}

/// Apply the watcher-relevant git settings to `registry`.
pub(super) fn apply_watch_settings(registry: &WatchRegistry, config: &Config) {
    registry.set_worktree_enabled(config.git.watch_worktree);
    registry.set_repo_filter(git_repo_filter(config));
}

/// Background the git pill of the repository at `root` is drawn with, the
/// `prev_bg` of the segment after it: the cached status's state colour (any
/// age, upstream hidden as rendered), clean when nothing is cached.
pub(crate) fn git_segment_bg<'a>(
    ctx: &AgentContext,
    root: &Path,
    config: &Config,
    theme: &'a ThemeConfig,
) -> &'a str {
    let state = ctx.cache.get_any(root).map_or(GitState::Clean, |status| {
        let rendered = if config.git.show_upstream {
            status
        } else {
            status.without_upstream()
        };
        GitState::from_status(&rendered)
    });
    theme.segments.git.get_bg_color(state)
}

/// Whether a config reload changes the rendered prompt and shells should redraw.
/// Helpers destructure without `..` so a new config field fails to compile until
/// classified. Script segment overrides change what the shells run.
pub(crate) fn config_refresh_required(old_config: &Config, new_config: &Config) -> bool {
    let Config {
        agent: old_agent,
        git: old_git,
        language: old_language,
        ui: old_ui,
        plugin: old_plugin,
    } = old_config;
    let Config {
        agent: new_agent,
        git: new_git,
        language: new_language,
        ui: new_ui,
        plugin: new_plugin,
    } = new_config;

    old_plugin != new_plugin
        || ui_refresh_required(old_ui, new_ui)
        || git_refresh_required(old_git, new_git)
        || language_refresh_required(old_language, new_language)
        || agent_refresh_required(old_agent, new_agent)
}

/// Every `UiSettings` field but `retire_unmodified_themes` (read by `gpy setup`
/// only) affects rendering; shells learn `transient_prompt` from the theme export.
pub(super) fn ui_refresh_required(
    old: &crate::config::UiSettings,
    new: &crate::config::UiSettings,
) -> bool {
    let crate::config::UiSettings {
        show_icons: old_show_icons,
        theme: old_theme,
        palette: old_palette,
        directory: old_directory,
        enabled_segments: old_enabled_segments,
        transient_prompt: old_transient_prompt,
        retire_unmodified_themes: _old_retire_unmodified_themes,
    } = old;
    let crate::config::UiSettings {
        show_icons: new_show_icons,
        theme: new_theme,
        palette: new_palette,
        directory: new_directory,
        enabled_segments: new_enabled_segments,
        transient_prompt: new_transient_prompt,
        retire_unmodified_themes: _new_retire_unmodified_themes,
    } = new;

    old_theme != new_theme
        || old_palette != new_palette
        || old_show_icons != new_show_icons
        || old_directory != new_directory
        || old_enabled_segments != new_enabled_segments
        || old_transient_prompt != new_transient_prompt
}

/// Whether a `[git]` change needs cached git prompts re-rendered and a reload repaint.
/// `timeout_seconds` and `watch_worktree` only affect later scans. `skip_paths`
/// decides which repos may show git at all: the regeneration this
/// triggers deletes the git caches of repos it newly excludes.
pub(super) fn git_refresh_required(
    old: &crate::config::GitSettings,
    new: &crate::config::GitSettings,
) -> bool {
    let crate::config::GitSettings {
        show_upstream: old_show_upstream,
        timeout_seconds: _old_timeout_seconds,
        skip_paths: old_skip_paths,
        max_branch_length: old_max_branch_length,
        max_ahead_behind: old_max_ahead_behind,
        watch_worktree: _old_watch_worktree,
        icons: old_icons,
        icon_set: old_icon_set,
        stash_enabled: old_stash_enabled,
    } = old;
    let crate::config::GitSettings {
        show_upstream: new_show_upstream,
        timeout_seconds: _new_timeout_seconds,
        skip_paths: new_skip_paths,
        max_branch_length: new_max_branch_length,
        max_ahead_behind: new_max_ahead_behind,
        watch_worktree: _new_watch_worktree,
        icons: new_icons,
        icon_set: new_icon_set,
        stash_enabled: new_stash_enabled,
    } = new;

    old_skip_paths != new_skip_paths
        || old_show_upstream != new_show_upstream
        || old_max_branch_length != new_max_branch_length
        || old_max_ahead_behind != new_max_ahead_behind
        || old_icons != new_icons
        || old_icon_set != new_icon_set
        || old_stash_enabled != new_stash_enabled
}

/// Detection-time fields are excluded; `filter` is render-time and included.
pub(super) fn language_refresh_required(
    old: &crate::config::LanguageSettings,
    new: &crate::config::LanguageSettings,
) -> bool {
    let crate::config::LanguageSettings {
        show_versions: old_show_versions,
        cache_ttl_hours: _old_cache_ttl_hours,
        enabled_languages: old_enabled_languages,
        display: old_display,
        filter: old_filter,
        detection_mode: _old_detection_mode,
        confidence_threshold: _old_confidence_threshold,
        icons: old_icons,
    } = old;
    let crate::config::LanguageSettings {
        show_versions: new_show_versions,
        cache_ttl_hours: _new_cache_ttl_hours,
        enabled_languages: new_enabled_languages,
        display: new_display,
        filter: new_filter,
        detection_mode: _new_detection_mode,
        confidence_threshold: _new_confidence_threshold,
        icons: new_icons,
    } = new;

    old_show_versions != new_show_versions
        || old_enabled_languages != new_enabled_languages
        || old_display != new_display
        || old_filter != new_filter
        || old_icons != new_icons
}

/// Process-management fields (`timeout_seconds`, `supervisor`) are excluded: they
/// render nothing, and shells get them from the theme export, whose change rings
/// the reload doorbell on its own.
pub(super) fn agent_refresh_required(
    old: &crate::config::AgentSettings,
    new: &crate::config::AgentSettings,
) -> bool {
    let crate::config::AgentSettings {
        enabled: old_enabled,
        timeout_seconds: _old_timeout_seconds,
        live_updates: old_live_updates,
        supervisor: _old_supervisor,
    } = old;
    let crate::config::AgentSettings {
        enabled: new_enabled,
        timeout_seconds: _new_timeout_seconds,
        live_updates: new_live_updates,
        supervisor: _new_supervisor,
    } = new;

    old_enabled != new_enabled || old_live_updates != new_live_updates
}
