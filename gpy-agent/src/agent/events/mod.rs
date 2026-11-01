//! Event handlers for git, config, and theme file changes.

use crate::cache::InstantPromptCache;
use crate::config::Config;
use crate::debug_log;
use crate::git::cache::GitStatusCache;
use crate::git::native::NativeGitBackend;
use crate::git::status::GitBackend;
use crate::git::{CompleteStatus, RepositoryStatus};
use crate::ipc::ClientDirectory;
use crate::language::detection_cache::LANGUAGE_REFRESH_INTERVAL;
use crate::language::display::{DisplayRequest, Versions, display_info};
use crate::palette::PaletteCache;
use crate::theme::{ThemeConfig, ThemeManager};
use crate::watcher::{DebouncedEvent, FileEvent, GitPaths, multi_repo::MultiRepoWatcher};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod coalesce;
mod refresh;
mod reload;
pub use coalesce::RepoRefreshCoordinator;
use coalesce::run_coalesced;
pub(crate) use refresh::{config_refresh_required, git_repo_filter, git_segment_bg};
pub(in crate::agent) use reload::{create_watcher, handle_config_reload, handle_theme_reload};

/// Shared thread-safe storage for the file watcher, allowing it to be replaced or stopped
pub type SharedWatcherSlot = Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>;

/// Context holding shared agent components required for event handling
#[derive(Clone)]
pub struct AgentContext {
    /// Registered shell clients.
    pub registry: Arc<ClientDirectory>,
    /// Git status cache.
    pub cache: Arc<GitStatusCache>,
    /// Live configuration.
    pub config_manager: Arc<crate::config::manager::ConfigManager>,
    /// Active theme.
    pub theme_manager: Arc<ThemeManager>,
    /// On-disk instant-prompt cache.
    pub instant_cache: Arc<InstantPromptCache>,
    /// Language detection cache.
    pub language_cache: crate::language::DetectionCache,
    /// Resolved palettes.
    pub palette_cache: Arc<PaletteCache>,
    /// Serializes per-repository refreshes.
    pub dispatch_guard: Arc<RepoRefreshCoordinator>,
    /// Registry shared by the repo, config, and theme watchers; `create_watcher` needs it.
    pub watch_registry: Arc<crate::watcher::WatchRegistry>,
    /// Where reloads write the shell theme exports (the gpy cache dir in production).
    pub theme_export_dir: PathBuf,
    /// Held by each config or theme reload over its theme reload and export
    /// write, so one theme change is seen as new, and rung for, by one path.
    pub announce_gate: Arc<Mutex<()>>,
}

/// Handle a debounced file system event.
pub fn handle_file_event(ctx: &AgentContext, event: &DebouncedEvent, config: &Config) {
    debug_log!("agent", "handle_file_event called: {:?}", event);
    match &event.event {
        FileEvent::Git { paths } => {
            if !config.agent.live_updates {
                debug_log!("agent", "Skipping git event (live_updates=false)");
                return;
            }

            debug_log!("agent", "Git event for: {paths:?}");

            // `event.repo` is canonical, so the cache `*_canonical` fast paths apply.
            let git_root = &event.repo;
            let paths_hint = git_paths_hint(paths, git_root);
            refresh_and_notify_coalesced(ctx, git_root, config, paths_hint.as_deref());
        }
        FileEvent::Language { path } => {
            debug_log!("agent", "Language event for: {}", path.display());

            if !config.agent.live_updates || !config.segment_enabled("language") {
                return;
            }

            let Some(repo_root) = crate::git::find_worktree_root(path)
                .or_else(|| path.parent().map(Path::to_path_buf))
            else {
                return;
            };

            // A language marker changed, so version-release data may be stale too.
            invalidate_language_caches(ctx, &repo_root);
            // Re-detection probes the now-cold versions (up to 10 s with mise),
            // so it must not hold the event thread.
            let refresh_ctx = ctx.clone();
            let refresh_config = config.clone();
            crate::ipc::handlers::spawn_detached(move || {
                let theme = refresh_ctx.theme_manager.get();
                refresh_language_for(
                    &refresh_ctx,
                    &repo_root,
                    &refresh_config,
                    LanguageRefreshMode::Immediate,
                    &theme,
                    None,
                );
            });
        }
        FileEvent::Config { path } => {
            debug_log!("agent", "Config event for: {}", path.display());
        }
        FileEvent::Theme { path } => {
            debug_log!("agent", "Theme event for: {}", path.display());
        }
    }
}

