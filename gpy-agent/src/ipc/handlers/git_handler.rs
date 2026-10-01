//! IPC handler for Git status requests.
//!
//! This handler validates the requested working directory, serves fresh results
//! from [`crate::git::cache::GitStatusCache`] when possible, and falls back to
//! native Git status detection when the cache is stale or missing. Response
//! formatting is delegated to the formatter layer after the domain result is built.

use super::{
    HandlerError, JobGuard, RenderDeps, RequestHandler, SharedJob, SingleFlight, notify_if_changed,
    spawn_detached,
};
use crate::config::Config;
use crate::git::{self, cache::GitStatusCache};
use crate::ipc::{Message, Response};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Upper bound on how old a stale cache entry may be before it is still
/// served immediately during the progressive-timeout fallback below.
///
/// This bound only applies once the fast 100ms window has already elapsed
/// *and* the earlier `get_canonical` (TTL-respecting) lookup already missed
/// -- i.e. the entry is already past the 30s `CachePolicy::git_status` TTL --
/// so the bound must be strictly greater than that TTL. A small multiple
/// (2x = 60s) keeps today's "prefer stale over blocking" behavior for
/// entries that are only a little past TTL (the common case: a background
/// recompute is already in flight and will land shortly), while refusing to
/// hand back arbitrarily old data immediately on a persistently slow repo
/// (#433).
const MAX_IMMEDIATE_STALE_AGE: Duration = Duration::from_secs(60);

/// Handler for Git repository status queries
pub struct GitHandler {
    git_cache: Arc<GitStatusCache>,
    /// Everything needed to publish a status to the instant-prompt cache and
    /// repaint live shells; see [`publish_and_repaint`].
    render: RenderDeps,
    in_flight: SingleFlight<PathBuf, SharedGitStatusResult>,
}

type SharedGitStatusResult = Result<Option<git::RepositoryStatus>, String>;

struct GitStatusJob {
    path: PathBuf,
    max_ahead_behind: usize,
    git_root: PathBuf,
    cache: Arc<GitStatusCache>,
    render: RenderDeps,
    /// Completes the shared job and frees the single-flight slot. Guarantees
    /// cleanup even if `load_repository_state` panics on the blocking thread
    /// below, degrading to an error result instead of poisoning the slot
    /// for this repo (#318).
    guard: JobGuard<PathBuf, SharedGitStatusResult>,
}

/// Write the instant-prompt cache for `status` and repaint live shells (forced
/// doorbell) only when the rendered output actually changed.
///
/// The single publish-and-repaint implementation for git status (#587): the
/// synchronous IPC request path, the background single-flight job, and the
/// client-registration initial scan all call this instead of each carrying
/// their own copy of the read-config/theme/palette → write → notify-or-log
/// sequence. `prev_bg` is the requesting shell's previous background color when
/// there is a request context, and `None` for background work.
pub(super) fn publish_and_repaint(
    deps: &RenderDeps,
    git_root: &Path,
    status: &git::RepositoryStatus,
    prev_bg: Option<&str>,
) {
    let config = deps.config_manager.get();
    let theme = deps.theme_manager.get();
    let palette = deps.palette_cache.get();

    let result = deps
        .instant_cache
        .write_git(git_root, status, &config, &theme, prev_bg, &palette);
    notify_if_changed(&deps.client_registry, git_root, result, "git");
}

impl GitHandler {
    /// Create a new `GitHandler` with the given dependencies
    #[must_use]
    pub fn new(render: RenderDeps, git_cache: Arc<GitStatusCache>) -> Self {
        Self {
            git_cache,
            render,
            in_flight: SingleFlight::new(),
        }
    }

    /// Check if git detection is enabled for the given path
    fn is_git_enabled(&self, path: &Path) -> bool {
        let config = self.render.config_manager.get();
        if !config.git.enabled {
            return false;
        }

        // Check if path should be skipped
        for skip_path in &config.git.skip_paths {
            if path.starts_with(skip_path) {
                return false;
            }
        }

        true
    }

