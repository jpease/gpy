//! IPC handler for Git status requests: serves from [`crate::git::cache::GitStatusCache`]
//! when fresh, else computes natively with a progressive timeout.

use super::{HandlerError, RenderDeps, notify_if_changed, spawn_detached};
use crate::config::Config;
use crate::git::{self, cache::GitStatusCache};
use crate::ipc::Response;
use crate::ipc::single_flight::{JobGuard, SharedJob, SingleFlight};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Max age at which a stale entry is still served immediately after the fast
/// window misses. Must exceed the 30s git-status TTL.
const MAX_IMMEDIATE_STALE_AGE: Duration = Duration::from_secs(60);

/// Handler for Git repository status queries.
pub struct GitHandler {
    git_cache: Arc<GitStatusCache>,
    /// Handles for publishing to the instant-prompt cache; see [`publish_and_repaint`].
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
    /// Frees the single-flight slot even if `load_repository_state` panics.
    guard: JobGuard<PathBuf, SharedGitStatusResult>,
}

/// Write the instant-prompt cache for `status` and repaint live shells only when
/// the rendered output changed. `prev_bg` is `None` for background work.
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
    /// Create a new `GitHandler`.
    #[must_use]
    pub fn new(render: RenderDeps, git_cache: Arc<GitStatusCache>) -> Self {
        Self {
            git_cache,
            render,
            in_flight: SingleFlight::new(),
        }
    }

    /// Get git status for a path, checking cache first.
    ///
    /// `prev_bg` is the raw wire string: it names both the chevron color and
    /// the cache-key token, so the reading shell resolves the same file.
    ///
    /// # Errors
    /// Fails if git is disabled or skipped, the path isn't in a repo, or status fails.
    pub fn status(
        &self,
        path: &crate::security::SafePath,
        prev_bg: Option<&str>,
    ) -> Result<Response, HandlerError> {
        let path_buf = path.as_path();
        let config = self.render.config_manager.get();

        if !config.segment_enabled("git") {
            return Err(HandlerError::FeatureDisabled("Git"));
        }

        // `find_repo_root` also detects bare repos, matching the oneshot path.
        let git_root_opt = git::find_repo_root(path_buf);
        let Some(git_root) = git_root_opt else {
            return Err(HandlerError::NotInRepository);
        };

        // `skip_paths` gates the repository root, as every background entry point does.
        if !config.git_enabled_for(&git_root) {
            return Err(HandlerError::FeatureDisabled("Git"));
        }

        // `git_root` is already canonical.
        if let Some(cached_status) = self.git_cache.get_canonical(&git_root) {
            // Whatever changed the cached status already published its files, so a hit
            // only refreshes their mtime; a missing file takes the write path.
            if !self.render.instant_cache.touch_git(&git_root, prev_bg) {
                publish_and_repaint(&self.render, &git_root, &cached_status, prev_bg);
            }

            // The watcher may have missed an event (submodules, dropped FSEvents),
            // so revalidate in the background past the cooldown; never awaited.
            self.revalidate_stale_hit(&git_root, path_buf, config.git.max_ahead_behind);

            return Ok(Self::make_git_response(
                cached_status,
                config.git.show_upstream,
            ));
        }

        let path_owned = path_buf.to_path_buf();
        let max_ahead_behind = config.git.max_ahead_behind;

        let job = self.git_status_job(&git_root, path_owned, max_ahead_behind);

        job.wait_timeout(Duration::from_millis(100)).map_or_else(
            || self.handle_git_status_timeout(&job, &git_root, &config, prev_bg),
            |result| match result {
                Ok(Some(status)) => {
                    publish_and_repaint(&self.render, &git_root, &status, prev_bg);

                    Ok(Self::make_git_response(status, config.git.show_upstream))
                }
                Ok(None) => Err(HandlerError::NotInRepository),
                Err(e) => Err(HandlerError::Git(e)),
            },
        )
    }

    /// Handle a miss of the fast 100ms window.
    ///
    /// Serves stale data immediately only within [`MAX_IMMEDIATE_STALE_AGE`];
    /// otherwise waits for the in-flight recompute, falling back to very-stale data.
    ///
    /// # Errors
    /// Fails if the path is no longer a repo, or the recompute fails or times out
    /// with no stale value.
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
            publish_and_repaint(&self.render, git_root, &stale, prev_bg);

            return Ok(Self::make_git_response(stale, config.git.show_upstream));
        }

        // Wait up to the git timeout (+500ms margin) for the in-flight recompute.
        let hard_timeout = Duration::from_secs(config.git.timeout_seconds.get())
            .saturating_add(Duration::from_millis(500));
        match job.wait_timeout(hard_timeout) {
            Some(Ok(Some(status))) => {
                publish_and_repaint(&self.render, git_root, &status, prev_bg);

                Ok(Self::make_git_response(status, config.git.show_upstream))
            }
            Some(Ok(None)) => Err(HandlerError::NotInRepository),
            Some(Err(e)) => Err(HandlerError::Git(e)),
            None => {
                // Fall back to the very-stale value if any.
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

    /// Revalidate a within-TTL cache hit in the background once past its cooldown.
    ///
    /// The cooldown throttles this to one recompute per window per repo; never
    /// awaited on the request path.
    fn revalidate_stale_hit(&self, git_root: &Path, path: &Path, max_ahead_behind: usize) {
        if !self.git_cache.is_fresh_canonical(git_root) {
            drop(self.git_status_job(git_root, path.to_path_buf(), max_ahead_behind));
        }
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
        // Fallback if the job panics before finishing.
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
            let started = Instant::now();
            let result = crate::git::native::NativeGitBackend::get_complete_status(
                &job_spec.path,
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
                    // `git_root` is already canonical. A newer full scan was
                    // already published, so its waiters get it instead.
                    if let Some(published) = job_spec.cache.apply_full_scan(
                        &job_spec.git_root,
                        started,
                        status.clone(),
                        files,
                    ) {
                        // The request path may have served stale data; publish the
                        // fresh status. No request context, so prev_bg is None.
                        publish_and_repaint(&job_spec.render, &job_spec.git_root, &published, None);
                        Ok(Some(published))
                    } else {
                        Ok(Some(
                            job_spec
                                .cache
                                .get_any_canonical(&job_spec.git_root)
                                .unwrap_or(status),
                        ))
                    }
                }
                Ok(None) => Ok(None),
                Err(e) => Err(e.to_string()),
            };

            job_spec.guard.finish(shared_result);
        };

        spawn_detached(job);
    }

    /// Convert git status to IPC response.
    const fn make_git_response(status: git::RepositoryStatus, show_upstream: bool) -> Response {
        let response_status = if show_upstream {
            status
        } else {
            status.without_upstream()
        };

        Response::RepositoryStatus(response_status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CachePolicy;
    use crate::config::manager::ConfigManager;
    use crate::git::status::load_repository_state;
    use crate::git::{RepositoryState, RepositoryStatus};
    use crate::ipc::ClientDirectory;
    use crate::ipc::fixtures::render_deps;
    use std::collections::HashMap;
    use std::time::Duration;

    fn make_handler() -> (GitHandler, Arc<ClientDirectory>) {
        make_handler_with_cache(Arc::new(GitStatusCache::new()))
    }

    fn make_handler_with_cache(
        git_cache: Arc<GitStatusCache>,
    ) -> (GitHandler, Arc<ClientDirectory>) {
        let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
        make_handler_with_config(git_cache, config_manager)
    }

    fn make_handler_with_config(
        git_cache: Arc<GitStatusCache>,
        config_manager: Arc<ConfigManager>,
    ) -> (GitHandler, Arc<ClientDirectory>) {
        let render = render_deps(config_manager);
        let registry = Arc::clone(&render.client_registry);
        (GitHandler::new(render, git_cache), registry)
    }

    /// Cache with a 0ms cooldown so revalidation always fires.
    fn zero_cooldown_cache() -> Arc<GitStatusCache> {
        let policy = CachePolicy {
            ttl: Duration::from_secs(30),
            cooldown: Duration::ZERO,
        };
        Arc::new(GitStatusCache::with_policy(policy))
    }

    /// Poll until the cached status for `root` reaches `state`, so the temp dir
    /// outlives the background job that produces it.
    fn wait_for_canonical_state(cache: &GitStatusCache, root: &Path, state: RepositoryState) {
        let started = std::time::Instant::now();
        while cache
            .get_any_canonical(root)
            .is_none_or(|cached| cached.state != state)
        {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "background revalidation did not reach {state:?} within the timeout"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Create a repo with one untracked file; returns the canonical root.
    fn init_dirty_repo(temp_dir: &tempfile::TempDir) -> PathBuf {
        init_dirty_repo_at(&temp_dir.path().join("repo"))
    }

    fn init_dirty_repo_at(repo: &Path) -> PathBuf {
        std::fs::create_dir_all(repo).expect("repo dir");

        let output = std::process::Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(repo)
            .output()
            .expect("run git init");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        std::fs::write(repo.join("untracked.txt"), "hello").expect("write untracked file");

        std::fs::canonicalize(repo).expect("canonicalize repo root")
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

    /// Repaint fires only when the rendered output changes.
    #[cfg(unix)]
    #[test]
    fn publish_and_repaint_notifies_only_on_content_change() {
        use std::sync::atomic::Ordering;

        let (handler, _registry) = make_handler();
        let (registry, rings) = ClientDirectory::counting_rings();
        let render = RenderDeps {
            client_registry: registry.shared(),
            ..handler.render.clone()
        };
        // Unique temp path so the first write is always a change.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(git_root.join(".git")).expect("git dir");

        let clean = status(0);
        let dirty = status(3);

        publish_and_repaint(&render, &git_root, &clean, None);
        assert_eq!(
            rings.load(Ordering::SeqCst),
            1,
            "first write should repaint live shells"
        );

        publish_and_repaint(&render, &git_root, &clean, None);
        assert_eq!(
            rings.load(Ordering::SeqCst),
            1,
            "an unchanged refresh must not wake terminals"
        );

        publish_and_repaint(&render, &git_root, &dirty, None);
        assert_eq!(
            rings.load(Ordering::SeqCst),
            2,
            "a changed status should repaint live shells"
        );
    }

    /// Bare repos (no worktree) can't render status on either path; the IPC path
    /// must reach the same git error as oneshot, not "Not in a git repository".
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

        let handler_result = handler.status(&safe_path, None);
        let oneshot_result = load_repository_state(&bare_path, 0, false);

        let Err(handler_error) = handler_result else {
            panic!("expected an error (git refuses status without a worktree), got Ok");
        };
        let handler_err = handler_error.to_string();
        assert!(
            !handler_err.contains("Not in a git repository"),
            "root resolution must not short-circuit before reaching git: {handler_err}"
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

    /// A symlinked `skip_paths` entry skips the canonical repo but not a prefix-sharing sibling.
    #[cfg(unix)]
    #[test]
    fn get_git_status_honors_skip_path_through_symlink() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let real = temp_dir.path().join("real");
        let repo = init_dirty_repo_at(&real.join("repo"));
        let other = init_dirty_repo_at(&temp_dir.path().join("realty").join("repo"));
        let link = temp_dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let mut config = crate::config::Config::default();
        config.git.skip_paths = vec![link.to_string_lossy().into_owned()];
        let config_manager = Arc::new(ConfigManager::from_config(config).expect("config"));
        let (handler, _registry) =
            make_handler_with_config(Arc::new(GitStatusCache::new()), config_manager);

        let skipped = crate::security::SafePath::new(repo.to_str().expect("utf8")).expect("safe");
        assert!(
            matches!(
                handler.status(&skipped, None),
                Err(HandlerError::FeatureDisabled("Git"))
            ),
            "repo under a symlinked skip_paths entry must be disabled"
        );

        let kept = crate::security::SafePath::new(other.to_str().expect("utf8")).expect("safe");
        assert!(
            handler.status(&kept, None).is_ok(),
            "a sibling sharing only a string prefix must not be skipped"
        );
    }

    /// `git` missing from `ui.enabled_segments` disables Git.
    #[test]
    fn unlisted_git_segment_is_feature_disabled() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);
        let mut config = crate::config::Config::default();
        config
            .ui
            .enabled_segments
            .retain(|segment| segment != "git");
        let config_manager = Arc::new(ConfigManager::from_config(config).expect("config"));
        let (handler, _registry) =
            make_handler_with_config(Arc::new(GitStatusCache::new()), config_manager);

        let path = crate::security::SafePath::new(git_root.to_str().expect("utf8")).expect("safe");
        assert!(matches!(
            handler.status(&path, None),
            Err(HandlerError::FeatureDisabled("Git"))
        ));
    }

    /// A within-TTL hit past its cooldown serves the cached value and revalidates in the background.
    #[test]
    fn stale_within_ttl_hit_serves_cached_then_revalidates_in_background() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        let cache = zero_cooldown_cache();
        let (handler, _registry) = make_handler_with_cache(Arc::clone(&cache));

        // Seed a wrong (Clean) status, simulating a watcher-missed change.
        cache.set_canonical(&git_root, status(0), HashMap::new());

        let safe_path = crate::security::SafePath::new(git_root.to_str().expect("utf8 path"))
            .expect("safe path");

        // (a) The hit path returns the seeded status immediately.
        let response = handler
            .status(&safe_path, None)
            .expect("cache hit should return the seeded status");
        let Response::RepositoryStatus(served) = response else {
            panic!("expected a RepositoryStatus response");
        };
        assert_eq!(
            served.state,
            RepositoryState::Clean,
            "hit path must serve the cached (stale) status without blocking on a recompute"
        );

        // (b) Background revalidation recomputes the true Dirty status.
        handler.revalidate_stale_hit(&git_root, &git_root, 0);
        wait_for_canonical_state(&cache, &git_root, RepositoryState::Dirty);

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

    /// A hit whose files exist only refreshes them (no repaint); a `prev_bg`
    /// the agent has not rendered takes the write path and repaints once.
    #[cfg(unix)]
    #[test]
    fn cache_hit_touches_existing_files_and_writes_missing_ones() {
        use std::sync::atomic::Ordering;

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);
        let cache = Arc::new(GitStatusCache::new());
        let (base_handler, _registry) = make_handler_with_cache(Arc::clone(&cache));
        let (registry, rings) = ClientDirectory::counting_rings();
        let handler = GitHandler::new(
            RenderDeps {
                client_registry: registry.shared(),
                ..base_handler.render.clone()
            },
            Arc::clone(&cache),
        );
        cache.set_canonical(&git_root, status(0), HashMap::new());
        let safe_path = crate::security::SafePath::new(git_root.to_str().expect("utf8 path"))
            .expect("safe path");

        handler.status(&safe_path, None).expect("first hit");
        assert_eq!(rings.load(Ordering::SeqCst), 1, "missing files are written");

        handler.status(&safe_path, None).expect("second hit");
        assert_eq!(
            rings.load(Ordering::SeqCst),
            1,
            "a hit on existing files must not repaint"
        );

        handler
            .status(&safe_path, Some("blue"))
            .expect("new prev_bg");
        assert_eq!(
            rings.load(Ordering::SeqCst),
            2,
            "an unrendered prev_bg takes the write path"
        );

        handler
            .status(&safe_path, Some("blue"))
            .expect("repeated prev_bg");
        assert_eq!(
            rings.load(Ordering::SeqCst),
            2,
            "a rendered prev_bg is only touched"
        );
    }

    /// Revalidation is throttled within the cooldown and spawned past it.
    #[test]
    fn revalidate_stale_hit_is_throttled_by_cooldown() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        // Default policy (150ms cooldown): throttled.
        let default_cache = Arc::new(GitStatusCache::new());
        let (fresh_handler, _r1) = make_handler_with_cache(Arc::clone(&default_cache));
        default_cache.set_canonical(&git_root, status(0), HashMap::new());
        fresh_handler.revalidate_stale_hit(&git_root, &git_root, 0);
        assert!(
            fresh_handler.in_flight.is_idle(),
            "a within-cooldown entry must not spawn a background revalidation"
        );

        // 0ms cooldown: spawned, and the recompute replaces the seeded Clean entry.
        let hot_cache = zero_cooldown_cache();
        let (hot_handler, _r2) = make_handler_with_cache(Arc::clone(&hot_cache));
        hot_cache.set_canonical(&git_root, status(0), HashMap::new());
        hot_handler.revalidate_stale_hit(&git_root, &git_root, 0);
        wait_for_canonical_state(&hot_cache, &git_root, RepositoryState::Dirty);
    }

    /// A stale entry older than `MAX_IMMEDIATE_STALE_AGE` isn't served; the fresh recompute is.
    #[test]
    fn handle_git_status_timeout_waits_when_stale_too_old() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        let cache = Arc::new(GitStatusCache::new());
        let (handler, _registry) = make_handler_with_cache(Arc::clone(&cache));

        // Seed a WRONG (Clean) status; the true state is Dirty (untracked file).
        cache.set_canonical(&git_root, status(0), HashMap::new());
        // Backdate past MAX_IMMEDIATE_STALE_AGE.
        let age = MAX_IMMEDIATE_STALE_AGE + Duration::from_secs(1);
        cache.age_entry_for_test(&git_root, age, age);

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

    /// A stale entry within `MAX_IMMEDIATE_STALE_AGE` is served immediately.
    #[test]
    fn handle_git_status_timeout_serves_within_bound_stale_immediately() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        let cache = Arc::new(GitStatusCache::new());
        let (handler, _registry) = make_handler_with_cache(Arc::clone(&cache));

        // Seed a WRONG (Clean) status, backdated but still within bound.
        cache.set_canonical(&git_root, status(0), HashMap::new());
        let age = MAX_IMMEDIATE_STALE_AGE.saturating_sub(Duration::from_secs(1));
        cache.age_entry_for_test(&git_root, age, age);

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
             (the fast path), even though it's the wrong/stale value"
        );

        // Drain the job so it doesn't outlive the temp dir.
        let _ = job.wait_timeout(Duration::from_secs(5));
    }
}