/// Pathspec hint for an incremental scan of a git event, or `None` for a full scan.
///
/// Escalates the whole burst on `.gitignore`, any `.git` path component (git
/// never reports on `.git` internals, so an empty result would read as clean),
/// the repo root or paths outside it, and [`GitPaths::WholeRepo`].
fn git_paths_hint(paths: &GitPaths, git_root: &Path) -> Option<Vec<PathBuf>> {
    let candidates = paths.as_slice()?;
    if candidates.is_empty() {
        return None;
    }

    for path in candidates {
        let relative = incremental_relative_path(path, git_root)?;
        if relative.components().any(|c| c.as_os_str() == ".git") {
            return None;
        }
        // Reclassifies files the pathspec would never name.
        if relative.ends_with(".gitignore") {
            return None;
        }
    }

    Some(candidates.to_vec())
}

/// Repo-relative path for an incremental scan, or `None` outside `git_root` or
/// for the root itself (git rejects an empty pathspec).
fn incremental_relative_path<'a>(changed: &'a Path, git_root: &Path) -> Option<&'a Path> {
    let relative_path = changed.strip_prefix(git_root).ok()?;
    if relative_path.as_os_str().is_empty() {
        return None;
    }
    Some(relative_path)
}

/// Clear the language and version caches for `repo_root`.
fn invalidate_language_caches(ctx: &AgentContext, repo_root: &Path) {
    ctx.language_cache.invalidate(repo_root);
    crate::language::version::invalidate_version_cache_at(repo_root);
}

/// How [`refresh_language_for`] obtains the detected-languages list.
#[derive(Clone, Copy)]
enum LanguageRefreshMode {
    /// Always re-detect; a marker-file event must not be throttled away.
    Immediate,
    /// Re-detect only if the detection cache says `signature` is due.
    Throttled { signature: u64 },
    /// Render cached data only (theme switch); write nothing if uncached.
    CachedOnly,
}

/// Refresh `root`'s languages per `mode`, write its instant-cache variants, and
/// repaint on change. Returns whether anything was written.
///
/// `theme` is passed in so callers deriving `extra_prev_bg` render from the same
/// snapshot (no torn read). `extra_prev_bg` seeds a render context no shell has
/// requested yet; known contexts are always refreshed.
fn refresh_language_for(
    ctx: &AgentContext,
    root: &Path,
    config: &Config,
    mode: LanguageRefreshMode,
    theme: &ThemeConfig,
    extra_prev_bg: Option<&str>,
) -> bool {
    let Some(detected) = detected_languages_for(ctx, root, mode) else {
        return false;
    };

    let palette = ctx.palette_cache.get();
    let stashed_venv = crate::language::venv::stashed_project_venv(root);
    // Off the request path, so a cold version is probed here.
    let request = DisplayRequest::new(&detected, theme, &config.language)
        .with_cwd(Some(root))
        .with_venv(stashed_venv.as_ref());
    let languages = display_info(&request, Versions::Probe).unwrap_or_default();

    let mut wrote = false;
    for prev_bg in std::iter::once(None).chain(extra_prev_bg.map(Some)) {
        match ctx
            .instant_cache
            .write_language_variants(root, &languages, config, theme, prev_bg, &palette)
        {
            Ok(changed) => wrote |= changed,
            Err(e) => {
                debug_log!(
                    "agent",
                    "Failed to write language instant cache for {}: {}",
                    root.display(),
                    e
                );
            }
        }
    }

    if wrote {
        ctx.registry.notify_repaint_force(Some(root));
    }
    wrote
}

