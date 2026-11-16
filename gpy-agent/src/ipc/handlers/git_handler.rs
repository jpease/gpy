//! IPC handler for Git status requests: serves from [`crate::git::cache::GitStatusCache`]
//! while change events keep the entry current, else computes natively with a
//! progressive timeout.

use super::{HandlerError, RenderDeps, notify_if_changed, spawn_detached};
use crate::cache::bounded::GIT_STATUS_CACHE_CAPACITY;
use crate::cache::ttl_map::TtlMap;
use crate::config::Config;
use crate::git::{self, cache::GitStatusCache};
use crate::ipc::Response;
use crate::ipc::single_flight::{JobGuard, SharedJob, SingleFlight};
use crate::sync_ext::MutexExt;
use crate::watcher::multi_repo::MultiRepoWatcher;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Max age at which a stale entry is still served immediately after the fast
/// window misses. Must exceed the 30s git-status TTL.
const MAX_IMMEDIATE_STALE_AGE: Duration = Duration::from_mins(1);

/// How long a failed status for a repository with no cached status is
/// answered from memory before git is run again, so a repository git always
/// refuses (dubious ownership, corruption) or a hung mount does not cost a
/// full capture on every prompt.
const FAILURE_TTL: Duration = Duration::from_secs(5);

/// How long the status of a repository nothing watches (no live watch,
/// `agent.live_updates = false`, or no watcher) is trusted without a rescan,
/// so prompts in quick succession share one scan. Only a status scanned
/// while the repository was already unwatched qualifies, and a noted change
/// ends it at once; see [`GitStatusCache::get_canonical_unwatched`]. Deliberately
/// not configurable: it bounds how stale an unwatched prompt can be.
const UNWATCHED_TRUST_TTL: Duration = Duration::from_secs(2);

/// Recent failures by canonical repository root.
type FailureCache = TtlMap<PathBuf, String>;

/// The agent's lazily-set repository watcher.
type WatcherSlot = Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>;

/// What keeps a repository's cached status current.
#[derive(Clone, Copy)]
enum Watch {
    /// No watcher was wired in ([`GitHandler::with_watcher`]): the TTL alone
    /// decides, as for embedders and handler tests.
    Untracked,
    /// Change events have been reported since this instant.
    Since(Instant),
    /// Nothing reports changes: the watch was torn down (no client in the
    /// repository), live updates are off, or the watcher failed to start.
    /// Only a young status scanned while unwatched is trusted; see
    /// [`UNWATCHED_TRUST_TTL`].
    Unwatched,
}

/// What keeps `git_root`'s cached status current, given the handler's
/// `watcher` slot (`None`: none wired in); see [`Watch`].
fn watch_state(watcher: Option<&WatcherSlot>, git_root: &Path, config: &Config) -> Watch {
    let Some(slot) = watcher else {
        return Watch::Untracked;
    };
    if !config.agent.live_updates {
        return Watch::Unwatched;
    }
    // Clone out so the slot lock is not held across the watcher call.
    let current = slot.lock_or_recover().clone();
    current
        .and_then(|repo_watcher| repo_watcher.watched_since(git_root))
        .map_or(Watch::Unwatched, Watch::Since)
}

/// Handler for Git repository status queries.
pub struct GitHandler {
    git_cache: Arc<GitStatusCache>,
    /// Handles for publishing to the instant-prompt cache; see [`publish_and_repaint`].
    render: RenderDeps,
    in_flight: SingleFlight<PathBuf, SharedGitStatusResult>,
    /// Failures with no status to fall back on; see [`FAILURE_TTL`].
    failures: Arc<FailureCache>,
    /// Tells whether change events keep a repository's cached status current.
    watcher: Option<WatcherSlot>,
}

type SharedGitStatusResult = Result<Option<git::RepositoryStatus>, String>;

