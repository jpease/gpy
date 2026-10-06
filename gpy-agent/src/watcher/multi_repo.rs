//! Dynamic multi-repository watcher with reference counting
//!
//! Manages watching multiple git repositories based on active client directories.
//! Uses reference counting to start/stop watchers as clients register/unregister.
//!
//! # Architecture: Registry Pattern
//!
//! This module implements a **registry pattern** for coordinating file watchers across
//! multiple repositories:
//!
//! ```text
//! MultiRepoWatcher
//! ├── WatchCoordinator (shared, single instance)
//! ├── watched_repos: HashMap<PathBuf, WatchedRepo>
//! │   ├── /home/user/project1 → {clients: [12345, 67890], git_dir: ...}
//! │   ├── /home/user/project2 → {clients: [11111], git_dir: ...}
//! │   └── ...
//! ```
//!
//! **Why a registry?** Fish shells can navigate between multiple repositories in a single
//! session. Without coordination, each repository might get its own watcher, wasting
//! resources. The registry ensures one watcher per repository, shared by all clients.
//!
//! ## Reference Counting
//!
//! **Lifecycle**:
//! 1. Client registers with cwd `/home/user/project` → Find git root → Start watching
//! 2. Another client registers same repo → Increment reference count (no new watcher)
//! 3. First client unregisters → Decrement count
//! 4. Last client unregisters → Stop watcher, clean up
//!
//! **Implementation**: `WatchedRepo.clients` tracks PIDs. When empty, watcher stops.
//!
//! ## Single `WatchCoordinator` Design
//!
//! **Architecture choice**: All repositories share one `WatchCoordinator` instance.
//!
//! **Why?** The `WatchCoordinator` has a global debouncer and flush thread. Multiple
//! coordinators would mean multiple threads (wasteful). Instead, we watch multiple
//! paths with one coordinator, and the debouncer deduplicates by repository path.
//!
//! **Trade-off**: All repos share the same debounce window (100ms by default).
//! This is acceptable
//! because the debouncer already groups events by repo path internally.
//!
//! ## Worktree Support
//!
//! **Challenge**: Git worktrees share a `.git` directory but have separate working trees.
//!
//! **Default**: Worktree watching is **enabled by default** (the
//! `.unwrap_or(true)` in [`super::WatchRegistry::new`]), so editing a tracked
//! file in a linked worktree updates the prompt instantly without extra
//! configuration.
//!
//! **Precedence**: The effective setting comes from the `git.watch_worktree` config
//! value, applied via [`MultiRepoWatcher::set_watch_worktree`], which stores it on
//! this watcher's [`super::WatchRegistry`]. An explicit
//! `GPY_WATCH_WORKTREE` environment variable overrides the config value: any of
//! `0`/`false`/`no`/`off` (case-insensitive) disables worktree watching, and any
//! other value enables it. When enabled, the
//! worktree directory is watched in addition to `.git`, catching file changes in
//! linked worktrees.
//!
//! ## Thread Safety
//!
//! **Shared state protection**:
//! - `watcher: Arc<Mutex<Option<WatchCoordinator>>>` - Single coordinator, nullable for shutdown
//! - `watched_repos: Arc<Mutex<HashMap<...>>>` - Registry of active watches
//!
//! Both mutexes are held briefly during register/unregister operations. The watcher
//! itself runs in background threads (see [`super::WatchCoordinator`] docs).

use super::watch_set::WatchMode;
use super::{EventCallback, WatchCoordinator, WatchRegistry, WatcherConfig};
use crate::debug_log;
use crate::{Error, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
struct WatchedRepo {
    git_root: PathBuf,
    git_dir: PathBuf,
    /// Identity of `git_dir` at watch time, used to detect same-path recreation
    /// (delete + recreate yields a new inode) so a stale OS watch is re-armed on
    /// the next re-register instead of silently missing events (issue #267).
    git_dir_id: DirIdentity,
    /// Requested vs. actually-effective worktree-watch state. See
    /// [`WorktreeWatch`].
    worktree: WorktreeWatch,
    /// Paths this watcher actually armed for this repository — exactly what
    /// [`MultiRepoWatcher::start_repo_watches`] (or its
    /// [`MultiRepoWatcher::fall_back_to_gitdir_only`] fallback) returned.
    /// Teardown (`unregister_client_locked`, `rearm_repo_watch`) unwatches
    /// exactly these paths, never more and never less — so a degraded repo,
    /// whose `git_root` was never armed, cannot have it handed to
    /// `unwatch_directory` alongside `git_dir` (#616).
    armed: Vec<PathBuf>,
    clients: HashSet<u32>,
}

/// Requested vs. actually-effective worktree-watching state for one
/// repository's watches.
///
/// Replaces the former pair of `watch_worktree`/`requested_worktree` bools
/// on `WatchedRepo` (#616): the two were never independent (`Armed`/`Disabled`
/// only ever occur together with `requested == effective`; only a failed
/// recursive watch peels them apart into `Degraded`), so representing the
/// three actually-reachable combinations as an enum makes the fourth,
/// meaningless one (`effective == true, requested == false`) unrepresentable
/// instead of merely absent from every code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorktreeWatch {
    /// Worktree watching was not requested, so only `git_dir` is watched.
    Disabled,
    /// Worktree watching was requested and the recursive watch is installed.
    Armed,
    /// Worktree watching was requested, but the recursive watch failed, so
    /// this repository degraded to `.git`-only watching (issue #158). Kept
    /// distinct from `Disabled` so a config comparison re-arms exactly when
    /// the *requested* setting changes, never on every re-register of an
    /// already-degraded repo (#444) — see [`WorktreeWatch::requested`].
    Degraded,
}

impl WorktreeWatch {
    /// Whether the recursive worktree watch actually took effect.
    ///
    /// Teardown no longer needs this: it unwatches [`WatchedRepo::armed`]
    /// directly, which already reflects exactly what got installed. Kept as
    /// a query for tests (and any future caller) that want to assert on the
    /// effective state without reasoning about `armed`'s contents.
    #[cfg(test)]
    const fn effective(self) -> bool {
        matches!(self, Self::Armed)
    }

    /// Whether worktree watching was *asked for* when these watches were
    /// armed, i.e. the effective `git.watch_worktree` value at that moment.
    /// Distinct from [`WorktreeWatch::effective`] so a config change can be
    /// distinguished from a degraded watch: comparing the config against the
    /// degraded effective state would re-arm a permanently failing repo on
    /// every single registration (watch thrash), whereas comparing against
    /// the requested state re-arms exactly when the setting really changed
    /// (#444).
    const fn requested(self) -> bool {
        matches!(self, Self::Armed | Self::Degraded)
    }
}

/// What [`MultiRepoWatcher::start_repo_watches`] (or its
/// [`MultiRepoWatcher::fall_back_to_gitdir_only`] fallback) actually armed
/// for one repository (#616).
struct RepoWatches {
    /// Paths armed for this repository. Teardown
    /// (`unregister_client_locked`, `rearm_repo_watch`) unwatches exactly
    /// these paths.
    armed: Vec<PathBuf>,
    /// The worktree-watch state that resulted.
    worktree: WorktreeWatch,
}

/// Bookkeeping for one shared common git directory watched on behalf of the
/// linked worktrees that depend on it (#468).
///
/// Instance-local, like the attribution mapping in
/// [`super::WatchRegistry::register_common_gitdir`], because watches belong to
/// *this* watcher's coordinator: a second `MultiRepoWatcher` in the same
/// process must arm its own.
#[derive(Debug, Default)]
struct CommonDirWatch {
    /// Canonical working roots of the registered linked worktrees reading this
    /// common directory. The reference count: the entry — and the watches it
    /// armed — live exactly as long as this set is non-empty.
    dependents: HashSet<PathBuf>,
    /// Paths this watcher actually armed for the common directory, each with
    /// the mode it was armed in so teardown releases exactly that
    /// registration (#687). Empty when an already-watched ancestor covered
    /// them (e.g. the main checkout's own recursive worktree watch), so
    /// teardown never unwatches a path it did not watch.
    armed: ArmedWatches,
}

/// Paths armed on behalf of a common directory, each with the mode it was
/// armed in (#687).
type ArmedWatches = Vec<(PathBuf, WatchMode)>;

/// Opaque `(dev, ino)` identity of a watched directory. `None` on platforms
/// without a portable inode (identity checks are skipped there; the periodic
/// reconcile remains the backstop).
type DirIdentity = Option<(u64, u64)>;

/// Capture the filesystem identity of `path`, or `None` if it cannot be stated
/// or the platform has no portable inode.
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

/// Cap on `.git` gitdir-pointer file reads: a single-line pointer file by
/// construction.
const GITDIR_POINTER_READ_CAP: usize = 4_096;

/// Cap on `commondir` pointer file reads: a single-line pointer file by
/// construction.
const COMMONDIR_POINTER_READ_CAP: usize = 4_096;

/// Test-only fault injection for the watch calls `start_repo_watches` (and
/// its `fall_back_to_gitdir_only` fallback) issue.
///
/// Keyed by the path a real OS watch call would receive. Returning
/// `Some(err)` for a path makes [`MultiRepoWatcher::arm_watch`] fail exactly
/// as a real watch-budget exhaustion would, without a real resource limit;
/// returning `None` lets the real `watch_directory` call proceed (#616).
///
/// Replaces the former pair of `TEST_FAIL_GIT_WATCH`/`TEST_FAIL_WORKTREE_WATCH`
/// process-wide `AtomicBool`s, which forced every test that flipped one to be
/// `#[serial]` with every other such test in the process. A per-instance
/// closure needs no such serialization: each test's watcher carries its own
/// fault, so two of these tests can now run concurrently.
#[cfg(test)]
type WatchFault = Arc<dyn Fn(&Path) -> Option<Error> + Send + Sync>;

/// Multi-repository watcher registry with reference counting.
///
/// Manages multiple git repositories by coordinating a single, shared
/// `WatchCoordinator`. Uses reference counting to automatically start/stop
/// watching repositories as clients register/unregister their working
/// directories.
///
/// All repositories share the single `WatchCoordinator` held in `watcher`;
/// debounced events are attributed back to their originating repository by
/// path. This registry tracks which client PIDs are interested in which
/// repositories.
pub struct MultiRepoWatcher {
    watcher: Arc<Mutex<Option<WatchCoordinator>>>,
    watched_repos: Arc<Mutex<HashMap<PathBuf, WatchedRepo>>>,
    /// State shared with the coordinator and the filesystem layer beneath it:
    /// the roots-only mirror of `watched_repos`'s keys used for event
    /// attribution, the external/common gitdir attribution maps, the
    /// worktree-watch flag and the ignore caches (see [`WatchRegistry`]).
    registry: Arc<WatchRegistry>,
    /// Shared common git directories watched for linked worktrees, keyed by
    /// canonical common directory (#468).
    common_watches: Mutex<HashMap<PathBuf, CommonDirWatch>>,
    ops_lock: Mutex<()>,
    /// Test-only fault injection consulted by [`Self::arm_watch`] (#616).
    #[cfg(test)]
    watch_fault: Option<WatchFault>,
}

impl MultiRepoWatcher {
    /// Create a new builder for `MultiRepoWatcher`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
    /// # use gpy_agent::watcher::DebouncedEvent;
    /// # use gpy_agent::Result;
    /// # fn main() -> Result<()> {
    /// let watcher = MultiRepoWatcher::builder()
    ///     .callback(Box::new(|event: DebouncedEvent| {
    ///         println!("Event: {:?}", event);
    ///     }))
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn builder() -> MultiRepoWatcherBuilder {
        MultiRepoWatcherBuilder::new()
    }