/// Languages to render under `mode`, updating the detection cache on re-detect.
/// `None` means write nothing (throttled, or no cached data for `CachedOnly`).
fn detected_languages_for(
    ctx: &AgentContext,
    root: &Path,
    mode: LanguageRefreshMode,
) -> Option<Vec<crate::language::DetectedLanguage>> {
    match mode {
        LanguageRefreshMode::Immediate => {
            let detected = detect_languages(ctx, root);
            if let Some(signature) = ctx
                .cache
                .get(root)
                .map(|status| git_status_signature(&status))
            {
                ctx.language_cache
                    .set_with_signature(root, detected.clone(), signature);
            } else {
                ctx.language_cache.set(root, detected.clone());
            }
            Some(detected)
        }
        LanguageRefreshMode::Throttled { signature } => {
            // Re-checked: another job may have refreshed this repo since scheduling.
            if !ctx
                .language_cache
                .should_refresh(root, signature, LANGUAGE_REFRESH_INTERVAL)
            {
                return None;
            }
            let detected = detect_languages(ctx, root);
            ctx.language_cache
                .set_with_signature(root, detected.clone(), signature);
            Some(detected)
        }
        LanguageRefreshMode::CachedOnly => ctx.language_cache.get(root),
    }
}

/// Run language detection for `root` with the configured detection mode.
fn detect_languages(ctx: &AgentContext, root: &Path) -> Vec<crate::language::DetectedLanguage> {
    let detection_mode = ctx.config_manager.get().language.detection_mode;
    crate::language::Detector::detect_directory_bounded(root, detection_mode)
}

/// Outcome of the instant-prompt cache write for a refresh, mirroring
/// [`InstantPromptCache::write_git`]. A failed write leaves the file and the
/// last-written memo untouched, so a later identical write reports `Written`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheWriteOutcome {
    /// No write attempted (repo unavailable or scan failed).
    NotAttempted,
    /// `write_git` returned `Err`; on-disk file unchanged.
    Failed,
    /// `write_git` returned `Ok(false)`; content already matched.
    Unchanged,
    /// `write_git` returned `Ok(true)`; fresh content on disk.
    Written,
}

/// New repository status plus the cache write outcome.
struct RepoRefresh {
    status: Option<RepositoryStatus>,
    cache_write: CacheWriteOutcome,
}

/// Recompute a repo's git status, refresh caches, and notify clients when the
/// status changed or a cache write recovered. A `Failed` write suppresses the
/// repaint (the shell would re-read a stale file); the next refresh retries.
pub(super) fn refresh_and_notify_if_changed(
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    paths_hint: Option<&[PathBuf]>,
) {
    // Ignore TTL: an expired entry must not count as "changed".
    let previous = ctx.cache.get_any_canonical(git_root);

    let refresh = refresh_repo_status(ctx, git_root, config, paths_hint);

    let should_notify = match refresh.cache_write {
        CacheWriteOutcome::Written => true,
        // Failed write: never notify on this pass; see doc comment.
        CacheWriteOutcome::Failed => false,
        CacheWriteOutcome::Unchanged | CacheWriteOutcome::NotAttempted => {
            status_changed(previous.as_ref(), refresh.status.as_ref())
        }
    };

    if should_notify {
        debug_log!(
            "agent",
            "Status changed for {}, notifying clients via SIGURG (forced)",
            git_root.display()
        );
        ctx.registry.notify_repaint_force(Some(git_root));
    } else {
        debug_log!(
            "agent",
            "Status unchanged for {}, skipping notification",
            git_root.display()
        );
    }
}

