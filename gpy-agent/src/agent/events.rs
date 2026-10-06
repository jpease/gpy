//! Event handling logic for file system changes and configuration reloads
//!
//! This module contains all event handlers that respond to:
//! - Git repository changes (via file watcher)
//! - Config file changes (hot-reload)
//! - Theme file changes
//!
//! All event coordination must flow through these handlers to maintain
//! architectural clarity and prevent circular dependencies.

#![expect(
    clippy::too_long_first_doc_paragraph,
    reason = "the module summary of what flows through this file's event handlers reads better as one paragraph than split across a summary line and a details paragraph"
)]

use crate::Result;
use crate::cache::InstantPromptCache;
use crate::config::Config;
use crate::debug_log;
use crate::git::cache::GitStatusCache;
use crate::git::native::NativeGitBackend;
use crate::git::status::{GitBackend, load_repository_state_with};
use crate::git::{CompleteStatus, RepositoryStatus};
use crate::ipc::ClientDirectory;
use crate::language::detection_cache::LANGUAGE_REFRESH_INTERVAL;
use crate::palette::PaletteCache;
use crate::theme::{ThemeConfig, ThemeManager};
use crate::watcher::{
    DebouncedEvent, FileEvent, GitPaths, WatcherConfig, multi_repo::MultiRepoWatcher,
};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared thread-safe storage for the file watcher, allowing it to be replaced or stopped
pub type SharedWatcherSlot = Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>;

/// Context holding shared agent components required for event handling
#[derive(Clone)]
pub struct AgentContext {
    #[expect(
        missing_docs,
        reason = "field name and type are self-explanatory; a doc comment would only restate them"
    )]
    pub registry: Arc<ClientDirectory>,
    #[expect(
        missing_docs,
        reason = "field name and type are self-explanatory; a doc comment would only restate them"
    )]
    pub cache: Arc<GitStatusCache>,
    #[expect(
        missing_docs,
        reason = "field name and type are self-explanatory; a doc comment would only restate them"
    )]
    pub config_manager: Arc<crate::config::manager::ConfigManager>,
    #[expect(
        missing_docs,
        reason = "field name and type are self-explanatory; a doc comment would only restate them"
    )]
    pub theme_manager: Arc<ThemeManager>,
    #[expect(
        missing_docs,
        reason = "field name and type are self-explanatory; a doc comment would only restate them"
    )]
    pub instant_cache: Arc<InstantPromptCache>,
    #[expect(
        missing_docs,
        reason = "field name and type are self-explanatory; a doc comment would only restate them"
    )]
    pub language_cache: crate::language::DetectionCache,
    #[expect(
        missing_docs,
        reason = "field name and type are self-explanatory; a doc comment would only restate them"
    )]
    pub palette_cache: Arc<PaletteCache>,
    #[expect(
        missing_docs,
        reason = "field name and type are self-explanatory; a doc comment would only restate them"
    )]
    pub dispatch_guard: Arc<RepoRefreshCoordinator>,
    /// The one [`crate::watcher::WatchRegistry`] the agent shares with its
    /// repo, config and theme watchers, so all three classify events against
    /// the same registered config paths and the same `git.watch_worktree`
    /// setting (#617). Held here because `create_watcher` — the single
    /// production entry point that builds the repo watcher, on startup and
    /// again after a config reload — has no other handle to it.
    pub watch_registry: Arc<crate::watcher::WatchRegistry>,
}

/// Serializes [`refresh_and_notify_if_changed`] **per repository** so that a
/// stale scan can never overwrite a fresher one, while different repositories
/// stay fully concurrent (#418).
///
/// Every git watcher event is dispatched on its own thread and the periodic
/// reconcile timer runs its own scan (#417); two of those can target the *same*
/// repo at once. `refresh_and_notify_if_changed` is a compound
/// read-`previous` → run-git-subprocess → write-cache → compare-and-signal
/// sequence that is **not** atomic as a whole, so two concurrent scans of one
/// repo can interleave and let whichever git subprocess *finishes* last win the
/// cache — even when its state is actually the *older* of the two. The
/// coordinator collapses concurrent same-repo work into "run once, then run
/// exactly once more if something changed while the run was in flight."
///
/// Cross-repo concurrency is preserved (regression guard for #306): the inner
/// `Mutex` is held only for the brief map bookkeeping inside [`Self::try_start`]
/// / [`Self::finish`], never across the expensive git-status scan, so a refresh
/// for repo B never waits on an in-flight refresh for repo A.
#[derive(Default)]
pub struct RepoRefreshCoordinator {
    /// Maps a repo root to whether another refresh was requested while the
    /// current one was still running. Presence of a key means "a refresh for
    /// this repo is in flight (owned by exactly one caller)"; the `bool` value
    /// is the pending-follow-up flag. The entry is removed once the owner
    /// finishes with nothing pending, so the map never grows unbounded across
    /// the process lifetime.
    state: Mutex<std::collections::HashMap<PathBuf, bool>>,
}

impl RepoRefreshCoordinator {
    /// Recover the guard even if a previous holder panicked. The lock only ever
    /// wraps `HashMap` bookkeeping (no user code runs under it), so a poisoned
    /// state is still perfectly consistent — recovering is strictly better than
    /// propagating a panic that would wedge every future refresh for the repo.
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<PathBuf, bool>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns `true` if the caller should run a refresh now (it becomes the
    /// sole owner of this repo's in-flight slot). Returns `false` if a refresh
    /// for this repo is already in flight: the caller must not run anything, and
    /// its change is recorded as pending so the in-flight owner picks it up via
    /// exactly one follow-up pass.
    fn try_start(&self, repo: &Path) -> bool {
        let mut guard = self.lock();
        match guard.entry(repo.to_path_buf()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                *entry.get_mut() = true;
                false
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(false);
                true
            }
        }
    }

    /// Call after finishing a run. Returns `true` if the caller must run exactly
    /// one more pass (a change was requested while this run was in flight); the
    /// pending flag is cleared so a fresh follow-up can re-detect it. Returns
    /// `false` when fully done — the repo's bookkeeping entry is removed so the
    /// map cannot grow unbounded.
    fn finish(&self, repo: &Path) -> bool {
        let mut guard = self.lock();
        if let Some(pending) = guard.get_mut(repo)
            && *pending
        {
            *pending = false;
            return true;
        }
        guard.remove(repo);
        false
    }

    /// Forcibly release a repo's in-flight slot without honoring a pending
    /// follow-up. Only used by the coalesce loop's defensive follow-up cap so a
    /// pathological churn burst can't wedge the repo forever; the dropped
    /// pending change is corrected by the next filesystem event or the reconcile
    /// backstop.
    fn abandon(&self, repo: &Path) {
        self.lock().remove(repo);
    }
}

/// Defensive upper bound on catch-up passes per [`refresh_and_notify_coalesced`]
/// invocation. Real drained events are already debounced ~100-150ms apart, so a
/// follow-up chain this long implies sustained sub-scan churn on a single repo
/// that would never converge; cap it (mirroring the debouncer's `max_delay`
/// defensive cap) and let the reconcile backstop mop up rather than spin
/// forever.
const MAX_COALESCED_FOLLOW_UPS: u32 = 32;

/// Handle file system events for prompt updates
pub fn handle_file_event(ctx: &AgentContext, event: &DebouncedEvent, config: &Config) {
    debug_log!("agent", "handle_file_event called: {:?}", event);
    match &event.event {
        FileEvent::Git { paths } => {
            if !config.agent.live_updates || !config.git.enabled {
                debug_log!(
                    "agent",
                    "Skipping git event (live_updates={}, git.enabled={})",
                    config.agent.live_updates,
                    config.git.enabled
                );
                return;
            }

            debug_log!("agent", "Git event for: {paths:?}");

            // `event.repo` is already canonical (find_git_root canonicalizes), so the
            // cache `*_canonical` fast paths apply.
            let git_root = event.repo.clone();
            let paths_hint = git_paths_hint(paths, &git_root);
            refresh_and_notify_coalesced(ctx, &git_root, config, paths_hint.as_deref());
        }
        FileEvent::Language { path } => {
            debug_log!("agent", "Language event for: {}", path.display());

            if !config.agent.live_updates || !config.language.enabled {
                return;
            }

            let Some(repo_root) = MultiRepoWatcher::find_git_root(path)
                .or_else(|| path.parent().map(Path::to_path_buf))
            else {
                return;
            };

            // Caller-specific pre-step: a language *marker* file changed, so
            // the version-release cache may be stale too. The other two
            // `refresh_language_for` callers must not do this.
            invalidate_language_caches(ctx, &repo_root);
            let theme = ctx.theme_manager.get();
            refresh_language_for(
                ctx,
                &repo_root,
                config,
                LanguageRefreshMode::Immediate,
                &theme,
                None,
            );
        }
        FileEvent::Config { path } => {
            debug_log!("agent", "Config event for: {}", path.display());
        }
        FileEvent::Theme { path } => {
            debug_log!("agent", "Theme event for: {}", path.display());
            // Theme events are handled by ThemeManager's own callback
            // This case exists to satisfy exhaustive match requirements
        }
    }
}

/// Decide whether a debounced git event can be served by a pathspec-limited
/// incremental status scan, returning the hint for [`refresh_repo_status`]
/// (`None` = full scan).
///
/// Escalation is per *event*, not per path: the hint becomes one git
/// invocation, so a single path that needs a full scan forces one for the whole
/// coalesced burst (#466). That is also what keeps a HEAD write from being
/// swallowed by the worktree edits it arrived with — branch, ahead/behind,
/// stash, detached, and operation state all come back from the full scan.
///
/// Three kinds of path force a full scan:
///
/// - **`.gitignore`**, wherever it sits in the worktree: it changes the status
///   of *other* files (a previously untracked file becomes ignored, or stops
///   being), so a path-limited scan would report stale results for paths the
///   pathspec never names. It is a worktree file, never a `.git` internal, so
///   it is matched by name rather than by location.
/// - **Anything under `.git/`** — the repository-global files (`config`,
///   `info/exclude`, `HEAD`, `packed-refs`; `git gc`/`git pack-refs` can move
///   any ref into `packed-refs`, #415) and every other internal (`index`,
///   `refs/…`, `ORIG_HEAD`, `logs/…`) alike. `git status` never reports on
///   `.git` internals, so a scan limited to one comes back empty; the
///   multi-path update reads "absent from the scan" as "clean now" — correct
///   for a worktree file, but for a ref write it would silently drop the
///   repository-wide refresh that write implies (#466). Before multi-path
///   these fell through to a full scan anyway, via an empty result; escalating
///   up front just skips the doomed subprocess.
/// - **The repository root itself**: a `Git` event whose path *is* the
///   watched root is a whole-repo signal; hinting it would strip down to the
///   empty pathspec git rejects outright, wasting a doomed subprocess before
///   falling back (#448). A rescan (#431 queue overflow, #442 native→poll
///   backend swap) reaches the same "no hint" outcome via
///   [`GitPaths::WholeRepo`] directly (#616) rather than by naming the root as
///   a path — this root-path handling stays in place as defence for any other
///   event that happens to carry the root as its path.
///
/// # Why the `.git` files are matched by location, not by filename (#611)
///
/// The four repository-global names (`config`, `exclude`, `HEAD`,
/// `packed-refs`) used to be tested with `Path::ends_with` against the raw
/// event path, *before* the repo-relative path was even computed.
/// `Path::ends_with` matches whole components, so that fired for any worktree
/// file or directory bare-named `config`, `HEAD`, `exclude` or `packed-refs` —
/// e.g. this repository's own `gpy-agent/src/config/`, which some watcher
/// backends report as a directory-level path on a rename or create batch —
/// forcing a whole-repo scan for an ordinary edit.
///
/// Gating those four on `.git` membership loses nothing versus the old
/// filename test — every case it could fire on already reaches `None` by
/// another route: a path outside `git_root`, or `git_root` itself, returns
/// `None` from `incremental_relative_path` (this is how a submodule's or
/// linked worktree's external git dir escalates — see
/// `external_git_dir_paths_yield_no_incremental_hint`), and a path *under*
/// `.git/` is caught by the membership check itself — and it additionally
/// catches a case the old test only caught by accident. The membership check
/// tests every component of the relative path, not just the first, so a
/// **nested** repository's own git-internal file (`nested-repo/.git/HEAD`,
/// where `.git` is the *second* component of the path relative to the
/// *parent* repo — e.g. because the watcher momentarily attributed the event
/// to the parent while the nested repo's own `.git` directory was briefly
/// absent, mid-move or mid-delete) still escalates: `git status` cannot
/// report on it any more than it can a top-level `.git` internal, so a scan
/// limited to it would come back empty and be misread as "clean now" (#466).
/// The old by-name filename test happened to also catch this, purely because
/// `Path::ends_with` matches a named component at any depth; that protection
/// would have been lost by a first-component-only membership check, which is
/// why this one walks every component instead. Only `.gitignore` is
/// genuinely a worktree file that must escalate, so it keeps its by-name test.
/// (A repository whose git directory is *not* named `.git`, via `GIT_DIR`,
/// never reaches here at all: the watcher classifies git events on a `.git`
/// path component, and `find_git_root` looks for `.git`.)
fn git_paths_hint(paths: &GitPaths, git_root: &Path) -> Option<Vec<PathBuf>> {
    // GitPaths::WholeRepo -- whether from accumulation past the debouncer's
    // bound or a rescan emitting it directly (#616) -- is already the
    // full-scan signal; the root-path check below is defence for any other
    // event that happens to carry the watched root as its path.
    let candidates = paths.as_slice()?;
    if candidates.is_empty() {
        return None;
    }

    for path in candidates {
        // Also rejects the repo root itself and anything outside the repo.
        let relative = incremental_relative_path(path, git_root)?;
        // Component-wise, not `starts_with(".git")`: a `.git` anywhere in the
        // relative path — not just as its first component — is still a
        // git-internal file `git status` cannot report on (see the doc
        // comment above for the nested-repo case this catches).
        if relative.components().any(|c| c.as_os_str() == ".git") {
            return None;
        }
        // A worktree file, at the root or in any subdirectory, whose change
        // reclassifies files the pathspec would never name.
        if relative.ends_with(".gitignore") {
            return None;
        }
    }

    Some(candidates.to_vec())
}

/// The repo-relative path to scan incrementally for `changed`, or `None` when
/// there is nothing scannable: a path outside `git_root`, or `git_root` itself.
///
/// The root strips to the empty path, which git rejects as a pathspec
/// (`fatal: empty string is not a valid pathspec`), so callers must fall back
/// to a full scan rather than shell out (#448).
fn incremental_relative_path<'a>(changed: &'a Path, git_root: &Path) -> Option<&'a Path> {
    let relative_path = changed.strip_prefix(git_root).ok()?;
    if relative_path.as_os_str().is_empty() {
        return None;
    }
    Some(relative_path)
}

/// Clear the language cache for `repo_root`, including the version-detection
/// cache, so the following re-detect reflects the file change that triggered it.
fn invalidate_language_caches(ctx: &AgentContext, repo_root: &Path) {
    ctx.language_cache.invalidate(repo_root);
    crate::language::version::invalidate_language_release_cache_at(repo_root);
}

/// How [`refresh_language_for`] should obtain the detected-languages list for a
/// repository before writing its instant-cache prompt variants.
#[derive(Clone, Copy)]
enum LanguageRefreshMode {
    /// Always re-detect, no throttling. Used when a discrete event (a language
    /// marker file changed) has already established that detection is due;
    /// throttling it would silently drop a real, individual change signal for
    /// up to [`LANGUAGE_REFRESH_INTERVAL`].
    Immediate,
    /// Re-detect only when
    /// [`crate::language::DetectionCache::should_refresh`] says a refresh is
    /// due for `signature` within [`LANGUAGE_REFRESH_INTERVAL`]. Rate-limits a
    /// burst of git-status-triggered checks that may carry the same signature.
    Throttled {
        /// The git-status signature the refreshed entry is recorded under.
        signature: u64,
    },
    /// Skip re-detection entirely and render whatever is already cached,
    /// reporting no write when nothing is. Used when only the *rendering*
    /// context changed (a theme switch), not the detected language set.
    CachedOnly,
}