    /// Register a client and begin watching the repository it resides in.
    ///
    /// # Errors
    ///
    /// Returns an error if the repository cannot be discovered or watched.
    pub fn register_client(&self, pid: u32, cwd: &Path) -> Result<()> {
        let _ops_guard = self
            .ops_lock
            .lock()
            .map_err(|_| Error::watcher("Failed to lock watcher operations"))?;
        self.attach_pid_to(pid, cwd)
    }

    /// Attach `pid` to the repository containing `cwd` and detach it from every
    /// other repository, so afterwards it belongs to exactly one repository
    /// (or none when `cwd` is outside any repository). Caller holds `ops_lock`.
    ///
    /// The target is attached (and armed, if new) *before* the others are
    /// released, so a repository the pid stays in is never torn down and
    /// re-armed, and a repository only the pid used is released once the new
    /// one is watching. If attaching fails the pid is still detached from every
    /// other repository and the attach error is returned.
    ///
    /// # Errors
    ///
    /// Returns an error if the repository cannot be discovered or watched, or
    /// if the stale repositories cannot be released.
    fn attach_pid_to(&self, pid: u32, cwd: &Path) -> Result<()> {
        debug_log!(
            "watcher",
            "attach_pid_to: pid={}, cwd={}",
            pid,
            cwd.display()
        );

        let attached = self.attach_to_repo_containing(pid, cwd);
        // On failure `keep` is `None`, so the pid also leaves the repository
        // it was already in if re-arming that one is what failed.
        let keep = attached.as_ref().ok().and_then(Option::as_deref);
        let detached = self.detach_pid_except(pid, keep);
        attached?;
        detached
    }

    /// Attach `pid` to the repository containing `cwd`, arming it if it is not
    /// yet watched. Returns its root, or `None` when `cwd` is outside any
    /// repository.
    ///
    /// # Errors
    ///
    /// Returns an error if the repository cannot be discovered or watched.
    fn attach_to_repo_containing(&self, pid: u32, cwd: &Path) -> Result<Option<PathBuf>> {
        let Some(git_root) = Self::find_git_root(cwd) else {
            debug_log!("watcher", "No git root found for {}", cwd.display());
            return Ok(None);
        };

        let git_dir = Self::resolve_gitdir(&git_root)?;
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

        // Record the watches that actually took effect: a failed recursive
        // worktree watch degrades to .git-only rather than failing registration.
        let repo_watches = self.start_repo_watches(&git_root, &git_dir, watch_worktree)?;
        self.track_new_repo(pid, git_root.clone(), git_dir, repo_watches)?;
        Ok(Some(git_root))
    }

    /// Remove a client and stop watching the repository if no clients remain.
    ///
    /// # Errors
    ///
    /// Returns an error when watcher state cannot be updated.
    pub fn unregister_client(&self, pid: u32) -> Result<()> {
        let _ops_guard = self
            .ops_lock
            .lock()
            .map_err(|_| Error::watcher("Failed to lock watcher operations"))?;
        self.detach_pid_except(pid, None)
    }

    /// Remove `pid` from every tracked repository other than `keep`, tearing
    /// down only the repositories whose client set becomes empty.
    ///
    /// # Errors
    ///
    /// Returns an error when watcher state cannot be updated.
    fn detach_pid_except(&self, pid: u32, keep: Option<&Path>) -> Result<()> {
        let repos_to_remove = {
            let mut repos_to_remove = Vec::new();
            {
                let mut repos = self
                    .watched_repos
                    .lock()
                    .map_err(|_| Error::watcher("Failed to lock watched repos"))?;

                for repo in repos.values_mut() {
                    if keep.is_some_and(|kept| kept == repo.git_root) {
                        continue;
                    }
                    if repo.clients.remove(&pid) && repo.clients.is_empty() {
                        repos_to_remove.push(repo.clone());
                    }
                }
            }
            repos_to_remove
        };

        self.tear_down_repos(&repos_to_remove)
    }

    /// Stop watching `repos` (each already out of clients) and drop every
    /// piece of state tied to them.
    ///
    /// # Errors
    ///
    /// Returns an error when watcher state cannot be updated.
    fn tear_down_repos(&self, repos_to_remove: &[WatchedRepo]) -> Result<()> {
        if let Ok(mut watcher) = self.watcher.lock()
            && let Some(w) = watcher.as_mut()
        {
            for repo in repos_to_remove {
                for path in &repo.armed {
                    Self::unwatch_logged(w, path, WatchMode::Recursive);
                }
            }
        }

        {
            let mut repos = self
                .watched_repos
                .lock()
                .map_err(|_| Error::watcher("Failed to lock watched repos"))?;
            for repo in repos_to_remove {
                repos.remove(&repo.git_root);
            }
        }

        self.registry
            .remove_roots(repos_to_remove.iter().map(|repo| repo.git_root.as_path()));

        // Mirror the removal into the external-gitdir map (#428). Harmless for
        // ordinary repos, whose gitdirs were never inserted.
        for repo in repos_to_remove {
            self.registry.unregister_external_gitdir(&repo.git_dir);
            // Reference-counted: only the last dependent worktree actually
            // drops the shared common-dir watch (#468).
            self.release_common_dir_watches(&repo.git_root);
        }

        Ok(())
    }

    /// Move a client to `new_cwd`: attach it to the repository containing
    /// `new_cwd` and detach it from every other one. A move within the
    /// repository the client already belongs to touches no OS watch.
    ///
    /// # Errors
    ///
    /// Returns an error if the new repository cannot be discovered or
    /// watched; the client is then attached to no repository.
    pub fn update_client(&self, pid: u32, new_cwd: &Path) -> Result<()> {
        let _ops_guard = self
            .ops_lock
            .lock()
            .map_err(|_| Error::watcher("Failed to lock watcher operations"))?;
        self.attach_pid_to(pid, new_cwd)
    }

    /// Stop watching all repositories and clear state.
    pub fn stop(&self) {
        if let Ok(mut watcher) = self.watcher.lock()
            && let Some(mut w) = watcher.take()
        {
            w.stop();
        }
        if let Ok(mut repos) = self.watched_repos.lock() {
            // Drop this instance's external-gitdir mappings from the shared
            // registry before clearing the local map (#428).
            for repo in repos.values() {
                self.registry.unregister_external_gitdir(&repo.git_dir);
            }
            repos.clear();
        }
        self.registry.clear_roots();
        // The coordinator above already tore every OS watch down, so this only
        // has to drop the bookkeeping and this instance's entries in the shared
        // common-gitdir attribution registry (#468).
        if let Ok(mut common) = self.common_watches.lock() {
            for (common_dir, entry) in common.drain() {
                for root in &entry.dependents {
                    self.registry.unregister_common_gitdir(&common_dir, root);
                }
            }
        }
    }

    /// Attach a client to an already-watched repository, re-arming the OS watch
    /// first if the tracked `git_dir` has been recreated or relocated on disk.
    ///
    /// Returns `Ok(true)` when the repository was already tracked (client added,
    /// watch re-armed if it had gone stale) and `Ok(false)` when it is unknown,
    /// leaving the caller to perform a fresh registration.
    ///
    /// A heartbeat re-register normally short-circuits here without touching the
    /// OS watch. But if the watched `.git` directory was deleted and recreated at
    /// the same path (new inode) — or the gitdir pointer moved — the original
    /// watch is dead, so re-arm it now rather than waiting for the ~45s reconcile
    /// to recover the missed events (issue #267, finding #4).
    ///
    /// The same applies when `git.watch_worktree` changed since these watches
    /// were armed: false → true means no worktree watch was ever installed even
    /// though event filtering now expects worktree events, and true → false
    /// leaves a needless recursive watch behind (#444). The comparison is
    /// against the repo's *requested* state, never its (possibly degraded)
    /// effective state — see [`WorktreeWatch::requested`].
    ///
    /// # Errors
    ///
    /// Returns an error when the map of watched repositories cannot be locked, or
    /// when re-arming the essential `.git` watch fails.
    fn attach_or_rearm_existing_repo(
        &self,
        pid: u32,
        git_root: &Path,
        git_dir: &Path,
        watch_worktree: bool,
    ) -> Result<bool> {
        // Snapshot the tracked state, then release the lock before touching the
        // watcher lock (the register/unregister ops_lock already serializes us,
        // so no other thread can mutate the entry in between).
        let tracked = {
            let repos = self
                .watched_repos
                .lock()
                .map_err(|_| Error::watcher("Failed to lock watched repos"))?;
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
            let mut repos = self
                .watched_repos
                .lock()
                .map_err(|_| Error::watcher("Failed to lock watched repos"))?;
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

    /// Re-arm a stale watch for an already-tracked repository: unwatch the old
    /// (recreated/relocated) paths, watch the current gitdir, and refresh the
    /// tracked entry's `git_dir`/identity/worktree flags and client set.
    ///
    /// `pid` is `Some` when a client triggered the re-arm (it is added to the
    /// repo's client set) and `None` for a config-driven bulk re-arm, which must
    /// leave the reference count exactly as it found it (#444).
    ///
    /// # Errors
    ///
    /// Returns an error if the essential `.git` watch cannot be re-established or
    /// the watched-repos map cannot be locked.
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

        // Re-read the stale watch paths to release before taking the watcher lock
        // (ops_lock still serializes us, so the entry cannot change underneath).
        let stale = {
            let repos = self
                .watched_repos
                .lock()
                .map_err(|_| Error::watcher("Failed to lock watched repos"))?;
            repos
                .get(git_root)
                .map(|repo| (repo.git_dir.clone(), repo.armed.clone()))
        };

        if let Ok(mut watcher) = self.watcher.lock()
            && let Some(w) = watcher.as_mut()
            && let Some((_, stale_armed)) = &stale
        {
            for path in stale_armed {
                Self::unwatch_logged(w, path, WatchMode::Recursive);
            }
        }

        // The gitdir pointer may have relocated, which can move the common
        // directory with it. Release the old dependency (unwatching only if this
        // was the last dependent) so `start_repo_watches` re-acquires against
        // whatever `git_dir` now points at (#468).
        self.release_common_dir_watches(git_root);

        let repo_watches = self.start_repo_watches(git_root, git_dir, watch_worktree)?;

        // The gitdir pointer may have relocated (e.g. gitdir file rewritten):
        // drop the stale external-gitdir mapping and re-register the current one
        // so attribution keeps resolving to this working root (#428). Both are
        // no-ops for ordinary in-tree gitdirs.
        if let Some((stale_git_dir, _)) = &stale {
            self.registry.unregister_external_gitdir(stale_git_dir);
        }
        self.register_external_gitdir_if_needed(git_dir, git_root);

        {
            let mut repos = self
                .watched_repos
                .lock()
                .map_err(|_| Error::watcher("Failed to lock watched repos"))?;
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

    /// Arm a single OS watch on `path`, the one seam every `watch_directory`
    /// call in [`Self::start_repo_watches`] and
    /// [`Self::fall_back_to_gitdir_only`] must route through.
    ///
    /// Under `#[cfg(test)]` this consults `self.watch_fault` first, so a test
    /// can deterministically simulate a watch failure (e.g. OS watch-budget
    /// exhaustion) for a specific path without a real resource limit and
    /// without a process-wide flag other tests would have to serialize
    /// around (#616).
    ///
    /// # Errors
    ///
    /// Returns an error if `path` cannot be watched, or (under
    /// `#[cfg(test)]` only) if `self.watch_fault` reports one for `path`.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "self carries watch_fault only under #[cfg(test)]; a non-test build has \
                      nothing on self to consult, so this parameter goes unused there by \
                      construction -- keeping one signature for both configurations is worth \
                      more than avoiding it in the tests-are-compiled-out case"
        )
    )]
    fn arm_watch(&self, w: &mut WatchCoordinator, path: &Path) -> Result<()> {
        #[cfg(test)]
        if let Some(fault) = self.watch_fault.as_ref()
            && let Some(err) = fault(path)
        {
            return Err(err);
        }
        w.watch_directory(path)
    }

