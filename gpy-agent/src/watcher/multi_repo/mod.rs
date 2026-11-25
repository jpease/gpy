//! Reference-counted watcher over multiple git repositories.
//!
//! One shared [`WatchCoordinator`] watches every repository (one debouncer and
//! flush thread total); `watched_repos` maps each git root to the client PIDs
//! using it. When the last client leaves, the watch stops, at once or after
//! the builder's teardown grace, so a shell alternating between two
//! repositories does not re-arm a whole tree on every `cd`.
//!
//! `ops_lock` serializes arming and teardown. A client joining a repository
//! that is already watched, or leaving one that keeps other clients, needs
//! neither and so does not wait behind another repository's arming walk.
//!
//! Worktree watching defaults on; `git.watch_worktree` sets it and
//! `GPY_WATCH_WORKTREE` (`0`/`false`/`no`/`off` disables) overrides the config.

mod arming;
mod common_dir;
#[cfg(test)]
mod lifecycle_tests;
mod scope;
#[cfg(test)]
mod scope_tests;

use super::watch_set::WatchMode;
use super::{EventCallback, WatchCoordinator, WatchRegistry, WatcherConfig};
use crate::debug_log;
use crate::sync_ext::MutexExt;
use crate::{Error, Result};
use common_dir::release_common_dir_watches;
use scope::RepoTarget;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Teardown grace the agent passes: how long a repository with no clients
/// stays armed before [`MultiRepoWatcher::release_idle_repos`] releases it.
/// Zero (tear down at once): the agent drops Git events for a repository no
/// client's directory is in, so a parked repository's cached status went stale
/// with nothing to refresh it (`tests/zsh/prompt_text_literal.test.zsh`).
pub const DEFAULT_TEARDOWN_GRACE: Duration = Duration::ZERO;

#[derive(Debug, Clone)]
struct WatchedRepo {
    git_root: PathBuf,
    git_dir: PathBuf,
    /// `(dev, ino)` at watch time; a change means `.git` was recreated and the
    /// OS watch is stale.
    git_dir_id: DirIdentity,
    worktree: WorktreeWatch,
    /// Exactly the paths armed for this repo; teardown unwatches only these.
    armed: Vec<PathBuf>,
    clients: HashSet<u32>,
    /// Last attempt to arm the worktree watch; paces retries while degraded.
    last_arm_attempt: Instant,
    /// When the last client left; the repo stays armed for the teardown grace.
    idle_since: Option<Instant>,
    /// Start of the current watched period: arming, re-arming, or a client
    /// returning to an idle repo. Changes before it were never reported.
    watched_since: Instant,
}

/// Requested vs. effective worktree-watch state; `effective && !requested`
/// is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorktreeWatch {
    Disabled,
    Armed,
    /// Requested, but the recursive watch failed; only `.git` is watched.
    Degraded,
}

impl WorktreeWatch {
    /// Compare config against this, not the effective state, so a degraded
    /// repo is not re-armed on every registration.
    const fn requested(self) -> bool {
        matches!(self, Self::Armed | Self::Degraded)
    }
}

/// What [`MultiRepoWatcher::start_repo_watches`] actually armed.
struct RepoWatches {
    armed: Vec<PathBuf>,
    worktree: WorktreeWatch,
}

/// A shared common git directory watched for its dependent linked worktrees.
/// Instance-local: watches belong to this watcher's coordinator.
#[derive(Debug, Default)]
struct CommonDirWatch {
    /// Reference count: the entry and its watches live while this is non-empty.
    dependents: HashSet<PathBuf>,
    /// Empty when an already-watched ancestor covers the paths, so teardown
    /// never unwatches what it did not watch.
    armed: ArmedWatches,
}

type ArmedWatches = Vec<(PathBuf, WatchMode)>;

/// `(dev, ino)`; `None` where no portable inode exists (reconcile is the backstop).
type DirIdentity = Option<(u64, u64)>;

