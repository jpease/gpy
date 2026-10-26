//! Reference-counted watcher over multiple git repositories.
//!
//! One shared [`WatchCoordinator`] watches every repository (one debouncer and
//! flush thread total); `watched_repos` maps each git root to the client PIDs
//! using it, and the watch stops when the last client leaves.
//!
//! Worktree watching defaults on; `git.watch_worktree` sets it and
//! `GPY_WATCH_WORKTREE` (`0`/`false`/`no`/`off` disables) overrides the config.

mod common_dir;
mod gitdir;

use super::watch_set::WatchMode;
use super::{EventCallback, WatchCoordinator, WatchRegistry, WatcherConfig};
use crate::debug_log;
use crate::sync_ext::MutexExt;
use crate::{Error, Result};
use common_dir::{acquire_common_dir_watches, release_common_dir_watches};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
    let meta = fs::metadata(path).ok()?;
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
}

impl MultiRepoWatcher {
    /// Create a new builder for `MultiRepoWatcher`.
    #[must_use]
    pub fn builder() -> MultiRepoWatcherBuilder {
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
        let _ops_guard = self.ops_lock.lock_or_recover();
        self.attach_pid_to(pid, cwd)
    }

    /// Attach `pid` to the repo containing `cwd` and detach it from all others.
    /// Caller holds `ops_lock`. Attach happens first so a kept repo is never
    /// torn down and re-armed.
    ///
    /// # Errors
    /// Returns an error if the repository cannot be discovered or watched.
    fn attach_pid_to(&self, pid: u32, cwd: &Path) -> Result<()> {
        debug_log!(
            "watcher",
            "attach_pid_to: pid={}, cwd={}",
            pid,
            cwd.display()
        );

        let attached = self.attach_to_repo_containing(pid, cwd);
        // On failure the pid also leaves the repo whose re-arm failed.
        let keep = attached.as_ref().ok().and_then(Option::as_deref);
        self.detach_pid_except(pid, keep);
        attached?;
        Ok(())
    }

    /// Attach `pid` to the repo containing `cwd`; `None` when outside any repo.
    ///
    /// # Errors
    /// Returns an error if the repository cannot be discovered or watched.
    fn attach_to_repo_containing(&self, pid: u32, cwd: &Path) -> Result<Option<PathBuf>> {
        let Some(git_root) = crate::git::find_git_dir_only(cwd) else {
            debug_log!("watcher", "No git root found for {}", cwd.display());
            return Ok(None);
        };

        let git_dir = crate::git::resolve_gitdir(&git_root)
            .map(|dir| fs::canonicalize(&dir).unwrap_or(dir))
            .ok_or_else(|| {
                Error::watcher(format!(
                    "Could not resolve git directory at {}",
                    git_root.display()
                ))
            })?;
        let watch_worktree = self.registry.worktree_enabled();

        debug_log!("watcher", "Found git root: {}", git_root.display());
        if watch_worktree {
            debug_log!(
                "watcher",
                "worktree watching enabled - watching both gitdir and worktree"
            );
        }

        if self.attach_or_rearm_existing_repo(pid, &git_root, &git_dir, watch_worktree)? {
            return Ok(Some(git_root));
        }

        debug_log!(
            "watcher",
            "Starting to watch gitdir {} for client {}",
            git_dir.display(),
            pid
        );

        // A failed worktree watch degrades to .git-only rather than failing.
        let repo_watches = self.start_repo_watches(&git_root, &git_dir, watch_worktree)?;
        self.track_new_repo(pid, git_root.clone(), git_dir, repo_watches);
        Ok(Some(git_root))
    }

    /// Remove a client and stop watching the repository if no clients remain.
    pub fn unregister_client(&self, pid: u32) {
        let _ops_guard = self.ops_lock.lock_or_recover();
        self.detach_pid_except(pid, None);
    }

