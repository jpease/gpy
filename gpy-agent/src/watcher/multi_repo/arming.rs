//! Arming a repository's watches, re-arming stale ones, and retrying a
//! worktree watch that degraded to `.git`-only.

use super::common_dir::{acquire_common_dir_watches, release_common_dir_watches};
use super::{
    MultiRepoWatcher, RepoWatches, WatchTarget, WorktreeWatch, dir_identity, unwatch_logged,
};
use crate::debug_log;
use crate::sync_ext::MutexExt;
use crate::watcher::watch_set::WatchMode;
use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Least time between two attempts to re-arm a degraded repo's worktree watch
/// on attach; a release of other repos' watches retries at once.
const DEGRADED_RETRY_INTERVAL: Duration = Duration::from_mins(1);

/// Report in the warning log that `git_root` is watched `.git`-only.
fn warn_degraded(git_root: &Path, reason: &str) {
    crate::warn_log!(
        "watcher",
        "could not watch the working tree of {} ({reason}); watching its .git only, \
         so file edits there may show only after the periodic reconcile",
        git_root.display()
    );
}

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the `arming` helpers live in their own submodule of the watcher"
)]
impl<T: WatchTarget> MultiRepoWatcher<T> {
    /// Re-arm a tracked repo's stale watches and refresh its entry. `pid` is
    /// `None` for a config-driven re-arm, which must not change the refcount.
    ///
    /// # Errors
    /// Returns an error if the essential `.git` watch cannot be re-established.
    pub(super) fn rearm_repo_watch(
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

        let keeps_git_dir = stale
            .as_ref()
            .is_some_and(|(stale_git_dir, _)| stale_git_dir == git_dir);
        let repo_watches = self
            .start_repo_watches(git_root, git_dir, watch_worktree)
            .inspect_err(|_| {
                if !keeps_git_dir {
                    self.registry.unregister_external_gitdir(git_dir);
                }
            })?;

        if let Some((stale_git_dir, _)) = &stale
            && !keeps_git_dir
        {
            self.registry.unregister_external_gitdir(stale_git_dir);
        }

        {
            let mut repos = self.watched_repos.lock_or_recover();
            if let Some(repo) = repos.get_mut(git_root) {
                repo.git_dir = git_dir.to_path_buf();
                repo.git_dir_id = dir_identity(git_dir);
                repo.worktree = repo_watches.worktree;
                repo.armed = repo_watches.armed;
                repo.last_arm_attempt = Instant::now();
                // The stale watch may have missed changes before this one went live.
                repo.watched_since = Instant::now();
                if let Some(client_pid) = pid {
                    repo.clients.insert(client_pid);
                    repo.idle_since = None;
                }
            }
        }

        Ok(())
    }

    /// Arm watches for a repo, prepare the watcher for its events
    /// ([`WatchTarget::repo_armed`]), queue one catch-up refresh for changes
    /// made before the watch was live (the debounce window delivers it after
    /// the caller records the repo), and log one `Watch ready` line, the
    /// marker the e2e tests wait on before touching the tree. An external
    /// `git_dir` is registered first, as the poll backend picks its poller
    /// from the registry; after a failed arm the caller unregisters it.
    ///
    /// # Errors
    /// Returns an error if the coordinator is gone or `.git` cannot be watched.
    pub(super) fn start_repo_watches(
        &self,
        git_root: &Path,
        git_dir: &Path,
        watch_worktree: bool,
    ) -> Result<RepoWatches> {
        self.register_external_gitdir_if_needed(git_dir, git_root);
        let watches = self.arm_watches(git_root, git_dir, watch_worktree)?;
        if let Some(w) = self.watcher.lock_or_recover().as_ref() {
            w.repo_armed(git_root, git_dir);
            w.request_refresh(git_root);
        }
        debug_log!("watcher", "Watch ready for {}", git_root.display());
        Ok(watches)
    }

    /// Arm watches for a repo. `.git` failure is fatal; a worktree-watch failure
    /// (likely watch-budget exhaustion) degrades to `.git`-only, is reported in
    /// the warning log, and is retried later
    /// ([`Self::retry_degraded_worktrees`]).
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
                warn_degraded(git_root, &e.to_string());
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
        warn_degraded(git_root, reason);
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

    /// Retry the recursive worktree watch of degraded repos: just `only` when
    /// given, each at most every [`DEGRADED_RETRY_INTERVAL`] unless `released`
    /// (other watches were just given back to the OS budget). Caller holds
    /// `ops_lock`.
    pub(super) fn retry_degraded_worktrees(&self, only: Option<&Path>, released: bool) {
        let now = Instant::now();
        let due: Vec<PathBuf> = {
            let mut repos = self.watched_repos.lock_or_recover();
            repos
                .values_mut()
                .filter(|repo| {
                    repo.worktree == WorktreeWatch::Degraded
                        && only.is_none_or(|root| repo.git_root.as_path() == root)
                        && (released
                            || now.saturating_duration_since(repo.last_arm_attempt)
                                >= DEGRADED_RETRY_INTERVAL)
                })
                .map(|repo| {
                    repo.last_arm_attempt = now;
                    repo.git_root.clone()
                })
                .collect()
        };
        if due.is_empty() {
            return;
        }

        let mut rearmed = Vec::new();
        if let Some(w) = self.watcher.lock_or_recover().as_mut() {
            for git_root in due {
                if !self.worktree_watch_wanted(&git_root) {
                    continue;
                }
                match w.watch(&git_root) {
                    Ok(()) => {
                        // Worktree edits while degraded were never seen.
                        w.request_refresh(&git_root);
                        rearmed.push(git_root);
                    }
                    Err(e) => debug_log!(
                        "watcher",
                        "Worktree watch of {} still failing: {e}",
                        git_root.display()
                    ),
                }
            }
        }

        let mut repos = self.watched_repos.lock_or_recover();
        for git_root in rearmed {
            if let Some(repo) = repos.get_mut(&git_root) {
                crate::warn_log!(
                    "watcher",
                    "watching the working tree of {} again",
                    git_root.display()
                );
                repo.armed.push(git_root);
                repo.worktree = WorktreeWatch::Armed;
            }
        }
    }
}
