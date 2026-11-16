//! Git status detection for prompt display.

use super::native::NativeGitBackend;
use super::{CompleteStatus, PathspecScan};
use crate::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the agent's refresh path gets status: [`NativeGitBackend`] in
/// production; tests substitute a backend that counts scans.
pub(crate) trait GitBackend {
    /// Get complete git status for a directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be accessed or if git operations fail.
    fn complete_status(
        &self,
        path: &Path,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>>;

    /// File statuses for only `paths` under the canonical `repo_root`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails or `timeout` passes.
    fn pathspec_status(
        &self,
        repo_root: &Path,
        paths: &[PathBuf],
        timeout: Duration,
    ) -> Result<PathspecScan>;
}

impl GitBackend for NativeGitBackend {
    fn complete_status(
        &self,
        path: &Path,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>> {
        Self::get_complete_status(path, max_ahead_behind, stash_enabled, timeout)
    }

    fn pathspec_status(
        &self,
        repo_root: &Path,
        paths: &[PathBuf],
        timeout: Duration,
    ) -> Result<PathspecScan> {
        Self::get_pathspec_status(repo_root, paths, timeout)
    }
}

/// Get complete git status for a directory using the native backend, every
/// git command finishing within `timeout` (`git.timeout_seconds`).
///
/// # Errors
///
/// Returns an error if the path cannot be accessed, git operations fail, or
/// `timeout` passes.
pub fn load_repository_state<P: AsRef<Path>>(
    path: P,
    max_ahead_behind: usize,
    stash_enabled: bool,
    timeout: Duration,
) -> Result<Option<CompleteStatus>> {
    NativeGitBackend::get_complete_status(path.as_ref(), max_ahead_behind, stash_enabled, timeout)
}