/// Refresh `root`'s detected languages according to `mode`, write its
/// instant-cache prompt variants, and repaint (SIGURG doorbell) when a cache file's
/// content actually changed.
///
/// The single writer behind all three language-cache refresh triggers: a
/// language marker file event ([`LanguageRefreshMode::Immediate`]), the
/// git-status-driven background refresh
/// ([`LanguageRefreshMode::Throttled`], see
/// [`schedule_language_refresh_from_git_status`]), and a theme switch
/// ([`LanguageRefreshMode::CachedOnly`], see [`regen_language_caches`]).
///
/// `theme` is the caller's own already-fetched snapshot rather than a fresh
/// `ctx.theme_manager.get()` taken in here: [`regen_language_caches`] must
/// derive its `extra_prev_bg` from the exact same snapshot it renders with,
/// or a theme switch landing between two independent fetches could pair a
/// stale `prev_bg` with a fresh render (a torn read). The other two callers
/// only ever fetch the theme once regardless, so threading it through costs
/// them nothing.
///
/// `extra_prev_bg`, when `Some`, additionally seeds that one `prev_bg` render
/// context. Every call here already refreshes every *previously tracked*
/// context automatically — [`crate::cache::InstantPromptCache::write_language`]
/// widens the requested context to the full known set — so `extra_prev_bg` is
/// only for proactively introducing a context no shell has asked for yet. Only
/// a theme switch needs that, because switching themes changes what the git
/// segment's background color even is.
///
/// Returns whether anything was written.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter is an independent piece of the refresh: context, target root, config, re-detection mode, the theme snapshot to render with, and an optional extra render context; a params struct would just move the same six names one level down"
)]
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

    let mut wrote = false;
    for prev_bg in std::iter::once(None).chain(extra_prev_bg.map(Some)) {
        match ctx.instant_cache.write_language_variants(
            root,
            &detected,
            config,
            theme,
            prev_bg,
            &palette,
            stashed_venv.as_deref(),
        ) {
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

/// Resolve the detected-languages list [`refresh_language_for`] should render,
/// applying `mode`'s re-detection and throttling policy and updating the
/// detection cache when it re-detects.
///
/// `None` means "nothing to render, write nothing": the throttle window has not
/// elapsed, or there is no cached data for a
/// [`LanguageRefreshMode::CachedOnly`] caller.
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
            // Re-checked here, not only by the caller: time passes between
            // scheduling this work and running it, and another job may have
            // refreshed the same repo in between.
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

/// Run language detection for `root` under the currently configured detection
/// mode.
fn detect_languages(ctx: &AgentContext, root: &Path) -> Vec<crate::language::DetectedLanguage> {
    let detection_mode = ctx.config_manager.get().language.detection_mode;
    crate::language::Detector::detect_directory_bounded(root, detection_mode)
}

/// Outcome of writing the rendered instant-prompt cache for a repository
/// refresh (#446).
///
/// Mirrors [`InstantPromptCache::write_git`]'s three-way `Result<bool>`
/// contract, plus a fourth state for when no write was even attempted:
///
/// - `write_git` returns `Err` -> [`Failed`](Self::Failed): the write did not
///   land. `write_cache_file`'s atomic temp-file-plus-rename means the
///   on-disk file is untouched (never partially written or deleted), so it is
///   simply whatever was there before this attempt — possibly stale relative
///   to `status`, possibly still correct if nothing changed. `write_git`'s
///   "last written" memo is also NOT updated on this path.
/// - `write_git` returns `Ok(false)` -> [`Unchanged`](Self::Unchanged):
///   content on disk already matched; nothing new was written.
/// - `write_git` returns `Ok(true)` -> [`Written`](Self::Written): fresh
///   content is now on disk. Because a failed write never updates the memo
///   (see above), this is also how a **recovery** write self-identifies: the
///   same repository status written again after a prior failure naturally
///   reports `Ok(true)`, since the memo still holds the pre-failure content.
/// - No write was attempted -> [`NotAttempted`](Self::NotAttempted): the
///   repository became unavailable or the scan itself failed, so there is no
///   status to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheWriteOutcome {
    /// No cache write was attempted this refresh (repo unavailable / scan
    /// failed).
    NotAttempted,
    /// `write_git` returned `Err`; the on-disk file is unchanged from before
    /// this attempt.
    Failed,
    /// `write_git` returned `Ok(false)`; content already matched, nothing new
    /// written.
    Unchanged,
    /// `write_git` returned `Ok(true)`; fresh content is now on disk (first
    /// write for this content, or a recovery from a prior failure).
    Written,
}

/// Result of [`refresh_repo_status`] / [`try_incremental_update_with`]: the new
/// repository status (if available) alongside what happened when writing the
/// rendered instant-prompt cache for it (#446). Kept as one struct (rather
/// than a bare tuple) so the repaint decision in
/// [`refresh_and_notify_if_changed`] reads its intent at the call site.
struct RepoRefresh {
    status: Option<RepositoryStatus>,
    cache_write: CacheWriteOutcome,
}

/// Recompute a repository's git status, refresh caches, and notify clients
/// when either the status changed OR the instant-prompt cache write just
/// recovered from a prior failure.
///
/// Shared by the file-watcher event path and the periodic reconcile timer so
/// that no-op events (e.g. ignored-file churn, a touch that changes nothing
/// git cares about) never wake terminals. `paths_hint` of a single path
/// enables an incremental single-file scan; `None` forces a full scan.
///
/// ## Why a failed write suppresses the repaint (#446)
///
/// A repaint doorbell does not push fresh content to shells — it only asks the shell
/// to re-render its *next* prompt from whatever is currently on disk in the
/// instant-prompt cache file (see `fish/core/ipc.fish`'s repaint-trigger
/// handler and `fish/segments/git.fish`'s serve-stale fast path, which reads
/// the cache file directly, serving stale content plus a background refresh
/// rather than a synchronous IPC query). So when `write_status_caches`
/// reports [`CacheWriteOutcome::Failed`], forcing a repaint right now could
/// only make the shell re-display the SAME stale file it would have shown
/// anyway — pure noise, and per the shell's serve-stale contract, never
/// corrupted or empty content, just old. This is suppressed even when
/// `status_changed` is independently true: the change is not lost, because
/// `ctx.cache`'s canonical status is updated unconditionally before this
/// write is attempted (both scan paths do this), so the *next* refresh (next
/// watcher event or the periodic reconcile tick) retries the write with the
/// latest status and, once storage recovers, reports `Written` and triggers
/// the repaint then (see [`CacheWriteOutcome::Written`] doc for why a
/// recovery write reports `Ok(true)`). A status change is therefore delayed
/// until content is actually ready to serve, never silently dropped forever.
///
/// One narrow case is deliberately traded away: on a *cold* miss (no cache
/// file at all, e.g. the very first write for a repo failed), `git.fish`
/// DOES fall back to a bounded synchronous IPC query, so today's
/// unconditional repaint would have let an idle shell self-heal immediately.
/// Suppressing it defers that self-heal to the shell's next prompt render,
/// which cold-misses and runs the same IPC query anyway. That is the same
/// "idle shell converges on next render, no periodic self-repaint tick"
/// residual bound `git.fish` already documents, so it is bounded and
/// consistent rather than a new failure mode.
pub(super) fn refresh_and_notify_if_changed(
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    paths_hint: Option<&[PathBuf]>,
) {
    // Compare against the last-known status regardless of TTL: a TTL-expired entry
    // must not be mistaken for "changed" (which would defeat reconcile gating).
    let previous = ctx.cache.get_any_canonical(git_root);

    let refresh = refresh_repo_status(ctx, git_root, config, paths_hint);

    let should_notify = match refresh.cache_write {
        // A recovery (or first-ever) write just landed: the shell can now
        // render something new, so notify regardless of whether the
        // repository status itself moved (#446 acceptance criterion 3).
        CacheWriteOutcome::Written => true,
        // The write failed: never notify solely on this pass, even if the
        // status changed — see the doc comment above for why, and how the
        // change is picked up later instead of dropped.
        CacheWriteOutcome::Failed => false,
        // No fresh content was written (either nothing to write, or content
        // already matched); fall back to the plain status comparison.
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

/// Per-repo-serialized entry point for [`refresh_and_notify_if_changed`].
///
/// Production callers (the file-watcher `FileEvent::Git` arm and the reconcile
/// `scan_fn`) MUST route through this wrapper instead of calling
/// `refresh_and_notify_if_changed` directly, so two scans for the *same* repo
/// can never run concurrently (#418). The raw function stays public for the
/// existing tests that exercise it in isolation.
///
/// The first pass uses the caller's `paths_hint` (enabling the incremental
/// single-file fast path in the common case). Any follow-up pass forces a full
/// scan (`paths_hint = None`): a follow-up represents an *unknown* change that
/// arrived while the first pass was in flight, so the original hint is stale and
/// must not be reused (mirrors reconcile's #417 catch-up re-fetching fresh
/// state rather than replaying a snapshot).
pub(super) fn refresh_and_notify_coalesced(
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    paths_hint: Option<&[PathBuf]>,
) {
    run_coalesced(&ctx.dispatch_guard, git_root, |is_follow_up| {
        let hint = if is_follow_up { None } else { paths_hint };
        refresh_and_notify_if_changed(ctx, git_root, config, hint);
    });
}

/// Drive the per-repo coalescing state machine, invoking `scan` for the initial
/// pass (`scan(false)`) and each drained follow-up (`scan(true)`).
///
/// Generic over `scan` so the coordinator's serialization logic — including the
/// defensive follow-up cap — is exercised by deterministic unit tests with a
/// fake scan, without spinning up real git subprocesses.
///
/// Drain-until-quiet (not a single bounded follow-up): unlike #417's reconcile
/// timer, nothing external re-drives this call, so a change arriving during the
/// follow-up pass must be serviced here or it would be lost until the next
/// filesystem event / reconcile tick. `finish` keeps returning `true` until the
/// pending flag is genuinely drained, bounded by [`MAX_COALESCED_FOLLOW_UPS`].
fn run_coalesced<F: FnMut(bool)>(coordinator: &RepoRefreshCoordinator, repo: &Path, mut scan: F) {
    if !coordinator.try_start(repo) {
        // Another thread already owns this repo's refresh; it will pick up the
        // change we just recorded as pending. Do not run a second scan.
        return;
    }

    scan(false);

    let mut follow_ups = 0_u32;
    while coordinator.finish(repo) {
        if follow_ups >= MAX_COALESCED_FOLLOW_UPS {
            debug_log!(
                "agent",
                "Coalesced refresh hit follow-up cap ({}) for {}; deferring remaining churn to reconcile",
                MAX_COALESCED_FOLLOW_UPS,
                repo.display()
            );
            // Cap hit, but finish() reported a real pending change: service it
            // once more so the final state is captured, then defer any
            // *further* churn to the reconcile backstop. Without this the final
            // pending state is dropped until an unrelated event / the ~45s
            // reconcile (#433).
            scan(true);
            coordinator.abandon(repo);
            break;
        }
        follow_ups = follow_ups.saturating_add(1);
        scan(true);
    }
}

/// Whether a status transition is user-visible and warrants notifying clients.
///
/// Notify when the status appeared, disappeared, or differs; suppress when it is
/// unchanged (so no-op events never wake terminals). A `None` previous with a
/// `None` new (repo stayed unavailable) is not a change.
///
/// This is the repaint gate: `PartialEq` on the whole [`RepositoryStatus`]
/// struct, which is exactly the content-hash comparison guarding the git
/// segment's cache write in `cache::instant_prompt::write_git` (both compare
/// the same rendered/renderable state, just at different layers). The gate is
/// intentionally content-based, not field-count-based: two distinct real
/// states that render to IDENTICAL displayed output are correctly treated as
/// unchanged, and suppressing the repaint for them is *correct*, not a
/// staleness bug. The clearest example is ahead/behind capping (#436): raw
/// counts of 100 and 101 both display as the capped value (e.g. "99") because
/// `git::native::parser::parse_v2_ahead_behind` clamps `ahead`/`behind` to
/// `max_ahead_behind` *before* they ever reach this struct, so both states
/// carry the identical `ahead = 99, ahead_capped = true` fields here — there is
/// no separate "exact count" field for `status_changed` to miss.
///
/// The only way a real change could slip past this gate is a field the git
/// segment deliberately does not display at all (as opposed to a displayed
/// field capped to a common value) — that would be a display/config choice
/// (e.g. what `formatter::git_resolver::GitResolver` chooses to render), not a
/// defect in this comparison.
fn status_changed(previous: Option<&RepositoryStatus>, new: Option<&RepositoryStatus>) -> bool {
    match (previous, new) {
        (None, None) => false,
        (Some(old), Some(current)) => old != current,
        _ => true, // repo appeared or became unavailable
    }
}

/// Map a repo-relative changed path that lies inside a nested working tree (a
/// submodule, or a plain nested repository) to that tree's root.
///
/// The superproject only knows the gitlink or untracked directory, so a
/// pathspec below it (`sub/s.txt`) matches nothing: an edit would be missed
/// and a cached `sub` entry would never be cleared on revert (#712). The
/// outermost directory holding a `.git` entry wins (a file for submodules, a
/// directory for plain nested repos). Paths outside any nested tree are
/// returned unchanged.
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

/// Try a pathspec-limited incremental status update for `changed` under
/// `git_root`, in one git invocation covering every path.
///
/// Returns [`RepoRefresh::status`] as `Some` on success, or `None` (so the
/// caller falls back to a full scan, with `cache_write` left
/// [`CacheWriteOutcome::NotAttempted`]) when any path is outside the repo or is
/// the repo root itself (#448), the repository is unreadable, or the cache has
/// no entry with complete file details to update.
///
/// A path the scan does not report back is **clean now**, not a reason to bail:
/// with a pathspec covering N paths, "absent" is the only way git expresses "no
/// longer changed", and it is how a rename's source path stops being counted
/// (git keys a rename under its destination alone). Treating absence as a
/// fallback signal instead would make a coalesced burst full-scan whenever one
/// of its files went clean. Paths git status cannot report on at all — `.git`
/// internals — never reach here: [`git_paths_hint`] escalates them to a full
/// scan precisely so this rule stays sound (#466).
///
/// Generic over the [`GitBackend`] used for the capture round so tests can
/// exercise this orchestration with an instrumented backend instead of a
/// real `git` subprocess (#470); [`refresh_repo_status_with`] calls this
/// directly (rather than through a non-generic wrapper of its own) so that
/// the same `backend` instance also covers its own full-scan call, letting a
/// single instrumented backend count capture rounds across both paths. There
/// is no separate non-generic `try_incremental_update`: unlike
/// `crate::git::status::load_repository_state`/[`load_repository_state_with`]
/// in `git::status`, this function has exactly one call site in production
/// code, so a thin wrapper here would have no caller and fail the dead-code
/// gate — [`refresh_repo_status`] is the production entry point instead.
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
        // Git's index may spell a non-ASCII name in another Unicode
        // normalization (NFC vs NFD on macOS), so a pathspec built from the
        // watcher's spelling can match nothing. A full scan is always
        // correct (#713).
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

    match load_repository_state_with(
        backend,
        git_root,
        Some(&relative_paths),
        config.git.max_ahead_behind,
        config.git.stash_enabled,
        Duration::from_secs(config.git.timeout_seconds.get()),
    ) {
        Ok(Some(CompleteStatus {
            files: file_map, ..
        })) => {
            let Some(updated_status) = ctx.cache.update_paths_canonical(
                git_root,
                &relative_paths,
                &file_map,
                NativeGitBackend::ignores_case(git_root),
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
        Ok(_) => not_attempted,
        Err(e) => {
            debug_log!("agent", "Incremental check failed: {}", e);
            not_attempted
        }
    }
}

/// Refresh a repository's git status and caches, returning the new status (or
/// `None` when the repository is unavailable) alongside the instant-prompt
/// cache write outcome (#446). Attempts a pathspec-limited incremental update
/// when `paths_hint` names paths under the repo, else a full scan.
fn refresh_repo_status(
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    paths_hint: Option<&[PathBuf]>,
) -> RepoRefresh {
    refresh_repo_status_with(&NativeGitBackend, ctx, git_root, config, paths_hint)
}

/// Generic form of [`refresh_repo_status`], parameterized over the
/// [`GitBackend`] used for both the incremental and full-scan capture
/// rounds, so tests can pin how many capture rounds a given event costs
/// (#470). The same `backend` instance is threaded into
/// [`try_incremental_update_with`] below, so an instrumented backend counts
/// invocations from *both* call sites — not just the full scan here — which
/// is what makes such a test able to catch a regression where an escalation
/// check silently regresses and the incremental attempt burns a capture
/// round before falling back to a full scan.
///
/// Production callers should use [`refresh_repo_status`], which delegates
/// here with [`NativeGitBackend`].
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

    // Full scan. Invalidate first so the result reflects the mutation that
    // triggered this refresh.
    ctx.cache.invalidate_canonical(git_root);
    match load_repository_state_with(
        backend,
        git_root,
        None,
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

/// Write the instant-prompt cache and schedule a language refresh for a
/// status, returning what happened to the cache write (#446).
fn write_status_caches(
    ctx: &AgentContext,
    git_root: &Path,
    config: &Config,
    status: &RepositoryStatus,
) -> CacheWriteOutcome {
    let theme = ctx.theme_manager.get();
    let palette = ctx.palette_cache.get();
    // Watcher-triggered write: no IPC request context, so prev_bg is None (graceful fallback).
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
    if !config.agent.live_updates || !config.language.enabled {
        return;
    }

    let signature = git_status_signature(status);

    // Pre-check: decides whether spawning a thread is worth it at all.
    // `refresh_language_for` re-checks the same window once the job actually
    // runs (the TOCTOU guard), against the same `LANGUAGE_REFRESH_INTERVAL`.
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

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(job);
    } else {
        std::thread::spawn(job);
    }
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

/// Create the filesystem watcher used for live updates.
///
/// # Errors
///
/// Returns an error if watcher initialization fails.
pub(super) fn create_watcher(
    watcher_config: WatcherConfig,
    ctx: &AgentContext,
) -> Result<Arc<MultiRepoWatcher>> {
    let ctx_for_callback = ctx.clone();

    MultiRepoWatcher::builder()
        .registry(Arc::clone(&ctx.watch_registry))
        .config(watcher_config)
        .callback(Box::new(move |event: DebouncedEvent| {
            // #480: gate `FileEvent::Git` on "would a refresh reach anybody",
            // via `ClientDirectory::has_subscriber` -- the same targeting
            // rule `notify_repaint*` itself applies, so this can never
            // silently drift from what a refresh's own notification would
            // deliver. A repo with no registered (or ancestor-registered)
            // client would spend a full git capture round producing a cache
            // write nobody reads and a doorbell with no recipient.
            //
            // This lives here -- the watcher callback, the single production
            // entry point from the watcher into the refresh pipeline -- and
            // deliberately NOT inside `handle_file_event`: that function has
            // ~10 existing unit tests that call it directly against an empty
            // registry and expect a refresh to run regardless (they are
            // testing scan/cache logic, not subscription), and gating inside
            // it would force a rewrite of every one of those fixtures for no
            // behavioral gain, since this callback is the only production
            // caller.
            //
            // Only `FileEvent::Git` is gated. `Config`/`Theme` events are
            // agent-global (theme cache invalidation, config reload) rather
            // than per-repo, so a subscriber check on them would be wrong,
            // not just unnecessary. `Language` events are arguably also
            // wasteful for an unsubscribed repo, but that is a distinct,
            // smaller cost and is intentionally out of scope here (#480).
            //
            // This does NOT make #468's `is_unregistered_common_gitdir_wake`
            // guard in `watcher/filesystem.rs` redundant: that guard
            // suppresses *emission* at the watcher layer (and the debouncer
            // work that would otherwise follow); this gate suppresses
            // *refresh* at the agent layer, one step further downstream.
            // Concretely, the #467/#468 wake-count tests in
            // `multi_repo_integration_tests.rs` build their own watcher via
            // `recording_watcher()`, with a callback that only records
            // `event.repo` -- they never reach this closure or
            // `handle_file_event` at all, so removing the watcher-layer guard
            // would fail those tests even though this gate remains in place.
            if matches!(event.event, FileEvent::Git { .. })
                && !ctx_for_callback.registry.has_subscriber(&event.repo)
            {
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

/// Switch the active theme when the config selects a different one.
///
/// # Errors
///
/// Returns an error when the requested theme cannot be loaded.
fn maybe_switch_theme(
    theme_manager: &Arc<ThemeManager>,
    old_config: &Config,
    new_config: &Config,
) -> Result<()> {
    if old_config.ui.theme == new_config.ui.theme {
        return Ok(());
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
    Ok(())
}

/// Whether a config reload changes anything that affects the RENDERED
/// prompt, and so should nudge already-open shells to redraw immediately
/// (via the reload doorbell) rather than waiting for the next command.
/// `ConfigManager::get()` always serves the freshest config regardless, so a
/// field missing from this classification only delays a redraw -- it never
/// causes incorrect data to be shown.
///
/// Each nested settings struct is destructured with NO `..` rest pattern in
/// its own `*_refresh_required` helper below. That makes every field
/// classification explicit and, more importantly, makes the crate FAIL TO
/// COMPILE the moment `Config` (or any of its nested settings structs) gains
/// a field that hasn't been assigned here to either a comparison or an
/// explicit `_`-prefixed "excluded" binding (#608's third acceptance
/// criterion: "a test fails if a new `Config` field is not classified for
/// refresh" -- a compile error is strictly stronger than a runtime test,
/// since it can't be forgotten to run).
fn config_refresh_required(old_config: &Config, new_config: &Config) -> bool {
    let Config {
        agent: old_agent,
        git: old_git,
        language: old_language,
        ui: old_ui,
    } = old_config;
    let Config {
        agent: new_agent,
        git: new_git,
        language: new_language,
        ui: new_ui,
    } = new_config;

    ui_refresh_required(old_ui, new_ui)
        || git_refresh_required(old_git, new_git)
        || language_refresh_required(old_language, new_language)
        || agent_refresh_required(old_agent, new_agent)
}

/// Every `UiSettings` field affects what's rendered -- all 5 are compared.
fn ui_refresh_required(old: &crate::config::UiSettings, new: &crate::config::UiSettings) -> bool {
    let crate::config::UiSettings {
        show_icons: old_show_icons,
        theme: old_theme,
        palette: old_palette,
        directory: old_directory,
        enabled_segments: old_enabled_segments,
    } = old;
    let crate::config::UiSettings {
        show_icons: new_show_icons,
        theme: new_theme,
        palette: new_palette,
        directory: new_directory,
        enabled_segments: new_enabled_segments,
    } = new;

    old_theme != new_theme
        || old_palette != new_palette
        || old_show_icons != new_show_icons
        || old_directory != new_directory
        || old_enabled_segments != new_enabled_segments
}

/// `timeout_seconds`, `skip_paths`, `watch_worktree` affect subprocess/watcher
/// behavior, not the rendered segment -- excluded. Everything else affects
/// what's drawn (branch length, ahead/behind cap, icons, stash indicator) --
/// newly included; this was previously entirely unclassified (only `enabled`/
/// `show_upstream` were checked).
fn git_refresh_required(
    old: &crate::config::GitSettings,
    new: &crate::config::GitSettings,
) -> bool {
    let crate::config::GitSettings {
        enabled: old_enabled,
        show_upstream: old_show_upstream,
        timeout_seconds: _old_timeout_seconds,
        skip_paths: _old_skip_paths,
        max_branch_length: old_max_branch_length,
        max_ahead_behind: old_max_ahead_behind,
        watch_worktree: _old_watch_worktree,
        icons: old_icons,
        icon_set: old_icon_set,
        stash_enabled: old_stash_enabled,
    } = old;
    let crate::config::GitSettings {
        enabled: new_enabled,
        show_upstream: new_show_upstream,
        timeout_seconds: _new_timeout_seconds,
        skip_paths: _new_skip_paths,
        max_branch_length: new_max_branch_length,
        max_ahead_behind: new_max_ahead_behind,
        watch_worktree: _new_watch_worktree,
        icons: new_icons,
        icon_set: new_icon_set,
        stash_enabled: new_stash_enabled,
    } = new;

    old_enabled != new_enabled
        || old_show_upstream != new_show_upstream
        || old_max_branch_length != new_max_branch_length
        || old_max_ahead_behind != new_max_ahead_behind
        || old_icons != new_icons
        || old_icon_set != new_icon_set
        || old_stash_enabled != new_stash_enabled
}

/// `cache_ttl_hours`, `detection_mode`, `confidence_threshold` affect what
/// gets (re-)detected on the NEXT detection pass, not how the current
/// detected-language list renders -- excluded. `filter` (which of the
/// already-detected languages to show) is a pure render-time decision over
/// the existing detected set -- newly included.
fn language_refresh_required(
    old: &crate::config::LanguageSettings,
    new: &crate::config::LanguageSettings,
) -> bool {
    let crate::config::LanguageSettings {
        enabled: old_enabled,
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
        enabled: new_enabled,
        show_versions: new_show_versions,
        cache_ttl_hours: _new_cache_ttl_hours,
        enabled_languages: new_enabled_languages,
        display: new_display,
        filter: new_filter,
        detection_mode: _new_detection_mode,
        confidence_threshold: _new_confidence_threshold,
        icons: new_icons,
    } = new;

    old_enabled != new_enabled
        || old_show_versions != new_show_versions
        || old_enabled_languages != new_enabled_languages
        || old_display != new_display
        || old_filter != new_filter
        || old_icons != new_icons
}

/// `timeout_seconds` (IPC socket timeout) and `supervisor` (auto-restart
/// policy) are process-management, not rendering -- excluded, matching
/// current (pre-#608) behavior exactly.
fn agent_refresh_required(
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

fn sync_watcher_state(
    watcher_slot: &SharedWatcherSlot,
    ctx: &AgentContext,
    watcher_config: WatcherConfig,
    new_config: &Config,
    refresh_required: bool,
) {
    let mut watcher_guard = match watcher_slot.lock() {
        Ok(guard) => guard,
        Err(err) => {
            debug_log!("config", "config watcher lock poisoned: {}", err);
            return;
        }
    };

    if !new_config.agent.live_updates || !(new_config.git.enabled || new_config.language.enabled) {
        if let Some(watcher) = watcher_guard.take() {
            watcher.stop();
        }
        drop(watcher_guard);
        if refresh_required {
            ctx.registry.notify_reload();
        }
        return;
    }

    if watcher_guard.is_none() {
        // `create_watcher` takes the config manager straight off `ctx`, which
        // holds a strong `Arc` for as long as this call is on the stack.
        match create_watcher(watcher_config, ctx) {
            Ok(new_watcher) => {
                // Re-populate watcher with existing client repositories
                let repos = ctx.registry.get_registered_repos();
                for (pid, path) in repos {
                    let _ = new_watcher.register_client(pid, &path);
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
        // The watcher already exists, so the repopulation above never runs and
        // active repositories keep the watches they were armed with. Bring them
        // in line with the (already-applied) `git.watch_worktree` value, so a
        // shell that never leaves its directory still gets live worktree updates
        // — or stops paying for a recursive watch it no longer needs (#444).
        // A no-op when nothing changed. Safe to call while holding the watcher
        // slot: `MultiRepoWatcher`'s internal locks are only ever taken *after*
        // this slot (same order as `register_client` above), never before.
        watcher.resync_watch_worktree();
    }
    drop(watcher_guard);

    if refresh_required {
        ctx.registry.notify_reload();
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "each parameter is an independent piece of a config-reload's side-effect state (context, watcher slot/config, disable flag, old/new config); a params struct would just move the same six names one level down"
)]
/// Apply non-filesystem runtime side effects for a validated config reload.
///
/// # Errors
///
/// Returns an error when a dependent runtime subsystem rejects the new
/// configuration, such as when a requested theme cannot be loaded.
pub(super) fn handle_config_reload(
    ctx: &AgentContext,
    watcher_slot: &SharedWatcherSlot,
    watcher_config: WatcherConfig,
    disable_watcher: bool,
    old_config: &Config,
    new_config: &Config,
) -> Result<()> {
    maybe_switch_theme(&ctx.theme_manager, old_config, new_config)?;

    // Resolve on every reload: an in-place edit of the active palette file
    // keeps `ui.palette` unchanged, so only contents can reveal it (#772).
    // Done before the export cache and instant-cache regeneration below so
    // both render with the new colors.
    let palette_changed = ctx
        .palette_cache
        .replace(crate::palette::active_palette(new_config));

    // Write theme export cache before notifying shells (ordering guarantee: cache
    // must be on disk before the reload doorbell so shells can source it immediately).
    if let Err(e) = crate::cache::write_theme_export_cache(&ctx.theme_manager, new_config) {
        debug_log!("agent", "Failed to write theme export cache: {}", e);
    }

    crate::language::version::set_version_cache_ttl(new_config.language.cache_ttl_hours.get());
    // Applied to the shared registry before `sync_watcher_state` below, which
    // either builds a new watcher against this same registry or calls
    // `resync_watch_worktree` on the existing one (#444).
    ctx.watch_registry
        .set_worktree_enabled(new_config.git.watch_worktree);

    let refresh_required = palette_changed || config_refresh_required(old_config, new_config);

    // Proactively regenerate instant-prompt caches with the new theme *before* any
    // reload doorbell so the repaint hits warm caches and recolors atomically — no
    // disappear/reappear flicker for the async-rendered git/language segments
    // (#223). Only re-renders already-cached data; never adds synchronous
    // detection to the reload path. Gated on `refresh_required` because that is
    // exactly when the shells will re-source the theme and repaint.
    if refresh_required {
        regenerate_instant_caches_for_theme_change(ctx, new_config);
    }

    if disable_watcher {
        if refresh_required {
            ctx.registry.notify_reload();
        }
        return Ok(());
    }

    sync_watcher_state(
        watcher_slot,
        ctx,
        watcher_config,
        new_config,
        refresh_required,
    );
    Ok(())
}

/// Re-render the instant-prompt caches for every registered repository using the
/// **already-cached** git/language data and the now-active theme, so the repaint
/// that follows a theme/config change recolors without the disappear/reappear
/// flicker (#223).
///
/// The slow part of git/language is gathering data (git subprocess, version
/// detection); that data is unchanged by a theme switch, so re-applying the new
/// theme template to it is cheap. Repositories with no warm data are skipped and
/// keep today's graceful cold-miss behavior (#145/#160) — this only improves the
/// warm case, which is the common theme-switch scenario.
///
/// The shell sends the *previous* segment's background as `prev_bg`, baked into
/// the cached ANSI's opening chevron and into the cache-file token. For the
/// standard layout (directory → git → language) those wire values come straight
/// from the theme, identical to the `__color_directory_bg` / `__color_git_clean_bg`
/// vars `theme export` emits. We render each segment with `prev_bg = None` (which
/// refreshes the context-free fallback plus every previously seen context) and
/// again with the new theme's computed `prev_bg`, so the shell's post-reload
/// request resolves a warm, correctly-keyed, new-theme cache file instead of
/// cold-missing on a token it has never used before.
///
/// Git renders by formatting the cached status (no subprocess), so it is
/// regenerated synchronously here. Language rendering performs version detection
/// (subprocesses), which MUST NOT run on this event-loop path — a single hung or
/// cold detector would block the reload handler, wedging live updates and
/// `gpy-agent stop` (#223 guarantee: no synchronous detection on the render path).
/// Language is therefore deferred to a background job that repaints via the doorbell.
pub(super) fn regenerate_instant_caches_for_theme_change(ctx: &AgentContext, config: &Config) {
    let theme = ctx.theme_manager.get();
    let palette = ctx.palette_cache.get();

    // prev_bg the shell will send for git under the new theme, for the standard
    // layout. Mirrors the resolution in `ThemeManager::export`.
    let git_prev_bg = theme.segments.directory.bg_color.as_str();

    // Multiple clients can share one repository (or sit in different subdirs of
    // it); dedup on the resolved key so we re-render each repo once.
    let mut seen_keys = std::collections::HashSet::new();
    for (_pid, cwd) in ctx.registry.get_registered_repos() {
        let git_root = MultiRepoWatcher::find_git_root(&cwd);
        // Language caches cover non-Git project dirs too, keyed by the request
        // path when there is no Git root (matches the watcher/handler paths).
        let lang_root = git_root.clone().unwrap_or_else(|| cwd.clone());

        if !seen_keys.insert(lang_root.clone()) {
            continue;
        }

        if config.git.enabled
            && let Some(root) = git_root.as_ref()
            && let Some(status) = ctx.cache.get(root)
        {
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

        // Only schedule the (subprocess-bearing) language re-render when the repo
        // already has warm language data; cold paths keep the graceful cold-miss.
        if config.language.enabled && ctx.language_cache.get(&lang_root).is_some() {
            schedule_language_cache_regen(ctx, &lang_root, config);
        }
    }
}

/// Re-render a repository's language instant caches **off** the event loop and
/// repaint on change, so theme-switch language version detection never blocks the
/// reload handler. Mirrors [`schedule_language_refresh_from_git_status`].
fn schedule_language_cache_regen(ctx: &AgentContext, lang_root: &Path, config: &Config) {
    let regen_ctx = ctx.clone();
    let root = lang_root.to_path_buf();
    let regen_config = config.clone();

    let job = move || {
        regen_language_caches(&regen_ctx, &root, &regen_config);
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(job);
    } else {
        std::thread::spawn(job);
    }
}

/// Synchronous body of the deferred language regeneration: re-render the cached
/// language data with the active theme into the context-free fallback plus the
/// new theme's `prev_bg` token, then ring the repaint doorbell when the output changed.
///
/// Returns `true` when a cache file changed (a repaint was warranted). Separated
/// from [`schedule_language_cache_regen`] so it can be unit-tested without a thread.
fn regen_language_caches(ctx: &AgentContext, lang_root: &Path, config: &Config) -> bool {
    // Fetched exactly once and threaded through to `refresh_language_for`:
    // `lang_prev_bg` (the theme's own git background — after a switch this
    // color has almost certainly never been requested by a live shell, so
    // seeding it here is what makes the first post-switch render a cache hit
    // rather than a cold miss) and the render itself must come from the SAME
    // theme snapshot. Fetching twice — once here, once inside
    // `refresh_language_for` — could observe two different themes if a
    // switch lands in between, pairing a stale `prev_bg` with a fresh
    // render; the analogous git-side path in
    // `regenerate_instant_caches_for_theme_change` takes exactly one
    // snapshot for the same reason.
    let theme = ctx.theme_manager.get();
    let lang_prev_bg = theme
        .segments
        .git
        .clean_bg_color
        .as_deref()
        .unwrap_or(theme.segments.git.bg_color.as_str());

    refresh_language_for(
        ctx,
        lang_root,
        config,
        LanguageRefreshMode::CachedOnly,
        &theme,
        Some(lang_prev_bg),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use crate::config::Config;
    use crate::git::{FileStatus, RepositoryState, StatusAggregate};
    use crate::ipc::ClientDirectory;
    use crate::theme::ThemeManager;
    use crate::watcher::{DebouncedEvent, FileEvent, WatcherConfig, multi_repo::MultiRepoWatcher};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::thread;
    use tempfile::tempdir;

    fn make_theme_manager() -> Arc<ThemeManager> {
        Arc::new(ThemeManager::new("default").expect("theme manager"))
    }

    fn make_client_registry() -> Arc<ClientDirectory> {
        ClientDirectory::new().shared()
    }

    fn make_git_cache() -> Arc<GitStatusCache> {
        Arc::new(GitStatusCache::new())
    }

    fn make_config_manager() -> Arc<crate::config::manager::ConfigManager> {
        Arc::new(crate::config::manager::ConfigManager::with_defaults().expect("config"))
    }

    fn make_instant_cache() -> Arc<InstantPromptCache> {
        Arc::new(InstantPromptCache::new_for_test())
    }

    fn make_agent_context() -> AgentContext {
        let config_manager = make_config_manager();
        let initial_config = config_manager.get();
        AgentContext {
            registry: make_client_registry(),
            cache: make_git_cache(),
            config_manager,
            theme_manager: make_theme_manager(),
            instant_cache: make_instant_cache(),
            language_cache: crate::language::DetectionCache::new(),
            palette_cache: Arc::new(crate::palette::PaletteCache::from_config(&initial_config)),
            dispatch_guard: Arc::new(RepoRefreshCoordinator::default()),
            watch_registry: Arc::new(crate::watcher::WatchRegistry::new()),
        }
    }

    /// # Panics
    ///
    /// Panics if creating or initializing the temporary repository fails.
    fn create_temp_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("create temp dir");
        std::process::Command::new("git")
            .arg("init")
            .current_dir(dir.path())
            .output()
            .expect("git init");
        let canonical = fs::canonicalize(dir.path()).expect("canonical repo");
        (dir, canonical)
    }

    /// Construct a `GitPaths::WholeRepo` event directly, exercising the same
    /// full-scan code path a MAX_PATHS-accumulation event would take, without
    /// needing to actually accumulate that many paths in a test. A real
    /// rescan produces this exact shape too (#616, `PendingEvent::git_whole_repo`);
    /// `GitPaths::single(root)` plus `git_paths_hint`'s repo-root escalation
    /// remains reachable for other root-path events (see
    /// `rescan_event_at_repo_root_refreshes_cache` for that path).
    fn whole_repo_event(repo: &Path) -> DebouncedEvent {
        DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::WholeRepo,
            },
            repo: repo.to_path_buf(),
        }
    }

    /// # Panics
    ///
    /// Panics if staging or committing the temporary repository fails.
    fn commit_all(repo: &Path) {
        for args in [
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["add", "-A"],
            vec!["commit", "-m", "init"],
        ] {
            // A developer's global `commit.gpgsign = true` makes every test
            // commit call gpg, which times out intermittently under a loaded
            // parallel test run.
            let output = std::process::Command::new("git")
                .args(["-c", "commit.gpgsign=false"])
                .args(&args)
                .current_dir(repo)
                .output()
                .expect("git command");
            assert!(output.status.success(), "git {args:?} failed");
        }
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo or writing files fails.
    fn git_event_refreshes_cache_immediately() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        let head_path = repo.join(".git/HEAD");
        let event = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(head_path),
            },
            repo: repo.clone(),
        };

        handle_file_event(&ctx, &event, &config);
        assert!(ctx.cache.get(&repo).is_some());

        // Introduce an untracked file which should be reflected on the very next event.
        let untracked_path = repo.join("new-file.txt");
        fs::write(&untracked_path, "hello world").expect("create untracked file");

        handle_file_event(&ctx, &event, &config);

        let updated_status = ctx
            .cache
            .get(&repo)
            .expect("updated status should be cached after event");
        assert!(
            updated_status.untracked >= 1,
            "expected untracked file to be reflected immediately"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the cache handling of repository errors regresses.
    fn git_event_clears_cache_when_repository_unavailable() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        let head_path = repo.join(".git/HEAD");
        let event = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(head_path),
            },
            repo: repo.clone(),
        };

        handle_file_event(&ctx, &event, &config);
        assert!(ctx.cache.get(&repo).is_some());

        let git_dir = repo.join(".git");
        let backup = repo.join(".git.bak");
        fs::rename(&git_dir, &backup).expect("rename git dir");

        handle_file_event(&ctx, &event, &config);

        assert!(
            ctx.cache.get(&repo).is_none(),
            "cache should be cleared when repository cannot be read"
        );
        let _ = fs::rename(&backup, &git_dir);
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo or writing files fails.
    ///
    /// Regression test for #415: `.git/packed-refs` must reach the same
    /// refresh path as `.git/HEAD` (a full, non-incremental status scan),
    /// since `git gc`/`git pack-refs` can move any ref into that single file.
    fn packed_refs_event_refreshes_cache_immediately() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        let packed_refs_path = repo.join(".git/packed-refs");
        let event = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(packed_refs_path),
            },
            repo: repo.clone(),
        };

        handle_file_event(&ctx, &event, &config);
        assert!(
            ctx.cache.get(&repo).is_some(),
            "packed-refs event should refresh the git status cache"
        );
    }

    #[test]
    /// Regression test for #448: a `Git` event carrying the repository root
    /// itself as its path (defence-in-depth for a root-path event; a real
    /// rescan emits `GitPaths::WholeRepo` directly, #616) is a whole-repo
    /// signal, not a single-file hint — hinting it would strip down to the
    /// empty pathspec git rejects.
    fn repo_root_path_yields_no_incremental_hint() {
        let repo = PathBuf::from("/tmp/example-repo");
        assert!(
            git_paths_hint(&GitPaths::single(repo.clone()), &repo).is_none(),
            "an event whose path IS the repo root must force a full scan"
        );
    }

    #[test]
    fn global_git_files_yield_no_incremental_hint() {
        let repo = PathBuf::from("/tmp/example-repo");
        for global in [
            ".gitignore",
            ".git/config",
            ".git/info/exclude",
            ".git/HEAD",
            ".git/packed-refs",
        ] {
            let path = repo.join(global);
            assert!(
                git_paths_hint(&GitPaths::single(path), &repo).is_none(),
                "{global} affects other files, so it must force a full scan"
            );
        }
    }

    #[test]
    /// #611: a worktree path whose final component is bare-named `config`,
    /// `HEAD`, `exclude` or `packed-refs` is an ordinary file, not its `.git`
    /// namesake, and must still get an incremental hint.
    ///
    /// `Path::ends_with` matches whole components, so the pre-#611 filename
    /// test — run against the raw event path, before repo-relative membership
    /// was even computed — fired for any of these anywhere in the watched
    /// tree. This repository's own `gpy-agent/src/config/` is a live example:
    /// a directory-level notification for it forced a whole-repo scan.
    ///
    /// Note the issue's literal wording (`worktree/config.toml`) would pass
    /// without the fix: `config.toml` and `config` are different components,
    /// so `ends_with("config")` was already false for it. The bare names below
    /// are what actually reproduce the bug.
    fn bare_named_worktree_files_still_hint_incrementally() {
        let repo = PathBuf::from("/tmp/example-repo");
        for worktree_relative in [
            "config",
            "src/config",
            "HEAD",
            "docs/HEAD",
            "exclude",
            "packed-refs",
        ] {
            let path = repo.join(worktree_relative);
            assert_eq!(
                git_paths_hint(&GitPaths::single(path.clone()), &repo),
                Some(vec![path]),
                "{worktree_relative} is a worktree file, not a .git internal: \
                 it must not force a whole-repo scan"
            );
        }
    }

    #[test]
    /// #611 regression guard in the other direction: narrowing the global-file
    /// test to `.git` membership must not lose the real `.git` files, nor
    /// `.gitignore`, which is a worktree file and so is still matched by name.
    fn real_git_global_files_still_escalate_after_the_bare_name_fix() {
        let repo = PathBuf::from("/tmp/example-repo");
        for global in [
            ".git/config",
            ".git/info/exclude",
            ".git/HEAD",
            ".git/packed-refs",
            ".gitignore",
            "src/.gitignore",
        ] {
            let path = repo.join(global);
            assert!(
                git_paths_hint(&GitPaths::single(path), &repo).is_none(),
                "{global} affects other files, so it must still force a full scan"
            );
        }
    }

    #[test]
    fn tracked_file_yields_single_path_hint() {
        let repo = PathBuf::from("/tmp/example-repo");
        let path = repo.join("src/main.rs");
        assert_eq!(
            git_paths_hint(&GitPaths::single(path.clone()), &repo),
            Some(vec![path]),
            "an ordinary file change should hint an incremental single-file scan"
        );
    }

    #[test]
    /// Regression test for #448: `try_incremental_update` must reject the
    /// repo root before shelling out, since the empty relative path becomes an
    /// empty pathspec that git rejects outright.
    fn incremental_relative_path_rejects_repo_root_and_outsiders() {
        let repo = PathBuf::from("/tmp/example-repo");
        assert_eq!(
            incremental_relative_path(&repo, &repo),
            None,
            "the repo root has no relative path to scan incrementally"
        );
        assert_eq!(
            incremental_relative_path(Path::new("/tmp/other/file.rs"), &repo),
            None,
            "a path outside the repo has no relative path to scan incrementally"
        );
        let inside = repo.join("src/main.rs");
        assert_eq!(
            incremental_relative_path(&inside, &repo),
            Some(Path::new("src/main.rs")),
            "a path inside the repo scans relative to the repo root"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo or writing files fails.
    ///
    /// Regression test for #448: any `Git` event whose path is the repo root
    /// (a real rescan now emits `GitPaths::WholeRepo` directly instead, see
    /// #616; this covers the defence-in-depth root-path handling that stays
    /// in place for any other event shaped this way) must still refresh the
    /// cache, via the full scan rather than a doomed incremental attempt.
    fn rescan_event_at_repo_root_refreshes_cache() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        fs::write(repo.join("new-file.txt"), "hello world").expect("create untracked file");

        let event = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(repo.clone()),
            },
            repo: repo.clone(),
        };
        handle_file_event(&ctx, &event, &config);

        let status = ctx
            .cache
            .get(&repo)
            .expect("rescan event should refresh the git status cache");
        assert!(
            status.untracked >= 1,
            "rescan should full-scan the repo and see the untracked file"
        );
    }

    #[test]
    /// #466: every path coalesced into one event has to reach the scan, or the
    /// published status is short by the files that were dropped.
    fn several_ordinary_files_yield_a_multi_path_hint() {
        let repo = PathBuf::from("/tmp/example-repo");
        let first = repo.join("a.txt");
        let second = repo.join("src/b.rs");

        assert_eq!(
            git_paths_hint(&GitPaths::Paths(vec![first.clone(), second.clone()]), &repo),
            Some(vec![first, second]),
            "a coalesced burst of ordinary files should scan every one of them"
        );
    }

    #[test]
    /// #466: escalation is per-event, not per-path. One global file in the
    /// burst changes the status of files the pathspec would never name, so the
    /// whole event degrades to a full scan.
    fn one_global_path_escalates_the_whole_burst() {
        let repo = PathBuf::from("/tmp/example-repo");
        let paths = GitPaths::Paths(vec![repo.join("a.txt"), repo.join(".git/HEAD")]);

        assert!(
            git_paths_hint(&paths, &repo).is_none(),
            "a HEAD write alongside worktree edits must still refresh branch state"
        );
    }

    #[test]
    /// #466/#470: `git status` never reports on `.git` internals, so a
    /// pathspec-limited scan for one comes back empty — which the multi-path
    /// update would otherwise read as "these files are clean now". They are
    /// also repository-wide signals (a ref write moves ahead/behind), so the
    /// only sound answer is a full scan.
    ///
    /// Covers #470 acceptance criterion 2's full enumerated set. Every entry
    /// here now escalates through the same `.git`-membership branch: #611
    /// removed the filename tests that used to catch the `*HEAD` names one
    /// step earlier, because matching on the filename alone also caught
    /// worktree files that merely shared the name (see
    /// `bare_named_worktree_files_still_hint_incrementally`).
    fn git_internal_paths_escalate_to_a_full_scan() {
        let repo = PathBuf::from("/tmp/example-repo");
        for internal in [
            ".git/index",
            ".git/refs/heads/main",
            ".git/refs/remotes/origin/main",
            ".git/refs/stash",
            ".git/ORIG_HEAD",
            ".git/MERGE_HEAD",
            ".git/REBASE_HEAD",
            ".git/CHERRY_PICK_HEAD",
            ".git/REVERT_HEAD",
            ".git/BISECT_LOG",
            ".git/FETCH_HEAD",
        ] {
            let paths = GitPaths::single(repo.join(internal));
            assert!(
                git_paths_hint(&paths, &repo).is_none(),
                "{internal} is invisible to git status and must force a full scan"
            );
        }
    }

    #[test]
    /// #611: a NESTED repository's own git-internal file, reached via a path
    /// that is relative to the *parent* repo, must still escalate even though
    /// `.git` is not the FIRST component of that relative path (e.g. because
    /// the watcher momentarily attributed the event to the parent while the
    /// nested repo's own `.git` directory was briefly absent, mid-move or
    /// mid-delete). `git status` cannot report on this path any more than it
    /// can a top-level `.git` internal, so it must not be scoped down to a
    /// path-limited scan the same way (#466's failure mode, one level deeper).
    fn nested_git_internal_paths_still_escalate_to_a_full_scan() {
        let repo = PathBuf::from("/tmp/example-repo");
        for nested in [
            "nested-repo/.git/HEAD",
            "vendor/nested-repo/.git/index",
            "a/b/.git/config",
        ] {
            let paths = GitPaths::single(repo.join(nested));
            assert!(
                git_paths_hint(&paths, &repo).is_none(),
                "{nested} has `.git` as a non-first component and must still force a full scan"
            );
        }
    }

    #[test]
    /// Regression test for #470 acceptance criterion 5: a submodule's or
    /// linked worktree's git directory lives outside the *attributed*
    /// working root entirely — `<super>/.git/modules/<name>/index` is not a
    /// descendant of the submodule's own working tree, and
    /// `<main>/.git/worktrees/<name>/index` is not a descendant of the
    /// linked worktree's own working tree.
    ///
    /// `git_paths_hint` must still force a full scan of the attributed
    /// working root for these paths, but it does so via the EARLIER
    /// `incremental_relative_path` call: `strip_prefix` fails outright
    /// because the path is not under `git_root` at all, so control returns
    /// `None` via the `?` operator before ever reaching the `.git`-component
    /// membership check below it. Making that check component-based (rather
    /// than first-component-only, #611) didn't change which route this test
    /// takes — the path is still rejected before the membership check ever
    /// sees it — but this comment exists so that distinction isn't lost.
    fn external_git_dir_paths_yield_no_incremental_hint() {
        // Submodule: `<super>/.git/modules/<name>/…` is watched but
        // attributed to the submodule's own working root, which is a
        // sibling path, not a descendant of the superproject's `.git`.
        let submodule_root = PathBuf::from("/tmp/superproject/sub");
        let submodule_git_dir_index = PathBuf::from("/tmp/superproject/.git/modules/sub/index");
        assert!(
            git_paths_hint(&GitPaths::single(submodule_git_dir_index), &submodule_root).is_none(),
            "a submodule's git-dir path must force a full scan of the submodule's working root"
        );

        // Linked worktree: `<main>/.git/worktrees/<name>/…` is watched but
        // attributed to the linked worktree's own working root, which lives
        // wherever `git worktree add` put it — not under `<main>/.git`.
        let worktree_root = PathBuf::from("/tmp/other-worktree");
        let worktree_git_dir_index = PathBuf::from("/tmp/main-repo/.git/worktrees/feature/index");
        assert!(
            git_paths_hint(&GitPaths::single(worktree_git_dir_index), &worktree_root).is_none(),
            "a linked worktree's git-dir path must force a full scan of the worktree's working root"
        );
    }

    #[test]
    /// #466: the debouncer degrades past its path bound; that has to arrive as
    /// a full scan rather than an empty pathspec.
    fn whole_repo_paths_yield_no_incremental_hint() {
        let repo = PathBuf::from("/tmp/example-repo");
        assert!(git_paths_hint(&GitPaths::WholeRepo, &repo).is_none());
    }

    /// A [`GitBackend`] that counts invocations and records the `paths`
    /// argument of each call, for pinning #470 acceptance criterion 6 (the
    /// number of complete git capture rounds a given event costs).
    ///
    /// Delegates to [`NativeGitBackend`] for the actual answer rather than
    /// returning a canned payload: the whole point of these tests is
    /// asserting on real routing behavior (incremental vs. full scan) against
    /// a real temporary repository, and a canned response would let the test
    /// pass without the cache-update / capture-round logic under test ever
    /// running for real. The subprocess cost is the same one every other
    /// `create_temp_repo`-based test in this module already pays.
    struct CountingGitBackend {
        /// One entry per `complete_status` call, in call order: the `paths`
        /// argument that call received (`None` = full scan, `Some` =
        /// pathspec-limited incremental scan).
        calls: Mutex<Vec<Option<Vec<PathBuf>>>>,
    }

    impl CountingGitBackend {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.lock().expect("calls lock").len()
        }

        fn call_paths(&self, index: usize) -> Option<Vec<PathBuf>> {
            self.calls
                .lock()
                .expect("calls lock")
                .get(index)
                .cloned()
                .flatten()
        }
    }

    impl GitBackend for CountingGitBackend {
        // A poisoned test-double lock means the test harness itself is broken;
        // panicking is more useful here than threading that through `Result`.
        #[allow(clippy::unwrap_in_result)]
        fn complete_status(
            &self,
            path: &Path,
            paths: Option<&[PathBuf]>,
            max_ahead_behind: usize,
            stash_enabled: bool,
            timeout: Duration,
        ) -> Result<Option<CompleteStatus>> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(paths.map(<[PathBuf]>::to_vec));
            NativeGitBackend.complete_status(path, paths, max_ahead_behind, stash_enabled, timeout)
        }
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo fails.
    ///
    /// Regression test for #470 acceptance criterion 6 (first half): a
    /// `.git/index` write must cost exactly one complete git capture round,
    /// not two. Before the #466 fix (commit d8ff5ae6), `git_paths_hint`
    /// hinted `.git/index` as an incremental target;
    /// `try_incremental_update_with` would spend a capture round on a
    /// pathspec `git status` can never
    /// report on, come back with an empty file map, find no cache entry to
    /// update, and the caller would burn a SECOND capture round on a full
    /// scan. `refresh_repo_status_with` threads the same backend into both
    /// call sites (see its doc comment), so this counter catches that
    /// regression instead of only ever seeing the outer, always-present full
    /// scan.
    fn git_index_write_triggers_exactly_one_capture_round() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();

        let index_path = repo.join(".git/index");
        let paths_hint = git_paths_hint(&GitPaths::single(index_path), &repo);
        assert!(
            paths_hint.is_none(),
            "precondition: a .git/index write must hint a full scan"
        );

        refresh_repo_status_with(&backend, &ctx, &repo, &config, paths_hint.as_deref());

        assert_eq!(
            backend.call_count(),
            1,
            "a .git/index write must cost exactly one complete git capture round"
        );
        assert_eq!(
            backend.call_paths(0),
            None,
            "the single capture round must be a full scan (paths == None)"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo or writing files fails.
    ///
    /// Regression test for #470 acceptance criterion 6 (second half): the
    /// contrast case for the test above. An ordinary worktree file change
    /// must still take the pathspec-limited incremental route, in exactly
    /// one capture round — this is what proves the counter can actually tell
    /// the two routes apart, so a pass on the `.git/index` test above is not
    /// vacuous.
    fn ordinary_file_write_takes_the_incremental_route_in_one_capture_round() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();

        let source = repo.join("a.txt");
        fs::write(&source, "one\n").expect("write a.txt");
        commit_all(&repo);
        fs::write(&source, "one changed\n").expect("modify a.txt");

        // Warm the cache first: the incremental route needs a pre-existing
        // complete-detail cache entry to update (see
        // `try_incremental_update_with`'s doc comment), same as every live
        // agent has after its initial full scan.
        refresh_repo_status_with(&backend, &ctx, &repo, &config, None);
        assert_eq!(backend.call_count(), 1, "warm-up scan should be one call");

        let paths_hint = git_paths_hint(&GitPaths::single(source.clone()), &repo);
        assert_eq!(
            paths_hint,
            Some(vec![source]),
            "precondition: an ordinary file write must hint a single-path incremental scan"
        );

        refresh_repo_status_with(&backend, &ctx, &repo, &config, paths_hint.as_deref());

        assert_eq!(
            backend.call_count(),
            2,
            "one warm-up call plus exactly one incremental call"
        );
        assert_eq!(
            backend.call_paths(1),
            Some(vec![PathBuf::from("a.txt")]),
            "the incremental capture round must be pathspec-limited to the changed file"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo fails.
    ///
    /// Pins the *harness* the two tests above rely on (#470): that
    /// [`refresh_repo_status_with`] threads one backend instance through
    /// both of its capture-round call sites.
    ///
    /// Neither test above can show this. Each asserts a count of one, which
    /// the outer full scan alone would satisfy — so if a later refactor gave
    /// the incremental attempt its own backend, both would still pass while
    /// the counter had gone blind to half the code it is supposed to watch.
    /// #467 and #468 assert on capture-round counts using this harness, so
    /// that blindness would silently invalidate their tests, not just these.
    ///
    /// Forcing the hint `git_paths_hint` would never produce reconstructs
    /// the pre-#466 route: the incremental attempt spends a round on a
    /// pathspec `git status` cannot report on, returns an empty file map,
    /// finds no cache entry to update, and the caller pays a second round on
    /// the full scan it always needed. More than one call is the assertion
    /// that matters; the exact total is pinned as documentation of the cost.
    fn a_git_dir_hint_burns_a_doomed_capture_round_before_the_full_scan() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();

        // A hint `git_paths_hint` refuses to produce, passed directly to
        // `refresh_repo_status_with` to exercise the incremental call site.
        let index_path = repo.join(".git/index");
        let forced_hint = vec![index_path];

        refresh_repo_status_with(&backend, &ctx, &repo, &config, Some(&forced_hint));

        assert!(
            backend.call_count() > 1,
            "the incremental call site must share the caller's backend, or a \
             capture-round counter is blind to it (saw {} call(s))",
            backend.call_count()
        );
        assert_eq!(
            backend.call_paths(0),
            Some(vec![PathBuf::from(".git/index")]),
            "the first round is the doomed pathspec-limited scan"
        );
        assert_eq!(
            backend.call_paths(backend.call_count() - 1),
            None,
            "the last round is the full scan the event always needed"
        );
    }

    // --- #480: no-subscriber gate ---

    /// Mirrors the `FileEvent::Git` gate in `create_watcher`'s callback:
    /// `ClientDirectory::has_subscriber` decides whether a capture round runs
    /// at all. Exercised here (rather than by driving `create_watcher`
    /// itself) because production hardcodes `NativeGitBackend` at
    /// `refresh_repo_status` with no injection seam for `CountingGitBackend`
    /// -- exactly the same reason every other capture-round test in this
    /// module calls `refresh_repo_status_with` directly instead of going
    /// through `handle_file_event`.
    fn simulate_gated_git_event<B: GitBackend>(
        backend: &B,
        ctx: &AgentContext,
        repo: &Path,
        config: &Config,
    ) {
        if ctx.registry.has_subscriber(repo) {
            refresh_repo_status_with(backend, ctx, repo, config, None);
        }
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo fails.
    ///
    /// Regression test for #480 acceptance criterion 5: a `FileEvent::Git`
    /// for a repository with no registered client must not run a git
    /// capture round at all.
    fn unregistered_repo_git_event_costs_zero_capture_rounds() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();

        simulate_gated_git_event(&backend, &ctx, &repo, &config);

        assert_eq!(
            backend.call_count(),
            0,
            "a repository with no registered client must not run a capture round"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo fails.
    ///
    /// Contrast case for the test above, proving the gate does not
    /// over-fire: a repository whose registered client's `cwd` is exactly
    /// the repo root still gets exactly one capture round.
    fn client_registered_at_repo_root_still_gets_one_capture_round() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();
        ctx.registry.register(4242, Some(repo.clone()));

        simulate_gated_git_event(&backend, &ctx, &repo, &config);

        assert_eq!(
            backend.call_count(),
            1,
            "a repository with a registered client must still refresh exactly once"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo fails.
    ///
    /// A client working inside a *subdirectory* of the repo -- the common
    /// case for a shell `cd`ed into a subproject -- must still count as a
    /// subscriber (`paths_related`'s `client.starts_with(target)` arm).
    fn client_registered_inside_repo_subdirectory_still_gets_one_capture_round() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();
        ctx.registry.register(4242, Some(repo.join("src")));

        simulate_gated_git_event(&backend, &ctx, &repo, &config);

        assert_eq!(
            backend.call_count(),
            1,
            "a client cwd'd inside the repo must still be a subscriber"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo or its parent directory fails.
    ///
    /// A client registered in an *ancestor* directory of the repo (e.g. a
    /// shell sitting in `$HOME` above `$HOME/proj`) must count as a
    /// subscriber (`paths_related`'s `target.starts_with(client)` arm),
    /// matching `notify_with_signal`'s own permissive targeting.
    fn client_registered_in_ancestor_directory_still_gets_one_capture_round() {
        let parent = tempdir().expect("create parent temp dir");
        let repo_dir = parent.path().join("proj");
        fs::create_dir_all(&repo_dir).expect("create repo dir");
        std::process::Command::new("git")
            .arg("init")
            .current_dir(&repo_dir)
            .output()
            .expect("git init");
        let repo = fs::canonicalize(&repo_dir).expect("canonical repo");
        let ancestor = fs::canonicalize(parent.path()).expect("canonical ancestor");

        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();
        ctx.registry.register(4242, Some(ancestor));

        simulate_gated_git_event(&backend, &ctx, &repo, &config);

        assert_eq!(
            backend.call_count(),
            1,
            "a client registered in an ancestor directory must still be a subscriber"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo fails.
    ///
    /// A registered client with no workspace (`cwd: None`) subscribes to
    /// every repository unconditionally -- matching `notify_with_signal`'s
    /// own rule, which only skips a client when BOTH sides have a path and
    /// they are unrelated.
    fn client_with_no_workspace_still_gets_one_capture_round() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();
        ctx.registry.register(4242, None);

        simulate_gated_git_event(&backend, &ctx, &repo, &config);

        assert_eq!(
            backend.call_count(),
            1,
            "a client with no workspace must be treated as a subscriber for every repo"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating two temp repos fails.
    ///
    /// A client registered for an unrelated repository must not turn an
    /// unregistered repository into a false subscriber -- the gate is
    /// per-repo, not "any client exists anywhere".
    fn client_registered_for_a_different_repo_does_not_count_as_a_subscriber() {
        let (_tmp_a, repo_a) = create_temp_repo();
        let (_tmp_b, repo_b) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = CountingGitBackend::new();
        ctx.registry.register(4242, Some(repo_a));

        simulate_gated_git_event(&backend, &ctx, &repo_b, &config);

        assert_eq!(
            backend.call_count(),
            0,
            "a client registered for a different, unrelated repo must not count"
        );
    }

    #[test]
    #[serial_test::file_serial(watcher_fsevents_bootstrap)]
    /// # Panics
    ///
    /// Panics if creating the temp repo, disabling `core.fsmonitor`, or
    /// registering with the real watcher fails.
    ///
    /// End-to-end regression test for #480: drives the REAL `create_watcher`
    /// callback -- the actual production wiring -- rather than simulating
    /// the gate. This is the necessary complement to
    /// `simulate_gated_git_event`'s tests above: those pin `has_subscriber`'s
    /// own correctness using `CountingGitBackend`, but none of them ever
    /// touch `create_watcher`'s closure, so none of them would fail if the
    /// gate were deleted from it. This test fails if it is deleted (verified
    /// by deliberately deleting it during development: this test failed --
    /// `ctx.cache` gained an entry for the unregistered case -- while all six
    /// `simulate_gated_git_event`-based tests kept passing).
    ///
    /// Observation point: `ctx.cache`, since production hardcodes
    /// `NativeGitBackend` (see `refresh_repo_status`) with no injection seam
    /// for `CountingGitBackend` through the real watcher path -- the same
    /// constraint documented on `simulate_gated_git_event`. When the gate
    /// fires, `handle_file_event` never runs, so the repo never gets a cache
    /// entry; when it does not fire, a full scan runs and populates one.
    ///
    /// `#[serial_test::file_serial(watcher_fsevents_bootstrap)]` mirrors
    /// `watcher::multi_repo::tests`' real-watcher tests: this is a real OS
    /// filesystem watch, and `FSEvents` stream setup is a shared resource
    /// that concurrent bootstraps can make flaky.
    fn create_watcher_skips_refresh_for_unregistered_repo_and_refreshes_registered_one() {
        let (_tmp, repo) = create_temp_repo();
        std::process::Command::new("git")
            .args(["config", "core.fsmonitor", "false"])
            .current_dir(&repo)
            .output()
            .expect("disable fsmonitor");

        let ctx = make_agent_context();
        let watcher = create_watcher(WatcherConfig::default(), &ctx).expect("create real watcher");
        // Arms the filesystem watch for `repo`. Deliberately independent of
        // `ctx.registry` (`ClientDirectory`) -- exactly as production keeps
        // `MultiRepoWatcher`'s watch registry and `ClientDirectory` as two
        // separate registries (`client_handler.rs` calls both, in that
        // order), which is what makes the gate meaningful: the repo is
        // watched throughout this test, but only sometimes subscribed to.
        watcher
            .register_client(std::process::id(), &repo)
            .expect("register with watcher");

        // Let the watch fully arm before the first mutation.
        thread::sleep(Duration::from_millis(100));

        // --- Unsubscribed case: no client registered in ctx.registry. ---
        fs::write(repo.join("a.txt"), "one\n").expect("write a.txt");

        // This is a negative assertion, so it needs a generous settle -- a
        // short wait would pass for the wrong reason (the event simply
        // hasn't arrived yet), not because the gate suppressed it. "Never
        // happened" and "hasn't happened yet" are indistinguishable at any
        // earlier point, so the full budget is still spent on the passing
        // path. Polling across it instead of checking once at the end only
        // changes the failing path: a leak is caught within ~100ms of
        // happening, which says *when* the gate let the event through.
        let settle_deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < settle_deadline {
            assert!(
                ctx.cache.get(&repo).is_none(),
                "an unregistered repo must not gain a cache entry from a real watcher event"
            );
            thread::sleep(Duration::from_millis(100));
        }
        // The loop's own deadline check runs BEFORE each sleep, so its last
        // in-loop assertion lands ~100ms short of the full 3s budget and the
        // final window exits uncovered. Assert once more here so the full
        // window -- including its very end -- is genuinely covered, matching
        // the original single `sleep(3s)` + assert this loop replaced.
        assert!(
            ctx.cache.get(&repo).is_none(),
            "an unregistered repo must not gain a cache entry from a real watcher event"
        );

        // --- Subscribed case: same repo, now with a registered client. ---
        // The load-bearing half: proves the gate isn't simply swallowing
        // everything, and fails if the condition were inverted.
        //
        // A nonexistent PID: a status change on a genuinely subscribed repo
        // reaches `notify_repaint_force`, whose `kill()` prunes it via ESRCH,
        // keeping the test process out of the delivery path entirely.
        let fake_pid = 999_999_u32;
        ctx.registry.register(fake_pid, Some(repo.clone()));
        fs::write(repo.join("b.txt"), "two\n").expect("write b.txt");

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut populated = false;
        while std::time::Instant::now() < deadline {
            if ctx.cache.get(&repo).is_some() {
                populated = true;
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert!(
            populated,
            "a registered repo must gain a cache entry through the real watcher within the budget"
        );

        watcher.stop();
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo or writing files fails.
    ///
    /// Regression test for #466 acceptance criterion 2: two files changed
    /// inside one debounce window must publish the same aggregate as a full
    /// scan. Before the fix the event carried only the last path and the
    /// incremental scan reported one modified file instead of two.
    fn coalesced_multi_file_event_matches_a_full_scan() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        let first = repo.join("a.txt");
        let second = repo.join("b.txt");
        fs::write(&first, "one\n").expect("write a.txt");
        fs::write(&second, "two\n").expect("write b.txt");
        commit_all(&repo);

        // Warm the cache on the clean tree, exactly as a live agent would be.
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        assert_eq!(ctx.cache.get(&repo).expect("warm cache").unstaged, 0);

        fs::write(&first, "one changed\n").expect("modify a.txt");
        fs::write(&second, "two changed\n").expect("modify b.txt");

        let event = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::Paths(vec![first, second]),
            },
            repo: repo.clone(),
        };
        handle_file_event(&ctx, &event, &config);
        let incremental = ctx.cache.get(&repo).expect("status after coalesced event");

        ctx.cache.invalidate(&repo);
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        let full = ctx.cache.get(&repo).expect("status after full scan");

        assert_eq!(
            incremental.unstaged, 2,
            "both modified files must be counted"
        );
        assert_eq!(incremental, full, "coalesced event must match a full scan");
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo or the rename fails.
    ///
    /// Regression test for #466 acceptance criterion 3: a rename arrives as two
    /// paths, and `git status` keys the result under the destination only. The
    /// source must be cleared from the cache or the deleted file stays counted.
    fn renamed_source_path_is_dropped_from_the_cache() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        let source = repo.join("a.txt");
        fs::write(&source, "one\n").expect("write a.txt");
        commit_all(&repo);

        // A plain edit first, so the source path holds a real cache entry.
        fs::write(&source, "one changed\n").expect("modify a.txt");
        let edit = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(source.clone()),
            },
            repo: repo.clone(),
        };
        handle_file_event(&ctx, &edit, &config);
        assert_eq!(ctx.cache.get(&repo).expect("edit cached").unstaged, 1);

        // `git mv`, so git reports one rename record keyed under the
        // destination — the shape that leaves the source path stale.
        let destination = repo.join("b.txt");
        let output = std::process::Command::new("git")
            .args(["mv", "a.txt", "b.txt"])
            .current_dir(&repo)
            .output()
            .expect("git mv");
        assert!(output.status.success(), "git mv failed");

        let rename = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::Paths(vec![source, destination]),
            },
            repo: repo.clone(),
        };
        handle_file_event(&ctx, &rename, &config);
        let incremental = ctx.cache.get(&repo).expect("status after rename burst");

        ctx.cache.invalidate(&repo);
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        let full = ctx.cache.get(&repo).expect("status after full scan");

        assert_eq!(
            incremental, full,
            "a rename burst must not leave the source path counted"
        );
    }

    /// Run `git <args>` in `repo`, returning whether it exited successfully.
    ///
    /// # Panics
    ///
    /// Panics if `git` cannot be spawned.
    fn git_in(repo: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(repo)
            .output()
            .expect("spawn git")
            .status
            .success()
    }

    /// Warm the cache with a full scan, edit `other.txt`, deliver a
    /// single-path event for it, and return `(incremental, full)` statuses.
    ///
    /// # Panics
    ///
    /// Panics if writing `other.txt` fails or the cache stays empty.
    fn incremental_and_full_after_other_edit(
        ctx: &AgentContext,
        repo: &Path,
        config: &Config,
    ) -> (RepositoryStatus, RepositoryStatus) {
        handle_file_event(ctx, &whole_repo_event(repo), config);
        assert!(ctx.cache.get(repo).is_some(), "warm cache");

        let other = repo.join("other.txt");
        fs::write(&other, "edit\n").expect("modify other.txt");
        let edit = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(other),
            },
            repo: repo.to_path_buf(),
        };
        handle_file_event(ctx, &edit, config);
        let incremental = ctx.cache.get(repo).expect("status after edit");

        ctx.cache.invalidate(repo);
        handle_file_event(ctx, &whole_repo_event(repo), config);
        let full = ctx.cache.get(repo).expect("status after full scan");
        (incremental, full)
    }

    #[test]
    /// # Panics
    ///
    /// Panics if building the conflicted repository fails.
    ///
    /// Regression test for #686: an incremental refresh during a merge
    /// conflict counted every conflicted file as staged and unstaged too.
    fn incremental_refresh_matches_full_scan_during_merge_conflict() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        fs::write(repo.join("a"), "base\n").expect("write a");
        fs::write(repo.join("b"), "base\n").expect("write b");
        fs::write(repo.join("other.txt"), "o\n").expect("write other.txt");
        commit_all(&repo);
        assert!(git_in(&repo, &["checkout", "-qb", "side"]));
        fs::write(repo.join("a"), "side\n").expect("write a");
        fs::write(repo.join("b"), "side\n").expect("write b");
        assert!(git_in(&repo, &["commit", "-qam", "side"]));
        assert!(git_in(&repo, &["checkout", "-q", "-"]));
        fs::write(repo.join("a"), "main\n").expect("write a");
        fs::write(repo.join("b"), "main\n").expect("write b");
        assert!(git_in(&repo, &["commit", "-qam", "main"]));
        // The merge conflicts by design, so its non-zero exit is expected.
        let _conflicted = git_in(&repo, &["merge", "side"]);

        let (incremental, full) = incremental_and_full_after_other_edit(&ctx, &repo, &config);

        assert_eq!(
            incremental, full,
            "incremental refresh must match a full scan"
        );
        assert_eq!(
            (
                incremental.staged,
                incremental.unstaged,
                incremental.conflicts
            ),
            (0, 1, 2)
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if building the repository fails.
    ///
    /// Regression test for #686: `git rm --cached f` reports a staged deletion
    /// and an untracked record for the same path; the untracked record used
    /// to overwrite the deletion in the file map, losing it on the next
    /// incremental refresh.
    fn incremental_refresh_keeps_staged_delete_with_untracked_same_path() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        fs::write(repo.join("secret.env"), "s\n").expect("write secret.env");
        fs::write(repo.join("other.txt"), "o\n").expect("write other.txt");
        commit_all(&repo);
        assert!(git_in(&repo, &["rm", "-q", "--cached", "secret.env"]));

        let (incremental, full) = incremental_and_full_after_other_edit(&ctx, &repo, &config);

        assert_eq!(
            incremental, full,
            "incremental refresh must match a full scan"
        );
        assert_eq!(
            (
                incremental.staged,
                incremental.unstaged,
                incremental.untracked
            ),
            (1, 1, 1)
        );
    }

    #[test]
    fn git_events_are_skipped_when_git_disabled() {
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let mut config = Config::default();
        config.git.enabled = false;

        let head_path = repo.join(".git/HEAD");
        let event = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(head_path),
            },
            repo,
        };

        handle_file_event(&ctx, &event, &config);
        assert!(
            ctx.cache.is_empty(),
            "cache should remain empty when git is disabled"
        );
    }

    fn status(untracked: u32, state: crate::git::RepositoryState) -> RepositoryStatus {
        RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked,
            conflicts: 0,
            state,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }
    }

    #[test]
    fn status_changed_detects_transitions() {
        use crate::git::RepositoryState;
        let clean = status(0, RepositoryState::Clean);
        let dirty = status(1, RepositoryState::Dirty);

        // Unchanged: same status, or stayed unavailable -> no notify.
        assert!(!status_changed(Some(&clean), Some(&clean)));
        assert!(!status_changed(None, None));

        // Changed: appeared, disappeared, or differs -> notify.
        assert!(status_changed(None, Some(&clean)));
        assert!(status_changed(Some(&clean), None));
        assert!(status_changed(Some(&clean), Some(&dirty)));
    }

    #[test]
    /// # Panics
    ///
    /// Panics if creating the temp repo fails.
    fn reconcile_full_scan_picks_up_untracked_changes() {
        // The reconcile path passes `paths_hint = None` (full scan). It should
        // detect a working-tree change made without any watcher event firing.
        let (_tmp, repo) = create_temp_repo();
        let ctx = make_agent_context();
        let config = Config::default();

        refresh_and_notify_if_changed(&ctx, &repo, &config, None);
        let baseline = ctx
            .cache
            .get(&repo)
            .expect("status cached after first scan");
        assert_eq!(baseline.untracked, 0, "fresh repo starts clean");

        // Mutate the working tree directly (simulating a dropped fs event).
        fs::write(repo.join("dropped.txt"), "x").expect("write file");

        let before = ctx.registry.notify_invocations();
        refresh_and_notify_if_changed(&ctx, &repo, &config, None);
        let after = ctx.registry.notify_invocations();

        let updated = ctx.cache.get(&repo).expect("status cached");
        assert!(
            updated.untracked >= 1,
            "reconcile full scan should reflect the untracked file"
        );
        assert!(
            after > before,
            "reconcile should notify on the detected change"
        );
    }

    #[test]
    #[serial_test::serial(global_ttl)]
    fn config_reload_disables_and_reenables_watcher() {
        let (
            theme_manager,
            watcher_config,
            watcher_slot,
            client_registry,
            git_cache,
            config_manager,
            instant_cache,
        ) = setup_reload_context();

        let initial_config = config_manager.get();
        let ctx = AgentContext {
            registry: client_registry,
            cache: git_cache,
            config_manager,
            theme_manager,
            instant_cache,
            language_cache: crate::language::DetectionCache::new(),
            palette_cache: Arc::new(crate::palette::PaletteCache::from_config(&initial_config)),
            dispatch_guard: Arc::new(RepoRefreshCoordinator::default()),
            watch_registry: Arc::new(crate::watcher::WatchRegistry::new()),
        };

        let apply = |old_cfg: &Config, new_cfg: &Config| {
            handle_config_reload(&ctx, &watcher_slot, watcher_config, false, old_cfg, new_cfg)
                .expect("config reload should succeed");
        };

        let original = Config::default();
        let mut git_disabled = original.clone();
        git_disabled.git.enabled = false;
        apply(&original, &git_disabled);
        expect_watcher_state(
            &watcher_slot,
            true,
            "watcher should remain active when language live updates still need it",
        );

        let mut disabled = git_disabled.clone();
        disabled.language.enabled = false;
        apply(&git_disabled, &disabled);
        expect_watcher_state(
            &watcher_slot,
            false,
            "watcher should be cleared when both git and language updates are disabled",
        );

        let mut reenabled = disabled.clone();
        reenabled.git.enabled = true;
        apply(&disabled, &reenabled);
        expect_watcher_state(
            &watcher_slot,
            true,
            "watcher should be recreated when git.enabled returns to true",
        );

        let mut no_live_updates = reenabled.clone();
        no_live_updates.agent.live_updates = false;
        apply(&reenabled, &no_live_updates);
        expect_watcher_state(
            &watcher_slot,
            false,
            "watcher should stop when agent.live_updates is false",
        );
    }

    #[test]
    #[serial_test::serial(global_ttl)]
    fn config_reload_notifies_on_language_toggles() {
        let ctx = make_agent_context();
        let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
        let watcher_config = WatcherConfig::default();

        let original = Config::default();
        let mut toggled = original.clone();
        toggled.language.show_versions = !original.language.show_versions;

        let before = ctx.registry.notify_invocations();
        handle_config_reload(
            &ctx,
            &watcher_slot,
            watcher_config,
            true,
            &original,
            &toggled,
        )
        .expect("config reload should succeed");
        let after = ctx.registry.notify_invocations();
        assert!(
            after > before,
            "toggling show_versions should trigger notification"
        );

        let mut toggled_enabled = toggled.clone();
        toggled_enabled.language.enabled = !toggled.language.enabled;
        let before_enabled = ctx.registry.notify_invocations();
        handle_config_reload(
            &ctx,
            &watcher_slot,
            watcher_config,
            true,
            &toggled,
            &toggled_enabled,
        )
        .expect("config reload should succeed");
        let after_enabled = ctx.registry.notify_invocations();
        assert!(
            after_enabled > before_enabled,
            "toggling language.enabled should trigger notification"
        );

        // "text" (a real builtin, distinct from the default-config theme) --
        // not "minimal", a placeholder that isn't a builtin, user, or plugin
        // theme name. #572 made an unresolvable theme name a hard error, so
        // this test's fixture needs a name `handle_config_reload` (via
        // `maybe_switch_theme`) can actually load; only the notify-on-change
        // behavior below is under test, not this specific theme's content.
        let mut theme_changed = toggled_enabled.clone();
        theme_changed.ui.theme =
            crate::config::types::ThemeName::new("text".to_owned()).expect("valid theme name");
        let before_theme = ctx.registry.notify_invocations();
        handle_config_reload(
            &ctx,
            &watcher_slot,
            watcher_config,
            true,
            &toggled_enabled,
            &theme_changed,
        )
        .expect("config reload should succeed");
        let after_theme = ctx.registry.notify_invocations();
        assert!(
            after_theme > before_theme,
            "changing ui.theme should trigger notification"
        );
    }

    #[allow(clippy::type_complexity)]
    fn setup_reload_context() -> (
        Arc<ThemeManager>,
        WatcherConfig,
        SharedWatcherSlot,
        Arc<ClientDirectory>,
        Arc<GitStatusCache>,
        Arc<crate::config::manager::ConfigManager>,
        Arc<InstantPromptCache>,
    ) {
        let theme_manager = make_theme_manager();
        let watcher_config = WatcherConfig::default();
        let mock_watcher = Arc::new(
            MultiRepoWatcher::builder()
                .config(watcher_config)
                .callback(Box::new(|_event: DebouncedEvent| {}))
                .build()
                .expect("mock watcher should initialize"),
        );
        let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(Some(mock_watcher)));
        let client_registry = make_client_registry();
        let git_cache = make_git_cache();
        let config_manager = make_config_manager();
        let instant_cache = make_instant_cache();

        (
            theme_manager,
            watcher_config,
            watcher_slot,
            client_registry,
            git_cache,
            config_manager,
            instant_cache,
        )
    }

    #[allow(clippy::significant_drop_tightening)]
    fn expect_watcher_state(watcher_slot: &SharedWatcherSlot, expect_some: bool, message: &str) {
        let watcher_guard = watcher_slot.lock().expect("watcher lock poisoned");
        assert_eq!(watcher_guard.is_some(), expect_some, "{message}");
    }

    #[test]
    #[serial_test::serial(global_ttl)]
    fn config_reload_updates_language_ttl() {
        use crate::language::version::{
            set_version_cache_ttl, version_cache_ttl_seconds_for_tests,
        };

        let ctx = make_agent_context();
        let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
        let watcher_config = WatcherConfig::default();

        // Capture whatever the current TTL is (likely 24h from default config)
        // The test goal is to verify the reload mechanism updates it, not the specific initial value
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
        let before = version_cache_ttl_seconds_for_tests();
        let initial_hours = before / 3600;

        let mut old_config = Config::default();
        old_config.language.cache_ttl_hours =
            crate::config::types::CacheTtlHours::new(initial_hours).expect("valid ttl");

        let mut updated = Config::default();
        updated.language.cache_ttl_hours =
            crate::config::types::CacheTtlHours::new(36).expect("valid ttl");

        handle_config_reload(
            &ctx,
            &watcher_slot,
            watcher_config,
            true,
            &old_config,
            &updated,
        )
        .expect("config reload should succeed");

        let after = version_cache_ttl_seconds_for_tests();
        assert_eq!(
            after,
            36 * 3600,
            "cache TTL should update to new value after reload"
        );
        assert_ne!(after, before, "TTL should change from previous value");

        // Clean up: restore default for other tests
        set_version_cache_ttl(24);
    }

    #[test]
    fn palette_change_triggers_refresh_required() {
        use crate::config::types::PaletteName;
        let old = Config::default();
        let mut new_cfg = Config::default();
        // Switch to any palette name that differs from the default.
        // config_refresh_required is a pure comparison — it does not load palettes.
        new_cfg.ui.palette = PaletteName::new("other-palette".to_owned()).expect("valid name");
        assert!(
            config_refresh_required(&old, &new_cfg),
            "a ui.palette change must be recognized as requiring a prompt refresh"
        );
    }

    #[test]
    fn unchanged_config_does_not_require_refresh() {
        let config = Config::default();
        assert!(
            !config_refresh_required(&config, &config),
            "identical configs must not trigger a prompt refresh"
        );
    }

    // Regression locks for #608's exhaustive per-field classification below:
    // one INCLUDED-field flip (asserts `true`) and one EXCLUDED-field flip
    // (asserts `false`, all else equal) per `*_refresh_required` helper.
    // `ui_refresh_required` has no excluded field (all 5 `UiSettings` fields
    // affect rendering), so it gets only the included-field test.

    #[test]
    fn ui_show_icons_change_triggers_refresh_required() {
        let old = crate::config::UiSettings::default();
        let new = crate::config::UiSettings {
            show_icons: !old.show_icons,
            ..crate::config::UiSettings::default()
        };
        assert!(
            ui_refresh_required(&old, &new),
            "ui.show_icons is rendered directly; changing it must require a refresh"
        );
    }

    #[test]
    fn git_stash_enabled_change_triggers_refresh_required() {
        let old = crate::config::GitSettings::default();
        let new = crate::config::GitSettings {
            stash_enabled: !old.stash_enabled,
            ..crate::config::GitSettings::default()
        };
        assert!(
            git_refresh_required(&old, &new),
            "git.stash_enabled affects the rendered $stash indicator; must require a refresh"
        );
    }

    #[test]
    fn git_timeout_seconds_change_does_not_trigger_refresh_required() {
        let old = crate::config::GitSettings::default();
        let new = crate::config::GitSettings {
            timeout_seconds: crate::config::types::GitTimeout::new(old.timeout_seconds.get() + 1)
                .expect("valid git timeout"),
            ..crate::config::GitSettings::default()
        };
        assert_ne!(
            old.timeout_seconds, new.timeout_seconds,
            "the mutation must actually change the field for this test to mean anything"
        );
        assert!(
            !git_refresh_required(&old, &new),
            "git.timeout_seconds is subprocess-management, not rendering; must not require a refresh"
        );
    }

    #[test]
    fn language_filter_change_triggers_refresh_required() {
        let old = crate::config::LanguageSettings::default();
        let new = crate::config::LanguageSettings {
            filter: crate::config::types::LanguageFilter::Primary,
            ..crate::config::LanguageSettings::default()
        };
        assert_ne!(
            old.filter, new.filter,
            "the mutation must actually change the field for this test to mean anything"
        );
        assert!(
            language_refresh_required(&old, &new),
            "language.filter is a render-time decision over already-detected languages; must require a refresh"
        );
    }

    #[test]
    fn language_cache_ttl_hours_change_does_not_trigger_refresh_required() {
        let old = crate::config::LanguageSettings::default();
        let new = crate::config::LanguageSettings {
            cache_ttl_hours: crate::config::types::CacheTtlHours::new(
                old.cache_ttl_hours.get() + 1,
            )
            .expect("valid cache ttl"),
            ..crate::config::LanguageSettings::default()
        };
        assert_ne!(
            old.cache_ttl_hours, new.cache_ttl_hours,
            "the mutation must actually change the field for this test to mean anything"
        );
        assert!(
            !language_refresh_required(&old, &new),
            "language.cache_ttl_hours affects the next detection pass, not current rendering; must not require a refresh"
        );
    }

    #[test]
    fn agent_live_updates_change_triggers_refresh_required() {
        let old = crate::config::AgentSettings::default();
        let new = crate::config::AgentSettings {
            live_updates: !old.live_updates,
            ..crate::config::AgentSettings::default()
        };
        assert!(
            agent_refresh_required(&old, &new),
            "agent.live_updates gates whether renders are pushed at all; must require a refresh"
        );
    }

    #[test]
    fn agent_timeout_seconds_change_does_not_trigger_refresh_required() {
        let old = crate::config::AgentSettings::default();
        let new = crate::config::AgentSettings {
            timeout_seconds: crate::config::types::AgentTimeout::new(old.timeout_seconds.get() + 1)
                .expect("valid agent timeout"),
            ..crate::config::AgentSettings::default()
        };
        assert_ne!(
            old.timeout_seconds, new.timeout_seconds,
            "the mutation must actually change the field for this test to mean anything"
        );
        assert!(
            !agent_refresh_required(&old, &new),
            "agent.timeout_seconds is IPC socket process-management, not rendering; must not require a refresh"
        );
    }

    /// Build a hermetic agent context: an instant cache rooted in `cache_dir` and
    /// the embedded builtin `default` theme (no `~/.config` reads).
    fn make_hermetic_ctx(cache_dir: &Path) -> AgentContext {
        let config_manager = make_config_manager();
        let initial_config = config_manager.get();
        AgentContext {
            registry: make_client_registry(),
            cache: make_git_cache(),
            config_manager,
            theme_manager: Arc::new(ThemeManager::builtin("default").expect("builtin default")),
            instant_cache: Arc::new(
                InstantPromptCache::new_in_dir(cache_dir.to_path_buf()).expect("instant cache"),
            ),
            language_cache: crate::language::DetectionCache::new(),
            palette_cache: Arc::new(crate::palette::PaletteCache::from_config(&initial_config)),
            dispatch_guard: Arc::new(RepoRefreshCoordinator::default()),
            watch_registry: Arc::new(crate::watcher::WatchRegistry::new()),
        }
    }

    fn cache_file_names(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .expect("read cache dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    }

    /// #223: a theme change must proactively regenerate the instant-prompt caches
    /// for an active (registered, warm) path — using the already-cached git and
    /// language data and the new theme — so the reload repaint hits warm caches
    /// and recolors atomically instead of cold-missing (disappear/reappear flicker).
    ///
    /// Drives the regeneration helper that `handle_config_reload` runs before any
    /// reload doorbell, with a hermetic instant cache and the embedded builtin theme.
    ///
    /// Git renders from cached status (no subprocess) and is regenerated
    /// synchronously; language detection is deferred off the event loop (see
    /// [`language_regen_renders_cached_data`]), so no language cache is written here.
    #[test]
    fn theme_change_regenerates_git_caches_for_active_path() {
        use crate::git::RepositoryState;
        use std::collections::HashMap;

        let (_tmp, repo) = create_temp_repo();
        let cache_tmp = tempdir().expect("cache dir");
        let ctx = make_hermetic_ctx(cache_tmp.path());

        // Active path: a registered client in the repo with WARM git data already
        // cached (the expensive detection is done). A theme switch changes only the
        // render, not this data.
        ctx.registry.register(4321, Some(repo.clone()));
        ctx.cache
            .set(&repo, status(0, RepositoryState::Clean), HashMap::new());

        regenerate_instant_caches_for_theme_change(&ctx, &Config::default());

        let files = cache_file_names(cache_tmp.path());

        let git_none = files
            .iter()
            .find(|f| f.ends_with(".git.none.ansi"))
            .expect("context-free git cache must be regenerated");
        for dialect in crate::formatter::PromptDialect::ALL {
            let ext = format!(".{}", dialect.cache_ext());
            assert!(
                files
                    .iter()
                    .any(|f| f.contains(".git.none.") && f.ends_with(&ext)),
                "context-free {ext} git cache must be regenerated: {files:?}"
            );
            // A non-`none` git token file proves the new-theme prev_bg context
            // was seeded, so the post-reload shell request resolves a warm file
            // instead of cold-missing on a token it has never used before.
            assert!(
                files.iter().any(|f| {
                    f.contains(".git.")
                        && f.ends_with(&ext)
                        && !f.contains(".git.none.")
                        && !f.contains(".git_last.none.")
                }),
                "a seeded prev_bg {ext} git cache must exist: {files:?}"
            );
        }

        let rendered = fs::read_to_string(cache_tmp.path().join(git_none)).expect("read git cache");
        assert!(
            rendered.contains("main"),
            "regenerated git cache must carry the cached branch: {rendered:?}"
        );
        assert!(
            rendered.contains('\u{1b}'),
            "regenerated git cache must be styled ANSI: {rendered:?}"
        );
    }

    /// #223: the deferred language regeneration body re-renders the cached language
    /// data into the instant caches (context-free fallback + the new theme's
    /// `prev_bg` token) and reports that a repaint is warranted.
    ///
    /// This is the synchronous body that [`schedule_language_cache_regen`] runs on a
    /// background thread, so version detection never blocks the reload event loop.
    #[test]
    fn language_regen_renders_cached_data() {
        use crate::language::DetectedLanguage;

        let (_tmp, repo) = create_temp_repo();
        let cache_tmp = tempdir().expect("cache dir");
        let ctx = make_hermetic_ctx(cache_tmp.path());

        ctx.language_cache.set(
            &repo,
            vec![DetectedLanguage {
                name: "rust".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 128,
            }],
        );

        let wrote = regen_language_caches(&ctx, &repo, &Config::default());
        assert!(wrote, "first language regen must report a write");

        let files = cache_file_names(cache_tmp.path());
        for dialect in crate::formatter::PromptDialect::ALL {
            let ext = format!(".{}", dialect.cache_ext());
            assert!(
                files
                    .iter()
                    .any(|f| f.contains(".lang.none.") && f.ends_with(&ext)),
                "context-free {ext} language cache must be regenerated: {files:?}"
            );
            // A non-`none` token file proves the new-theme prev_bg context was seeded.
            assert!(
                files.iter().any(|f| {
                    f.contains(".lang.")
                        && f.ends_with(&ext)
                        && !f.contains(".lang.none.")
                        && !f.contains(".lang_last.none.")
                }),
                "a seeded prev_bg {ext} language cache must exist: {files:?}"
            );
        }
    }

    /// A language regen for a path with no warm data is a no-op (no write, no
    /// repaint) — it must never trigger synchronous detection.
    #[test]
    fn language_regen_skips_cold_path() {
        let (_tmp, repo) = create_temp_repo();
        let cache_tmp = tempdir().expect("cache dir");
        let ctx = make_hermetic_ctx(cache_tmp.path());

        assert!(
            !regen_language_caches(&ctx, &repo, &Config::default()),
            "a path with no cached language data must report no write"
        );
        assert_eq!(
            cache_file_names(cache_tmp.path()).len(),
            0,
            "no language cache files should be written for a cold path"
        );
    }

    /// #611: `refresh_language_for` is the single writer behind all three
    /// language-cache refresh triggers, and each of them used to call
    /// `notify_repaint_force` unconditionally; now they all go through the
    /// `if wrote { notify }` gate this function applies. Pins that gate
    /// directly: a render that actually changes cache-file content must
    /// notify, and a repeat render of the exact same content must not.
    #[test]
    fn language_refresh_notifies_only_when_a_cache_write_actually_happens() {
        use crate::language::DetectedLanguage;

        let (_tmp, repo) = create_temp_repo();
        let cache_tmp = tempdir().expect("cache dir");
        let ctx = make_hermetic_ctx(cache_tmp.path());
        let config = Config::default();
        let theme = ctx.theme_manager.get();

        ctx.language_cache.set(
            &repo,
            vec![DetectedLanguage {
                name: "rust".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 128,
            }],
        );

        // First render into an empty cache dir: content genuinely changes
        // (nothing was on disk before), so this must notify.
        let before_first = ctx.registry.notify_invocations();
        let wrote_first = refresh_language_for(
            &ctx,
            &repo,
            &config,
            LanguageRefreshMode::CachedOnly,
            &theme,
            None,
        );
        let after_first = ctx.registry.notify_invocations();

        assert!(
            wrote_first,
            "first render into an empty cache dir must write"
        );
        assert!(
            after_first > before_first,
            "a genuine cache write must notify (before: {before_first}, after: {after_first})"
        );

        // Second render of the identical state: `write_language_variants`
        // reports no change, so no notify should fire.
        let wrote_second = refresh_language_for(
            &ctx,
            &repo,
            &config,
            LanguageRefreshMode::CachedOnly,
            &theme,
            None,
        );
        let after_second = ctx.registry.notify_invocations();

        assert!(
            !wrote_second,
            "a re-render of unchanged content must report no write"
        );
        assert_eq!(
            after_second, after_first,
            "a no-op re-render must not notify (after first: {after_first}, after second: {after_second})"
        );
    }

    /// #223 cold-path guarantee (no regression to #145/#160): a registered repo
    /// with NO warm cached data is skipped entirely — regeneration writes nothing
    /// and never runs synchronous git/language detection on the reload path.
    #[test]
    fn theme_change_skips_paths_without_warm_data() {
        let (_tmp, repo) = create_temp_repo();
        let cache_tmp = tempdir().expect("cache dir");
        let ctx = make_hermetic_ctx(cache_tmp.path());

        // Registered, but the agent holds no cached git/language data for it.
        ctx.registry.register(4321, Some(repo));

        regenerate_instant_caches_for_theme_change(&ctx, &Config::default());

        assert_eq!(
            cache_file_names(cache_tmp.path()).len(),
            0,
            "no instant-prompt cache files should be written for a cold path"
        );
    }

    /// #446: when the instant-cache write fails transiently and a later refresh
    /// with the SAME `RepositoryStatus` recovers it, that recovery write must
    /// still trigger a repaint even though `status_changed` alone reports no
    /// change (see `write_cache_file`'s memo: a failed write never updates it,
    /// so the recovery write naturally reports `Ok(true)`).
    ///
    /// Uses `make_hermetic_ctx`'s caller-supplied cache dir as the injection
    /// seam: making the directory unwritable (mode `0o500`) forces
    /// `InstantPromptCache::write_git` to return `Err` without touching any
    /// production-only test hook.
    #[test]
    #[cfg(unix)]
    fn instant_cache_recovery_write_triggers_repaint_on_unchanged_status() {
        use std::os::unix::fs::PermissionsExt;

        let (_tmp, repo) = create_temp_repo();
        let cache_tmp = tempdir().expect("cache dir");
        let ctx = make_hermetic_ctx(cache_tmp.path());
        let mut config = Config::default();
        // Disable the async language-refresh side channel: it calls
        // `notify_repaint_force` itself on a background thread (see
        // `schedule_language_refresh_from_git_status`), independent of the
        // instant git-cache write outcome under test here. Left enabled it
        // would race with this test's synchronous notification counts.
        config.language.enabled = false;

        let baseline_notifications = ctx.registry.notify_invocations();
        assert_eq!(baseline_notifications, 0, "no refresh has run yet");

        // Make the cache directory unwritable BEFORE the very first refresh, so
        // that refresh's instant-cache write fails outright: no on-disk file
        // and no `write_cache_file` "last written" memo entry exist yet for
        // this repo, so there is no identical-content short-circuit to dodge
        // the write attempt (simulates a transient storage failure, #446).
        let cache_dir = cache_tmp.path();
        let writable_perms = fs::metadata(cache_dir)
            .expect("cache dir metadata")
            .permissions();
        fs::set_permissions(cache_dir, std::fs::Permissions::from_mode(0o500))
            .expect("make cache dir read-only");

        // First-ever refresh: the repository status "appears" (previous ==
        // None), which alone would normally notify, but the cache write
        // fails. This must NOT notify (a repaint here could only serve a
        // missing/stale instant-cache file, not fresh content — see judgment
        // call reasoning in the doc comment on `refresh_and_notify_if_changed`).
        refresh_and_notify_if_changed(&ctx, &repo, &config, None);
        let after_failed_write = ctx.registry.notify_invocations();

        // Restore write access: storage has "recovered".
        fs::set_permissions(cache_dir, writable_perms).expect("restore cache dir permissions");

        // Same unchanged repository status (no working-tree mutation happened
        // between calls), but now the write succeeds. Since the prior attempt
        // never updated `write_cache_file`'s last-written memo (and never
        // created the file), this recovery write reports `Ok(true)` and must
        // trigger a repaint even though `status_changed` alone would say
        // nothing moved.
        refresh_and_notify_if_changed(&ctx, &repo, &config, None);
        let after_recovery_write = ctx.registry.notify_invocations();

        assert_eq!(
            after_failed_write, baseline_notifications,
            "a failed cache write must not trigger a repaint that can only serve stale content"
        );
        assert!(
            after_recovery_write > after_failed_write,
            "the first successful recovery write must trigger a repaint even though \
             repository status did not change (before: {baseline_notifications}, \
             after failed write: {after_failed_write}, after recovery: {after_recovery_write})"
        );
    }

    /// Per-path flags from an independent parse of
    /// `git status --porcelain=v2 -z`: (staged, unstaged, untracked, conflicted).
    type OracleFlags = (bool, bool, bool, bool);

    /// The differential oracle for the incremental git pipeline (#675 step 1):
    /// the per-path status derived straight from git, sharing no code with
    /// `git::native::parser`, so a parser bug (#775) cannot hide by appearing
    /// identically on both sides. Same merge rule as the agent's file map:
    /// one entry per path, flags OR-merged, a conflict recorded only as a
    /// conflict (#686).
    fn oracle_files(repo: &Path) -> HashMap<PathBuf, FileStatus> {
        let output = std::process::Command::new("git")
            .args(["status", "--porcelain=v2", "-z"])
            .current_dir(repo)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("oracle git status");
        assert!(output.status.success(), "oracle git status failed");
        let text = String::from_utf8(output.stdout).expect("utf-8 status");

        let mut files: HashMap<String, OracleFlags> = HashMap::new();
        let mut records = text.split('\0');
        while let Some(record) = records.next() {
            let (path, flags) = match record.chars().next() {
                Some('1') => {
                    let mut fields = record.splitn(9, ' ');
                    let xy = fields.nth(1).unwrap_or_default();
                    let path = fields.nth(6).unwrap_or_default();
                    (path, xy_flags(xy))
                }
                Some('2') => {
                    let mut fields = record.splitn(10, ' ');
                    let xy = fields.nth(1).unwrap_or_default();
                    let path = fields.nth(7).unwrap_or_default();
                    // The rename's original path is the next NUL field.
                    records.next();
                    (path, xy_flags(xy))
                }
                Some('u') => {
                    let path = record.splitn(11, ' ').nth(10).unwrap_or_default();
                    (path, (false, false, false, true))
                }
                Some('?') => (
                    record.get(2..).unwrap_or_default(),
                    (false, false, true, false),
                ),
                _ => continue,
            };
            let entry = files.entry(path.to_owned()).or_default();
            *entry = (
                entry.0 || flags.0,
                entry.1 || flags.1,
                entry.2 || flags.2,
                entry.3 || flags.3,
            );
        }

        files
            .into_iter()
            .map(|(path, (staged, unstaged, untracked, conflicted))| {
                (
                    PathBuf::from(path),
                    FileStatus {
                        staged,
                        unstaged,
                        untracked,
                        conflicted,
                    },
                )
            })
            .collect()
    }

    fn xy_flags(xy: &str) -> OracleFlags {
        let mut chars = xy.chars();
        let staged = chars.next().is_some_and(|c| c != '.');
        let unstaged = chars.next().is_some_and(|c| c != '.');
        (staged, unstaged, false, false)
    }

    /// xorshift64: deterministic, dependency-free, so a failing seed replays.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13_u32;
            self.0 ^= self.0 >> 7_u32;
            self.0 ^= self.0 << 17_u32;
            let bound64 = u64::try_from(bound).expect("small bound");
            usize::try_from(self.0.checked_rem(bound64).expect("non-zero bound"))
                .expect("fits usize")
        }
    }

    /// Repo-relative names the generator draws from. They cover the spellings
    /// that have broken the pipeline before: a TAB (#775), a leading `:`
    /// (pathspec magic, #713), spaces, and files inside directories that do
    /// not exist yet, which git collapses into one untracked `dir/` entry
    /// (#711).
    const ORACLE_NAMES: [&str; 8] = [
        "a.txt",
        "b c.txt",
        "tab\tname.txt",
        ":colon.txt",
        "src/lib.txt",
        "newdir/x.txt",
        "newdir/deep/y.txt",
        "other/z.txt",
    ];

    /// Apply one random mutation and return the paths a watcher would report
    /// for it, plus a description for the failure log. Half the time it
    /// reuses the previous operation's file, so write → add → edit → rm
    /// chains on one path (where stale cache keys show up) are common rather
    /// than rare.
    fn apply_random_op(rng: &mut Rng, repo: &Path, previous: &mut usize) -> (Vec<PathBuf>, String) {
        if rng.below(2) == 0 {
            *previous = rng.below(ORACLE_NAMES.len());
        }
        let name = ORACLE_NAMES.get(*previous).copied().unwrap_or("a.txt");
        let path = repo.join(name);
        let index = vec![repo.join(".git/index")];
        match rng.below(6) {
            0 | 1 => {
                // Write (create or modify). A watcher reports every directory
                // it saw created plus the file itself.
                let mut reported = Vec::new();
                let mut dir = path.parent();
                while let Some(d) = dir {
                    if d == repo || d.exists() {
                        break;
                    }
                    reported.push(d.to_path_buf());
                    dir = d.parent();
                }
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).expect("create parent");
                }
                let body = format!("{}\n", rng.below(1000));
                fs::write(&path, body).expect("write file");
                reported.push(path);
                (reported, format!("write {name:?}"))
            }
            2 => {
                let _ = fs::remove_file(&path);
                // Removing the last file of a directory leaves it empty;
                // git stops reporting it, so delete it like an editor would.
                let mut reported = vec![path.clone()];
                let mut dir = path.parent();
                while let Some(d) = dir {
                    if d == repo || fs::remove_dir(d).is_err() {
                        break;
                    }
                    reported.push(d.to_path_buf());
                    dir = d.parent();
                }
                (reported, format!("rm {name:?}"))
            }
            3 => {
                git_in(repo, &["add", "--", &format!(":(literal){name}")]);
                (index, format!("git add {name:?}"))
            }
            4 => {
                git_in(
                    repo,
                    &["rm", "-q", "--cached", "--", &format!(":(literal){name}")],
                );
                (index, format!("git rm --cached {name:?}"))
            }
            _ => {
                git_in(repo, &["commit", "-q", "-m", "step", "--allow-empty"]);
                (index, "git commit".to_owned())
            }
        }
    }

    #[test]
    /// Differential test for the incremental git pipeline (#675 step 1):
    /// after every random mutation, the status the agent derives through
    /// `refresh_repo_status_with`, fed the paths a watcher would report, must
    /// equal an independent parse of a full `git status`. Fixed seeds keep it
    /// deterministic; a failure prints the seed and the operation log.
    fn incremental_refresh_matches_full_git_status_oracle() {
        for seed in 1..=24_u64 {
            let (_tmp, repo) = create_temp_repo();
            fs::create_dir_all(repo.join("src")).expect("src dir");
            fs::write(repo.join("a.txt"), "a\n").expect("seed a.txt");
            fs::write(repo.join("src/lib.txt"), "lib\n").expect("seed lib");
            commit_all(&repo);

            let ctx = make_agent_context();
            let config = Config::default();
            let backend = NativeGitBackend;
            refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut log = Vec::new();
            let mut previous = 0_usize;
            for _ in 0_u32..16_u32 {
                let (reported, op) = apply_random_op(&mut rng, &repo, &mut previous);
                log.push(op);
                let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
                let refreshed =
                    refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
                let status = refreshed.status.expect("repository status");
                let want_files = oracle_files(&repo);
                let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
                assert_eq!(
                    got_files, want_files,
                    "seed {seed}: cached file map diverged from git after {log:#?}"
                );
                let got = StatusAggregate {
                    staged: status.staged,
                    unstaged: status.unstaged,
                    untracked: status.untracked,
                    conflicts: status.conflicts,
                };
                assert_eq!(
                    got,
                    StatusAggregate::from_file_statuses(want_files.values()),
                    "seed {seed}: status counts diverged from git after {log:#?}"
                );
            }
        }
    }

    /// A parent repo with one committed submodule `sub` (holding `s.txt`).
    /// The returned `TempDir` owns the submodule source and must outlive use.
    ///
    /// # Panics
    ///
    /// Panics if any git setup step fails.
    fn repo_with_submodule() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
        let (source_tmp, source) = create_temp_repo();
        fs::write(source.join("s.txt"), "s\n").expect("write s.txt");
        commit_all(&source);

        let (parent_tmp, repo) = create_temp_repo();
        fs::write(repo.join("t.txt"), "t\n").expect("write t.txt");
        commit_all(&repo);
        let source_arg = source.to_str().expect("utf-8 source path");
        assert!(
            git_in(
                &repo,
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    "-q",
                    source_arg,
                    "sub",
                ],
            ),
            "git submodule add failed"
        );
        commit_all(&repo);
        let sub = repo.join("sub");
        assert!(git_in(&sub, &["config", "user.email", "test@example.com"]));
        assert!(git_in(&sub, &["config", "user.name", "Test"]));
        (source_tmp, parent_tmp, repo)
    }

    /// Deliver a single-path watcher event for `path` and return the cached status.
    ///
    /// # Panics
    ///
    /// Panics if the cache stays empty after the event.
    fn deliver_path(
        ctx: &AgentContext,
        repo: &Path,
        config: &Config,
        path: PathBuf,
    ) -> RepositoryStatus {
        let event = DebouncedEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(path),
            },
            repo: repo.to_path_buf(),
        };
        handle_file_event(ctx, &event, config);
        ctx.cache.get(repo).expect("status after event")
    }

    #[test]
    /// Regression test for #712: a path below a gitlink matches no pathspec in
    /// the superproject, so an edit inside a submodule must be mapped to the
    /// gitlink itself.
    fn incremental_refresh_maps_submodule_child_paths_to_gitlink() {
        let (_source, _tmp, repo) = repo_with_submodule();
        let ctx = make_agent_context();
        let config = Config::default();
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);

        let file = repo.join("sub/s.txt");
        fs::write(&file, "s\nmodified\n").expect("modify submodule file");
        let incremental = deliver_path(&ctx, &repo, &config, file);
        assert_eq!(
            incremental.unstaged, 1,
            "the submodule must show as modified"
        );

        ctx.cache.invalidate(&repo);
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        let full = ctx.cache.get(&repo).expect("status after full scan");
        assert_eq!(incremental, full);
    }

    #[test]
    /// Regression test for #712: reverting the edit must clear the cached
    /// gitlink entry instead of leaving the parent dirty.
    fn incremental_refresh_clears_gitlink_after_submodule_revert() {
        let (_source, _tmp, repo) = repo_with_submodule();
        let ctx = make_agent_context();
        let config = Config::default();

        let file = repo.join("sub/s.txt");
        fs::write(&file, "s\nmodified\n").expect("modify submodule file");
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        assert_eq!(ctx.cache.get(&repo).expect("warm").unstaged, 1);

        assert!(
            git_in(&repo.join("sub"), &["checkout", "--", "s.txt"]),
            "git checkout failed"
        );
        let incremental = deliver_path(&ctx, &repo, &config, file);
        assert_eq!(incremental.state, RepositoryState::Clean);

        ctx.cache.invalidate(&repo);
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        let full = ctx.cache.get(&repo).expect("status after full scan");
        assert_eq!(incremental, full);
    }

    #[test]
    /// Regression test for #712: a change inside a plain nested repository
    /// (no gitlink) yields the same counts as a full scan of the parent.
    fn incremental_refresh_maps_nested_repo_child_paths_to_nested_root() {
        let (_tmp, repo) = create_temp_repo();
        fs::write(repo.join("t.txt"), "t\n").expect("write t.txt");
        commit_all(&repo);
        let ctx = make_agent_context();
        let config = Config::default();
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);

        let child = repo.join("deep/child");
        fs::create_dir_all(&child).expect("create nested repo dir");
        assert!(git_in(&child, &["init", "-q"]), "git init failed");
        let file = child.join("f.txt");
        fs::write(&file, "f\n").expect("write nested file");
        let incremental = deliver_path(&ctx, &repo, &config, file);
        assert_eq!(incremental.untracked, 1);

        ctx.cache.invalidate(&repo);
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        let full = ctx.cache.get(&repo).expect("status after full scan");
        assert_eq!(incremental, full);
    }

    #[test]
    /// Differential scenario for #712: random edits, reverts, untracked files
    /// and commits inside a submodule, reported as the paths a watcher sees,
    /// must leave the parent's cached file map equal to a fresh `git status`.
    fn incremental_refresh_matches_git_status_for_submodule_edits() {
        for seed in 1..=8_u64 {
            let (_source, _tmp, repo) = repo_with_submodule();
            let sub = repo.join("sub");
            let ctx = make_agent_context();
            let config = Config::default();
            let backend = NativeGitBackend;
            refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut log = Vec::new();
            for _ in 0_u32..10_u32 {
                let (reported, op) = match rng.below(4) {
                    0 => {
                        let file = sub.join("s.txt");
                        fs::write(&file, format!("{}\n", rng.below(1000))).expect("write");
                        (vec![file], "edit s.txt")
                    }
                    1 => {
                        git_in(&sub, &["checkout", "--", "s.txt"]);
                        (vec![sub.join("s.txt")], "revert s.txt")
                    }
                    2 => {
                        let file = sub.join("extra/new.txt");
                        fs::create_dir_all(sub.join("extra")).expect("mkdir");
                        fs::write(&file, "n\n").expect("write");
                        (vec![sub.join("extra"), file], "add untracked")
                    }
                    _ => {
                        git_in(&sub, &["commit", "-q", "-a", "-m", "step", "--allow-empty"]);
                        (vec![sub.join("s.txt")], "commit in submodule")
                    }
                };
                log.push(op);
                let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
                let refreshed =
                    refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
                let status = refreshed.status.expect("repository status");
                let want_files = oracle_files(&repo);
                let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
                assert_eq!(
                    got_files, want_files,
                    "seed {seed}: cached file map diverged from git after {log:#?}"
                );
                assert_eq!(
                    StatusAggregate {
                        staged: status.staged,
                        unstaged: status.unstaged,
                        untracked: status.untracked,
                        conflicts: status.conflicts,
                    },
                    StatusAggregate::from_file_statuses(want_files.values()),
                    "seed {seed}: status counts diverged from git after {log:#?}"
                );
            }
        }
    }

    /// Whether git treats `repo` as case-insensitive (`core.ignorecase=true`),
    /// the setting `git init` derives from the volume.
    ///
    /// # Panics
    ///
    /// Panics if `git config` cannot be spawned.
    fn repo_ignores_case(repo: &Path) -> bool {
        let output = std::process::Command::new("git")
            .args(["config", "--type=bool", "--default", "false"])
            .arg("core.ignorecase")
            .current_dir(repo)
            .output()
            .expect("git config");
        String::from_utf8_lossy(&output.stdout).trim() == "true"
    }

    /// Rename `from` to `to` (which differ only in case) via a temporary
    /// name, as the issue's reproduction does, so it works on every volume.
    ///
    /// # Panics
    ///
    /// Panics if either rename fails.
    fn rename_via_temp(repo: &Path, from: &str, to: &str) {
        let temp = repo.join("case-rename.tmp");
        fs::rename(repo.join(from), &temp).expect("rename to temp");
        fs::rename(&temp, repo.join(to)).expect("rename from temp");
    }

    #[test]
    /// Regression test for #713: git indexes `Readme.md` while the disk (and
    /// so the watcher) spells it `README.md`. On a case-insensitive repository
    /// the edit must still be seen, and the revert must clear it.
    fn incremental_refresh_handles_index_case_mismatch() {
        let (_tmp, repo) = create_temp_repo();
        if !repo_ignores_case(&repo) {
            return;
        }
        fs::write(repo.join("Readme.md"), "a\n").expect("write Readme.md");
        commit_all(&repo);
        rename_via_temp(&repo, "Readme.md", "README.md");

        let ctx = make_agent_context();
        let config = Config::default();
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        assert_eq!(
            ctx.cache.get(&repo).expect("warm").state,
            RepositoryState::Clean
        );

        let file = repo.join("README.md");
        fs::write(&file, "b\n").expect("modify README.md");
        let edited = deliver_path(&ctx, &repo, &config, file.clone());
        assert_eq!(edited.unstaged, 1, "the edit must be seen incrementally");
        ctx.cache.invalidate(&repo);
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        assert_eq!(edited, ctx.cache.get(&repo).expect("full scan"));

        fs::write(&file, "a\n").expect("restore README.md");
        let reverted = deliver_path(&ctx, &repo, &config, file);
        assert_eq!(
            reverted.state,
            RepositoryState::Clean,
            "the revert must clear the entry cached under the index spelling"
        );
        ctx.cache.invalidate(&repo);
        handle_file_event(&ctx, &whole_repo_event(&repo), &config);
        assert_eq!(reverted, ctx.cache.get(&repo).expect("full scan"));
    }

    /// A tracked path that the case-variant differential test can respell on
    /// disk: `index` as committed, `variant` the other spelling of its first
    /// component, `tail` the unchanged remainder below it.
    struct CaseSlot {
        index: &'static str,
        variant: &'static str,
        tail: &'static str,
        on_disk: &'static str,
    }

    impl CaseSlot {
        fn file(&self, repo: &Path) -> PathBuf {
            repo.join(format!("{}{}", self.on_disk, self.tail))
        }
    }

    /// Apply one random mutation to the case-variant scenario and return the
    /// paths a watcher would report for it, plus a description for the log.
    fn apply_case_op(rng: &mut Rng, repo: &Path, slots: &mut [CaseSlot]) -> (Vec<PathBuf>, String) {
        let choice = rng.below(slots.len());
        let slot = slots.get_mut(choice).expect("slot in range");
        match rng.below(7) {
            0 | 1 => {
                let file = slot.file(repo);
                fs::write(&file, format!("{}\n", rng.below(1000))).expect("write");
                (vec![file], format!("edit {}", slot.on_disk))
            }
            2 => {
                let file = slot.file(repo);
                fs::write(&file, "orig\n").expect("restore");
                (vec![file], format!("revert {}", slot.on_disk))
            }
            3 | 4 => {
                let old = repo.join(slot.on_disk);
                let target = if slot.on_disk == slot.index {
                    slot.variant
                } else {
                    slot.index
                };
                rename_via_temp(repo, slot.on_disk, target);
                let op = format!("rename {} -> {target}", slot.on_disk);
                slot.on_disk = target;
                (vec![old, repo.join(target), slot.file(repo)], op)
            }
            5 => {
                git_in(repo, &["add", "-u"]);
                (vec![repo.join(".git/index")], "git add -u".to_owned())
            }
            _ => {
                git_in(repo, &["commit", "-q", "-m", "step", "--allow-empty"]);
                (vec![repo.join(".git/index")], "git commit".to_owned())
            }
        }
    }

    #[test]
    /// Differential scenario for #713: random edits, reverts, case-only
    /// renames of files and of a directory, `git add -u` and commits, reported
    /// as the on-disk spellings a watcher sees, must leave the cached file map
    /// equal to a fresh `git status` on a case-insensitive repository.
    fn incremental_refresh_matches_git_status_for_case_variant_edits() {
        for seed in 1..=12_u64 {
            let (_tmp, repo) = create_temp_repo();
            if !repo_ignores_case(&repo) {
                return;
            }
            let mut slots = [
                ("Readme.md", "README.md", ""),
                ("Notes.txt", "NOTES.TXT", ""),
                ("Src", "SRC", "/lib.txt"),
            ]
            .map(|(index, variant, tail)| CaseSlot {
                index,
                variant,
                tail,
                on_disk: index,
            });
            fs::create_dir_all(repo.join("Src")).expect("src dir");
            for slot in &slots {
                fs::write(slot.file(&repo), "orig\n").expect("seed file");
            }
            commit_all(&repo);

            let ctx = make_agent_context();
            let config = Config::default();
            let backend = NativeGitBackend;
            refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut log = Vec::new();
            for _ in 0_u32..14_u32 {
                let (reported, op) = apply_case_op(&mut rng, &repo, &mut slots);
                log.push(op);
                let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
                let refreshed =
                    refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
                let status = refreshed.status.expect("repository status");
                let want_files = oracle_files(&repo);
                let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
                assert_eq!(
                    got_files, want_files,
                    "seed {seed}: cached file map diverged from git after {log:#?}"
                );
                assert_eq!(
                    StatusAggregate {
                        staged: status.staged,
                        unstaged: status.unstaged,
                        untracked: status.untracked,
                        conflicts: status.conflicts,
                    },
                    StatusAggregate::from_file_statuses(want_files.values()),
                    "seed {seed}: status counts diverged from git after {log:#?}"
                );
            }
        }
    }
}

