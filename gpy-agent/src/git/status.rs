//! Git status detection for prompt display.

use super::CompleteStatus;
use super::native::NativeGitBackend;
use crate::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the agent's refresh path gets complete status: [`NativeGitBackend`]
/// in production; tests substitute a backend that counts capture rounds.
pub(crate) trait GitBackend {
    /// Get complete git status for a directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be accessed or if git operations fail.
    fn complete_status(
        &self,
        path: &Path,
        paths: Option<&[PathBuf]>,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>>;
}

impl GitBackend for NativeGitBackend {
    fn complete_status(
        &self,
        path: &Path,
        paths: Option<&[PathBuf]>,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>> {
        Self::get_complete_status(path, paths, max_ahead_behind, stash_enabled, timeout)
    }
}

/// Get complete git status for a directory using the native backend.
///
/// # Errors
///
/// Returns an error if the path cannot be accessed or if git operations fail.
pub fn load_repository_state<P: AsRef<Path>>(
    path: P,
    paths: Option<&[PathBuf]>,
    max_ahead_behind: usize,
    stash_enabled: bool,
) -> Result<Option<CompleteStatus>> {
    NativeGitBackend::get_complete_status(
        path.as_ref(),
        paths,
        max_ahead_behind,
        stash_enabled,
        Duration::from_secs(crate::config::types::GitTimeout::DEFAULT),
    )
}
