//! Reference-counted watcher over multiple git repositories.
//!
//! One shared [`WatchCoordinator`] watches every repository (one debouncer and
//! flush thread total); `watched_repos` maps each git root to the client PIDs
//! using it, and the watch stops when the last client leaves.
//!
//! Worktree watching defaults on; `git.watch_worktree` sets it and
//! `GPY_WATCH_WORKTREE` (`0`/`false`/`no`/`off` disables) overrides the config.

use super::watch_set::WatchMode;
use super::{EventCallback, WatchCoordinator, WatchRegistry, WatcherConfig};
use crate::debug_log;
use crate::sync_ext::MutexExt;
use crate::{Error, Result};
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

/// Single-line pointer file read caps.
const GITDIR_POINTER_READ_CAP: usize = 4_096;
const COMMONDIR_POINTER_READ_CAP: usize = 4_096;

/// Test-only per-path watch failure injection (e.g. watch-budget exhaustion).
#[cfg(test)]
type WatchFault = Arc<dyn Fn(&Path) -> Option<Error> + Send + Sync>;

/// Multi-repository watcher registry with reference counting.
pub struct MultiRepoWatcher {
    watcher: Arc<Mutex<Option<WatchCoordinator>>>,
    watched_repos: Arc<Mutex<HashMap<PathBuf, WatchedRepo>>>,
    /// Root/gitdir attribution, worktree flag and ignore caches shared with the coordinator.
    registry: Arc<WatchRegistry>,
    /// Keyed by canonical common directory.
    common_watches: Mutex<HashMap<PathBuf, CommonDirWatch>>,
    ops_lock: Mutex<()>,
    #[cfg(test)]
    watch_fault: Option<WatchFault>,
}

impl MultiRepoWatcher {
    /// Create a new builder for `MultiRepoWatcher`.
    #[must_use]
    pub fn builder() -> MultiRepoWatcherBuilder {
        MultiRepoWatcherBuilder::new()
    }