/// Per-repo-serialized [`refresh_and_notify_if_changed`]; production callers
/// (watcher events, reconcile ticks) must use this, as it applies the
/// [`Config::git_enabled_for`](crate::config::Config::git_enabled_for)
/// gate. Follow-up passes force a full scan since the original hint is stale.
pub(super) fn refresh_and_notify_coalesced(
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    paths_hint: Option<&[PathBuf]>,
) {
    if !config.git_enabled_for(git_root) {
        debug_log!(
            "agent",
            "Skipping git refresh for {} (git unlisted or git.skip_paths)",
            git_root.display()
        );
        return;
    }
    run_coalesced(&ctx.dispatch_guard, git_root, |is_follow_up| {
        let hint = if is_follow_up { None } else { paths_hint };
        refresh_and_notify_if_changed(ctx, git_root, config, hint);
    });
}

/// Whether a status transition warrants notifying clients.
///
/// Content-based: states that render identically (e.g. ahead/behind already
/// clamped by the parser) are correctly treated as unchanged.
fn status_changed(previous: Option<&RepositoryStatus>, new: Option<&RepositoryStatus>) -> bool {
    match (previous, new) {
        (None, None) => false,
        (Some(old), Some(current)) => old != current,
        _ => true, // repo appeared or became unavailable
    }
}

/// Map a path inside a nested working tree (submodule or nested repo) to that
/// tree's root, since the superproject pathspec would match nothing below it.
/// The outermost directory holding `.git` wins.
fn nested_worktree_root(relative: &Path, git_root: &Path) -> PathBuf {
    relative
        .ancestors()
        .skip(1)
        .take_while(|dir| !dir.as_os_str().is_empty())
        .filter(|dir| git_root.join(dir).join(".git").symlink_metadata().is_ok())
        .last()
        .unwrap_or(relative)
        .to_path_buf()
}

/// Pathspec-limited incremental status update for `changed`; `None` means fall
/// back to a full scan.
///
/// A path missing from the output is clean now (that is how git reports it,
/// including a rename's source); `.git` paths never reach here.
fn try_incremental_update_with<B: GitBackend>(
    backend: &B,
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    changed: &[PathBuf],
) -> RepoRefresh {
    let not_attempted = RepoRefresh {
        status: None,
        cache_write: CacheWriteOutcome::NotAttempted,
    };

    let mut relative_paths: Vec<PathBuf> = Vec::with_capacity(changed.len());
    for path in changed {
        let Some(relative) = incremental_relative_path(path, git_root) else {
            return not_attempted;
        };
        let scan_path = nested_worktree_root(relative, git_root);
        // Index may use another Unicode normalization (NFC vs NFD on macOS),
        // so the pathspec could match nothing; fall back to a full scan.
        if !scan_path.to_str().is_some_and(str::is_ascii) {
            return not_attempted;
        }
        if !relative_paths.contains(&scan_path) {
            relative_paths.push(scan_path);
        }
    }
    if relative_paths.is_empty() {
        return not_attempted;
    }

    match backend.pathspec_status(
        git_root,
        &relative_paths,
        Duration::from_secs(config.git.timeout_seconds.get()),
    ) {
        Ok(scan) => {
            let Some(updated_status) = ctx.cache.update_paths_canonical(
                git_root,
                &relative_paths,
                &scan.files,
                scan.ignore_case,
            ) else {
                return not_attempted;
            };
            debug_log!(
                "agent",
                "Incremental cache update for {} path(s) under {}",
                relative_paths.len(),
                git_root.display()
            );
            let cache_write = write_status_caches(ctx, git_root, config, &updated_status);
            RepoRefresh {
                status: Some(updated_status),
                cache_write,
            }
        }
        Err(e) => {
            debug_log!("agent", "Incremental check failed: {}", e);
            not_attempted
        }
    }
}

/// Refresh a repo's git status and caches, incrementally when `paths_hint` allows.
fn refresh_repo_status(
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    paths_hint: Option<&[PathBuf]>,
) -> RepoRefresh {
    refresh_repo_status_with(&NativeGitBackend, ctx, git_root, config, paths_hint)
}