    /// Get git status for a path, checking cache first
    ///
    /// # Errors
    ///
    /// Returns an error if git detection is disabled, the path should be skipped,
    /// the path is not in a git repository, or git status collection fails.
    fn get_git_status(
        &self,
        path: &crate::security::SafePath,
        prev_bg: Option<&str>,
    ) -> Result<Response, HandlerError> {
        let path_buf = path.as_path();

        // Check if git detection is enabled. `is_git_enabled` already rejects
        // any path under a configured `git.skip_paths` prefix (same
        // `Path::starts_with` scan over the same strings), so a second skip-path
        // scan here would be dead code — the disabled-via-config error below is
        // always returned first for a skipped path.
        if !self.is_git_enabled(path_buf) {
            return Err(HandlerError::FeatureDisabled("Git"));
        }

        let config = self.render.config_manager.get();

        // Find git root for cache key. `git::find_repo_root` tries the cheap
        // `.git`-only walk first, falling back to bare-repo detection (root
        // directly contains HEAD/objects/refs/config, no `.git` subentry) so
        // this IPC path stays at parity with the oneshot path (#321). This only
        // affects status detection -- bare repos still aren't registered for
        // live file watching, since `MultiRepoWatcher`'s registration path is
        // unchanged.
        let git_root_opt = git::find_repo_root(path_buf);
        let Some(git_root) = git_root_opt else {
            return Err(HandlerError::NotInRepository);
        };

        // Check cache first. `git_root` is already canonical (both
        // `find_git_root` and `find_repo_root` canonicalize), so use the fast
        // path to avoid a redundant syscall.
        if let Some(cached_status) = self.git_cache.get_canonical(&git_root) {
            // Write instant-prompt cache to ensure it's fresh
            publish_and_repaint(&self.render, &git_root, &cached_status, prev_bg);

            // Opportunistically revalidate in the background: the watcher may
            // have missed the event that made this entry stale (submodule /
            // FSEvents-drop cases), so a within-TTL hit alone would serve the
            // same stale status for up to the full TTL. This spawns a
            // fire-and-forget recompute (deduped by `in_flight`, diff-gated
            // repaint) when the entry is past its cooldown, adding zero latency
            // to the hot path — the returned handle is dropped, never awaited
            // here — while still returning the cached value immediately below.
            let _ = self.revalidate_stale_hit(&git_root, path_buf, config.git.max_ahead_behind);

            return Ok(Self::make_git_response(
                cached_status,
                config.git.show_upstream,
            ));
        }

        // Cache miss - compute fresh status with progressive timeout
        let path_owned = path_buf.to_path_buf();
        let max_ahead_behind = config.git.max_ahead_behind;

        let job = self.git_status_job(&git_root, path_owned, max_ahead_behind);

        // Progressive timeout: 100ms
        job.wait_timeout(Duration::from_millis(100)).map_or_else(
            || self.handle_git_status_timeout(&job, &git_root, &config, prev_bg),
            |result| match result {
                Ok(Some(status)) => {
                    // Write instant-prompt cache with fresh status
                    publish_and_repaint(&self.render, &git_root, &status, prev_bg);

                    Ok(Self::make_git_response(status, config.git.show_upstream))
                }
                Ok(None) => Err(HandlerError::NotInRepository),
                Err(e) => Err(HandlerError::Git(e)),
            },
        )
    }

