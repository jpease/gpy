//! Config, theme, and palette reload handling and the watcher that feeds file events.

use super::{
    AgentContext, LanguageRefreshMode, SharedWatcherSlot, config_refresh_required,
    git_event_lacks_subscriber, git_segment_bg, handle_file_event, refresh, refresh_language_for,
};
use crate::config::Config;
use crate::debug_log;
use crate::sync_ext::MutexExt;
use crate::theme::ThemeManager;
use crate::watcher::{DebouncedEvent, WatcherConfig, multi_repo::MultiRepoWatcher};
use crate::{Error, Result};
use std::path::Path;
use std::sync::Arc;

/// Create the filesystem watcher used for live updates.
///
/// # Errors
/// Returns an error if watcher initialization fails.
pub(in crate::agent) fn create_watcher(
    watcher_config: WatcherConfig,
    ctx: &AgentContext,
) -> Result<Arc<MultiRepoWatcher>> {
    let ctx_for_callback = ctx.clone();

    MultiRepoWatcher::builder()
        .registry(Arc::clone(&ctx.watch_registry))
        .config(watcher_config)
        .callback(Box::new(move |event: DebouncedEvent| {
            if git_event_lacks_subscriber(&ctx_for_callback, &event) {
                debug_log!(
                    "agent",
                    "Skipping git refresh for {}: no registered client subscribes to it",
                    event.repo.display()
                );
                return;
            }
            let config = ctx_for_callback.config_manager.get();
            handle_file_event(&ctx_for_callback, &event, &config);
        }))
        .build()
        .map(Arc::new)
}

/// Switch to the config's theme when its name changed; returns whether it did.
///
/// Runs outside [`AgentContext::announce_gate`]: `switch_theme` joins the theme
/// poll thread, whose reloads take that gate.
///
/// # Errors
/// Returns an error when a newly selected theme cannot be loaded.
fn switch_renamed_theme(
    theme_manager: &ThemeManager,
    old_config: &Config,
    new_config: &Config,
) -> Result<bool> {
    if old_config.ui.theme == new_config.ui.theme {
        return Ok(false);
    }

    debug_log!(
        "config",
        "Theme name changed from '{}' to '{}', switching theme...",
        old_config.ui.theme,
        new_config.ui.theme
    );

    theme_manager.switch_theme(new_config.ui.theme.as_str())?;
    debug_log!(
        "config",
        "Successfully switched to theme: {}",
        new_config.ui.theme
    );
    Ok(true)
}

/// Reload the active name from wherever it now resolves, so a user override
/// added or removed, or an edit the theme watcher has not applied yet, takes
/// effect. Returns whether the theme changed; an edit the watcher already
/// applied reports `false`. A failed reload keeps the current theme, as the
/// theme watcher does, rather than rejecting unrelated config changes.
fn reload_active_theme(theme_manager: &ThemeManager, config: &Config) -> bool {
    theme_manager.reload().unwrap_or_else(|error| {
        crate::debug::warn_fallback(
            &format!("Theme '{}' reload", config.ui.theme),
            "keeping the current theme",
            &error,
        );
        false
    })
}

fn sync_watcher_state(
    watcher_slot: &SharedWatcherSlot,
    ctx: &AgentContext,
    watcher_config: WatcherConfig,
    new_config: &Config,
    reload_required: bool,
) {
    let mut watcher_guard = watcher_slot.lock_or_recover();

    if !new_config.agent.live_updates || !new_config.any_agent_segment_enabled() {
        if let Some(watcher) = watcher_guard.take() {
            watcher.stop();
        }
        drop(watcher_guard);
        if reload_required {
            ctx.registry.notify_reload();
        }
        return;
    }

    if watcher_guard.is_none() {
        match create_watcher(watcher_config, ctx) {
            Ok(new_watcher) => {
                let repos = ctx.registry.get_registered_repos();
                for (pid, path) in repos {
                    if let Err(e) = new_watcher.attach_client(pid, &path) {
                        debug_log!("config", "registration failed for {pid}: {e}");
                    }
                }
                *watcher_guard = Some(new_watcher);
            }
            Err(err) => {
                debug_log!(
                    "config",
                    "Failed to start watcher after config reload: {err}"
                );
            }
        }
    } else if let Some(watcher) = watcher_guard.as_ref() {
        // Existing watcher: apply the new `git.watch_worktree` to active repos.
        // Lock order: this slot before `MultiRepoWatcher`'s internal locks.
        watcher.resync_watch_worktree();
        watcher.resync_repo_filter(&ctx.registry.get_registered_repos());
    }
    drop(watcher_guard);

    if reload_required {
        ctx.registry.notify_reload();
    }
}