    /// Release one `mode` watch of `path`, logging (never silently dropping)
    /// a failure.
    ///
    /// Teardown must never leave a stale OS watch unaccounted for, but a
    /// failure here is not actionable by the caller — the watch is going
    /// away regardless — so this logs and moves on rather than propagating
    /// an error (#616).
    fn unwatch_logged(w: &mut WatchCoordinator, path: &Path, mode: WatchMode) {
        let released = match mode {
            WatchMode::Recursive => w.unwatch_directory(path),
            WatchMode::Shallow => w.unwatch_directory_shallow(path),
        };
        if let Err(err) = released {
            debug_log!("watcher", "Failed to unwatch {}: {err}", path.display());
        }
    }

    /// Start filesystem watching for the supplied repository.
    ///
    /// Watching the `.git` directory is essential: a failure there is propagated
    /// so the caller can avoid tracking a repository that is not actually
    /// watched. Recursive worktree watching is best-effort — it is the most
    /// likely operation to exhaust the OS watch budget on large repositories, so
    /// a failure there degrades to `.git`-only watching instead of failing the
    /// whole registration. Returns the paths actually armed and the
    /// watch-worktree state that took effect.
    ///
    /// # Errors
    ///
    /// Returns an error if the watcher coordinator lock is poisoned, if the
    /// coordinator is gone (e.g. after `stop()`), or if the essential `.git`
    /// directory cannot be watched. A missing/poisoned coordinator must be an
    /// error here rather than a silent no-op: falling through would let the
    /// caller record the repository as watched even though no OS watch was
    /// ever established (#323).
    #[expect(
        clippy::significant_drop_tightening,
        reason = "the guard is held for the whole function body on purpose -- `w` is reused across several sequential watch_directory calls in different branches below, and they must run against one consistent coordinator reference rather than re-locking (and potentially observing a coordinator swapped out by stop()) between calls"
    )]
    fn start_repo_watches(
        &self,
        git_root: &Path,
        git_dir: &Path,
        watch_worktree: bool,
    ) -> Result<RepoWatches> {
        let mut watcher_guard = self
            .watcher
            .lock()
            .map_err(|_| Error::watcher("Failed to lock watch coordinator"))?;
        let Some(w) = watcher_guard.as_mut() else {
            return Err(Error::watcher(
                "No watch coordinator available; repository would not actually be watched",
            ));
        };

        // #388: on macOS, notify's FSEvents backend cannot incrementally add a
        // path to an already-running FSEventStream -- it tears the stream down
        // and rebuilds it. Two sequential `watch_directory` calls on the same
        // coordinator (one for `git_dir`, one for `git_root`) rebuild the
        // stream a moment after it's armed, and the rebuilt stream can
        // silently fail to deliver its first event. When `git_dir` is nested
        // under `git_root` (the common case -- anything but a linked
        // worktree/submodule with an external gitdir pointer, see
        // `resolve_gitdir`), a single recursive watch on `git_root` already
        // covers `.git` internally, so the second call is both redundant and
        // the source of the rebuild. Skip it in that case.
        let gitdir_covered_by_worktree_watch = git_dir.starts_with(git_root);

        if watch_worktree && gitdir_covered_by_worktree_watch {
            if let Err(e) = self.arm_watch(w, git_root) {
                return self.fall_back_to_gitdir_only(w, git_root, git_dir, &e.to_string());
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

        self.arm_watch(w, git_dir)?;
        debug_log!(
            "watcher",
            "Successfully started watching {}",
            git_dir.display()
        );
        let mut armed = vec![git_dir.to_path_buf()];

        // Only linked worktrees reach here with a `commondir` pointer, so this
        // is a no-op for submodules and (via the branch above) ordinary repos
        // (#468). Best-effort like the worktree watch below: shared-metadata
        // fan-out is an enhancement over the per-worktree admin dir watch just
        // armed, never a precondition for registering the repository. Its own
        // watches are tracked separately in `common_watches`, not in this
        // repo's `armed` set: they are reference-counted across dependent
        // worktrees and torn down by `release_common_dir_watches`, never by
        // this repo's own teardown.
        self.acquire_common_dir_watches(w, git_root, git_dir);

        let worktree = if watch_worktree {
            if let Err(e) = self.arm_watch(w, git_root) {
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

    /// Resolve the *common* git directory shared by every worktree of the
    /// repository whose per-worktree admin directory is `git_dir`.
    ///
    /// Returns `None` for anything that is not a linked worktree — an ordinary
    /// repository and a submodule's own checkout both have a git directory that
    /// *is* the common directory, and neither carries a `commondir` pointer — so
    /// their watch behavior is untouched (#468).
    fn resolve_common_dir(git_dir: &Path) -> Option<PathBuf> {
        let common =
            crate::fs_util::read_small_file(&git_dir.join("commondir"), COMMONDIR_POINTER_READ_CAP)
                .ok()
                .and_then(|contents| Self::join_commondir_pointer(git_dir, &contents))
                .or_else(|| Self::common_dir_from_worktrees_layout(git_dir))?;
        (common != git_dir).then_some(common)
    }

    /// Parse a `commondir` pointer file's contents into the
    /// (un-canonicalized) path it names, joined against `git_dir` if
    /// relative. `None` for an empty or whitespace-only pointer.
    #[must_use]
    fn parse_commondir_pointer(git_dir: &Path, contents: &str) -> Option<PathBuf> {
        let target = Path::new(contents.trim());
        if target.as_os_str().is_empty() {
            return None;
        }
        // Same absolute-passthrough / relative-join rule as a `gitdir:` pointer,
        // minus the prefix — a `commondir` file holds a bare path.
        Some(crate::git::resolve_relative_to(git_dir, target))
    }

    /// Resolve the contents of a `commondir` pointer file (typically `../..`)
    /// against the admin directory holding it.
    fn join_commondir_pointer(git_dir: &Path, contents: &str) -> Option<PathBuf> {
        let resolved = Self::parse_commondir_pointer(git_dir, contents)?;
        fs::canonicalize(&resolved).ok()
    }

    /// Fallback used only when `<git_dir>/commondir` is missing or unreadable.
    ///
    /// git lays a linked worktree's admin directory out at
    /// `<common>/worktrees/<name>`, so dropping those two components recovers
    /// the common directory. That is a layout assumption rather than something
    /// git promises, which is why the `commondir` file is preferred; this only
    /// keeps a worktree whose pointer file cannot be read from silently losing
    /// shared-metadata refreshes.
    fn common_dir_from_worktrees_layout(git_dir: &Path) -> Option<PathBuf> {
        let parent = git_dir.parent()?;
        if parent.file_name()? != "worktrees" {
            return None;
        }
        fs::canonicalize(parent.parent()?).ok()
    }

    /// Register `git_root` as a dependent of its common git directory and, if it
    /// is the first dependent, arm the watches covering that directory (#468).
    fn acquire_common_dir_watches(
        &self,
        w: &mut WatchCoordinator,
        git_root: &Path,
        git_dir: &Path,
    ) {
        let Some(common_dir) = Self::resolve_common_dir(git_dir) else {
            return;
        };

        if let Ok(mut guard) = self.common_watches.lock() {
            let entry = guard.entry(common_dir.clone()).or_default();
            let first_dependent = entry.dependents.is_empty();
            entry.dependents.insert(git_root.to_path_buf());
            if first_dependent {
                entry.armed = Self::arm_common_dir(w, &common_dir);
            }
        }

        self.registry.register_common_gitdir(&common_dir, git_root);
    }

    /// Arm the minimal watch set covering a common git directory's shared
    /// status inputs, returning the paths actually armed.
    ///
    /// Two watches, neither recursive over the common directory itself:
    ///
    /// - `<common>` shallow — `packed-refs` and `config` sit directly inside.
    ///   Watching the directory rather than those two files survives git's
    ///   write-lock-then-rename, which orphans an inode-based file watch.
    /// - `<common>/refs` recursive — a small tree holding `heads/**` and
    ///   `remotes/**`. Watching `refs` rather than those two subdirectories also
    ///   covers a `remotes/` that does not exist yet, in a repository whose
    ///   first remote is added later.
    ///
    /// A recursive watch on `<common>` is what both of these avoid: under the
    /// poll backend it would expand into `objects/**` for a stat walk every poll
    /// interval (#463), over churn `filter_ignored` discards anyway.
    ///
    /// Each path is skipped when an existing watch already covers it — when the
    /// main checkout is registered, its single recursive worktree watch already
    /// covers `<common>` — so nothing rebuilds the `FSEventStream` for no gain
    /// (#388).
    fn arm_common_dir(w: &mut WatchCoordinator, common_dir: &Path) -> ArmedWatches {
        let mut armed = Vec::new();

        if w.covers_directory(common_dir) {
            debug_log!(
                "watcher",
                "Common gitdir {} already covered by an existing watch",
                common_dir.display()
            );
        } else if let Err(e) = w.watch_directory_shallow(common_dir) {
            debug_log!(
                "watcher",
                "Shallow watch of common gitdir {} failed ({}); shared metadata changes may be missed",
                common_dir.display(),
                e
            );
        } else {
            armed.push((common_dir.to_path_buf(), WatchMode::Shallow));
        }

        let refs_dir = common_dir.join("refs");
        if refs_dir.is_dir() && !w.covers_directory(&refs_dir) {
            if let Err(e) = w.watch_directory(&refs_dir) {
                debug_log!(
                    "watcher",
                    "Watch of common refs {} failed ({}); shared ref changes may be missed",
                    refs_dir.display(),
                    e
                );
            } else {
                armed.push((refs_dir, WatchMode::Recursive));
            }
        }

        armed
    }

    /// Drop `git_root` as a dependent of every common git directory it was
    /// registered against, unwatching the shared watches only once the last
    /// dependent worktree is gone, and re-arm any surviving common directory
    /// whose covering ancestor watch has just been dropped (#468).
    fn release_common_dir_watches(&self, git_root: &Path) {
        let mut to_unwatch: ArmedWatches = Vec::new();
        let mut to_rearm: Vec<PathBuf> = Vec::new();

        // Scoped so `common_watches` is released before the watcher lock is
        // taken: `acquire_common_dir_watches` runs with the watcher lock already
        // held, so nesting them the other way round here would invert the order.
        {
            let Ok(mut guard) = self.common_watches.lock() else {
                return;
            };
            guard.retain(|common_dir, entry| {
                if entry.dependents.remove(git_root) {
                    self.registry.unregister_common_gitdir(common_dir, git_root);
                }
                if entry.dependents.is_empty() {
                    to_unwatch.append(&mut entry.armed);
                    return false;
                }
                true
            });
            // A surviving entry that armed nothing was relying on an ancestor
            // watch — typically the main checkout's own recursive worktree
            // watch — and that ancestor may be exactly the watch
            // `unregister_client_locked` just dropped. Note this is checked for
            // *every* removal, not only the removal of a dependent: it is the
            // main checkout leaving, not a worktree, that drops the covering
            // watch out from under the worktrees that stay.
            to_rearm.extend(
                guard
                    .iter()
                    .filter(|(_, entry)| entry.armed.is_empty())
                    .map(|(common_dir, _)| common_dir.clone()),
            );
        }

        if to_unwatch.is_empty() && to_rearm.is_empty() {
            return;
        }

        let mut rearmed: Vec<(PathBuf, ArmedWatches)> = Vec::new();
        if let Ok(mut watcher) = self.watcher.lock()
            && let Some(w) = watcher.as_mut()
        {
            for (path, mode) in &to_unwatch {
                Self::unwatch_logged(w, path, *mode);
            }
            for common_dir in to_rearm {
                // A no-op returning an empty vec while the ancestor watch is
                // still in place, so this costs nothing in the common case.
                let armed = Self::arm_common_dir(w, &common_dir);
                if !armed.is_empty() {
                    rearmed.push((common_dir, armed));
                }
            }
        }

        if rearmed.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.common_watches.lock() {
            for (common_dir, armed) in rearmed {
                if let Some(entry) = guard.get_mut(&common_dir) {
                    entry.armed = armed;
                }
            }
        }
    }

    /// Fall back to watching `git_dir` alone after a worktree-root watch
    /// attempt failed (or was injected to fail in tests), logging `reason`.
    /// Always returns the degraded [`WorktreeWatch::Degraded`] state, armed
    /// on `git_dir` only, on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the `.git`-only watch also fails.
    fn fall_back_to_gitdir_only(
        &self,
        w: &mut WatchCoordinator,
        git_root: &Path,
        git_dir: &Path,
        reason: &str,
    ) -> Result<RepoWatches> {
        debug_log!(
            "watcher",
            "Worktree watch failed for {} ({reason}); falling back to .git-only watching",
            git_root.display()
        );
        self.arm_watch(w, git_dir)?;
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
    ///
    /// # Errors
    ///
    /// Returns an error when the watcher map cannot be locked.
    fn track_new_repo(
        &self,
        pid: u32,
        git_root: PathBuf,
        git_dir: PathBuf,
        repo_watches: RepoWatches,
    ) -> Result<()> {
        let mut clients = HashSet::new();
        clients.insert(pid);
        let git_dir_id = dir_identity(&git_dir);
        let root_key = git_root.clone();

        // Only submodules/linked worktrees have an *external* gitdir (one not
        // under the working root); register those so the filesystem layer can
        // attribute their gitdir-internal events back to this working root
        // (#428). Ordinary repos (gitdir under the root) never enter the map.
        // Done before the move into `WatchedRepo` so the paths are still owned.
        self.register_external_gitdir_if_needed(&git_dir, &git_root);

        {
            let mut repos = self
                .watched_repos
                .lock()
                .map_err(|_| Error::watcher("Failed to lock watched repos"))?;

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

        Ok(())
    }

    /// Insert an external gitdir → working-root mapping when `git_dir` lives
    /// outside `git_root` (submodule/linked worktree). No-op for ordinary repos
    /// whose gitdir is nested under the working tree.
    fn register_external_gitdir_if_needed(&self, git_dir: &Path, git_root: &Path) {
        if !git_dir.starts_with(git_root) {
            self.registry.register_external_gitdir(git_dir, git_root);
        }
    }

    /// Find the canonical git root directory for the given path.
    ///
    /// Walks up the directory tree from `path` looking for a `.git` directory,
    /// then returns the canonicalized path to the repository root.
    ///
    /// The walk itself lives in [`crate::git::find_git_dir_only`], shared with
    /// [`crate::git::find_repo_root`]; this stays the watcher-facing name and
    /// behavior is unchanged. It deliberately does *not* recognize bare
    /// repositories — use `crate::git::find_repo_root` when that matters.
    #[must_use]
    pub fn find_git_root(path: &Path) -> Option<PathBuf> {
        crate::git::find_git_dir_only(path)
    }

    /// Resolve the actual git directory for the supplied repository.
    ///
    /// # Errors
    ///
    /// Returns an error when the `.git` entry cannot be read or parsed.
    fn resolve_gitdir(git_root: &Path) -> Result<PathBuf> {
        let git_entry = git_root.join(".git");

        if git_entry.is_dir() {
            return Ok(fs::canonicalize(&git_entry).unwrap_or(git_entry));
        }

        if git_entry.is_file() {
            let contents = crate::fs_util::read_small_file(&git_entry, GITDIR_POINTER_READ_CAP)
                .map_err(|e| {
                    Error::watcher(format!(
                        "Failed to read gitdir pointer at {}: {e}",
                        git_entry.display()
                    ))
                })?;

            let resolved = crate::git::parse_gitdir_link(&contents, git_root).ok_or_else(|| {
                Error::watcher(format!(
                    "Invalid gitdir pointer contents at {}",
                    git_entry.display()
                ))
            })?;

            return Ok(fs::canonicalize(&resolved).unwrap_or(resolved));
        }

        Err(Error::watcher(format!(
            "Could not locate git metadata at {}",
            git_entry.display()
        )))
    }

    /// The [`WatchRegistry`] this watcher shares with its coordinator and the
    /// filesystem layer beneath it.
    #[must_use]
    pub const fn registry(&self) -> &Arc<WatchRegistry> {
        &self.registry
    }

    /// Apply the effective worktree-watching setting to this watcher's registry.
    ///
    /// `config_enabled` comes from `git.watch_worktree`; an explicit
    /// `GPY_WATCH_WORKTREE` env var overrides it. Newly registered repositories
    /// pick up the new value; already-active ones are brought into line by
    /// [`Self::resync_watch_worktree`], which the config-reload path calls right
    /// after this (#444).
    pub fn set_watch_worktree(&self, config_enabled: bool) {
        self.registry.set_worktree_enabled(config_enabled);
    }

    /// Bring every already-watched repository into line with the current
    /// effective `watch_worktree` setting, re-arming the ones whose watches were
    /// armed under the previous value (#444).
    ///
    /// Without this, a `false → true` change only updates the registry flag: event
    /// filtering starts expecting worktree events while active repositories
    /// still carry a `.git`-only watch, so edits, deletions and untracked files
    /// stay invisible until the client happens to re-register or the ~45s
    /// reconcile runs. The reverse change leaves redundant recursive watches
    /// installed.
    ///
    /// Comparison is against each repo's *requested* state, so a repository
    /// whose recursive watch permanently fails (and is recorded as degraded to
    /// `.git`-only) is not re-armed on every reload.
    ///
    /// Best-effort by design: a repository that fails to re-arm is logged and
    /// skipped so one repo exhausting the OS watch budget cannot leave the
    /// remaining shells stuck on the old configuration. Locking mirrors
    /// `register_client`: `ops_lock` first, then snapshot `watched_repos` and
    /// release it before `rearm_repo_watch` takes the coordinator lock.
    pub fn resync_watch_worktree(&self) {
        let Ok(_ops_guard) = self.ops_lock.lock() else {
            debug_log!("watcher", "resync_watch_worktree: ops lock poisoned");
            return;
        };

        let desired = self.registry.worktree_enabled();

        let Ok(repos) = self.watched_repos.lock() else {
            debug_log!(
                "watcher",
                "resync_watch_worktree: watched repos lock poisoned"
            );
            return;
        };
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
        self.watched_repos.lock().map_or(0, |repos| repos.len())
    }

    /// Snapshot of the git roots currently being watched.
    ///
    /// Used by the periodic reconcile to recompute status for each active repo.
    #[must_use]
    pub fn watched_roots(&self) -> Vec<PathBuf> {
        self.watched_repos
            .lock()
            .map_or_else(|_| Vec::new(), |repos| repos.keys().cloned().collect())
    }
}

/// Builder for [`MultiRepoWatcher`] instances.
///
/// Provides a flexible way to construct `MultiRepoWatcher` with optional configuration
/// and required callback. Uses the builder pattern to ensure required fields are set
/// before construction.
///
/// # Examples
///
/// Basic usage with default config:
///
/// ```no_run
/// # use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
/// # use gpy_agent::watcher::DebouncedEvent;
/// # use gpy_agent::Result;
/// # fn main() -> Result<()> {
/// let watcher = MultiRepoWatcher::builder()
///     .callback(Box::new(|event: DebouncedEvent| {
///         println!("Event: {:?}", event);
///     }))
///     .build()?;
/// # Ok(())
/// # }
/// ```
///
/// With custom configuration:
///
/// ```no_run
/// # use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
/// # use gpy_agent::watcher::{WatcherConfig, DebouncedEvent};
/// # use gpy_agent::Result;
/// # fn main() -> Result<()> {
/// let config = WatcherConfig::new(200, 300); // 200ms debounce, 300ms throttle
///
/// let watcher = MultiRepoWatcher::builder()
///     .config(config)
///     .callback(Box::new(|event: DebouncedEvent| {
///         println!("Event: {:?}", event);
///     }))
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct MultiRepoWatcherBuilder {
    config: Option<WatcherConfig>,
    callback: Option<EventCallback>,
    registry: Option<Arc<WatchRegistry>>,
    #[cfg(test)]
    watch_fault: Option<WatchFault>,
}

impl MultiRepoWatcherBuilder {
    /// Create a new builder with default values.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: None,
            callback: None,
            registry: None,
            #[cfg(test)]
            watch_fault: None,
        }
    }

    /// Set the watcher configuration.
    ///
    /// If not set, defaults to [`WatcherConfig::default()`] (100ms debounce, 150ms throttle).
    ///
    /// # Example
    ///
    /// ```
    /// # use gpy_agent::watcher::multi_repo::MultiRepoWatcherBuilder;
    /// # use gpy_agent::watcher::WatcherConfig;
    /// let builder = MultiRepoWatcherBuilder::new()
    ///     .config(WatcherConfig::new(200, 300));
    /// ```
    #[must_use]
    pub const fn config(mut self, config: WatcherConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the event callback function.
    ///
    /// **Required**: This field must be set before calling `build()`.
    ///
    /// # Example
    ///
    /// ```
    /// # use gpy_agent::watcher::multi_repo::MultiRepoWatcherBuilder;
    /// # use gpy_agent::watcher::DebouncedEvent;
    /// let builder = MultiRepoWatcherBuilder::new()
    ///     .callback(Box::new(|event: DebouncedEvent| {
    ///         println!("Event: {:?}", event);
    ///     }));
    /// ```
    #[must_use]
    pub fn callback(mut self, callback: EventCallback) -> Self {
        self.callback = Some(callback);
        self
    }

    /// Share an existing [`WatchRegistry`] with this watcher.
    ///
    /// The agent passes the one registry it also hands to its config and theme
    /// coordinators, so a config path registered by one is visible to the
    /// event path of all three — the process-global behavior the five
    /// `OnceLock` registries used to provide (#617). If not set, the watcher
    /// gets a private registry and shares nothing with any other watcher.
    #[must_use]
    pub fn registry(mut self, registry: Arc<WatchRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Inject a test-only fault into the watches this watcher arms (#616).
    ///
    /// `fault` is consulted by [`MultiRepoWatcher::arm_watch`] for every path
    /// a real `watch_directory` call would receive; returning `Some(err)` for
    /// a path makes that watch fail exactly as a real watch-budget
    /// exhaustion would, without a real resource limit and without the
    /// process-wide `AtomicBool` flags (and the `#[serial]` they demanded of
    /// every test using them) this replaces.
    #[cfg(test)]
    #[must_use]
    fn watch_fault(mut self, fault: WatchFault) -> Self {
        self.watch_fault = Some(fault);
        self
    }

    /// Build the [`MultiRepoWatcher`] instance.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The callback field was not set (required field)
    /// - The underlying filesystem watcher cannot be created
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
    /// # use gpy_agent::watcher::DebouncedEvent;
    /// # use gpy_agent::Result;
    /// # fn main() -> Result<()> {
    /// let watcher = MultiRepoWatcher::builder()
    ///     .callback(Box::new(|event: DebouncedEvent| {
    ///         println!("Event: {:?}", event);
    ///     }))
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
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
            #[cfg(test)]
            watch_fault: self.watch_fault,
        })
    }
}

impl Default for MultiRepoWatcherBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use crate::watcher::DebouncedEvent;
    use std::sync::Arc;

    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn parse_commondir_pointer_table() {
        let git_dir = Path::new("/repo/.git/worktrees/feature");
        let cases: &[(&str, Option<&str>)] = &[
            ("../..\n", Some("/repo/.git/worktrees/feature/../..")),
            ("../..", Some("/repo/.git/worktrees/feature/../..")),
            ("/absolute/common\n", Some("/absolute/common")),
            ("", None),
            ("   \n", None),
        ];
        for (input, expected) in cases {
            let actual = MultiRepoWatcher::parse_commondir_pointer(git_dir, input);
            assert_eq!(actual, expected.map(PathBuf::from), "input: {input:?}");
        }
    }

    /// Test that builder succeeds with required callback
    #[test]
    fn test_builder_with_callback_succeeds() {
        let result = MultiRepoWatcher::builder()
            .callback(Box::new(|_event: DebouncedEvent| {
                // Test callback
            }))
            .build();

        assert!(result.is_ok(), "Builder should succeed with callback");
    }

    /// Test that builder uses default config when not specified
    #[test]
    fn test_builder_uses_default_config() {
        let result = MultiRepoWatcher::builder()
            .callback(Box::new(|_event: DebouncedEvent| {
                // Test callback
            }))
            .build();

        assert!(result.is_ok(), "Builder should succeed with default config");
        // Config defaults are applied by WatcherConfig::default()
        // which is tested separately in watcher/mod.rs
    }

    /// Test that builder accepts custom config
    #[test]
    fn test_builder_with_custom_config() {
        let custom_config = WatcherConfig::new(200, 300);

        let result = MultiRepoWatcher::builder()
            .config(custom_config)
            .callback(Box::new(|_event: DebouncedEvent| {
                // Test callback
            }))
            .build();

        assert!(result.is_ok(), "Builder should succeed with custom config");
    }

    /// Test that builder fails when callback is missing (required field error)
    #[test]
    fn test_builder_fails_without_callback() {
        let result = MultiRepoWatcher::builder().build();

        assert!(result.is_err(), "Builder should fail without callback");

        if let Err(err) = result {
            let err_msg = format!("{err}");
            assert!(
                err_msg.contains("callback"),
                "Error message should mention missing callback: {err_msg}"
            );
        }
    }

    /// Test that builder can be chained fluently
    #[test]
    fn test_builder_fluent_interface() {
        let config = WatcherConfig::new(150, 200);

        let result = MultiRepoWatcher::builder()
            .config(config)
            .callback(Box::new(|_event: DebouncedEvent| {
                // Test callback
            }))
            .build();

        assert!(result.is_ok(), "Fluent builder chain should succeed");
    }

    /// Test that builder implements Default
    #[test]
    fn test_builder_default_trait() {
        let builder = MultiRepoWatcherBuilder::default();

        // Should be able to build after setting required field
        let result = builder
            .callback(Box::new(|_event: DebouncedEvent| {
                // Test callback
            }))
            .build();

        assert!(result.is_ok(), "Default builder should work");
    }

    /// Test that `MultiRepoWatcher::builder()` returns correct type
    #[test]
    fn test_multi_repo_watcher_builder_method() {
        let builder = MultiRepoWatcher::builder();

        // Verify it's the correct type by using it
        let result = builder
            .callback(Box::new(|_event: DebouncedEvent| {
                // Test callback
            }))
            .build();

        assert!(
            result.is_ok(),
            "MultiRepoWatcher::builder() should return working builder"
        );
    }

    #[test]
    fn register_blocks_when_ops_lock_held() {
        use std::sync::{Barrier, mpsc};

        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();

        let watcher = Arc::new(
            MultiRepoWatcher::builder()
                .callback(Box::new(|_event: DebouncedEvent| {}))
                .build()
                .unwrap(),
        );

        let ops_guard = watcher.ops_lock.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        let watcher_clone = Arc::clone(&watcher);
        let repo_path = repo_root.to_path_buf();

        let barrier = Arc::new(Barrier::new(2));
        let barrier_clone = Arc::clone(&barrier);

        std::thread::spawn(move || {
            barrier_clone.wait(); // Sync with main thread
            let _ = watcher_clone.register_client(42, &repo_path);
            let _ = tx.send(());
        });

        // Ensure background thread is running and ready to acquire lock
        barrier.wait();

        // Give the background thread a moment to try acquiring the lock
        // This is not for timing assertions, but to ensure the scheduler schedules the thread
        std::thread::sleep(Duration::from_millis(10));

        // It should NOT have finished yet because we hold the lock
        assert!(
            rx.try_recv().is_err(),
            "register_client finished while ops_lock was held"
        );

        drop(ops_guard);

        // Now it should complete successfully.
        // Watcher setup can contend with filesystem notifications and scheduler load in CI,
        // so this timeout is intentionally long: the assertion is about eventual progress,
        // not startup latency.
        assert!(
            rx.recv_timeout(Duration::from_secs(20)).is_ok(),
            "register_client did not complete after lock release"
        );
    }

    /// A worktree-level file creation must be delivered through a REAL
    /// `notify::RecommendedWatcher` (not a hand-constructed synthetic event).
    /// Regression test for issue #388: `start_repo_watches` used to always
    /// issue two sequential `watch_directory` calls (`.git`, then the
    /// worktree root) on the same coordinator; on macOS this tears down and
    /// rebuilds the underlying `FSEventStream`, which could silently drop the
    /// worktree watch's first event. `find_git_root`/`register_client` route
    /// through the real filesystem watcher (not a fake), so this test would
    /// have failed against the pre-fix code path.
    #[test]
    #[serial_test::file_serial(watcher_fsevents_bootstrap)]
    fn worktree_level_file_creation_is_delivered() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        let status = std::process::Command::new("git")
            .arg("init")
            .current_dir(repo_root)
            .status()
            .expect("git init");
        assert!(status.success(), "git init should succeed");
        std::process::Command::new("git")
            .args(["config", "core.fsmonitor", "false"])
            .current_dir(repo_root)
            .status()
            .expect("disable fsmonitor");

        let events: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let watcher = MultiRepoWatcher::builder()
            .registry(test_registry(true))
            .callback(Box::new(move |event: DebouncedEvent| {
                events_clone.lock().unwrap().push(event.repo);
            }))
            .build()
            .expect("create watcher");

        watcher
            .register_client(std::process::id(), repo_root)
            .expect("register client");

        let canonical_root = std::fs::canonicalize(repo_root).expect("canonicalize repo root");

        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(repo_root.join("untracked.txt"), b"content")
            .expect("write worktree-level file");

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut delivered = false;
        while std::time::Instant::now() < deadline {
            if events.lock().unwrap().contains(&canonical_root) {
                delivered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        assert!(
            delivered,
            "worktree-level file creation should be delivered through the real watcher"
        );
    }

    /// Acceptance scenario for #416: `mkdir subdir && write subdir/file` must
    /// surface a git event for the repo within ~1s through a REAL notify
    /// watcher, without waiting for the periodic reconcile.
    ///
    /// This is a best-effort realistic test. It reliably PASSES with the fix
    /// because a directory-create always schedules a follow-up rescan of the
    /// repo regardless of whether the file-create event itself was dropped.
    /// It does NOT reliably fail without the fix: the underlying watch-arming
    /// race is nondeterministic, and the mkdir's own directory-create event is
    /// usually delivered anyway. The deterministic proof that the follow-up is
    /// scheduled lives in `filesystem.rs`'s `process_event` unit tests; this
    /// test guards the end-to-end wiring against regressions/hangs.
    #[test]
    #[serial_test::file_serial(watcher_fsevents_bootstrap)]
    fn mkdir_then_immediate_write_is_delivered() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        let status = std::process::Command::new("git")
            .arg("init")
            .current_dir(repo_root)
            .status()
            .expect("git init");
        assert!(status.success(), "git init should succeed");
        std::process::Command::new("git")
            .args(["config", "core.fsmonitor", "false"])
            .current_dir(repo_root)
            .status()
            .expect("disable fsmonitor");

        let events: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        let watcher = MultiRepoWatcher::builder()
            .registry(test_registry(true))
            .callback(Box::new(move |event: DebouncedEvent| {
                events_clone.lock().unwrap().push(event.repo);
            }))
            .build()
            .expect("create watcher");

        watcher
            .register_client(std::process::id(), repo_root)
            .expect("register client");

        let canonical_root = std::fs::canonicalize(repo_root).expect("canonicalize repo root");

        // Let the recursive worktree watch arm before racing it.
        std::thread::sleep(Duration::from_millis(100));

        // The racing pattern from the issue: create a directory and immediately
        // write a file into it, before the OS is guaranteed to have armed the
        // watch on the new subdirectory.
        let new_dir = repo_root.join("newmod");
        std::fs::create_dir_all(&new_dir).expect("create subdir");
        std::fs::write(new_dir.join("mod.rs"), b"pub fn f() {}\n").expect("write file in subdir");

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut delivered = false;
        while std::time::Instant::now() < deadline {
            if events.lock().unwrap().contains(&canonical_root) {
                delivered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        assert!(
            delivered,
            "mkdir + immediate file write should surface a git event for the repo"
        );
    }

    /// #616: unregistering the last client must unwatch exactly the paths
    /// actually armed.
    ///
    /// For a repository whose `git_dir` sits under `git_root` (the ordinary
    /// in-tree layout) with worktree watching on, that is `git_root` alone
    /// (a single recursive watch already covers `.git` internally, see
    /// `start_repo_watches`) -- and never `git_dir`, which was never armed as
    /// its own OS watch in this case.
    ///
    /// Before `armed` existed, `unregister_client_locked` unconditionally
    /// unwatched `git_dir` (armed or not) and additionally unwatched
    /// `git_root` only when `watch_worktree` was set, which happened to
    /// unwatch the right *set* of paths here only because the two policies
    /// coincided; the coordinator's own record of what was actually asked to
    /// be unwatched is the more direct proof (#616's TDD requirement (a)).
    ///
    /// # Panics
    ///
    /// Panics if locking the watcher/repo map fails.
    #[test]
    fn unregister_unwatches_exactly_the_armed_paths() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();

        let watcher = MultiRepoWatcher::builder()
            .registry(test_registry(true))
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .unwrap();

        watcher.register_client(99, repo_root).unwrap();

        // Confirm the precondition this test is built on: a single recursive
        // watch on `git_root` covers `.git` internally for this layout, so
        // `armed` must be `[git_root]` alone, not `[git_dir]` or both.
        let armed = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&canonical_root)
            .expect("repo tracked")
            .armed
            .clone();
        assert_eq!(
            armed,
            vec![canonical_root.clone()],
            "worktree watching with git_dir under git_root should arm git_root alone"
        );

        watcher.unregister_client(99).unwrap();

        let unwatched = watcher
            .watcher
            .lock()
            .unwrap()
            .as_ref()
            .expect("coordinator still present")
            .unwatch_calls();
        assert_eq!(
            unwatched,
            vec![canonical_root],
            "teardown must unwatch exactly the recorded armed paths -- git_root once, \
             and never git_dir, which was never armed as its own watch"
        );
    }

    /// A recursive worktree watch failure must degrade to `.git`-only
    /// watching, not fail client registration.
    ///
    /// The repository stays tracked (so the periodic reconcile still
    /// recovers missed events) with `watch_worktree` recorded as `false`.
    /// Regression test for issue #158.
    ///
    /// Not `#[serial]` (#616): the fault is scoped to this watcher's own
    /// `watch_fault` closure, not a process-wide flag other tests must avoid
    /// tripping over.
    #[test]
    fn worktree_watch_failure_degrades_to_gitdir_only() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();

        // "Worktree watch fails" = fault only the path equal to the repo's
        // git_root -- the essential .git watch must still succeed.
        let faulty_root = canonical_root.clone();
        let watcher = MultiRepoWatcher::builder()
            .registry(test_registry(true))
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .watch_fault(Arc::new(move |path: &Path| {
                (path == faulty_root)
                    .then(|| Error::watcher("injected worktree watch failure (test)"))
            }))
            .build()
            .unwrap();

        let result = watcher.register_client(7, repo_root);

        assert!(
            result.is_ok(),
            "registration must succeed despite worktree watch failure: {result:?}"
        );
        assert_eq!(
            watcher.watched_repo_count(),
            1,
            "repo should still be tracked after the worktree fallback"
        );

        // Copy the recorded flag out in a single statement so the lock guard is
        // released immediately, before calling watched_roots() (which locks the
        // same mutex) — otherwise the non-reentrant Mutex would self-deadlock.
        let degraded_worktree = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&canonical_root)
            .expect("repo tracked under canonical root")
            .worktree
            .effective();
        assert!(
            !degraded_worktree,
            "worktree flag must degrade to false after fallback"
        );
        assert!(
            watcher.watched_roots().contains(&canonical_root),
            "degraded repo must remain visible to the periodic reconcile"
        );
    }

    /// Failure to watch the essential `.git` directory must roll back cleanly.
    ///
    /// Registration surfaces the error and no repository is left tracked, so
    /// the registry and watcher cannot disagree. Regression test for issue
    /// #158.
    ///
    /// Not `#[serial]` (#616): scoped to this watcher's own `watch_fault`.
    #[test]
    fn gitdir_watch_failure_rolls_back_without_tracking() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();

        // "Git watch fails" = fault every path: with the default worktree
        // watching (git_dir under git_root) this fails the recursive
        // git_root attempt, falls back, then fails the .git-only attempt too
        // -- the same `Err` the old process-wide `TEST_FAIL_GIT_WATCH` produced
        // by returning `Err` before any watch was tried.
        let watcher = MultiRepoWatcher::builder()
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .watch_fault(Arc::new(|_path: &Path| {
                Some(Error::watcher("injected git-dir watch failure (test)"))
            }))
            .build()
            .unwrap();

        let result = watcher.register_client(8, repo_root);

        assert!(
            result.is_err(),
            "registration should surface the essential .git watch failure"
        );
        assert_eq!(
            watcher.watched_repo_count(),
            0,
            "no repo should be tracked after a rolled-back watch setup"
        );
    }

    /// Registering a client with no coordinator must fail loudly.
    ///
    /// Regression test for #323: once the watch coordinator is gone (e.g.
    /// after `stop()`), registering a client must fail loudly instead of
    /// silently recording the repository as watched with no OS watch behind
    /// it.
    ///
    #[test]
    fn register_client_errors_when_coordinator_is_gone() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();

        let watcher = MultiRepoWatcher::builder()
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .unwrap();

        watcher.stop();

        let result = watcher.register_client(9, repo_root);
        assert!(
            result.is_err(),
            "registration must fail rather than silently record an unwatched repo"
        );
        assert_eq!(
            watcher.watched_repo_count(),
            0,
            "no repo should be tracked when the coordinator is gone"
        );
    }

    /// A re-register after the gitdir was recreated must re-arm the watch.
    ///
    /// Finding #4 (#267): a heartbeat re-register after the watched `.git` dir was
    /// deleted and recreated at the same path must re-arm the watch — a
    /// directory with a different inode than the tracked one means the
    /// original OS watch is dead. Before the fix the registry short-circuited
    /// on the stale entry, leaving the repo unwatched until the ~45s
    /// reconcile. Identity is inode-based, so this is a Unix-only guarantee.
    ///
    /// The replacement directory is created and swapped in via `rename`
    /// rather than deleted-then-recreated at the same path: immediate
    /// `rmdir`+`mkdir` reuse of the just-freed inode number is common on
    /// Linux (not a bug — see `dir_identity`'s doc comment: identity is a
    /// best-effort optimization with periodic reconcile as the backstop for
    /// exactly this case), which made this test itself flaky on
    /// `ubuntu-latest` (#292). Two directories that exist simultaneously on
    /// the same device are guaranteed distinct inodes, so renaming a
    /// pre-created directory over the old one deterministically exercises
    /// the "identity changed" code path this test is meant to cover.
    #[cfg(unix)]
    #[test]
    fn recreated_gitdir_rearms_watch_on_reregister() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();

        let watcher = MultiRepoWatcher::builder()
            .registry(test_registry(false))
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .unwrap();

        watcher.register_client(1, repo_root).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();

        let first_id = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&canonical_root)
            .expect("repo tracked after first registration")
            .git_dir_id;
        assert!(first_id.is_some(), "unix identity should be captured");

        // Swap in a directory that coexisted with the original .git, rather
        // than delete-then-recreate at the same path (see doc comment above).
        let replacement = repo_root.join(".git-replacement");
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::remove_dir_all(repo_root.join(".git")).unwrap();
        std::fs::rename(&replacement, repo_root.join(".git")).unwrap();

        // Heartbeat re-register for the same client.
        watcher.register_client(1, repo_root).unwrap();

        let second_id = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&canonical_root)
            .expect("repo still tracked after re-register")
            .git_dir_id;

        assert_ne!(
            first_id, second_id,
            "recreated gitdir must update the tracked identity (watch re-armed)"
        );
        assert_eq!(
            watcher.watched_repo_count(),
            1,
            "repo should remain tracked exactly once after re-arm"
        );
    }

    /// Read the effective worktree-watch flag recorded for `root`.
    ///
    /// Copies the value out in one statement so the map guard is released
    /// immediately (the mutex is not reentrant — see
    /// `worktree_watch_failure_degrades_to_gitdir_only`).
    fn tracked_worktree(watcher: &MultiRepoWatcher, root: &Path) -> bool {
        watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(root)
            .expect("repo tracked")
            .worktree
            .effective()
    }

    /// Read the client PID set recorded for `root`.
    fn tracked_clients(watcher: &MultiRepoWatcher, root: &Path) -> HashSet<u32> {
        watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(root)
            .expect("repo tracked")
            .clients
            .clone()
    }

    /// A private registry with `worktree_enabled` already applied.
    ///
    /// The per-watcher replacement for the process-global `WATCH_WORKTREE`
    /// flag #617 retired.
    fn test_registry(worktree_enabled: bool) -> Arc<WatchRegistry> {
        let registry = Arc::new(WatchRegistry::new());
        registry.set_worktree_enabled(worktree_enabled);
        registry
    }

    /// Build a watcher over a fresh temp repo with a bare `.git` directory.
    fn watcher_over_temp_repo(worktree_enabled: bool) -> (TempDir, MultiRepoWatcher) {
        watcher_over_temp_repo_with_fault(worktree_enabled, |_canonical_root| {
            Arc::new(|_path: &Path| None)
        })
    }

    /// Like [`watcher_over_temp_repo`], but also installs a [`WatchFault`]
    /// built from the repo's canonical root (#616).
    ///
    /// `fault_for_root` is a factory rather than a plain [`WatchFault`]
    /// because most fault closures need to compare against the tracked
    /// repo's canonical root, which is only known once the temp repo exists.
    fn watcher_over_temp_repo_with_fault(
        worktree_enabled: bool,
        fault_for_root: impl FnOnce(&Path) -> WatchFault,
    ) -> (TempDir, MultiRepoWatcher) {
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
        let canonical_root = std::fs::canonicalize(temp_dir.path()).unwrap();
        let watcher = MultiRepoWatcher::builder()
            .registry(test_registry(worktree_enabled))
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .watch_fault(fault_for_root(&canonical_root))
            .build()
            .unwrap();
        (temp_dir, watcher)
    }

    /// A re-register after `git.watch_worktree` flipped on must re-arm the repo.
    ///
    /// #444: a re-register after `git.watch_worktree` flipped false → true must
    /// re-arm the already-tracked repo. Before the fix
    /// `attach_or_rearm_existing_repo` compared only gitdir path/identity, so the
    /// repo kept its `.git`-only watch while event filtering already expected
    /// worktree events — pure edits stayed invisible until the ~45s reconcile.
    ///
    #[test]
    fn reregister_after_worktree_enabled_rearms_repo() {
        let (temp_dir, watcher) = watcher_over_temp_repo(false);
        let repo_root = temp_dir.path();

        watcher.register_client(1, repo_root).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();
        assert!(
            !tracked_worktree(&watcher, &canonical_root),
            "repo should start out .git-only"
        );

        watcher.set_watch_worktree(true);
        watcher.register_client(1, repo_root).unwrap();

        assert!(
            tracked_worktree(&watcher, &canonical_root),
            "re-register after the flag flipped on must install the worktree watch"
        );
    }

    /// #444: the reverse transition (true → false) must drop the recursive
    /// worktree watch while keeping the essential `.git` coverage.
    #[test]
    fn reregister_after_worktree_disabled_rearms_repo() {
        let (temp_dir, watcher) = watcher_over_temp_repo(true);
        let repo_root = temp_dir.path();

        watcher.register_client(1, repo_root).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();
        assert!(
            tracked_worktree(&watcher, &canonical_root),
            "repo should start out watching the worktree"
        );

        watcher.set_watch_worktree(false);
        watcher.register_client(1, repo_root).unwrap();

        assert!(
            !tracked_worktree(&watcher, &canonical_root),
            "re-register after the flag flipped off must drop the worktree watch"
        );
        assert_eq!(
            watcher.watched_repo_count(),
            1,
            "repo must stay tracked (gitdir coverage preserved)"
        );
    }

    /// #444 watch-thrash guard: a degraded repo must not re-arm on an
    /// unchanged-config re-register.
    ///
    /// A repo whose worktree watch FAILED and degraded to `.git`-only
    /// records `watch_worktree == false` while the config still requests
    /// `true`. Re-arming on that mismatch would re-arm on every single
    /// heartbeat re-register forever, so the comparison must be against the
    /// *requested* state, not the degraded effective one.
    ///
    /// Not `#[serial]` (#616): the fault is scoped to this watcher's own
    /// `watch_fault` closure.
    #[test]
    fn degraded_worktree_repo_does_not_rearm_on_reregister() {
        let (temp_dir, watcher) = watcher_over_temp_repo_with_fault(true, |canonical_root| {
            let faulty_root = canonical_root.to_path_buf();
            Arc::new(move |path: &Path| {
                (path == faulty_root)
                    .then(|| Error::watcher("injected worktree watch failure (test)"))
            })
        });
        let repo_root = temp_dir.path();
        watcher.register_client(1, repo_root).unwrap();

        let canonical_root = std::fs::canonicalize(repo_root).unwrap();
        assert!(
            !tracked_worktree(&watcher, &canonical_root),
            "worktree watch failure should have degraded to .git-only"
        );

        // Nothing changed in the config; failure injection is off again, so a
        // spurious re-arm here would flip the flag back to true and reveal itself.
        watcher.register_client(1, repo_root).unwrap();

        assert!(
            !tracked_worktree(&watcher, &canonical_root),
            "a degraded repo must not re-arm on an unchanged-config re-register"
        );
    }

    /// A resync must re-arm every ACTIVE repo when the flag flips on.
    ///
    /// #444 config-reload path: flipping the effective flag false → true must
    /// re-arm every ACTIVE repository without waiting for a client to
    /// re-register (a shell that never leaves its directory would otherwise stay
    /// on a `.git`-only watch indefinitely).
    ///
    #[test]
    fn resync_installs_worktree_watch_on_active_repos() {
        let (temp_dir, watcher) = watcher_over_temp_repo(false);
        let repo_root = temp_dir.path();

        watcher.register_client(1, repo_root).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();
        assert!(!tracked_worktree(&watcher, &canonical_root));

        watcher.set_watch_worktree(true);
        watcher.resync_watch_worktree();

        assert!(
            tracked_worktree(&watcher, &canonical_root),
            "config reload must re-arm active repos with the worktree watch"
        );
    }

    /// #444 config-reload path, reverse transition: the recursive worktree watch
    /// is dropped while the repo stays tracked with gitdir coverage.
    #[test]
    fn resync_removes_worktree_watch_on_active_repos() {
        let (temp_dir, watcher) = watcher_over_temp_repo(true);
        let repo_root = temp_dir.path();

        watcher.register_client(1, repo_root).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();
        assert!(tracked_worktree(&watcher, &canonical_root));

        watcher.set_watch_worktree(false);
        watcher.resync_watch_worktree();

        assert!(
            !tracked_worktree(&watcher, &canonical_root),
            "config reload must drop the worktree watch when disabled"
        );
        assert_eq!(
            watcher.watched_repo_count(),
            1,
            "repo must stay tracked with its essential gitdir watch"
        );
    }

    /// #444: a repository shared by several shells must keep every client
    /// registered across a reconfiguration — the re-arm must not drop the
    /// reference count and orphan the watch.
    #[test]
    fn resync_preserves_multiple_registered_clients() {
        let (temp_dir, watcher) = watcher_over_temp_repo(false);
        let repo_root = temp_dir.path();

        for pid in [11_u32, 22_u32, 33_u32] {
            watcher.register_client(pid, repo_root).unwrap();
        }
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();
        let before = tracked_clients(&watcher, &canonical_root);
        assert_eq!(before.len(), 3, "all three clients should be registered");

        watcher.set_watch_worktree(true);
        watcher.resync_watch_worktree();

        assert!(tracked_worktree(&watcher, &canonical_root));
        assert_eq!(
            tracked_clients(&watcher, &canonical_root),
            before,
            "reconfiguration must preserve the full client set"
        );
        assert_eq!(watcher.watched_repo_count(), 1);
    }

    /// #444: a resync whose recursive worktree watch fails must degrade to
    /// `.git`-only watching.
    ///
    /// The repo must never end up untracked or unwatched, and the degraded
    /// repo must not be re-armed again on the next resync.
    ///
    /// Not `#[serial]` (#616): the fault is scoped to this watcher's own
    /// `watch_fault` closure. It is safe to leave armed for the whole test:
    /// the initial `register_client` call never arms `git_root` at all
    /// (worktree watching starts disabled), so the fault only ever fires on
    /// the resync this test is exercising.
    #[test]
    fn resync_worktree_failure_degrades_to_gitdir_only() {
        let (temp_dir, watcher) = watcher_over_temp_repo_with_fault(false, |canonical_root| {
            let faulty_root = canonical_root.to_path_buf();
            Arc::new(move |path: &Path| {
                (path == faulty_root)
                    .then(|| Error::watcher("injected worktree watch failure (test)"))
            })
        });
        let repo_root = temp_dir.path();
        watcher.register_client(1, repo_root).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();

        watcher.set_watch_worktree(true);
        watcher.resync_watch_worktree();

        assert!(
            !tracked_worktree(&watcher, &canonical_root),
            "a failed worktree watch must degrade to .git-only"
        );
        assert_eq!(
            watcher.watched_repo_count(),
            1,
            "the degraded repo must remain tracked and reconcilable"
        );

        // Requested state now matches the config, so a second pass is a no-op:
        // with injection off a spurious re-arm would flip the flag to true.
        watcher.resync_watch_worktree();
        assert!(
            !tracked_worktree(&watcher, &canonical_root),
            "a degraded repo must not thrash its watch on every resync"
        );
    }

    /// #444: when the effective flag is unchanged, a resync must not touch a
    /// single watch. Proven by arming a `.git`-watch failure injection that
    /// would corrupt the tracked entry if any re-arm ran.
    ///
    /// Not `#[serial]` (#616): the fault is gated by an `AtomicBool` owned by
    /// this test alone, not a process-wide flag -- it must start disarmed
    /// (the initial `register_client` below must succeed) and is armed only
    /// around the resync under test.
    #[test]
    fn resync_is_a_noop_when_nothing_changed() {
        let fault_armed = Arc::new(AtomicBool::new(false));
        let fault_armed_for_closure = Arc::clone(&fault_armed);
        let (temp_dir, watcher) = watcher_over_temp_repo_with_fault(true, move |_root| {
            Arc::new(move |_path: &Path| {
                fault_armed_for_closure
                    .load(Ordering::Relaxed)
                    .then(|| Error::watcher("injected git-dir watch failure (test)"))
            })
        });
        let repo_root = temp_dir.path();
        watcher.register_client(1, repo_root).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();
        let identity_before = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&canonical_root)
            .expect("repo tracked")
            .git_dir_id;

        // Injected failure would corrupt the tracked state if a re-arm ran.
        fault_armed.store(true, Ordering::Relaxed);
        watcher.resync_watch_worktree();
        fault_armed.store(false, Ordering::Relaxed);

        assert!(tracked_worktree(&watcher, &canonical_root));
        assert_eq!(
            watcher
                .watched_repos
                .lock()
                .unwrap()
                .get(&canonical_root)
                .expect("repo tracked")
                .git_dir_id,
            identity_before,
            "an unchanged flag must not touch any watch"
        );
    }

    /// A plain heartbeat re-register (nothing changed on disk) must keep the
    /// tracked identity stable — the fast path, no needless re-arm churn.
    #[cfg(unix)]
    #[test]
    fn unchanged_gitdir_keeps_identity_on_reregister() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();

        let watcher = MultiRepoWatcher::builder()
            .registry(test_registry(false))
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .unwrap();

        watcher.register_client(1, repo_root).unwrap();
        let canonical_root = std::fs::canonicalize(repo_root).unwrap();
        let first_id = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&canonical_root)
            .expect("repo tracked")
            .git_dir_id;

        watcher.register_client(1, repo_root).unwrap();
        let second_id = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&canonical_root)
            .expect("repo still tracked")
            .git_dir_id;

        assert_eq!(
            first_id, second_id,
            "an unchanged gitdir must not have its identity churned on re-register"
        );
    }

    /// #617: two `MultiRepoWatcher`s in one process must not share attribution
    /// or worktree-watch state.
    ///
    /// Deliberately NOT `#[serial]`: the whole point of the registry is that
    /// this test is independent of anything else running concurrently. Before
    /// the five process-global `OnceLock` registries became `WatchRegistry`
    /// fields, this could not even be written — `set_watch_worktree` was a
    /// static that wrote one process-wide `AtomicBool`, and the external-gitdir
    /// map was a module-private `OnceLock` with no per-instance handle.
    #[test]
    fn two_watchers_do_not_share_registry_state() {
        let first = MultiRepoWatcher::builder()
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .unwrap();
        let second = MultiRepoWatcher::builder()
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .unwrap();

        // External-gitdir attribution registered on `first` must be invisible
        // to `second`.
        let git_dir = PathBuf::from("/super/.git/modules/sub");
        let work_root = PathBuf::from("/super/sub");
        first
            .registry()
            .register_external_gitdir(&git_dir, &work_root);
        let head = git_dir.join("HEAD");
        assert_eq!(
            first.registry().attribute_external_gitdir(&head),
            Some(work_root.clone()),
            "the registering watcher must attribute its own external gitdir"
        );
        assert_eq!(
            second.registry().attribute_external_gitdir(&head),
            None,
            "a second watcher must not see the first watcher's external gitdir"
        );

        // Common-gitdir attribution is likewise per-watcher.
        let common = PathBuf::from("/main/.git");
        first.registry().register_common_gitdir(&common, &work_root);
        assert_eq!(
            first
                .registry()
                .attribute_common_gitdir(&common.join("packed-refs")),
            vec![work_root.clone()],
        );
        assert!(
            second
                .registry()
                .attribute_common_gitdir(&common.join("packed-refs"))
                .is_empty(),
            "a second watcher must not see the first watcher's common gitdir"
        );

        // Config-path registration is per-watcher too.
        let config_path = PathBuf::from("/nowhere/gpy/config.toml");
        first.registry().register_config_path(&config_path);
        assert!(first.registry().is_registered_config_path(&config_path));
        assert!(
            !second.registry().is_registered_config_path(&config_path),
            "a second watcher must not see the first watcher's config path"
        );

        // And so is the worktree-watch flag.
        assert!(first.registry().worktree_enabled());
        assert!(second.registry().worktree_enabled());
        first.set_watch_worktree(false);
        assert!(!first.registry().worktree_enabled());
        assert!(
            second.registry().worktree_enabled(),
            "flipping watch_worktree on one watcher must not change the other"
        );

        first.registry().unregister_external_gitdir(&git_dir);
        first
            .registry()
            .unregister_common_gitdir(&common, &work_root);
        first.registry().unregister_config_path(&config_path);
    }

    /// The backend's unwatch log for `watcher`.
    fn unwatch_log(watcher: &MultiRepoWatcher) -> Vec<PathBuf> {
        watcher
            .watcher
            .lock()
            .unwrap()
            .as_ref()
            .expect("coordinator present")
            .unwatch_calls()
    }

    /// Two plain repositories (bare `.git` directories) plus a `sub` directory
    /// in the first, canonicalized.
    fn two_plain_repos() -> (TempDir, PathBuf, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let base = std::fs::canonicalize(temp_dir.path()).unwrap();
        for name in ["a", "b"] {
            std::fs::create_dir_all(base.join(name).join(".git")).unwrap();
        }
        std::fs::create_dir_all(base.join("a").join("sub")).unwrap();
        let (a, b) = (base.join("a"), base.join("b"));
        (temp_dir, a, b)
    }

    fn plain_watcher() -> MultiRepoWatcher {
        MultiRepoWatcher::builder()
            .registry(test_registry(true))
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .unwrap()
    }

    /// #718: a `cd` inside the repository the pid is already attached to must
    /// not touch the OS watches or the tracked entry.
    #[test]
    fn update_client_within_same_repo_does_not_rearm() {
        let (_temp, repo, _other) = two_plain_repos();
        let watcher = plain_watcher();
        watcher.register_client(1, &repo).unwrap();
        let before = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&repo)
            .unwrap()
            .clone();

        watcher.update_client(1, &repo.join("sub")).unwrap();

        assert!(
            unwatch_log(&watcher).is_empty(),
            "an intra-repo cd must not unwatch anything, got {:?}",
            unwatch_log(&watcher)
        );
        let after = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&repo)
            .unwrap()
            .clone();
        assert_eq!(after.armed, before.armed);
        assert_eq!(after.git_dir_id, before.git_dir_id);
        assert_eq!(after.clients, before.clients);
        assert_eq!(tracked_clients(&watcher, &repo), HashSet::from([1]));
    }

    /// #718: `register` for a pid already attached elsewhere detaches it from
    /// the old repository, tearing it down when it was the last client.
    #[test]
    fn register_client_detaches_pid_from_previous_repo() {
        let (_temp, a, b) = two_plain_repos();
        let watcher = plain_watcher();
        watcher.register_client(1, &a).unwrap();
        let a_armed = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&a)
            .unwrap()
            .armed
            .clone();

        watcher.register_client(1, &b).unwrap();

        assert!(
            !watcher.watched_repos.lock().unwrap().contains_key(&a),
            "the old repository must be released"
        );
        assert_eq!(unwatch_log(&watcher), a_armed);
        assert_eq!(tracked_clients(&watcher, &b), HashSet::from([1]));
    }

    /// #718: moving one pid away from a repository other pids still use must
    /// leave that repository, and its watches, alone.
    #[test]
    fn update_client_to_other_repo_keeps_shared_repo() {
        let (_temp, a, b) = two_plain_repos();
        let watcher = plain_watcher();
        watcher.register_client(1, &a).unwrap();
        watcher.register_client(2, &a).unwrap();

        watcher.update_client(1, &b).unwrap();

        assert_eq!(tracked_clients(&watcher, &a), HashSet::from([2]));
        assert_eq!(tracked_clients(&watcher, &b), HashSet::from([1]));
        assert!(
            !unwatch_log(&watcher).contains(&a),
            "a still has a client, its watch must stay: {:?}",
            unwatch_log(&watcher)
        );
    }

    /// #718 error semantics: a failed arm of the target still detaches the pid
    /// from the repository it was attached to, leaving it attached nowhere.
    #[test]
    fn update_client_failed_arm_still_detaches_from_previous_repo() {
        let (_temp, a, b) = two_plain_repos();
        let fault_root = b.clone();
        let watcher = MultiRepoWatcher::builder()
            .registry(test_registry(true))
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .watch_fault(Arc::new(move |path: &Path| {
                path.starts_with(&fault_root)
                    .then(|| Error::watcher("injected watch failure"))
            }))
            .build()
            .unwrap();
        watcher.register_client(1, &a).unwrap();

        assert!(watcher.update_client(1, &b).is_err());

        assert!(watcher.watched_repos.lock().unwrap().is_empty());
        assert_eq!(unwatch_log(&watcher), vec![a]);
    }

    // ---- Watch-set class test (#718, #687) --------------------------------
    //
    // Randomized register/unregister/update sequences over a fixture with
    // overlapping repositories, checked after every step against a model that
    // is computed from the fixture alone (never from the watcher's own state).

    /// One repository of the fixture.
    struct FixtureRepo {
        root: PathBuf,
        git_dir: PathBuf,
        /// A linked worktree: its common directory is shared with `main`.
        linked: bool,
    }

    struct Fixture {
        _temp: TempDir,
        repos: Vec<FixtureRepo>,
        /// Directories a pid can be in, with the index of the repository that
        /// owns each (`None` outside any repository).
        cwds: Vec<(PathBuf, Option<usize>)>,
        /// `main`'s `.git`: the common directory of both linked worktrees.
        common: PathBuf,
    }

    impl Fixture {
        fn root_of(&self, index: usize) -> &Path {
            &self.repos.get(index).expect("repo index in range").root
        }
    }

    /// A set of `(path, mode)` OS watches.
    type WatchSpec = std::collections::BTreeSet<(PathBuf, WatchMode)>;

    /// The overlapping-repository fixture.
    ///
    /// `main` (with a nested repo, a submodule-shaped repo with an external
    /// gitdir, and two linked worktrees sharing its common dir), `other`, and
    /// a plain directory outside any repository. No `git` process needed:
    /// the watcher only reads `.git` entries and pointer files.
    #[expect(
        clippy::too_many_lines,
        reason = "one flat table of fixture directories reads better split nowhere"
    )]
    fn build_fixture() -> Fixture {
        let temp = TempDir::new().unwrap();
        let base = std::fs::canonicalize(temp.path()).unwrap();
        let mk = |rel: &str| {
            let path = base.join(rel);
            std::fs::create_dir_all(&path).unwrap();
            path
        };
        let main = mk("main");
        let common = mk("main/.git");
        mk("main/.git/refs");
        mk("main/src");
        let libs = mk("main/libs");
        let sub_admin = mk("main/.git/modules/sub");
        let sub = mk("main/libs/sub");
        std::fs::write(
            sub.join(".git"),
            format!("gitdir: {}\n", sub_admin.display()),
        )
        .unwrap();
        let nested = mk("main/nested");
        mk("main/nested/.git");
        mk("main/nested/src");
        let mut linked = Vec::new();
        for name in ["wt", "wt2"] {
            let admin = mk(&format!("main/.git/worktrees/{name}"));
            std::fs::write(admin.join("commondir"), "../..\n").unwrap();
            let root = mk(name);
            mk(&format!("{name}/src"));
            std::fs::write(root.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
            linked.push((root, admin));
        }
        let other = mk("other");
        mk("other/.git");
        mk("other/deep");
        let plain = mk("plain");

        let mut repos = vec![
            FixtureRepo {
                root: main.clone(),
                git_dir: common.clone(),
                linked: false,
            },
            FixtureRepo {
                root: nested.clone(),
                git_dir: nested.join(".git"),
                linked: false,
            },
            FixtureRepo {
                root: sub.clone(),
                git_dir: sub_admin,
                linked: false,
            },
        ];
        for (root, admin) in &linked {
            repos.push(FixtureRepo {
                root: root.clone(),
                git_dir: admin.clone(),
                linked: true,
            });
        }
        repos.push(FixtureRepo {
            root: other.clone(),
            git_dir: other.join(".git"),
            linked: false,
        });

        let wt = linked.first().unwrap().0.clone();
        let wt2 = linked.get(1).expect("two linked worktrees").0.clone();
        let cwds = vec![
            (main.clone(), Some(0)),
            (main.join("src"), Some(0)),
            (libs, Some(0)),
            (nested.clone(), Some(1)),
            (nested.join("src"), Some(1)),
            (sub, Some(2)),
            (wt.clone(), Some(3)),
            (wt.join("src"), Some(3)),
            (wt2, Some(4)),
            (other.clone(), Some(5)),
            (other.join("deep"), Some(5)),
            (plain, None),
        ];
        Fixture {
            _temp: temp,
            repos,
            cwds,
            common,
        }
    }

    /// The OS watches the live repositories need: `(required, optional)`.
    ///
    /// Optional ones are the linked worktrees' shared common-dir watches,
    /// which are skipped when the main checkout's recursive watch already
    /// covers them, so their presence depends on arrival order.
    fn needed_watches(
        fx: &Fixture,
        live: &std::collections::BTreeSet<usize>,
    ) -> (WatchSpec, WatchSpec) {
        let mut required = std::collections::BTreeSet::new();
        let mut optional = std::collections::BTreeSet::new();
        for repo in live.iter().filter_map(|index| fx.repos.get(*index)) {
            if !repo.git_dir.starts_with(&repo.root) {
                required.insert((repo.git_dir.clone(), WatchMode::Recursive));
            }
            required.insert((repo.root.clone(), WatchMode::Recursive));
            if repo.linked {
                optional.insert((fx.common.clone(), WatchMode::Shallow));
                optional.insert((fx.common.join("refs"), WatchMode::Recursive));
            }
        }
        (required, optional)
    }

    fn held_watches_of(watcher: &MultiRepoWatcher) -> crate::watcher::filesystem::HeldWatches {
        watcher
            .watcher
            .lock()
            .unwrap()
            .as_ref()
            .expect("coordinator present")
            .held_watches()
    }

    /// Assert the tracked repositories and the held OS watches equal what the
    /// model says the remaining clients need.
    fn assert_matches_model(
        watcher: &MultiRepoWatcher,
        fx: &Fixture,
        model: &std::collections::BTreeMap<u32, Option<usize>>,
        trace: &[(u8, u32, usize)],
    ) {
        use std::collections::{BTreeMap, BTreeSet};

        // (a) tracked repos == repos with >= 1 modelled client, same clients.
        let mut expected: BTreeMap<PathBuf, BTreeSet<u32>> = BTreeMap::new();
        let mut live = BTreeSet::new();
        for (pid, repo) in model {
            if let Some(index) = repo {
                live.insert(*index);
                let root = fx.root_of(*index).to_path_buf();
                expected.entry(root).or_default().insert(*pid);
            }
        }
        let tracked: BTreeMap<PathBuf, BTreeSet<u32>> = watcher
            .watched_repos
            .lock()
            .unwrap()
            .iter()
            .map(|(root, repo)| (root.clone(), repo.clients.iter().copied().collect()))
            .collect();
        assert_eq!(tracked, expected, "tracked repos/clients after {trace:?}");

        // (b) the held OS watches are exactly what the live repos need.
        let (owners, armed) = held_watches_of(watcher);
        let held: BTreeSet<(PathBuf, WatchMode)> = owners.keys().cloned().collect();
        let armed_pairs: BTreeSet<(PathBuf, WatchMode)> = armed.into_iter().collect();
        assert_eq!(
            armed_pairs, held,
            "OS watch set vs registrations after {trace:?}"
        );
        assert!(
            owners.values().all(|count| *count == 1),
            "every watch has exactly one owner: {owners:?} after {trace:?}"
        );
        let (required, optional) = needed_watches(fx, &live);
        let missing: Vec<_> = required.difference(&held).collect();
        assert!(
            missing.is_empty(),
            "missing watches {missing:?} after {trace:?}"
        );
        let leaked: Vec<_> = held
            .iter()
            .filter(|watch| !required.contains(*watch) && !optional.contains(*watch))
            .collect();
        assert!(
            leaked.is_empty(),
            "leaked watches {leaked:?} after {trace:?}"
        );
        for (path, mode) in &optional {
            let covered = held.iter().any(|(held_path, held_mode)| {
                path.starts_with(held_path)
                    && (*held_mode == WatchMode::Recursive || held_path == path)
                    && (*mode == WatchMode::Shallow || *held_mode == WatchMode::Recursive)
            });
            assert!(covered, "common watch {path:?} uncovered after {trace:?}");
        }
    }

    /// Held watches, unwatch-log length, and the tracked `armed`/`git_dir_id`/
    /// `clients` of one repository.
    type RearmSnapshot = (
        crate::watcher::filesystem::HeldWatches,
        usize,
        Vec<PathBuf>,
        DirIdentity,
        HashSet<u32>,
    );

    /// What a same-repository re-attach must leave untouched.
    fn rearm_snapshot(watcher: &MultiRepoWatcher, root: &Path) -> RearmSnapshot {
        let repo = watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(root)
            .unwrap()
            .clone();
        (
            held_watches_of(watcher),
            unwatch_log(watcher).len(),
            repo.armed,
            repo.git_dir_id,
            repo.clients,
        )
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            cases: 48,
            failure_persistence: None,
            ..proptest::prelude::ProptestConfig::default()
        })]

        /// Random sequences of register / update / unregister for a few pids
        /// leave the watcher tracking, and watching, exactly what the
        /// remaining clients need (#718, #687), and an attach to the repo a
        /// pid already belongs to performs no backend watch operation.
        #[test]
        fn watch_set_matches_client_model(
            steps in proptest::collection::vec((0_u8..3_u8, 1_u32..=3_u32, 0_usize..12_usize), 1..=30),
        ) {
            let fx = build_fixture();
            let watcher = plain_watcher();
            let mut model: std::collections::BTreeMap<u32, Option<usize>> =
                std::collections::BTreeMap::new();
            for (done, (kind, pid, cwd_index)) in steps.iter().copied().enumerate() {
                let trace = steps.get(..=done).expect("in range");
                let (cwd, target) = fx.cwds.get(cwd_index).expect("cwd index in range");
                if kind == 2 {
                    watcher.unregister_client(pid).unwrap();
                    model.insert(pid, None);
                } else {
                    let same_repo = target.is_some() && model.get(&pid) == Some(target);
                    let snapshot_before = same_repo.then(|| {
                        rearm_snapshot(&watcher, fx.root_of(target.unwrap()))
                    });
                    if kind == 0 {
                        watcher.register_client(pid, cwd).unwrap();
                    } else {
                        watcher.update_client(pid, cwd).unwrap();
                    }
                    model.insert(pid, *target);
                    if let Some(snapshot) = snapshot_before {
                        let after =
                            rearm_snapshot(&watcher, fx.root_of(target.unwrap()));
                        assert_eq!(snapshot, after, "same-repo attach touched watches after {trace:?}");
                    }
                }
                assert_matches_model(&watcher, &fx, &model, trace);
            }
        }
    }
}