/// [`refresh_repo_status`] over an arbitrary [`GitBackend`], shared with the
/// incremental path so tests can count capture rounds.
fn refresh_repo_status_with<B: GitBackend>(
    backend: &B,
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    paths_hint: Option<&[PathBuf]>,
) -> RepoRefresh {
    if let Some(paths) = paths_hint
        && !paths.is_empty()
    {
        let incremental = try_incremental_update_with(backend, ctx, git_root, config, paths);
        if incremental.status.is_some() {
            return incremental;
        }
    }

    // Invalidate first so the scan reflects the triggering mutation.
    ctx.cache.invalidate_canonical(git_root);
    match backend.complete_status(
        git_root,
        config.git.max_ahead_behind,
        config.git.stash_enabled,
        Duration::from_secs(config.git.timeout_seconds.get()),
    ) {
        Ok(Some(CompleteStatus { status, files })) => {
            debug_log!(
                "agent",
                "Refreshing cache (full scan) for: {}",
                git_root.display()
            );
            ctx.cache.set_canonical(git_root, status.clone(), files);
            let cache_write = write_status_caches(ctx, git_root, config, &status);
            RepoRefresh {
                status: Some(status),
                cache_write,
            }
        }
        Ok(None) => {
            debug_log!(
                "agent",
                "Repository state unavailable for: {}",
                git_root.display()
            );
            RepoRefresh {
                status: None,
                cache_write: CacheWriteOutcome::NotAttempted,
            }
        }
        Err(error) => {
            debug_log!(
                "agent",
                "Failed to refresh cache for {}: {}",
                git_root.display(),
                error
            );
            RepoRefresh {
                status: None,
                cache_write: CacheWriteOutcome::NotAttempted,
            }
        }
    }
}

/// Write the instant-prompt cache and schedule a language refresh for `status`.
fn write_status_caches(
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    status: &RepositoryStatus,
) -> CacheWriteOutcome {
    let theme = ctx.theme_manager.get();
    let palette = ctx.palette_cache.get();
    // No IPC request context, so prev_bg is None.
    let cache_write = match ctx
        .instant_cache
        .write_git(git_root, status, config, &theme, None, &palette)
    {
        Ok(true) => CacheWriteOutcome::Written,
        Ok(false) => CacheWriteOutcome::Unchanged,
        Err(e) => {
            debug_log!("agent", "Failed to write instant-prompt cache: {}", e);
            CacheWriteOutcome::Failed
        }
    };
    schedule_language_refresh_from_git_status(ctx, git_root, config, status);
    cache_write
}

fn schedule_language_refresh_from_git_status(
    ctx: &AgentContext,
    repo_root: &Path,
    config: &Config,
    status: &crate::git::RepositoryStatus,
) {
    if !config.agent.live_updates || !config.segment_enabled("language") {
        return;
    }

    let signature = git_status_signature(status);
    // Pre-check; `refresh_language_for` re-checks when the job runs.
    if !ctx
        .language_cache
        .should_refresh(repo_root, signature, LANGUAGE_REFRESH_INTERVAL)
    {
        return;
    }

    let refresh_ctx = ctx.clone();
    let refresh_repo_root = repo_root.to_path_buf();
    let refresh_config = config.clone();
    let job = move || {
        let theme = refresh_ctx.theme_manager.get();
        refresh_language_for(
            &refresh_ctx,
            &refresh_repo_root,
            &refresh_config,
            LanguageRefreshMode::Throttled { signature },
            &theme,
            None,
        );
    };

    crate::ipc::handlers::spawn_detached(job);
}

fn git_status_signature(status: &crate::git::RepositoryStatus) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (
        status.staged,
        status.unstaged,
        status.untracked,
        status.conflicts,
    )
        .hash(&mut hasher);
    hasher.finish()
}

/// Whether `event` is a git change no registered client would see, so its
/// refresh is skipped. Applied by the watcher callback, not in
/// `handle_file_event`, whose tests use an empty registry.
fn git_event_lacks_subscriber(ctx: &AgentContext, event: &DebouncedEvent) -> bool {
    matches!(event.event, FileEvent::Git { .. }) && !ctx.registry.has_subscriber(&event.repo)
}

#[cfg(test)]
mod coordinator_tests;
#[cfg(test)]
mod tests;