    /// Handle the case where the fast 100ms progressive-timeout window
    /// elapsed without the in-flight `job` producing a result yet.
    ///
    /// Serves a stale cache hit immediately only when it is within
    /// [`MAX_IMMEDIATE_STALE_AGE`] of "now" -- otherwise the entry is too old
    /// to trust without first giving the already-in-flight recompute a
    /// chance to land, so this waits up to the configured hard timeout for
    /// it, falling back to the very-stale value (better than nothing) only if
    /// that recompute also times out or errors (#433).
    ///
    /// # Errors
    ///
    /// Returns an error if the repository is no longer a git repository, the
    /// in-flight recompute fails, or it times out with no stale value to fall
    /// back to.
    fn handle_git_status_timeout(
        &self,
        job: &Arc<SharedJob<SharedGitStatusResult>>,
        git_root: &Path,
        config: &Config,
        prev_bg: Option<&str>,
    ) -> Result<Response, HandlerError> {
        if let Some((stale, age)) = self.git_cache.get_any_with_age_canonical(git_root)
            && age <= MAX_IMMEDIATE_STALE_AGE
        {
            // Write instant-prompt cache with stale data (better than nothing)
            publish_and_repaint(&self.render, git_root, &stale, prev_bg);

            return Ok(Self::make_git_response(stale, config.git.show_upstream));
        }

        // No stale cache, or it's too old to trust immediately: wait up to the
        // configured git timeout (+500ms margin) for the in-flight recompute.
        let hard_timeout = Duration::from_secs(config.git.timeout_seconds.get())
            .saturating_add(Duration::from_millis(500));
        match job.wait_timeout(hard_timeout) {
            Some(Ok(Some(status))) => {
                // Write instant-prompt cache with delayed status
                publish_and_repaint(&self.render, git_root, &status, prev_bg);

                Ok(Self::make_git_response(status, config.git.show_upstream))
            }
            Some(Ok(None)) => Err(HandlerError::NotInRepository),
            Some(Err(e)) => Err(HandlerError::Git(e)),
            None => {
                // The recompute also timed out: fall back to the very-stale
                // value if one exists (better than nothing), else the
                // existing timeout error.
                self.git_cache.get_any_canonical(git_root).map_or_else(
                    || Err(HandlerError::TimedOut("Git status")),
                    |stale| {
                        publish_and_repaint(&self.render, git_root, &stale, prev_bg);

                        Ok(Self::make_git_response(stale, config.git.show_upstream))
                    },
                )
            }
        }
    }

    /// Opportunistically revalidate a within-TTL cache hit when the entry is
    /// past its cooldown.
    ///
    /// The file watcher may have missed the event that changed the working tree
    /// (submodule / dropped-FSEvents cases), leaving a stale entry that a plain
    /// hit would keep serving until TTL expiry. This recomputes in the
    /// background — deduped by `in_flight` (no double-compute), with a
    /// content-change-gated repaint via `publish_and_repaint` — without blocking
    /// the request path.
    ///
    /// The cooldown gate (via [`GitStatusCache::is_fresh_canonical`]) throttles
    /// this to at most one revalidation per cooldown window per repo: every
    /// completed job calls `cache.set`, resetting `cached_at`, so rapid Enter
    /// presses collapse to a single background recompute.
    ///
    /// Returns the spawned job handle (for tests) or `None` when throttled by
    /// the cooldown. NEVER awaited on the request path.
    #[must_use]
    fn revalidate_stale_hit(
        &self,
        git_root: &Path,
        path: &Path,
        max_ahead_behind: usize,
    ) -> Option<Arc<SharedJob<SharedGitStatusResult>>> {
        if self.git_cache.is_fresh_canonical(git_root) {
            return None;
        }
        Some(self.git_status_job(git_root, path.to_path_buf(), max_ahead_behind))
    }

    fn git_status_job(
        &self,
        git_root: &Path,
        path: PathBuf,
        max_ahead_behind: usize,
    ) -> Arc<SharedJob<SharedGitStatusResult>> {
        let cache = Arc::clone(&self.git_cache);
        let render = self.render.clone();
        let git_root_owned = git_root.to_path_buf();
        // On a panic mid-job, the guard's Drop completes the shared job with
        // this error instead of leaving the slot permanently poisoned (#318).
        let fallback: SharedGitStatusResult = Err("git status job panicked".to_owned());
        self.in_flight
            .get_or_start(git_root_owned.clone(), fallback, move |guard| {
                Self::spawn_git_status_job(GitStatusJob {
                    path,
                    max_ahead_behind,
                    git_root: git_root_owned,
                    cache,
                    render,
                    guard,
                });
            })
    }