/// Deterministic (timing-free) tests for the #418 per-repo refresh
/// serialization. These exercise [`RepoRefreshCoordinator`] and
/// [`run_coalesced`] directly with fake scans, so the concurrency-safety logic
/// is proven without racing real git subprocesses. Any real interleaving is
/// forced via rendezvous channels, never sleeps.
#[cfg(test)]
mod coordinator_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{MAX_COALESCED_FOLLOW_UPS, RepoRefreshCoordinator, run_coalesced};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;

    /// State-machine basics: the first caller is granted the in-flight slot; a
    /// second concurrent caller for the same repo is refused and instead records
    /// a pending follow-up.
    #[test]
    fn try_start_grants_first_caller_and_records_pending() {
        let coordinator = RepoRefreshCoordinator::default();
        let repo = PathBuf::from("/repo/state");

        assert!(coordinator.try_start(&repo), "first caller runs");
        assert!(
            !coordinator.try_start(&repo),
            "second concurrent caller for the same repo is refused"
        );
        // The refused caller recorded a pending follow-up, so finish must ask
        // for exactly one more pass, then report done and clean up.
        assert!(coordinator.finish(&repo), "pending follow-up owed");
        assert!(!coordinator.finish(&repo), "no further work; entry removed");
        // Entry cleaned up: a brand-new refresh is granted again.
        assert!(coordinator.try_start(&repo));
        coordinator.abandon(&repo);
    }

    /// Cross-repo concurrency at the state-machine level (regression guard for
    /// #306): an in-flight refresh for repo A never blocks or refuses a refresh
    /// for a *different* repo B.
    #[test]
    fn different_repos_never_serialize_in_state_machine() {
        let coordinator = RepoRefreshCoordinator::default();
        let repo_a = PathBuf::from("/repo/a");
        let repo_b = PathBuf::from("/repo/b");

        assert!(coordinator.try_start(&repo_a));
        // A is in flight and NOT finished; B must still be granted immediately.
        assert!(
            coordinator.try_start(&repo_b),
            "a different repo must never be serialized behind an in-flight repo"
        );
        assert!(!coordinator.finish(&repo_a));
        assert!(!coordinator.finish(&repo_b));
    }

    /// Two concurrent same-repo events arriving mid-scan must coalesce into
    /// exactly ONE follow-up pass, not one per event.
    #[test]
    fn run_coalesced_coalesces_concurrent_events_into_single_follow_up() {
        let coordinator = RepoRefreshCoordinator::default();
        let repo = PathBuf::from("/repo/single");
        let mut runs = 0_u32;

        run_coalesced(&coordinator, &repo, |_is_follow_up| {
            runs += 1;
            if runs == 1 {
                // Simulate two concurrent same-repo events landing during the
                // initial pass: both are coalesced (refused) and together owe
                // exactly one follow-up.
                assert!(!coordinator.try_start(&repo));
                assert!(!coordinator.try_start(&repo));
            }
        });

        assert_eq!(
            runs, 2,
            "one initial pass plus exactly one coalesced follow-up"
        );
        // Bookkeeping entry removed once drained.
        assert!(coordinator.try_start(&repo));
        coordinator.abandon(&repo);
    }

    /// Core #418 guarantee, forced deterministically with channels: a
    /// slower-finishing-but-OLDER scan can never leave its stale result in the
    /// cache. Only one scan runs at a time, and a change arriving mid-scan drives
    /// a fresh follow-up that re-reads the newest state.
    #[test]
    fn run_coalesced_stale_scan_cannot_overwrite_fresher() {
        let coordinator = Arc::new(RepoRefreshCoordinator::default());
        let repo = PathBuf::from("/repo/interleave");
        // "disk" is the repo's true state; each scan reads it and writes it into
        // "cache". Starts at the older value (1).
        let disk = Arc::new(AtomicUsize::new(1));
        let cache = Arc::new(AtomicUsize::new(0));

        let (pass1_reading_tx, pass1_reading_rx) = mpsc::channel::<()>();
        let (release_pass1_tx, release_pass1_rx) = mpsc::channel::<()>();

        let owner = {
            let thread_coordinator = Arc::clone(&coordinator);
            let thread_repo = repo.clone();
            let thread_disk = Arc::clone(&disk);
            let thread_cache = Arc::clone(&cache);
            thread::spawn(move || {
                let mut pass = 0_u32;
                run_coalesced(&thread_coordinator, &thread_repo, |_is_follow_up| {
                    // Every pass re-reads fresh disk state, as a real scan would.
                    let value = thread_disk.load(Ordering::Acquire);
                    thread_cache.store(value, Ordering::Release);
                    pass += 1;
                    if pass == 1 {
                        // Pass 1 has captured the OLD value. Block so the test can
                        // advance disk and inject a concurrent event before this
                        // (older) pass is allowed to complete.
                        pass1_reading_tx.send(()).expect("announce pass 1");
                        release_pass1_rx.recv().expect("await pass 1 release");
                    }
                });
            })
        };

        // Pass 1 has written the stale value.
        pass1_reading_rx.recv().expect("pass 1 should start");
        assert_eq!(
            cache.load(Ordering::Acquire),
            1,
            "pass 1 wrote the older (stale) value"
        );

        // A newer change lands while pass 1 is still in flight. Simulate the
        // concurrent watcher event for the SAME repo: it must be coalesced (never
        // run its own parallel scan) and record a pending follow-up.
        disk.store(2, Ordering::Release);
        assert!(
            !coordinator.try_start(&repo),
            "a concurrent same-repo refresh must be coalesced, not run in parallel"
        );

        // Let the older pass 1 finish. The pending flag now forces exactly one
        // follow-up, which re-reads the newest disk state (2).
        release_pass1_tx.send(()).expect("release pass 1");
        owner.join().expect("owner thread panicked");

        assert_eq!(
            cache.load(Ordering::Acquire),
            2,
            "the fresher state must win; the stale pass 1 result must not survive"
        );
    }

    /// Cross-repo concurrency with real threads (regression guard for #306):
    /// while repo A's scan is blocked indefinitely, repo B's coalesced refresh
    /// still runs to completion promptly rather than serializing behind A.
    #[test]
    fn run_coalesced_different_repos_run_concurrently() {
        let coordinator = Arc::new(RepoRefreshCoordinator::default());
        let repo_a = PathBuf::from("/repo/a");
        let repo_b = PathBuf::from("/repo/b");

        let (a_started_tx, a_started_rx) = mpsc::channel::<()>();
        let (release_a_tx, release_a_rx) = mpsc::channel::<()>();

        let handle_a = {
            let thread_coordinator = Arc::clone(&coordinator);
            let thread_repo_a = repo_a;
            thread::spawn(move || {
                run_coalesced(&thread_coordinator, &thread_repo_a, |_is_follow_up| {
                    a_started_tx.send(()).expect("announce repo A scan");
                    release_a_rx.recv().expect("await repo A release");
                });
            })
        };

        // Repo A is now mid-scan (and the coordinator holds no lock across it).
        a_started_rx.recv().expect("repo A scan should start");

        // Repo B must complete even though repo A is still blocked.
        let b_ran = AtomicBool::new(false);
        run_coalesced(&coordinator, &repo_b, |_is_follow_up| {
            b_ran.store(true, Ordering::Release);
        });
        assert!(
            b_ran.load(Ordering::Acquire),
            "repo B must run to completion while repo A's scan is still blocked"
        );

        // Release repo A so its thread can join.
        release_a_tx.send(()).expect("release repo A");
        handle_a.join().expect("repo A thread panicked");
    }

    /// The follow-up cap must stop an otherwise-unbounded coalesce loop under
    /// sustained same-repo churn and clean up the bookkeeping entry so the repo
    /// is not wedged forever.
    #[test]
    fn run_coalesced_follow_up_cap_prevents_unbounded_loop() {
        let coordinator = RepoRefreshCoordinator::default();
        let repo = PathBuf::from("/repo/churn");
        let mut runs = 0_u32;

        run_coalesced(&coordinator, &repo, |_is_follow_up| {
            runs += 1;
            // Every scan re-arms pending, so without the cap this would spin
            // forever.
            coordinator.try_start(&repo);
        });

        assert_eq!(
            runs,
            MAX_COALESCED_FOLLOW_UPS + 2,
            "initial pass plus exactly MAX_COALESCED_FOLLOW_UPS follow-ups, plus the \
             final cap-triggered scan that captures the last pending state before \
             abandoning (#433)"
        );
        // The cap path abandons the entry; the repo must be runnable again.
        assert!(
            coordinator.try_start(&repo),
            "repo must be runnable again after the cap-triggered abandon"
        );
        coordinator.abandon(&repo);
    }

    /// #433 regression: when a churn burst is enough to trip the follow-up cap
    /// but then genuinely settles (no further changes arrive), the final
    /// pending state recorded right at the cap must still be captured by the
    /// extra cap-triggered scan, not silently dropped.
    ///
    /// Models a "disk" value that each churn pass bumps *after* the current
    /// scan has already read the old value (mirroring a real change landing
    /// concurrently with / just after a scan) and re-arms pending so the
    /// coordinator schedules exactly one more pass. Churn stops after
    /// `churn_passes` bumps, so the last bumped value is the true settled
    /// state. Without the fix, the scan that would have observed it never
    /// runs, so `captured` would be stuck one bump behind.
    #[test]
    fn run_coalesced_cap_scan_captures_settled_state_after_burst() {
        let coordinator = RepoRefreshCoordinator::default();
        let repo = PathBuf::from("/repo/settles-after-cap");

        let disk = AtomicUsize::new(0);
        let captured = AtomicUsize::new(0);
        let mut calls = 0_usize;

        // Initial pass + every follow-up: the same burst size that trips the
        // cap in `run_coalesced_follow_up_cap_prevents_unbounded_loop`.
        let churn_passes = usize::try_from(MAX_COALESCED_FOLLOW_UPS + 1).expect("fits usize");

        run_coalesced(&coordinator, &repo, |_is_follow_up| {
            calls += 1;
            // Observe whatever is on disk as of when this scan started, before
            // this pass's own churn (if any) lands.
            captured.store(disk.load(Ordering::SeqCst), Ordering::SeqCst);

            if calls <= churn_passes {
                // A new change lands right after this scan started: bump disk
                // and re-arm pending so the coordinator schedules one more
                // pass. After `churn_passes` calls, churn genuinely stops --
                // no further bumps or re-arming.
                disk.store(calls, Ordering::SeqCst);
                coordinator.try_start(&repo);
            }
        });

        assert_eq!(
            calls,
            churn_passes + 1,
            "the final cap-triggered scan must run to observe the settled state"
        );
        assert_eq!(
            captured.load(Ordering::SeqCst),
            churn_passes,
            "the final cap scan must observe the settled disk state written by the \
             last churn pass, proving it is not dropped when the cap is hit"
        );
    }
}