    /// Register a client and begin watching the repository it resides in.
    ///
    /// # Errors
    /// Returns an error if the repository cannot be discovered or watched.
    pub fn register_client(&self, pid: u32, cwd: &Path) -> Result<()> {
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
                    Self::unwatch_logged(w, path, WatchMode::Recursive);
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
            self.release_common_dir_watches(&repo.git_root);
        }
    }

    /// Move a client to the repo containing `new_cwd`.
    ///
    /// # Errors
    /// Returns an error if the new repository cannot be discovered or watched;
    /// the client is then attached to no repository.
    pub fn update_client(&self, pid: u32, new_cwd: &Path) -> Result<()> {
        let _ops_guard = self.ops_lock.lock_or_recover();
        self.attach_pid_to(pid, new_cwd)
    }

    /// Stop watching all repositories and clear state.
    pub fn stop(&self) {
        {
            let mut watcher = self.watcher.lock_or_recover();
            if let Some(mut w) = watcher.take() {
                w.stop();
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
                Self::unwatch_logged(w, path, WatchMode::Recursive);
            }
        }

        // A relocated gitdir can move the common dir; re-acquire against the new one.
        self.release_common_dir_watches(git_root);

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

    /// Single seam for every `watch_directory` call, so tests can inject faults.
    ///
    /// # Errors
    /// Returns an error if `path` cannot be watched or a test fault fires.
    #[cfg_attr(
        not(test),
        expect(clippy::unused_self, reason = "self is used only under cfg(test)")
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

    /// Release one `mode` watch of `path`; failures are logged, not propagated.
    fn unwatch_logged(w: &mut WatchCoordinator, path: &Path, mode: WatchMode) {
        let released = match mode {
            WatchMode::Recursive => w.unwatch_directory(path),
            WatchMode::Shallow => w.unwatch_directory_shallow(path),
        };
        if let Err(err) = released {
            debug_log!("watcher", "Failed to unwatch {}: {err}", path.display());
        }
    }

    /// Arm watches for a repo. `.git` failure is fatal; a worktree-watch failure
    /// (likely watch-budget exhaustion) degrades to `.git`-only.
    ///
    /// # Errors
    /// Returns an error if the coordinator is gone or `.git` cannot be watched.
    fn start_repo_watches(
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

        // Linked worktrees only. Best-effort; refcounted in `common_watches`,
        // not in this repo's `armed`.
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

    /// Common git directory of a linked worktree; `None` for ordinary repos and
    /// submodules (no `commondir` pointer).
    fn resolve_common_dir(git_dir: &Path) -> Option<PathBuf> {
        let common =
            crate::fs_util::read_small_file(&git_dir.join("commondir"), COMMONDIR_POINTER_READ_CAP)
                .ok()
                .and_then(|contents| Self::join_commondir_pointer(git_dir, &contents))
                .or_else(|| Self::common_dir_from_worktrees_layout(git_dir))?;
        (common != git_dir).then_some(common)
    }

    /// Parse `commondir` contents, joining relative paths onto `git_dir`.
    #[must_use]
    fn parse_commondir_pointer(git_dir: &Path, contents: &str) -> Option<PathBuf> {
        let target = Path::new(contents.trim());
        if target.as_os_str().is_empty() {
            return None;
        }
        Some(crate::git::resolve_relative_to(git_dir, target))
    }

    /// Resolve a `commondir` pointer (typically `../..`) to a canonical path.
    fn join_commondir_pointer(git_dir: &Path, contents: &str) -> Option<PathBuf> {
        let resolved = Self::parse_commondir_pointer(git_dir, contents)?;
        fs::canonicalize(&resolved).ok()
    }

    /// Fallback when `commondir` is unreadable: git's (unpromised) layout is
    /// `<common>/worktrees/<name>`.
    fn common_dir_from_worktrees_layout(git_dir: &Path) -> Option<PathBuf> {
        let parent = git_dir.parent()?;
        if parent.file_name()? != "worktrees" {
            return None;
        }
        fs::canonicalize(parent.parent()?).ok()
    }

    /// Add `git_root` as a dependent of its common dir; the first arms watches.
    fn acquire_common_dir_watches(
        &self,
        w: &mut WatchCoordinator,
        git_root: &Path,
        git_dir: &Path,
    ) {
        let Some(common_dir) = Self::resolve_common_dir(git_dir) else {
            return;
        };

        {
            let mut guard = self.common_watches.lock_or_recover();
            let entry = guard.entry(common_dir.clone()).or_default();
            let first_dependent = entry.dependents.is_empty();
            entry.dependents.insert(git_root.to_path_buf());
            if first_dependent {
                entry.armed = Self::arm_common_dir(w, &common_dir);
            }
            drop(guard);
        }

        self.registry.register_common_gitdir(&common_dir, git_root);
    }

    /// Watch `<common>` shallow (`packed-refs`, `config`; a directory watch
    /// survives git's lock-then-rename) and `<common>/refs` recursive (covers a
    /// future `remotes/`). Never recursive over `<common>`: the poll backend
    /// would stat-walk `objects/**`. Already-covered paths are skipped.
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

    /// Drop `git_root` from every common dir; unwatch when the last dependent
    /// leaves, and re-arm survivors whose covering ancestor watch just went.
    fn release_common_dir_watches(&self, git_root: &Path) {
        let mut to_unwatch: ArmedWatches = Vec::new();
        let mut to_rearm: Vec<PathBuf> = Vec::new();

        // Release `common_watches` before taking the watcher lock: acquire
        // takes them in the opposite order.
        {
            let mut guard = self.common_watches.lock_or_recover();
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
            // Entries that armed nothing relied on an ancestor watch (usually
            // the main checkout's), which may have just been dropped.
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
        if let Some(w) = self.watcher.lock_or_recover().as_mut() {
            for (path, mode) in &to_unwatch {
                Self::unwatch_logged(w, path, *mode);
            }
            for common_dir in to_rearm {
                let armed = Self::arm_common_dir(w, &common_dir);
                if !armed.is_empty() {
                    rearmed.push((common_dir, armed));
                }
            }
        }

        if rearmed.is_empty() {
            return;
        }
        let mut guard = self.common_watches.lock_or_recover();
        for (common_dir, armed) in rearmed {
            if let Some(entry) = guard.get_mut(&common_dir) {
                entry.armed = armed;
            }
        }
    }

    /// Watch `git_dir` alone after a worktree-root watch failed.
    ///
    /// # Errors
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

    /// Find the canonical git root for `path`. Does not recognize bare repos;
    /// use [`crate::git::find_repo_root`] for that.
    #[must_use]
    pub fn find_git_root(path: &Path) -> Option<PathBuf> {
        crate::git::find_git_dir_only(path)
    }

    /// Resolve the actual git directory for the supplied repository.
    ///
    /// # Errors
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
mod tests;