/// Load the palette a config reload selects, re-reading it even when `ui.palette`
/// is unchanged because palette file edits leave the name alone.
///
/// A newly selected palette that fails to load rejects the reload, like an unknown
/// theme. A failed re-read of the same palette keeps the last good one (`None`),
/// so a broken palette file neither blanks segments nor blocks unrelated changes.
///
/// # Errors
/// Returns an error when a newly selected palette cannot be loaded.
fn reload_palette(
    old_config: &Config,
    new_config: &Config,
) -> Result<Option<crate::template::Palette>> {
    let name = &new_config.ui.palette;
    match crate::palette::load_active_palette(new_config) {
        Ok(palette) => Ok(Some(palette)),
        Err(error) if old_config.ui.palette != *name => Err(Error::config(format!(
            "palette '{name}' could not be loaded: {error}"
        ))),
        Err(error) => {
            crate::debug::warn_fallback(
                &format!("Palette '{name}' reload"),
                "keeping the last good palette",
                &error,
            );
            Ok(None)
        }
    }
}

/// Apply non-filesystem runtime side effects for a validated config reload.
///
/// # Errors
/// Returns an error when a runtime subsystem rejects the config (e.g. unknown theme
/// or palette); nothing is applied in that case.
pub(in crate::agent) fn handle_config_reload(
    ctx: &AgentContext,
    watcher_slot: &SharedWatcherSlot,
    watcher_config: WatcherConfig,
    disable_watcher: bool,
    old_config: &Config,
    new_config: &Config,
) -> Result<()> {
    // Loaded before the theme switch so a rejected palette leaves nothing half-applied.
    let palette = reload_palette(old_config, new_config)?;
    let theme_switched = switch_renamed_theme(&ctx.theme_manager, old_config, new_config)?;
    let palette_changed = palette.is_some_and(|loaded| ctx.palette_cache.replace(loaded));

    let (theme_changed, export_changed) = {
        let _announcing = ctx.announce_gate.lock_or_recover();
        let theme_changed = theme_switched || reload_active_theme(&ctx.theme_manager, new_config);
        // Theme export must be on disk before the reload doorbell.
        (theme_changed, write_theme_export(ctx, new_config))
    };

    crate::language::version::set_version_cache_ttl(new_config.language.cache_ttl_hours.get());
    refresh::apply_watch_settings(&ctx.watch_registry, new_config);

    let refresh_required =
        theme_changed || palette_changed || config_refresh_required(old_config, new_config);

    // Re-render cached data with the new theme before the doorbell to avoid flicker.
    // No synchronous detection on this path.
    if refresh_required {
        regenerate_instant_caches_for_theme_change(ctx, new_config);
    }
    // Shells re-read a changed export only on the doorbell, including settings
    // that render nothing (e.g. `agent.supervisor`), so it rings for those too.
    let reload_required = refresh_required || export_changed;

    if disable_watcher {
        if reload_required {
            ctx.registry.notify_reload();
        }
        return Ok(());
    }

    sync_watcher_state(
        watcher_slot,
        ctx,
        watcher_config,
        new_config,
        reload_required,
    );
    Ok(())
}

/// Write the shell theme exports; returns whether any changed. A failure is
/// logged and reported as unchanged.
fn write_theme_export(ctx: &AgentContext, config: &Config) -> bool {
    crate::cache::theme_export::write_theme_export_to_dir(
        &ctx.theme_export_dir,
        &ctx.theme_manager,
        config,
        &ctx.palette_cache.get(),
    )
    .unwrap_or_else(|e| {
        debug_log!("agent", "Failed to write theme export cache: {}", e);
        false
    })
}