struct GitStatusJob {
    path: PathBuf,
    max_ahead_behind: usize,
    git_root: PathBuf,
    cache: Arc<GitStatusCache>,
    failures: Arc<FailureCache>,
    render: RenderDeps,
    /// Whether each scan starts with the repository unwatched; see [`watch_state`].
    watcher: Option<WatcherSlot>,
    /// Frees the single-flight slot even if the status capture panics.
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
    notify_if_changed(deps, git_root, result, "git");
}

/// How many times a status job scans again because the repository changed
/// while it ran, before it settles for its last view.
const MAX_CHANGE_RESCANS: u8 = 2;

/// Run `scan` again while the repository changed (see
/// [`GitStatusCache::note_change_canonical`]) after the latest run started,
/// at most [`MAX_CHANGE_RESCANS`] times. Requests that join a job in flight
/// across a change (a checkout during the registration scan) thereby get the
/// view from after it. Returns the last run's start and result.
fn scan_past_changes<T>(
    cache: &GitStatusCache,
    git_root: &Path,
    mut scan: impl FnMut() -> crate::Result<T>,
) -> (Instant, crate::Result<T>) {
    let mut started = Instant::now();
    let mut result = scan();
    let mut rescans = 0_u8;
    while rescans < MAX_CHANGE_RESCANS
        && result.is_ok()
        && cache.changed_since_canonical(git_root, started)
    {
        rescans = rescans.saturating_add(1);
        started = Instant::now();
        result = scan();
    }
    (started, result)
}

impl GitHandler {
    /// Create a new `GitHandler`.
    #[must_use]
    pub fn new(render: RenderDeps, git_cache: Arc<GitStatusCache>) -> Self {
        Self {
            git_cache,
            render,
            in_flight: SingleFlight::new(),
            failures: Arc::new(TtlMap::with_ttl(GIT_STATUS_CACHE_CAPACITY, FAILURE_TTL)),
            watcher: None,
        }
    }

    /// Trust a cached status only while `watcher` reports changes to its
    /// repository, and only if it was scanned after that watch went live;
    /// a repository nothing watches trusts only a status scanned while
    /// unwatched, for [`UNWATCHED_TRUST_TTL`]. Otherwise a request rescans,
    /// keeping the cached status as the fallback for a scan that times out.
    #[must_use]
    pub fn with_watcher(mut self, watcher: WatcherSlot) -> Self {
        self.watcher = Some(watcher);
        self
    }

    /// What keeps `git_root`'s cached status current; see [`Watch`].
    fn watch_state(&self, git_root: &Path, config: &Config) -> Watch {
        watch_state(self.watcher.as_ref(), git_root, config)
    }

    /// The cached status a request may answer with without waiting on a
    /// scan: within its TTL and current per `watch` (for an unwatched
    /// repository, see [`UNWATCHED_TRUST_TTL`]), or still within the
    /// cooldown of its last update.
    fn trusted_cached(&self, git_root: &Path, watch: Watch) -> Option<git::RepositoryStatus> {
        let current = match watch {
            Watch::Untracked => self.git_cache.get_canonical(git_root),
            Watch::Since(since) => self.git_cache.get_canonical_scanned_since(git_root, since),
            Watch::Unwatched => self
                .git_cache
                .get_canonical_unwatched(git_root, UNWATCHED_TRUST_TTL),
        };
        current.or_else(|| {
            if self.git_cache.is_fresh_canonical(git_root) {
                self.git_cache.get_canonical(git_root)
            } else {
                None
            }
        })
    }

    /// Number of repositories with a cached status.
    #[must_use]
    pub(crate) fn cached_repo_count(&self) -> usize {
        self.git_cache.len()
    }