    fn spawn_git_status_job(job_spec: GitStatusJob) {
        let job = move || {
            let stash_enabled = job_spec.render.config_manager.get().git.stash_enabled;
            let result = crate::git::status::load_repository_state_with(
                &crate::git::native::NativeGitBackend,
                &job_spec.path,
                None,
                job_spec.max_ahead_behind,
                stash_enabled,
                Duration::from_secs(
                    job_spec
                        .render
                        .config_manager
                        .get()
                        .git
                        .timeout_seconds
                        .get(),
                ),
            );
            let shared_result = match result {
                Ok(Some(crate::git::CompleteStatus { status, files })) => {
                    job_spec
                        .cache
                        .set(&job_spec.git_root, status.clone(), files);
                    // The synchronous handler path may have already returned a
                    // stale prompt to a backgrounded refresh (serve-stale, #160).
                    // Refresh the instant cache here so the freshly computed
                    // status reaches disk, and repaint live shells when the
                    // rendered output actually changed.
                    //
                    // Background job: no IPC request context, so prev_bg is
                    // None (graceful fallback).
                    publish_and_repaint(&job_spec.render, &job_spec.git_root, &status, None);
                    Ok(Some(status))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(e.to_string()),
            };

            job_spec.guard.finish(shared_result);
        };

        spawn_detached(job);
    }

    /// Convert git status to IPC response
    const fn make_git_response(status: git::RepositoryStatus, show_upstream: bool) -> Response {
        let response_status = if show_upstream {
            status
        } else {
            status.without_upstream()
        };

        Response::RepositoryStatus(response_status)
    }
}

impl RequestHandler for GitHandler {
    fn handle(&self, message: &Message) -> Result<Response, HandlerError> {
        match message {
            Message::RepositoryStatus { path, prev_bg, .. } => {
                // Thread the raw wire string: `write_git` uses it as the single
                // source of truth for both the rendered chevron color and the
                // cache-key token, so the reading shell resolves the same file.
                self.get_git_status(path, prev_bg.as_deref())
            }
            _ => Err(HandlerError::UnexpectedMessage {
                handler: "GitHandler",
                qualifier: "non-git",
                message: format!("{message:?}"),
            }),
        }
    }

    fn name(&self) -> &'static str {
        "GitHandler"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    use super::*;
    use crate::cache::{CachePolicy, InstantPromptCache};
    use crate::config::manager::ConfigManager;
    use crate::git::status::load_repository_state;
    use crate::git::{RepositoryState, RepositoryStatus};
    use crate::ipc::ClientDirectory;
    use crate::palette::PaletteCache;
    use crate::theme::ThemeManager;
    use std::collections::HashMap;
    use std::time::Duration;

    fn make_handler() -> (GitHandler, Arc<ClientDirectory>) {
        make_handler_with_cache(Arc::new(GitStatusCache::new()))
    }

    fn make_handler_with_cache(
        git_cache: Arc<GitStatusCache>,
    ) -> (GitHandler, Arc<ClientDirectory>) {
        let registry = ClientDirectory::new().shared();
        let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
        let palette_cache = Arc::new(PaletteCache::from_config(&config_manager.get()));
        let render = RenderDeps {
            config_manager: Arc::clone(&config_manager),
            // Use the embedded builtin theme so the test is hermetic against a
            // stale on-disk ~/.config/gpy/themes/default.toml (which may predate
            // the format+palette migration and render empty segments).
            theme_manager: Arc::new(ThemeManager::builtin("default").expect("theme")),
            palette_cache,
            instant_cache: Arc::new(InstantPromptCache::new_for_test()),
            client_registry: Arc::clone(&registry),
        };
        let handler = GitHandler::new(render, git_cache);
        (handler, registry)
    }

    /// Cache with a 0ms cooldown so every seeded entry is immediately past its
    /// cooldown window, making the cooldown-gated `revalidate_stale_hit` path
    /// deterministically fire without any sleeps.
    fn zero_cooldown_cache() -> Arc<GitStatusCache> {
        let policy = CachePolicy::builder()
            .ttl(Duration::from_secs(30))
            .cooldown(Duration::from_millis(0))
            .build()
            .expect("cache policy");
        Arc::new(GitStatusCache::with_policy(policy))
    }

