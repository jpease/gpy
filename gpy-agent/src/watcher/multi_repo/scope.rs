//! What [`MultiRepoWatcher`] arms: the repository filter and the home/root
//! exclusion from recursive worktree watching.

use super::{MultiRepoWatcher, WatchTarget, WatchedRepo};
use crate::debug_log;
use crate::sync_ext::MutexExt;
use std::fs;
use std::path::{Path, PathBuf};

/// A repository rooted at `/` or at the user's home directory (a dotfiles
/// repository) owns every non-repository directory beneath it, so a recursive
/// worktree watch would cover far more than the project the user is in.
fn is_home_or_filesystem_root(git_root: &Path) -> bool {
    git_root.parent().is_none()
        || crate::paths::home_path()
            .is_some_and(|home| fs::canonicalize(&home).unwrap_or(home) == git_root)
}

impl<T: WatchTarget> MultiRepoWatcher<T> {
    /// Whether `git_root` gets a recursive worktree watch: `git.watch_worktree`
    /// is in effect and the root is neither the home directory nor `/`.
    pub(super) fn worktree_watch_wanted(&self, git_root: &Path) -> bool {
        self.registry.worktree_enabled() && !is_home_or_filesystem_root(git_root)
    }

    /// Log once per arming that `git_root` gets its gitdir watched only.
    pub(super) fn log_gitdir_only_scope(&self, git_root: &Path) {
        if self.registry.worktree_enabled() && is_home_or_filesystem_root(git_root) {
            debug_log!(
                "watcher",
                "{} is the home directory or a filesystem root; watching its gitdir only",
                git_root.display()
            );
        }
    }

    /// Apply the registry's repository filter to a running watcher: stop
    /// watching rejected repositories, then re-attach every client in
    /// `clients` (`(pid, cwd)`), which arms repositories the filter now allows.
    pub fn resync_repo_filter(&self, clients: &[(u32, PathBuf)]) {
        let _ops_guard = self.ops_lock.lock_or_recover();

        let rejected: Vec<WatchedRepo> = self
            .watched_repos
            .lock_or_recover()
            .values_mut()
            .filter(|repo| !self.registry.repo_allowed(&repo.git_root))
            .map(|repo| {
                repo.clients.clear();
                repo.clone()
            })
            .collect();
        for repo in &rejected {
            debug_log!(
                "watcher",
                "Repository {} is now skipped; unwatching",
                repo.git_root.display()
            );
        }
        self.tear_down_repos(&rejected);

        for (pid, cwd) in clients {
            if let Err(err) = self.attach_pid_to(*pid, cwd) {
                debug_log!(
                    "watcher",
                    "Failed to re-attach client {pid} at {}: {err}",
                    cwd.display()
                );
            }
        }
    }
}