    /// Fill the caches for the repository holding `path` ahead of its first
    /// prompt (client registration), in the background.
    ///
    /// Shares the request path's single-flight and `git.timeout_seconds`, so
    /// shells registering together in one repository start one scan. Does
    /// nothing outside a repository, when git is off or skipped for the root,
    /// while the cached status is within its cooldown, or after a recent failure.
    pub(crate) fn prewarm(&self, path: &Path) {
        let Some(git_root) = git::find_repo_root(path) else {
            return;
        };
        let config = self.render.config_manager.get();
        if !config.git_enabled_for(&git_root)
            || self.git_cache.is_fresh_canonical(&git_root)
            || self.failures.get(&git_root).is_some()
        {
            return;
        }
        drop(self.git_status_job(&git_root, git_root.clone(), config.git.max_ahead_behind));
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
        let watch = self.watch_state(&git_root, &config);
        if let Some(cached_status) = self.trusted_cached(&git_root, watch) {
            // Whatever changed the cached status already published its files, so a hit
            // only refreshes their mtime; a missing file takes the write path.
            if !self.render.instant_cache.touch_git(&git_root, prev_bg) {
                publish_and_repaint(&self.render, &git_root, &cached_status, prev_bg);
            }

            // The watcher may have missed an event (submodules, dropped FSEvents),
            // so revalidate in the background past the cooldown; never awaited.
            // An unwatched hit is young by construction and rescans once it is not.
            if !matches!(watch, Watch::Unwatched) {
                self.revalidate_stale_hit(&git_root, path_buf, config.git.max_ahead_behind);
            }

            return Ok(Self::make_git_response(
                cached_status,
                config.git.show_upstream,
            ));
        }

        // A failure with nothing cached is answered again until it expires.
        if let Some(error) = self.failures.get(&git_root) {
            return Err(HandlerError::Git(error));
        }

        let path_owned = path_buf.to_path_buf();
        let max_ahead_behind = config.git.max_ahead_behind;

        let job = self.git_status_job(&git_root, path_owned, max_ahead_behind);

        job.wait_timeout(Duration::from_millis(100)).map_or_else(
            || self.handle_git_status_timeout(&job, &git_root, &config, prev_bg, watch),
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
    /// While change events report on the repository, serves stale data
    /// immediately within [`MAX_IMMEDIATE_STALE_AGE`] (the events will correct
    /// it); otherwise waits for the in-flight recompute, falling back to the
    /// stale value only if that times out.
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
        watch: Watch,
    ) -> Result<Response, HandlerError> {
        if !matches!(watch, Watch::Unwatched)
            && let Some((stale, age)) = self.git_cache.get_any_with_age_canonical(git_root)
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
        let failures = Arc::clone(&self.failures);
        let render = self.render.clone();
        let watcher = self.watcher.clone();
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
                    failures,
                    render,
                    watcher,
                    guard,
                });
            })
    }

    fn spawn_git_status_job(job_spec: GitStatusJob) {
        let job = move || {
            let config = job_spec.render.config_manager.get();
            // Whether the latest scan started with nothing watching the repository.
            let mut unwatched = false;
            let (started, result) = scan_past_changes(&job_spec.cache, &job_spec.git_root, || {
                unwatched = matches!(
                    watch_state(job_spec.watcher.as_ref(), &job_spec.git_root, &config),
                    Watch::Unwatched
                );
                crate::git::native::NativeGitBackend::get_complete_status(
                    &job_spec.path,
                    job_spec.max_ahead_behind,
                    config.git.stash_enabled,
                    Duration::from_secs(config.git.timeout_seconds.get()),
                )
            });
            if result.is_ok() {
                job_spec.failures.remove(&job_spec.git_root);
            }
            let shared_result = match result {
                Ok(Some(crate::git::CompleteStatus { status, files })) => {
                    // `git_root` is already canonical.
                    let applied = if unwatched {
                        job_spec.cache.apply_unwatched_full_scan(
                            &job_spec.git_root,
                            started,
                            status.clone(),
                            files,
                        )
                    } else {
                        job_spec.cache.apply_full_scan(
                            &job_spec.git_root,
                            started,
                            status.clone(),
                            files,
                        )
                    };
                    if let Some(published) = applied {
                        // The request path may have served stale data; publish the
                        // fresh status. No request context, so prev_bg is None.
                        publish_and_repaint(&job_spec.render, &job_spec.git_root, &published, None);
                        Ok(Some(published))
                    } else if job_spec
                        .cache
                        .changed_since_canonical(&job_spec.git_root, started)
                    {
                        // Changes kept coming past the rescan cap: this scan is
                        // still the newest view, though too old to cache.
                        Ok(Some(status))
                    } else {
                        // A full scan that started later was already published.
                        Ok(Some(
                            job_spec
                                .cache
                                .get_any_canonical(&job_spec.git_root)
                                .unwrap_or(status),
                        ))
                    }
                }
                Ok(None) => Ok(None),
                Err(e) => {
                    // Serve and publish the last good status with unknown
                    // upstream counts (`↑?`); no last good status keeps the error.
                    if let Some(stale) = job_spec.cache.mark_stale_error(&job_spec.git_root, None) {
                        publish_and_repaint(&job_spec.render, &job_spec.git_root, &stale, None);
                        Ok(Some(stale))
                    } else {
                        let message = e.to_string();
                        job_spec
                            .failures
                            .insert(job_spec.git_root.clone(), message.clone());
                        Err(message)
                    }
                }
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
    use std::assert_matches;
    use std::collections::HashMap;
    use std::time::Duration;

    /// Bound on each status load.
    const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

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
            stale_error: false,
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
        let oneshot_result = load_repository_state(&bare_path, 0, false, STATUS_TIMEOUT);

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
        assert_matches!(
            handler.status(&skipped, None),
            Err(HandlerError::FeatureDisabled("Git")),
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
        assert_matches!(
            handler.status(&path, None),
            Err(HandlerError::FeatureDisabled("Git"))
        );
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

    /// With a watcher wired in, a repository nothing watches (its last
    /// client left, so its watch was torn down) does not answer from a
    /// status scanned under that watch, however young: a change made since
    /// the teardown is read from git.
    #[test]
    fn unwatched_repo_does_not_trust_its_cached_status() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);
        let cache = zero_cooldown_cache();
        let (unwired, _registry) = make_handler_with_cache(Arc::clone(&cache));
        let handler = unwired.with_watcher(Arc::new(Mutex::new(None)));
        // A Clean status from before the change the watcher never saw.
        cache.set_canonical(&git_root, status(0), HashMap::new());
        let safe_path = crate::security::SafePath::new(git_root.to_str().expect("utf8 path"))
            .expect("safe path");

        let Response::RepositoryStatus(served) =
            handler.status(&safe_path, None).expect("a rescan answers")
        else {
            panic!("expected a RepositoryStatus response");
        };
        assert_eq!(served.state, RepositoryState::Dirty);
        assert_eq!(
            cache
                .get_any_canonical(&git_root)
                .map(|cached| cached.state),
            Some(RepositoryState::Dirty)
        );
    }

    fn served_state(handler: &GitHandler, root: &Path) -> RepositoryState {
        let safe_path =
            crate::security::SafePath::new(root.to_str().expect("utf8 path")).expect("safe path");
        let Response::RepositoryStatus(served) = handler.status(&safe_path, None).expect("status")
        else {
            panic!("expected a RepositoryStatus response");
        };
        served.state
    }

    /// An unwatched repository answers from a status scanned while it was
    /// unwatched for `UNWATCHED_TRUST_TTL`, without starting a scan, and
    /// rescans once that status is older.
    #[test]
    fn unwatched_repo_trusts_a_young_unwatched_scan() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);
        let cache = zero_cooldown_cache();
        let (unwired, _registry) = make_handler_with_cache(Arc::clone(&cache));
        let handler = unwired.with_watcher(Arc::new(Mutex::new(None)));
        // Clean, scanned unwatched; the tree is really Dirty.
        assert!(
            cache
                .apply_unwatched_full_scan(&git_root, Instant::now(), status(0), HashMap::new())
                .is_some()
        );

        assert_eq!(served_state(&handler, &git_root), RepositoryState::Clean);
        assert!(
            handler.in_flight.is_idle(),
            "a trusted unwatched hit starts neither a rescan nor a revalidation"
        );

        cache.age_entry_for_test(&git_root, UNWATCHED_TRUST_TTL, UNWATCHED_TRUST_TTL);
        assert_eq!(served_state(&handler, &git_root), RepositoryState::Dirty);
    }

    /// A noted change ends the unwatched trust at once, whatever the age.
    #[test]
    fn unwatched_repo_rescans_after_a_noted_change() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);
        let cache = zero_cooldown_cache();
        let (unwired, _registry) = make_handler_with_cache(Arc::clone(&cache));
        let handler = unwired.with_watcher(Arc::new(Mutex::new(None)));
        assert!(
            cache
                .apply_unwatched_full_scan(&git_root, Instant::now(), status(0), HashMap::new())
                .is_some()
        );
        std::thread::sleep(Duration::from_millis(2));
        cache.note_change_canonical(&git_root);

        assert_eq!(served_state(&handler, &git_root), RepositoryState::Dirty);
    }

    /// A job records whether its scan started unwatched: with the watcher
    /// slot empty (or live updates off) the result is trusted unwatched;
    /// with no watcher wired in it is not.
    #[test]
    fn status_job_marks_scans_taken_while_unwatched() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let git_root = init_dirty_repo(&temp_dir);

        let cache = zero_cooldown_cache();
        let (unwired, _registry) = make_handler_with_cache(Arc::clone(&cache));
        let handler = unwired.with_watcher(Arc::new(Mutex::new(None)));
        handler
            .git_status_job(&git_root, git_root.clone(), 0)
            .wait_timeout(STATUS_TIMEOUT)
            .expect("the job finishes")
            .expect("status succeeds");
        assert_eq!(
            cache
                .get_canonical_unwatched(&git_root, UNWATCHED_TRUST_TTL)
                .map(|cached| cached.state),
            Some(RepositoryState::Dirty)
        );

        let untracked_cache = zero_cooldown_cache();
        let (untracked, _r) = make_handler_with_cache(Arc::clone(&untracked_cache));
        untracked
            .git_status_job(&git_root, git_root.clone(), 0)
            .wait_timeout(STATUS_TIMEOUT)
            .expect("the job finishes")
            .expect("status succeeds");
        assert!(untracked_cache.get_any_canonical(&git_root).is_some());
        assert_eq!(
            untracked_cache.get_canonical_unwatched(&git_root, UNWATCHED_TRUST_TTL),
            None
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
            .handle_git_status_timeout(&job, &git_root, &config, None, Watch::Untracked)
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
            .handle_git_status_timeout(&job, &git_root, &config, None, Watch::Untracked)
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

    /// A bare repository makes `git status` fail, standing in for any failing query.
    fn init_failing_repo(temp_dir: &tempfile::TempDir) -> PathBuf {
        let bare_path = temp_dir.path().join("bare.git");
        std::fs::create_dir_all(&bare_path).expect("bare dir");
        let output = std::process::Command::new("git")
            .args(["init", "--bare", "--initial-branch=main"])
            .arg(&bare_path)
            .output()
            .expect("run git init --bare");
        assert!(output.status.success(), "git init --bare failed");
        std::fs::canonicalize(&bare_path).expect("canonicalize bare root")
    }

    /// A failed query serves and caches the last good status with unknown
    /// upstream counts instead of the failure.
    #[test]
    fn failed_query_serves_last_good_status_marked_stale() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let root = init_failing_repo(&temp_dir);
        let cache = Arc::new(GitStatusCache::new());
        let (handler, _registry) = make_handler_with_cache(Arc::clone(&cache));
        let last_good = RepositoryStatus {
            ahead: 2,
            ..status(0)
        };
        cache.set_canonical(&root, last_good.clone(), HashMap::new());

        let job = handler.git_status_job(&root, root.clone(), 0);
        let served = job
            .wait_timeout(Duration::from_secs(10))
            .expect("the job finishes")
            .expect("a last good status turns the failure into a served status")
            .expect("repository found");

        assert_eq!(
            served,
            RepositoryStatus {
                stale_error: true,
                ..last_good
            }
        );
        assert_eq!(cache.get_any_canonical(&root), Some(served));
    }

    /// With no last good status the failure is reported as before.
    #[test]
    fn failed_query_without_last_good_status_still_errors() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let root = init_failing_repo(&temp_dir);
        let cache = Arc::new(GitStatusCache::new());
        let (handler, _registry) = make_handler_with_cache(Arc::clone(&cache));

        let job = handler.git_status_job(&root, root.clone(), 0);
        let result = job
            .wait_timeout(Duration::from_secs(10))
            .expect("the job finishes");

        assert!(result.is_err(), "got {result:?}");
        assert_eq!(cache.get_any_canonical(&root), None);
    }

    /// A failure with no status to fall back on is remembered, and answered
    /// from memory without starting git again while it lasts.
    #[test]
    fn failure_without_last_good_status_is_answered_from_memory() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let root = init_failing_repo(&temp_dir);
        let (handler, _registry) = make_handler();
        let safe_path =
            crate::security::SafePath::new(root.to_str().expect("utf8 path")).expect("safe path");

        let Err(HandlerError::Git(first)) = handler.status(&safe_path, None) else {
            panic!("a bare repository fails git status");
        };
        assert_eq!(handler.failures.get(&root), Some(first));

        // A healthy repository with a remembered failure: a capture would succeed,
        // so the remembered error and an idle single-flight prove none started.
        let worktree = temp_dir.path().join("worktree");
        let output = std::process::Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(&worktree)
            .output()
            .expect("run git init");
        assert!(output.status.success(), "git init failed");
        let worktree_root = std::fs::canonicalize(&worktree).expect("canonical worktree");
        handler
            .failures
            .insert(worktree_root.clone(), "remembered".to_owned());
        let worktree_path =
            crate::security::SafePath::new(worktree_root.to_str().expect("utf8 path"))
                .expect("safe path");

        assert_matches!(
            handler.status(&worktree_path, None),
            Err(HandlerError::Git(message)) if message == "remembered"
        );
        handler.prewarm(&worktree_root);
        assert!(handler.in_flight.is_idle());
    }

    /// A change noted while a job's scan runs (a checkout while a prompt
    /// request has joined the registration scan) makes the job scan again,
    /// so every waiter gets the view from after the change.
    #[test]
    fn change_during_scan_rescans_before_waiters_are_answered() {
        let cache = GitStatusCache::new();
        let root = PathBuf::from("/repo/changed-mid-scan");
        let mut runs = 0_u8;
        let (_, result) = scan_past_changes(&cache, &root, || {
            runs = runs.saturating_add(1);
            if runs == 1 {
                std::thread::sleep(Duration::from_millis(2));
                cache.note_change_canonical(&root);
                Ok("before checkout")
            } else {
                Ok("after checkout")
            }
        });
        assert_eq!(result.ok(), Some("after checkout"));
        assert_eq!(runs, 2);
    }

    /// Without a change one scan answers; endless churn stops at the cap.
    #[test]
    fn scan_past_changes_runs_once_without_change_and_caps_churn() {
        let cache = GitStatusCache::new();
        let root = PathBuf::from("/repo/rescan-cap");
        let mut runs = 0_u8;
        let (_, quiet) = scan_past_changes(&cache, &root, || {
            runs = runs.saturating_add(1);
            Ok(runs)
        });
        assert_eq!(quiet.ok(), Some(1));

        let mut churn_runs = 0_u8;
        let (_, churned) = scan_past_changes(&cache, &root, || {
            churn_runs = churn_runs.saturating_add(1);
            std::thread::sleep(Duration::from_millis(2));
            cache.note_change_canonical(&root);
            Ok(churn_runs)
        });
        assert_eq!(churned.ok(), Some(MAX_CHANGE_RESCANS.saturating_add(1)));
    }
}