    /// Create a real repository with a dirty working tree (one untracked file).
    /// Returns the canonical worktree root, matching the key `get_git_status`
    /// derives via `find_git_root` (which canonicalizes).
    fn init_dirty_repo(temp_dir: &tempfile::TempDir) -> PathBuf {
        let repo = temp_dir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");

        let output = std::process::Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(&repo)
            .output()
            .expect("run git init");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // An untracked file makes the true status Dirty.
        std::fs::write(repo.join("untracked.txt"), "hello").expect("write untracked file");

        std::fs::canonicalize(&repo).expect("canonicalize repo root")
    }

    fn status(untracked: u32) -> RepositoryStatus {
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
            state: if untracked > 0 {
                RepositoryState::Dirty
            } else {
                RepositoryState::Clean
            },
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }
    }

    /// The serve-stale repaint contract (#160).
    ///
    /// Writing the instant cache rings a forced repaint doorbell only when the rendered
    /// output actually changed, so no-op refreshes never wake terminals
    /// (preserving #145/#146 clock gating).
    #[test]
    fn publish_and_repaint_notifies_only_on_content_change() {
        let (handler, registry) = make_handler();
        // A unique temp path keeps the instant-cache key (and on-disk file)
        // isolated from other runs so the first write is always a change.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(git_root.join(".git")).expect("git dir");

        let clean = status(0);
        let dirty = status(3);

        // First write of fresh content -> repaint.
        publish_and_repaint(&handler.render, &git_root, &clean, None);
        assert_eq!(
            registry.notify_invocations(),
            1,
            "first write should repaint live shells"
        );

        // Identical status -> no repaint (change-gated).
        publish_and_repaint(&handler.render, &git_root, &clean, None);
        assert_eq!(
            registry.notify_invocations(),
            1,
            "an unchanged refresh must not wake terminals"
        );

        // Changed status -> repaint.
        publish_and_repaint(&handler.render, &git_root, &dirty, None);
        assert_eq!(
            registry.notify_invocations(),
            2,
            "a changed status should repaint live shells"
        );
    }

