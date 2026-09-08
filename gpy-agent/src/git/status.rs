//! Git status detection
//!
//! Provides fast git status information for prompt display using native git subprocess.

use super::CompleteStatus;
use super::native::NativeGitBackend;
use crate::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Abstraction over a git status backend, allowing the orchestration logic in
/// this module to be exercised with a mock in tests, without spawning a real
/// `git` subprocess.
///
/// `NativeGitBackend` is the sole production implementation; it delegates to
/// its existing `get_complete_status` associated function.
pub trait GitBackend {
    /// Get complete git status for a directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be accessed or if git operations fail.
    #[expect(
        clippy::too_many_arguments,
        reason = "each parameter (path, paths, max_ahead_behind, stash_enabled, timeout) is an independent tuning knob the caller sets from config; grouping them would only hide the same five names in a struct"
    )]
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

/// Main git status detection orchestrator, generic over the backend used.
///
/// Production callers should use [`load_repository_state`]; this generic
/// form exists so orchestration can be tested with a mock [`GitBackend`]
/// instead of a real git subprocess.
///
/// # Errors
///
/// Returns an error if the path cannot be accessed or if git operations fail.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter (backend, path, paths, max_ahead_behind, stash_enabled, timeout) is an independent tuning knob the caller sets from config; grouping them would only hide the same names in a struct"
)]
pub fn load_repository_state_with<B: GitBackend>(
    backend: &B,
    path: &Path,
    paths: Option<&[PathBuf]>,
    max_ahead_behind: usize,
    stash_enabled: bool,
    timeout: Duration,
) -> Result<Option<CompleteStatus>> {
    backend.complete_status(path, paths, max_ahead_behind, stash_enabled, timeout)
}

/// Main git status detection orchestrator.
///
/// Get complete git status for a directory
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
    load_repository_state_with(
        &NativeGitBackend,
        path.as_ref(),
        paths,
        max_ahead_behind,
        stash_enabled,
        Duration::from_secs(crate::config::types::GitTimeout::DEFAULT),
    )
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::git::{FileStatus, RepositoryState, RepositoryStatus};
    use std::collections::HashMap;

    /// Canned status payload for the mock backend fixtures below.
    type MockStatusResult = Result<Option<CompleteStatus>>;

    /// A backend stub returning a canned response, without touching the
    /// filesystem or spawning any process. Proves `load_repository_state_with`
    /// dispatches purely through the `GitBackend` trait.
    struct MockGitBackend {
        response: MockStatusResult,
    }

    impl GitBackend for MockGitBackend {
        fn complete_status(
            &self,
            _path: &Path,
            _paths: Option<&[PathBuf]>,
            _max_ahead_behind: usize,
            _stash_enabled: bool,
            _timeout: Duration,
        ) -> Result<Option<CompleteStatus>> {
            match &self.response {
                Ok(value) => Ok(value.clone()),
                Err(Error::Git { message, .. }) => Err(Error::Git {
                    message: message.clone(),
                    source: None,
                }),
                Err(other) => panic!("unexpected error variant in mock fixture: {other}"),
            }
        }
    }

    fn sample_status() -> RepositoryStatus {
        RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 1,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 2,
            unstaged: 0,
            untracked: 1,
            conflicts: 0,
            state: RepositoryState::Dirty,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }
    }

    #[test]
    fn load_repository_state_with_returns_mock_backend_status() {
        let mut files = HashMap::new();
        files.insert(
            PathBuf::from("src/main.rs"),
            FileStatus {
                staged: true,
                unstaged: false,
                untracked: false,
                conflicted: false,
            },
        );
        let expected = CompleteStatus {
            status: sample_status(),
            files,
        };
        let mock = MockGitBackend {
            response: Ok(Some(expected.clone())),
        };

        let result = load_repository_state_with(
            &mock,
            Path::new("/does/not/matter"),
            None,
            10,
            true,
            Duration::from_secs(5),
        )
        .expect("mock backend should not error");

        assert_eq!(result, Some(expected));
    }

    #[test]
    fn load_repository_state_with_returns_none_for_non_repo() {
        let mock = MockGitBackend { response: Ok(None) };

        let result = load_repository_state_with(
            &mock,
            Path::new("/not/a/repo"),
            None,
            10,
            true,
            Duration::from_secs(5),
        )
        .expect("mock backend should not error");

        assert_eq!(result, None);
    }

    #[test]
    fn load_repository_state_with_propagates_backend_error() {
        let mock = MockGitBackend {
            response: Err(Error::Git {
                message: "mock failure".to_owned(),
                source: None,
            }),
        };

        let err = load_repository_state_with(
            &mock,
            Path::new("/does/not/matter"),
            None,
            10,
            true,
            Duration::from_secs(5),
        )
        .expect_err("mock backend error should propagate");

        assert!(err.to_string().contains("mock failure"));
    }
}