/// Run a theme watcher or poll reload: `reload` swaps in the active theme and
/// reports whether it changed. Under [`AgentContext::announce_gate`], so a
/// change a concurrent config reload already applied, and rang for, reads as
/// unchanged here; otherwise refresh the export and caches, then ring.
pub(in crate::agent) fn handle_theme_reload(ctx: &AgentContext, reload: &dyn Fn() -> bool) {
    let _announcing = ctx.announce_gate.lock_or_recover();
    if !reload() {
        return;
    }
    let config = ctx.config_manager.get();
    write_theme_export(ctx, &config);
    regenerate_instant_caches_for_theme_change(ctx, &config);
    ctx.registry.notify_reload();
}

/// Re-render instant-prompt caches for every registered repo from cached data
/// with the new theme, so the post-reload repaint is warm and flicker-free.
/// A repo that an unlisted `git` or `git.skip_paths` excludes has its git caches
/// deleted instead, so shells stop showing a segment nothing refreshes.
///
/// Renders both the context-free fallback and the new theme's `prev_bg` token the
/// shell will send. Language rendering runs version detection, so it is deferred
/// to a background job rather than blocking the reload handler.
pub(super) fn regenerate_instant_caches_for_theme_change(ctx: &AgentContext, config: &Config) {
    let theme = ctx.theme_manager.get();
    let palette = ctx.palette_cache.get();
    // prev_bg for git under the new theme; mirrors `ThemeManager::export`.
    let git_prev_bg = theme.segments.directory.bg_color.as_str();
    // Repos `config` includes again may publish git caches once more.
    ctx.instant_cache
        .release_git_suppression(|root| config.git_enabled_for(root));

    // Dedup: several clients may share one repo.
    let mut seen_keys = std::collections::HashSet::new();
    for (_pid, cwd) in ctx.registry.get_registered_repos() {
        let git_root = crate::git::find_worktree_root(&cwd);
        // Keyed by request path when there is no git root.
        let lang_root = git_root.clone().unwrap_or_else(|| cwd.clone());
        if !seen_keys.insert(lang_root.clone()) {
            continue;
        }
        if let Some(root) = git_root.as_ref() {
            if config.git_enabled_for(root) {
                if let Some(status) = ctx.cache.get(root) {
                    for prev_bg in [None, Some(git_prev_bg)] {
                        if let Err(e) = ctx
                            .instant_cache
                            .write_git(root, &status, config, &theme, prev_bg, &palette)
                        {
                            debug_log!(
                                "agent",
                                "theme-change git cache regen failed for {}: {}",
                                root.display(),
                                e
                            );
                        }
                    }
                }
            } else {
                ctx.instant_cache.remove_git(root);
                // Nothing refreshes an excluded repo, so its status goes stale;
                // re-including it must rescan rather than republish this one.
                ctx.cache.invalidate_canonical(root);
            }
        }

        if config.segment_enabled("language") && ctx.language_cache.get(&lang_root).is_some() {
            schedule_language_cache_regen(ctx, &lang_root, config);
        }
    }
}

/// Re-render a repo's language caches off the event loop and repaint on change.
fn schedule_language_cache_regen(ctx: &AgentContext, lang_root: &Path, config: &Config) {
    let regen_ctx = ctx.clone();
    let root = lang_root.to_path_buf();
    let regen_config = config.clone();

    let job = move || {
        regen_language_caches(&regen_ctx, &root, &regen_config);
    };

    crate::ipc::handlers::spawn_detached(job);
}

/// Synchronous body of [`schedule_language_cache_regen`]; returns `true` when a
/// cache file changed.
pub(super) fn regen_language_caches(ctx: &AgentContext, lang_root: &Path, config: &Config) -> bool {
    // One theme snapshot for both `lang_prev_bg` and the render, to avoid a torn read.
    let theme = ctx.theme_manager.get();
    let lang_prev_bg = git_segment_bg(ctx, lang_root, config, &theme);

    refresh_language_for(
        ctx,
        lang_root,
        config,
        LanguageRefreshMode::CachedOnly,
        &theme,
        Some(lang_prev_bg),
    )
}