    /// #321: a bare repository has no `.git` subentry.
    ///
    /// Its root directly contains HEAD/objects/refs/config, so
    /// `MultiRepoWatcher::find_git_root`'s `.git`-only walk misses it and
    /// `get_git_status` used to short-circuit with "Not in a git repository"
    /// before ever resolving a root.
    ///
    /// `git status` itself refuses to run without a worktree ("fatal: this
    /// operation must be run in a work tree"), so a bare repo can never render
    /// a full status on *either* path -- that's a `git` limitation, not a gpy
    /// bug. The fix here is root-resolution parity: `get_git_status` now uses
    /// `crate::git::find_repo_root`, whose bare-repo fallback the oneshot path
    /// reaches too (`agent/oneshot.rs` -> `load_repository_state` ->
    /// `get_complete_status` -> `find_repo_root`), so both paths reach the *same* git-subprocess
    /// error instead of diverging on a premature "Not in a git repository"
    /// from the agent IPC path alone.
    #[test]
    fn get_git_status_resolves_bare_repository_root_like_oneshot() {
        let (handler, _registry) = make_handler();
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let bare_path = temp_dir.path().join("bare.git");
        std::fs::create_dir_all(&bare_path).expect("bare dir");

        let output = std::process::Command::new("git")
            .args(["init", "--bare", "--initial-branch=main"])
            .arg(&bare_path)
            .output()
            .expect("run git init --bare");
        assert!(
            output.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let safe_path = crate::security::SafePath::new(bare_path.to_str().expect("utf8 path"))
            .expect("safe path");

        // Same path, same failure the oneshot path already hits via
        // `load_repository_state` -- proves the handler no longer bails out
        // early on an unresolved root.
        let handler_result = handler.get_git_status(&safe_path, None);
        let oneshot_result = load_repository_state(&bare_path, None, 0, false);

        let Err(handler_error) = handler_result else {
            panic!("expected an error (git refuses status without a worktree), got Ok");
        };
        let handler_err = handler_error.to_string();
        assert!(
            !handler_err.contains("Not in a git repository"),
            "root resolution should no longer short-circuit before reaching git: {handler_err}"
        );

        let Err(oneshot_err) = oneshot_result else {
            panic!("expected the oneshot path to hit the same work-tree error");
        };
        assert!(
            handler_err.contains("work tree") && oneshot_err.to_string().contains("work tree"),
            "handler and oneshot paths should fail with the same git work-tree error: \
             handler={handler_err}, oneshot={oneshot_err}"
        );
    }

    /// #429: a within-TTL cache hit whose entry is past its cooldown must both serve and
    /// revalidate.
    ///
    /// (a) return the cached value immediately (no added hot-path latency) and (b) spawn a
    /// background revalidation that recomputes the real status when the watcher missed the
    /// event.
    ///
    /// Seeds a deliberately-wrong CLEAN status for a repo whose true state is
    /// Dirty, then proves the hit path serves the stale value while the
    /// background job repairs the cache — without waiting for TTL expiry.
    #[test]
    fn stale_within_ttl_hit_serves_cached_then_revalidates_in_background() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        // 0ms cooldown so the freshly-seeded entry is immediately revalidatable.
        let cache = zero_cooldown_cache();
        let (handler, _registry) = make_handler_with_cache(Arc::clone(&cache));

        // Seed a WRONG (Clean) status, simulating a watcher-missed change.
        cache.set_canonical(&git_root, status(0), HashMap::new());

        let safe_path = crate::security::SafePath::new(git_root.to_str().expect("utf8 path"))
            .expect("safe path");

        // Part (a): the hit path returns the seeded CLEAN status immediately.
        let response = handler
            .get_git_status(&safe_path, None)
            .expect("cache hit should return the seeded status");
        let Response::RepositoryStatus(served) = response else {
            panic!("expected a RepositoryStatus response");
        };
        assert_eq!(
            served.state,
            RepositoryState::Clean,
            "hit path must serve the cached (stale) status without blocking on a recompute"
        );

        // Part (b): the background revalidation recomputes the true Dirty status
        // and updates the cache before TTL expiry. Await the actual job handle
        // (condvar-backed) instead of sleeping.
        let handle = handler
            .revalidate_stale_hit(&git_root, &git_root, 0)
            .expect("entry is past cooldown, so revalidation must be spawned");
        let job_result = handle
            .wait_timeout(Duration::from_secs(5))
            .expect("background revalidation should complete within the timeout");
        assert!(
            matches!(job_result, Ok(Some(_))),
            "revalidation job should produce a fresh status: {job_result:?}"
        );

        let refreshed = cache
            .get_any_canonical(&git_root)
            .expect("cache should hold the recomputed status");
        assert_eq!(
            refreshed.state,
            RepositoryState::Dirty,
            "background revalidation must correct the stale Clean entry to the true Dirty status"
        );
        assert!(
            refreshed.untracked >= 1,
            "recomputed status should reflect the untracked file"
        );
    }

    /// The cooldown gate throttles revalidation: a within-cooldown entry must
    /// not spawn a recompute (prevents a background job per rapid Enter press),
    /// while an entry past its cooldown does.
    #[test]
    fn revalidate_stale_hit_is_throttled_by_cooldown() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        // Default policy (150ms cooldown): a freshly-seeded entry is fresh, so
        // revalidation is throttled to None.
        let default_cache = Arc::new(GitStatusCache::new());
        let (fresh_handler, _r1) = make_handler_with_cache(Arc::clone(&default_cache));
        default_cache.set_canonical(&git_root, status(0), HashMap::new());
        assert!(
            fresh_handler
                .revalidate_stale_hit(&git_root, &git_root, 0)
                .is_none(),
            "a within-cooldown entry must not spawn a background revalidation"
        );