#[cfg(unix)]
fn dir_identity(path: &Path) -> DirIdentity {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn dir_identity(_path: &Path) -> DirIdentity {
    None
}

/// The watch operations [`MultiRepoWatcher`] performs: [`WatchCoordinator`] in
/// production; tests substitute targets that record or fail watches.
pub trait WatchTarget {
    /// Watch `path` recursively ([`WatchCoordinator::watch_directory`]).
    ///
    /// # Errors
    /// Returns an error if `path` cannot be watched.
    fn watch(&mut self, path: &Path) -> Result<()>;

    /// Watch `path` alone ([`WatchCoordinator::watch_directory_shallow`]).
    ///
    /// # Errors
    /// Returns an error if `path` cannot be watched.
    fn watch_shallow(&mut self, path: &Path) -> Result<()>;

    /// Whether an armed watch already delivers events for `path`.
    fn covers(&self, path: &Path) -> bool;

    /// Release one recursive watch of `path`.
    ///
    /// # Errors
    /// Returns an error if `path` is not watched recursively.
    fn unwatch(&mut self, path: &Path) -> Result<()>;

    /// Release one shallow watch of `path`.
    ///
    /// # Errors
    /// Returns an error if `path` is not watched shallowly.
    fn unwatch_shallow(&mut self, path: &Path) -> Result<()>;

    /// Stop all watching.
    fn stop_watching(&mut self);

    /// Prepare for events from a freshly armed repository (force-added
    /// listing, filesystem probe); a no-op unless overridden.
    fn repo_armed(&self, _git_root: &Path, _git_dir: &Path) {}

    /// Queue one Git refresh of `git_root`, for changes no event delivered
    /// (made before its watch was live, or while it had no client); a no-op
    /// unless overridden.
    fn request_refresh(&self, _git_root: &Path) {}
}

impl WatchTarget for WatchCoordinator {
    fn watch(&mut self, path: &Path) -> Result<()> {
        self.watch_directory(path)
    }

    fn watch_shallow(&mut self, path: &Path) -> Result<()> {
        self.watch_directory_shallow(path)
    }

    fn covers(&self, path: &Path) -> bool {
        self.covers_directory(path)
    }

    fn unwatch(&mut self, path: &Path) -> Result<()> {
        self.unwatch_directory(path)
    }

    fn unwatch_shallow(&mut self, path: &Path) -> Result<()> {
        self.unwatch_directory_shallow(path)
    }

    fn stop_watching(&mut self) {
        self.stop();
    }

    fn repo_armed(&self, git_root: &Path, git_dir: &Path) {
        self.prepare_armed_repo(git_root, git_dir);
    }

    fn request_refresh(&self, git_root: &Path) {
        self.queue_refresh(git_root);
    }
}

/// Release one `mode` watch of `path`; failures are logged, not propagated.
fn unwatch_logged(w: &mut impl WatchTarget, path: &Path, mode: WatchMode) {
    let released = match mode {
        WatchMode::Recursive => w.unwatch(path),
        WatchMode::Shallow => w.unwatch_shallow(path),
    };
    if let Err(err) = released {
        debug_log!("watcher", "Failed to unwatch {}: {err}", path.display());
    }
}

/// Multi-repository watcher registry with reference counting.
pub struct MultiRepoWatcher<T = WatchCoordinator> {
    watcher: Arc<Mutex<Option<T>>>,
    watched_repos: Arc<Mutex<HashMap<PathBuf, WatchedRepo>>>,
    /// Root/gitdir attribution, worktree flag and ignore caches shared with the coordinator.
    registry: Arc<WatchRegistry>,
    /// Keyed by canonical common directory.
    common_watches: Mutex<HashMap<PathBuf, CommonDirWatch>>,
    ops_lock: Mutex<()>,
    /// How long a client-free repo stays armed; zero tears it down at once.
    teardown_grace: Duration,
}

impl MultiRepoWatcher {
    /// Create a new builder for `MultiRepoWatcher`.
    #[must_use]
    pub const fn builder() -> MultiRepoWatcherBuilder {
        MultiRepoWatcherBuilder::new()
    }
}

impl<T: WatchTarget> MultiRepoWatcher<T> {
    /// Attach `pid` to the repository containing `cwd` and begin watching it,
    /// detaching the client from any repository it was in before. When the new
    /// repository cannot be discovered or watched the client ends up attached
    /// to none.
    ///
    /// # Errors
    /// Returns an error if the repository cannot be discovered or watched.
    pub fn attach_client(&self, pid: u32, cwd: &Path) -> Result<()> {
        let target = self.discover_repo(cwd);
        if let Ok(Some(found)) = &target
            && self.attach_without_arming(pid, found)
        {
            return Ok(());
        }
        let _ops_guard = self.ops_lock.lock_or_recover();
        self.release_expired_idle_repos();
        self.attach_pid_to_target(pid, cwd, target)
    }

    /// Attach `pid` to the repo containing `cwd` and detach it from all others.
    /// Caller holds `ops_lock`.
    ///
    /// # Errors
    /// Returns an error if the repository cannot be discovered or watched.
    fn attach_pid_to(&self, pid: u32, cwd: &Path) -> Result<()> {
        let target = self.discover_repo(cwd);
        self.attach_pid_to_target(pid, cwd, target)
    }

    /// Attach `pid` to `target` (discovered from `cwd`) and detach it from all
    /// other repos. Caller holds `ops_lock`. Attach happens first so a kept
    /// repo is never torn down and re-armed.
    ///
    /// # Errors
    /// Returns the discovery error, or an error if the repository cannot be watched.
    fn attach_pid_to_target(
        &self,
        pid: u32,
        cwd: &Path,
        target: Result<Option<RepoTarget>>,
    ) -> Result<()> {
        debug_log!(
            "watcher",
            "attach_pid_to: pid={}, cwd={}",
            pid,
            cwd.display()
        );

        let attached = target.and_then(|found| {
            found.map_or_else(|| Ok(None), |repo| self.attach_to_repo(pid, repo).map(Some))
        });
        // On failure the pid also leaves the repo whose re-arm failed.
        let keep = attached.as_ref().ok().and_then(Option::as_deref);
        self.detach_pid_except(pid, keep);
        attached?;
        Ok(())
    }

    /// Attach `pid` to an already watched `target` whose watches are current,
    /// leaving its other repos, when that needs neither arming nor teardown;
    /// `false` (nothing changed) otherwise, for the `ops_lock` path. A repo
    /// with no clients is never joined here: it may be mid-teardown.
    fn attach_without_arming(&self, pid: u32, target: &RepoTarget) -> bool {
        let current_id = dir_identity(&target.git_dir);
        let mut repos = self.watched_repos.lock_or_recover();
        let current = repos.get(&target.git_root).is_some_and(|repo| {
            // An unknown identity on either side counts as unchanged, as in
            // `attach_or_rearm_existing_repo`.
            let same_git_dir = repo.git_dir == target.git_dir
                && (repo.git_dir_id.is_none()
                    || current_id.is_none()
                    || repo.git_dir_id == current_id);
            !repo.clients.is_empty()
                && same_git_dir
                && repo.worktree.requested() == target.watch_worktree
                && repo.worktree != WorktreeWatch::Degraded
        });
        let empties_another = repos.iter().any(|(root, repo)| {
            root != &target.git_root && repo.clients.contains(&pid) && repo.clients.len() == 1
        });
        if !current || empties_another {
            return false;
        }
        for (root, repo) in repos.iter_mut() {
            if root == &target.git_root {
                repo.clients.insert(pid);
            } else {
                repo.clients.remove(&pid);
            }
        }
        true
    }

    /// Attach `pid` to `target`, arming it if untracked; returns its root.
    ///
    /// # Errors
    /// Returns an error if the repository cannot be watched.
    fn attach_to_repo(&self, pid: u32, target: RepoTarget) -> Result<PathBuf> {
        let RepoTarget {
            git_root,
            git_dir,
            watch_worktree,
        } = target;
        debug_log!("watcher", "Found git root: {}", git_root.display());
        if watch_worktree {
            debug_log!(
                "watcher",
                "worktree watching enabled - watching both gitdir and worktree"
            );
        }

        if self.attach_or_rearm_existing_repo(pid, &git_root, &git_dir, watch_worktree)? {
            self.retry_degraded_worktrees(Some(&git_root), false);
            return Ok(git_root);
        }

        self.log_gitdir_only_scope(&git_root);
        debug_log!(
            "watcher",
            "Starting to watch gitdir {} for client {}",
            git_dir.display(),
            pid
        );

        // A failed worktree watch degrades to .git-only rather than failing.
        let repo_watches = self
            .start_repo_watches(&git_root, &git_dir, watch_worktree)
            .inspect_err(|_| self.registry.unregister_external_gitdir(&git_dir))?;
        self.track_new_repo(pid, git_root.clone(), git_dir, repo_watches);
        Ok(git_root)
    }

    /// Remove a client and stop watching the repository if no clients remain
    /// (after the teardown grace, if one is set).
    pub fn unregister_client(&self, pid: u32) {
        if self.detach_without_teardown(pid) {
            return;
        }
        let _ops_guard = self.ops_lock.lock_or_recover();
        self.release_expired_idle_repos();
        self.detach_pid_except(pid, None);
    }

    /// Remove `pid` from every repo when none is left client-free; `false`
    /// (nothing changed) otherwise, for the `ops_lock` path.
    fn detach_without_teardown(&self, pid: u32) -> bool {
        let mut repos = self.watched_repos.lock_or_recover();
        if repos
            .values()
            .any(|repo| repo.clients.contains(&pid) && repo.clients.len() == 1)
        {
            return false;
        }
        for repo in repos.values_mut() {
            repo.clients.remove(&pid);
        }
        true
    }

    /// Remove `pid` from every repo except `keep`. An emptied repo is torn down
    /// now, or parked for the teardown grace. Caller holds `ops_lock`.
    fn detach_pid_except(&self, pid: u32, keep: Option<&Path>) {
        let now = Instant::now();
        let mut repos_to_remove = Vec::new();
        {
            let mut repos = self.watched_repos.lock_or_recover();

            for repo in repos.values_mut() {
                if keep.is_some_and(|kept| kept == repo.git_root) {
                    continue;
                }
                if repo.clients.remove(&pid) && repo.clients.is_empty() {
                    if self.teardown_grace.is_zero() {
                        repos_to_remove.push(repo.clone());
                    } else {
                        debug_log!(
                            "watcher",
                            "{} has no clients; keeping it armed for {:?}",
                            repo.git_root.display(),
                            self.teardown_grace
                        );
                        repo.idle_since = Some(now);
                    }
                }
            }
        }

        self.tear_down_repos(&repos_to_remove);
    }

    /// Tear down repos that have had no clients for the teardown grace. The
    /// agent calls this on its periodic reconcile; attach and unregister also
    /// release them.
    pub fn release_idle_repos(&self) {
        let _ops_guard = self.ops_lock.lock_or_recover();
        self.release_expired_idle_repos();
    }

    /// [`Self::release_idle_repos`] for a caller already holding `ops_lock`.
    fn release_expired_idle_repos(&self) {
        if self.teardown_grace.is_zero() {
            return;
        }
        let expired: Vec<WatchedRepo> = self
            .watched_repos
            .lock_or_recover()
            .values()
            .filter(|repo| {
                repo.clients.is_empty()
                    && repo
                        .idle_since
                        .is_some_and(|since| since.elapsed() >= self.teardown_grace)
            })
            .cloned()
            .collect();
        self.tear_down_repos(&expired);
    }

    /// Stop watching `repos` (already client-free) and drop their state, then
    /// give degraded repos a chance at the watch budget just released. Caller
    /// holds `ops_lock`.
    fn tear_down_repos(&self, repos_to_remove: &[WatchedRepo]) {
        if repos_to_remove.is_empty() {
            return;
        }
        if let Some(w) = self.watcher.lock_or_recover().as_mut() {
            for repo in repos_to_remove {
                for path in &repo.armed {
                    unwatch_logged(w, path, WatchMode::Recursive);
                }
            }
        }

        {
            let mut repos = self.watched_repos.lock_or_recover();
            for repo in repos_to_remove {
                repos.remove(&repo.git_root);
            }
        }

        self.registry
            .remove_roots(repos_to_remove.iter().map(|repo| repo.git_root.as_path()));

        for repo in repos_to_remove {
            self.registry.unregister_external_gitdir(&repo.git_dir);
            release_common_dir_watches(self, &repo.git_root);
        }

        self.retry_degraded_worktrees(None, true);
    }

    /// Stop watching all repositories and clear state.
    pub fn stop(&self) {
        {
            let mut watcher = self.watcher.lock_or_recover();
            if let Some(mut w) = watcher.take() {
                w.stop_watching();
            }
        }
        {
            let mut repos = self.watched_repos.lock_or_recover();
            for repo in repos.values() {
                self.registry.unregister_external_gitdir(&repo.git_dir);
            }
            repos.clear();
        }
        self.registry.clear_roots();
        // The coordinator already dropped every OS watch; only bookkeeping remains.
        let mut common = self.common_watches.lock_or_recover();
        for (common_dir, entry) in common.drain() {
            for root in &entry.dependents {
                self.registry.unregister_common_gitdir(&common_dir, root);
            }
        }
    }

    /// Add a client to an already-tracked repo, re-arming if `.git` was
    /// recreated/relocated or the requested worktree setting changed, rather
    /// than waiting for the periodic reconcile. `Ok(false)` if untracked.
    ///
    /// # Errors
    /// Returns an error when re-arming the essential `.git` watch fails.
    fn attach_or_rearm_existing_repo(
        &self,
        pid: u32,
        git_root: &Path,
        git_dir: &Path,
        watch_worktree: bool,
    ) -> Result<bool> {
        // ops_lock serializes us, so the snapshot cannot go stale.
        let tracked = {
            let repos = self.watched_repos.lock_or_recover();
            repos.get(git_root).map(|repo| {
                (
                    repo.git_dir.clone(),
                    repo.git_dir_id,
                    repo.worktree.requested(),
                )
            })
        };

        let Some((tracked_git_dir, tracked_id, tracked_requested_worktree)) = tracked else {
            return Ok(false);
        };

        let current_id = dir_identity(git_dir);
        let recreated = tracked_id.is_some() && current_id.is_some() && tracked_id != current_id;
        let relocated = tracked_git_dir != git_dir;
        let worktree_setting_changed = tracked_requested_worktree != watch_worktree;

        if recreated || relocated || worktree_setting_changed {
            self.rearm_repo_watch(Some(pid), git_root, git_dir, watch_worktree)?;
            return Ok(true);
        }

        let was_idle = {
            let mut repos = self.watched_repos.lock_or_recover();
            let Some(repo) = repos.get_mut(git_root) else {
                return Ok(false);
            };
            debug_log!(
                "watcher",
                "Already watching {}, adding client {}",
                git_root.display(),
                pid
            );
            repo.clients.insert(pid);
            let was_idle = repo.idle_since.take().is_some();
            if was_idle {
                repo.watched_since = Instant::now();
            }
            was_idle
        };
        // Git events of a client-free repo are dropped, so catch up once.
        if was_idle && let Some(w) = self.watcher.lock_or_recover().as_ref() {
            w.request_refresh(git_root);
        }
        Ok(true)
    }

    /// Insert a newly watched repository into the internal map.
    fn track_new_repo(
        &self,
        pid: u32,
        git_root: PathBuf,
        git_dir: PathBuf,
        repo_watches: RepoWatches,
    ) {
        let mut clients = HashSet::new();
        clients.insert(pid);
        let git_dir_id = dir_identity(&git_dir);
        let root_key = git_root.clone();

        {
            let mut repos = self.watched_repos.lock_or_recover();

            repos.insert(
                root_key.clone(),
                WatchedRepo {
                    git_root,
                    git_dir,
                    git_dir_id,
                    worktree: repo_watches.worktree,
                    armed: repo_watches.armed,
                    clients,
                    last_arm_attempt: Instant::now(),
                    idle_since: None,
                    watched_since: Instant::now(),
                },
            );
        }

        self.registry.insert_root(root_key);
    }

    /// Map `git_dir` to `git_root` when the gitdir lives outside the worktree.
    fn register_external_gitdir_if_needed(&self, git_dir: &Path, git_root: &Path) {
        if !git_dir.starts_with(git_root) {
            self.registry.register_external_gitdir(git_dir, git_root);
        }
    }

    /// The [`WatchRegistry`] shared with the coordinator.
    #[must_use]
    pub const fn registry(&self) -> &Arc<WatchRegistry> {
        &self.registry
    }

    /// Apply `git.watch_worktree` (`GPY_WATCH_WORKTREE` overrides). Follow
    /// with [`Self::resync_watch_worktree`] to update active repos.
    pub fn set_watch_worktree(&self, config_enabled: bool) {
        self.registry.set_worktree_enabled(config_enabled);
    }

    /// Re-arm watched repos whose *requested* worktree state differs from the
    /// current setting. Best-effort: failures are logged and skipped so one
    /// repo exhausting the watch budget cannot block the rest.
    pub fn resync_watch_worktree(&self) {
        let _ops_guard = self.ops_lock.lock_or_recover();

        let repos = self.watched_repos.lock_or_recover();
        let snapshot: Vec<(PathBuf, PathBuf, bool)> = repos
            .values()
            .map(|repo| {
                (
                    repo.git_root.clone(),
                    repo.git_dir.clone(),
                    repo.worktree.requested(),
                )
            })
            .collect();
        drop(repos);

        for (git_root, git_dir, requested) in snapshot {
            let desired = self.worktree_watch_wanted(&git_root);
            if requested == desired {
                continue;
            }
            debug_log!(
                "watcher",
                "Reconfiguring {} for watch_worktree={}",
                git_root.display(),
                desired
            );
            if let Err(err) = self.rearm_repo_watch(None, &git_root, &git_dir, desired) {
                debug_log!(
                    "watcher",
                    "Failed to reconfigure watch for {}: {err}; leaving previous watch in place",
                    git_root.display()
                );
            }
        }
    }

    /// Number of repositories watched for at least one client (repos kept
    /// armed through their teardown grace are not counted).
    #[must_use]
    pub fn watched_repo_count(&self) -> usize {
        self.watched_repos
            .lock_or_recover()
            .values()
            .filter(|repo| !repo.clients.is_empty())
            .count()
    }

    /// Sorted, deduplicated PIDs across all watched repos.
    #[must_use]
    pub fn client_pids(&self) -> Vec<u32> {
        let mut pids: Vec<u32> = self
            .watched_repos
            .lock_or_recover()
            .values()
            .flat_map(|repo| repo.clients.iter().copied())
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    }

    /// Snapshot of the git roots watched for at least one client.
    #[must_use]
    pub fn watched_roots(&self) -> Vec<PathBuf> {
        self.watched_repos
            .lock_or_recover()
            .values()
            .filter(|repo| !repo.clients.is_empty())
            .map(|repo| repo.git_root.clone())
            .collect()
    }

    /// Start of `git_root`'s current watched period, while a client is
    /// attached to it: change events after that instant are reported; `None`
    /// when nothing would report a change to it.
    #[must_use]
    pub fn watched_since(&self, git_root: &Path) -> Option<Instant> {
        self.watched_repos
            .lock_or_recover()
            .get(git_root)
            .filter(|repo| !repo.clients.is_empty())
            .map(|repo| repo.watched_since)
    }
}

/// Builder for [`MultiRepoWatcher`]; the callback is required.
pub struct MultiRepoWatcherBuilder {
    config: Option<WatcherConfig>,
    callback: Option<EventCallback>,
    registry: Option<Arc<WatchRegistry>>,
    teardown_grace: Duration,
}

impl MultiRepoWatcherBuilder {
    /// Create a new builder with default values.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            config: None,
            callback: None,
            registry: None,
            teardown_grace: Duration::ZERO,
        }
    }

    /// Set the watcher configuration (default: [`WatcherConfig::default()`]).
    #[must_use]
    pub const fn config(mut self, config: WatcherConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the event callback (required).
    #[must_use]
    pub fn callback(mut self, callback: EventCallback) -> Self {
        self.callback = Some(callback);
        self
    }

    /// Share an existing [`WatchRegistry`] (default: a private one).
    #[must_use]
    pub fn registry(mut self, registry: Arc<WatchRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Keep a repository armed for `grace` after its last client leaves
    /// (default: zero, tear down at once). With a grace, call
    /// [`MultiRepoWatcher::release_idle_repos`] periodically; see
    /// [`DEFAULT_TEARDOWN_GRACE`].
    #[must_use]
    pub const fn teardown_grace(mut self, grace: Duration) -> Self {
        self.teardown_grace = grace;
        self
    }

    /// Build the [`MultiRepoWatcher`].
    ///
    /// # Errors
    /// Returns an error if no callback was set or the OS watcher cannot be created.
    pub fn build(self) -> Result<MultiRepoWatcher> {
        let callback = self.callback.ok_or_else(|| {
            Error::watcher("MultiRepoWatcher requires a callback function".to_owned())
        })?;

        let config = self.config.unwrap_or_default();

        let registry = self
            .registry
            .unwrap_or_else(|| Arc::new(WatchRegistry::new()));
        let watcher = WatchCoordinator::new(
            config.debounce_duration(),
            callback,
            Some(Arc::clone(&registry)),
        )?;

        Ok(MultiRepoWatcher {
            watcher: Arc::new(Mutex::new(Some(watcher))),
            watched_repos: Arc::new(Mutex::new(HashMap::new())),
            registry,
            common_watches: Mutex::new(HashMap::new()),
            ops_lock: Mutex::new(()),
            teardown_grace: self.teardown_grace,
        })
    }
}

impl Default for MultiRepoWatcherBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