    /// Remove `pid` from every repo except `keep`, tearing down emptied ones.
    fn detach_pid_except(&self, pid: u32, keep: Option<&Path>) {
        let mut repos_to_remove = Vec::new();
        {
            let mut repos = self.watched_repos.lock_or_recover();

            for repo in repos.values_mut() {
                if keep.is_some_and(|kept| kept == repo.git_root) {
                    continue;
                }
                if repo.clients.remove(&pid) && repo.clients.is_empty() {
                    repos_to_remove.push(repo.clone());
                }
            }
        }

        self.tear_down_repos(&repos_to_remove);
    }

    /// Stop watching `repos` (already client-free) and drop their state.
    fn tear_down_repos(&self, repos_to_remove: &[WatchedRepo]) {
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

        {
            let mut repos = self.watched_repos.lock_or_recover();
            if let Some(repo) = repos.get_mut(git_root) {
                debug_log!(
                    "watcher",
                    "Already watching {}, adding client {}",
                    git_root.display(),
                    pid
                );
                repo.clients.insert(pid);
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Re-arm a tracked repo's stale watches and refresh its entry. `pid` is
    /// `None` for a config-driven re-arm, which must not change the refcount.
    ///
    /// # Errors
    /// Returns an error if the essential `.git` watch cannot be re-established.
    fn rearm_repo_watch(
        &self,
        pid: Option<u32>,
        git_root: &Path,
        git_dir: &Path,
        watch_worktree: bool,
    ) -> Result<()> {
        debug_log!(
            "watcher",
            "Re-arming stale watch for {} (client {:?}): gitdir {}",
            git_root.display(),
            pid,
            git_dir.display()
        );

        let stale = {
            let repos = self.watched_repos.lock_or_recover();
            repos
                .get(git_root)
                .map(|repo| (repo.git_dir.clone(), repo.armed.clone()))
        };

        if let Some((_, stale_armed)) = &stale
            && let Some(w) = self.watcher.lock_or_recover().as_mut()
        {
            for path in stale_armed {
                unwatch_logged(w, path, WatchMode::Recursive);
            }
        }

        // A relocated gitdir can move the common dir; re-acquire against the new one.
        release_common_dir_watches(self, git_root);

        let repo_watches = self.start_repo_watches(git_root, git_dir, watch_worktree)?;

        if let Some((stale_git_dir, _)) = &stale {
            self.registry.unregister_external_gitdir(stale_git_dir);
        }
        self.register_external_gitdir_if_needed(git_dir, git_root);

        {
            let mut repos = self.watched_repos.lock_or_recover();
            if let Some(repo) = repos.get_mut(git_root) {
                repo.git_dir = git_dir.to_path_buf();
                repo.git_dir_id = dir_identity(git_dir);
                repo.worktree = repo_watches.worktree;
                repo.armed = repo_watches.armed;
                if let Some(client_pid) = pid {
                    repo.clients.insert(client_pid);
                }
            }
        }

        Ok(())
    }

    /// Arm watches for a repo and log one `Watch ready` line, the marker the
    /// e2e tests wait on before touching the tree.
    ///
    /// # Errors
    /// Returns an error if the coordinator is gone or `.git` cannot be watched.
    fn start_repo_watches(
        &self,
        git_root: &Path,
        git_dir: &Path,
        watch_worktree: bool,
    ) -> Result<RepoWatches> {
        let watches = self.arm_watches(git_root, git_dir, watch_worktree)?;
        debug_log!("watcher", "Watch ready for {}", git_root.display());
        Ok(watches)
    }

    /// Arm watches for a repo. `.git` failure is fatal; a worktree-watch failure
    /// (likely watch-budget exhaustion) degrades to `.git`-only.
    ///
    /// # Errors
    /// Returns an error if the coordinator is gone or `.git` cannot be watched.
    fn arm_watches(
        &self,
        git_root: &Path,
        git_dir: &Path,
        watch_worktree: bool,
    ) -> Result<RepoWatches> {
        let mut watcher_guard = self.watcher.lock_or_recover();
        let Some(w) = watcher_guard.as_mut() else {
            return Err(Error::watcher(
                "No watch coordinator available; repository would not actually be watched",
            ));
        };

        // macOS FSEvents rebuilds its stream on each added path, and a rebuilt
        // stream can drop its first event. A nested `.git` is already covered
        // by the recursive `git_root` watch, so skip the second call.
        let gitdir_covered_by_worktree_watch = git_dir.starts_with(git_root);

        if watch_worktree && gitdir_covered_by_worktree_watch {
            if let Err(e) = w.watch(git_root) {
                return Self::fall_back_to_gitdir_only(w, git_root, git_dir, &e.to_string());
            }
            debug_log!(
                "watcher",
                "Watching worktree {} (single recursive watch covers .git)",
                git_root.display()
            );
            return Ok(RepoWatches {
                armed: vec![git_root.to_path_buf()],
                worktree: WorktreeWatch::Armed,
            });
        }

        w.watch(git_dir)?;
        debug_log!(
            "watcher",
            "Successfully started watching {}",
            git_dir.display()
        );
        let mut armed = vec![git_dir.to_path_buf()];

        // Linked worktrees only. Best-effort; refcounted in `common_watches`,
        // not in this repo's `armed`.
        acquire_common_dir_watches(self, w, git_root, git_dir);

        let worktree = if watch_worktree {
            if let Err(e) = w.watch(git_root) {
                debug_log!(
                    "watcher",
                    "Worktree watch failed for {} ({}); falling back to .git-only watching",
                    git_root.display(),
                    e
                );
                WorktreeWatch::Degraded
            } else {
                debug_log!("watcher", "Watching worktree {}", git_root.display());
                armed.push(git_root.to_path_buf());
                WorktreeWatch::Armed
            }
        } else {
            WorktreeWatch::Disabled
        };

        Ok(RepoWatches { armed, worktree })
    }

    /// Watch `git_dir` alone after a worktree-root watch failed.
    ///
    /// # Errors
    /// Returns an error if the `.git`-only watch also fails.
    fn fall_back_to_gitdir_only(
        w: &mut T,
        git_root: &Path,
        git_dir: &Path,
        reason: &str,
    ) -> Result<RepoWatches> {
        debug_log!(
            "watcher",
            "Worktree watch failed for {} ({reason}); falling back to .git-only watching",
            git_root.display()
        );
        w.watch(git_dir)?;
        debug_log!(
            "watcher",
            "Successfully started watching {}",
            git_dir.display()
        );
        Ok(RepoWatches {
            armed: vec![git_dir.to_path_buf()],
            worktree: WorktreeWatch::Degraded,
        })
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

        // Map external gitdirs (submodules/linked worktrees) back to this root.
        self.register_external_gitdir_if_needed(&git_dir, &git_root);

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

        let desired = self.registry.worktree_enabled();

        let repos = self.watched_repos.lock_or_recover();
        let stale: Vec<(PathBuf, PathBuf)> = repos
            .values()
            .filter(|repo| repo.worktree.requested() != desired)
            .map(|repo| (repo.git_root.clone(), repo.git_dir.clone()))
            .collect();
        drop(repos);

        for (git_root, git_dir) in stale {
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

    /// Number of repositories currently being watched.
    #[must_use]
    pub fn watched_repo_count(&self) -> usize {
        self.watched_repos.lock_or_recover().len()
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

    /// Snapshot of the git roots currently being watched.
    #[must_use]
    pub fn watched_roots(&self) -> Vec<PathBuf> {
        self.watched_repos
            .lock_or_recover()
            .keys()
            .cloned()
            .collect()
    }
}

/// Builder for [`MultiRepoWatcher`]; the callback is required.
pub struct MultiRepoWatcherBuilder {
    config: Option<WatcherConfig>,
    callback: Option<EventCallback>,
    registry: Option<Arc<WatchRegistry>>,
}

impl MultiRepoWatcherBuilder {
    /// Create a new builder with default values.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: None,
            callback: None,
            registry: None,
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