        // 0ms cooldown: the same freshly-seeded entry is immediately past its
        // cooldown, so revalidation is spawned.
        let hot_cache = zero_cooldown_cache();
        let (hot_handler, _r2) = make_handler_with_cache(Arc::clone(&hot_cache));
        hot_cache.set_canonical(&git_root, status(0), HashMap::new());
        let handle = hot_handler.revalidate_stale_hit(&git_root, &git_root, 0);
        assert!(
            handle.is_some(),
            "an entry past its cooldown must spawn a background revalidation"
        );
        // Drain the job so it doesn't outlive the temp dir.
        if let Some(job) = handle {
            let _ = job.wait_timeout(Duration::from_secs(5));
        }
    }

    /// #433: `handle_git_status_timeout` must only serve a stale cache entry immediately when
    /// it is within `MAX_IMMEDIATE_STALE_AGE`.
    ///
    /// Beyond that bound, the too-old value must not be trusted -- the handler must wait for
    /// the in-flight recompute instead (deterministically, via the job's own condvar-backed
    /// `wait_timeout`; no sleeps).
    ///
    /// Seeds a deliberately-wrong CLEAN status for a repo whose true state is
    /// Dirty, backdates it well past the bound, and proves the served
    /// response is the FRESH recomputed (Dirty) status, not the too-old
    /// stale one.
    #[test]
    fn handle_git_status_timeout_ignores_too_old_stale_and_waits_for_fresh_job() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        let cache = Arc::new(GitStatusCache::new());
        let (handler, _registry) = make_handler_with_cache(Arc::clone(&cache));

        // Seed a WRONG (Clean) status; the true state is Dirty (untracked file).
        cache.set_canonical(&git_root, status(0), HashMap::new());
        // Backdate it well past MAX_IMMEDIATE_STALE_AGE so it must not be
        // trusted for the immediate-serve fast path.
        cache.age_entry_for_test(&git_root, MAX_IMMEDIATE_STALE_AGE + Duration::from_secs(1));

        let job = handler.git_status_job(&git_root, git_root.clone(), 0);
        let config = handler.render.config_manager.get();
        let response = handler
            .handle_git_status_timeout(&job, &git_root, &config, None)
            .expect("recompute should succeed for a real repo");

        let Response::RepositoryStatus(served) = response else {
            panic!("expected a RepositoryStatus response");
        };
        assert_eq!(
            served.state,
            RepositoryState::Dirty,
            "a too-old stale entry must not be served immediately; the handler \
             must wait for the in-flight recompute and serve the FRESH status"
        );
    }

    /// Contrast case for #433: a stale entry that's still within
    /// `MAX_IMMEDIATE_STALE_AGE` is served immediately (today's fast path),
    /// without waiting on the in-flight recompute.
    #[test]
    fn handle_git_status_timeout_serves_within_bound_stale_immediately() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        let cache = Arc::new(GitStatusCache::new());
        let (handler, _registry) = make_handler_with_cache(Arc::clone(&cache));

        // Seed a WRONG (Clean) status, backdated but still within bound.
        cache.set_canonical(&git_root, status(0), HashMap::new());
        cache.age_entry_for_test(
            &git_root,
            MAX_IMMEDIATE_STALE_AGE.saturating_sub(Duration::from_secs(1)),
        );

        let job = handler.git_status_job(&git_root, git_root.clone(), 0);
        let config = handler.render.config_manager.get();
        let response = handler
            .handle_git_status_timeout(&job, &git_root, &config, None)
            .expect("stale-serve should succeed");

        let Response::RepositoryStatus(served) = response else {
            panic!("expected a RepositoryStatus response");
        };
        assert_eq!(
            served.state,
            RepositoryState::Clean,
            "a within-bound stale entry must be served immediately as-is \
             (today's fast path), even though it's the wrong/stale value"
        );

        // Drain the job so it doesn't outlive the temp dir.
        let _ = job.wait_timeout(Duration::from_secs(5));
    }
}
