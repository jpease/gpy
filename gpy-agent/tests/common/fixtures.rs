//! Test fixtures for GPY agent testing
//!
//! This module provides comprehensive test fixtures to eliminate duplicate test setup code
//! and accelerate test development.
//!
//! # Available Fixtures
//!
//! - [`TempGitRepo`] - Creates temporary git repositories with configurable state
//! - [`MockConfig`] - Creates test configurations with sensible defaults
//! - [`TempSocket`] - Creates temporary Unix socket paths with auto-cleanup
//! - [`ServerGuard`] - Wraps tokio `JoinHandle` with automatic abort on drop
//!
//! # Example Usage
//!
//! ```rust,no_run
//! use gpy_agent::tests::common::fixtures::TempGitRepo;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a git repo with specific state
//! let repo = TempGitRepo::new()?
//!     .with_branch("feature")?
//!     .with_commits(3)?
//!     .with_staged("file.txt")?
//!     .build()?;
//!
//! // Cleanup happens automatically on drop
//! # Ok(())
//! # }
//! ```

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(dead_code)]
#![allow(clippy::error_impl_error)]
#![allow(clippy::io_other_error)]
#![allow(clippy::if_not_else)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::std_instead_of_core)]
#![allow(clippy::std_instead_of_alloc)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::unused_self)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::mem_forget)] // Intentional leak of TempDir to keep socket path alive

use gpy_agent::config::Config;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// Creates test configurations with sensible defaults
///
/// Provides a builder API for creating test configurations without loading
/// from disk.
///
/// # Example
///
/// ```rust,no_run
/// use gpy_agent::tests::common::fixtures::MockConfig;
///
/// let config = MockConfig::default()
///     .with_git_enabled(true)
///     .with_language_enabled(false)
///     .with_timeout(30)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct MockConfig {
    config: Config,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            config: Config::default(),
        }
    }
}

impl MockConfig {
    /// Create new mock config with defaults
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable/disable git segment
    #[must_use]
    pub fn with_git_enabled(mut self, enabled: bool) -> Self {
        self.config.git.enabled = enabled;
        self
    }

    /// Enable/disable language segment
    #[must_use]
    pub fn with_language_enabled(mut self, enabled: bool) -> Self {
        self.config.language.enabled = enabled;
        self
    }

    /// Set timeout in seconds
    #[must_use]
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.config.agent.timeout_seconds =
            gpy_agent::config::types::AgentTimeout::new(seconds).expect("Valid timeout");
        self
    }

    /// Build final config
    #[must_use]
    pub fn build(self) -> Config {
        self.config
    }
}

/// Creates temporary Unix socket paths with auto-cleanup
///
/// Provides temporary socket paths for IPC testing.
///
/// # Example
///
/// ```rust,no_run
/// use gpy_agent::tests::common::fixtures::TempSocket;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let socket = TempSocket::new()?;
/// let path = socket.path();
///
/// // Use path for testing
/// // Cleanup happens automatically on drop
/// # Ok(())
/// # }
/// ```
pub struct TempSocket {
    path: PathBuf,
}

impl TempSocket {
    /// Create new temporary socket path
    ///
    /// # Errors
    ///
    /// Returns error if temp directory creation fails
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            path: create_temp_socket_path()?,
        })
    }

    /// Get the socket path
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempSocket {
    fn drop(&mut self) {
        // Clean up socket file if it exists
        let _ = fs::remove_file(&self.path);
    }
}

/// The single GPY-owned root all test-harness sockets are created beneath.
///
/// Scoping every test socket to one explicit, per-user root (rather than
/// scattering them across the bare system temp directory) lets cleanup
/// tooling (`scripts/cleanup-test-agents.sh`) delete leaked sockets by
/// location instead of by name alone, so it can never remove a same-named
/// socket it did not create (#619).
///
/// Honors `TMPDIR`/`TMP`/`TEMP` the same way [`std::env::temp_dir`] does, and
/// namespaces by `USER`/`LOGNAME` so concurrent users on a shared machine
/// don't collide. Creates the directory if it doesn't already exist.
///
/// # Errors
///
/// Returns error if the root directory cannot be created.
pub fn gpy_test_root() -> std::io::Result<PathBuf> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".to_owned());
    let root = std::env::temp_dir().join(format!("gpy-test-{user}"));
    fs::create_dir_all(&root)?;
    Ok(root)
}

/// Create a temporary socket path
///
/// # Errors
///
/// Returns error if temp directory creation fails
pub fn create_temp_socket_path() -> std::io::Result<PathBuf> {
    let root = gpy_test_root()?;
    let temp_dir = tempfile::Builder::new().prefix("sock-").tempdir_in(&root)?;
    let socket_path = temp_dir.path().join("gpy-test.sock");

    // Keep temp_dir alive by leaking it (socket paths need to persist)
    // This is acceptable in tests
    std::mem::forget(temp_dir);

    Ok(socket_path)
}

/// Wait for agent to be ready to accept connections
///
/// Polls the socket path until it exists or timeout is reached.
///
/// # Errors
///
/// Returns error if timeout is reached before agent is ready
pub fn wait_for_agent_ready(socket_path: &Path, timeout: Duration) -> std::io::Result<()> {
    let start = std::time::Instant::now();

    while start.elapsed() < timeout {
        if socket_path.exists() {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(100));
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!("Agent did not become ready within {timeout:?}"),
    ))
}

/// Guard for tokio spawned server tasks that ensures cleanup on drop
///
/// Wraps a `tokio::task::JoinHandle` and automatically calls `.abort()` when dropped.
/// This prevents test process leaks when tests fail, panic, or are interrupted.
///
/// # Example
///
/// ```rust,no_run
/// use tokio;
/// use gpy_agent::tests::common::fixtures::ServerGuard;
///
/// # #[tokio::test]
/// # async fn test_example() -> Result<(), Box<dyn std::error::Error>> {
/// // Spawn server task
/// let server_handle = tokio::spawn(async {
///     // Server code...
///     Ok(())
/// });
///
/// // Wrap in guard for automatic cleanup
/// let _guard = ServerGuard::new(server_handle);
///
/// // Test code...
/// // If test panics or fails, guard ensures server is aborted
/// # Ok(())
/// # }
/// ```
///
/// # Why This Exists
///
/// Without this guard, interrupted tests (Ctrl+C, panics, assertions) can leave
/// tokio tasks running, which:
/// - Hold resources (sockets, file descriptors, memory)
/// - Can accumulate and exhaust system limits (e.g., 511 PTYs on macOS)
/// - Interfere with subsequent test runs
///
/// The guard pattern ensures cleanup happens via Rust's `Drop` trait, which
/// runs even during unwinding from panics.
pub struct ServerGuard<T> {
    handle: tokio::task::JoinHandle<T>,
}

impl<T> ServerGuard<T> {
    /// Create a new server guard wrapping a tokio `JoinHandle`
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use tokio;
    /// use gpy_agent::tests::common::fixtures::ServerGuard;
    ///
    /// # #[tokio::test]
    /// # async fn example() {
    /// let handle = tokio::spawn(async { /* server code */ });
    /// let guard = ServerGuard::new(handle);
    /// // Server will be aborted when guard is dropped
    /// # }
    /// ```
    #[must_use]
    pub fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self { handle }
    }

    /// Consume the guard and return the inner `JoinHandle`
    ///
    /// Use this if you need to manually control the task lifecycle.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use gpy_agent::tests::common::fixtures::ServerGuard;
    /// # #[tokio::test]
    /// # async fn example() {
    /// # let handle = tokio::spawn(async {});
    /// let guard = ServerGuard::new(handle);
    /// let handle = guard.into_inner();
    /// // Now you must manually abort/await the handle
    /// handle.abort();
    /// # }
    /// ```
    #[must_use]
    pub fn into_inner(self) -> tokio::task::JoinHandle<T> {
        // Skip Drop by consuming self
        let handle = unsafe { std::ptr::read(std::ptr::addr_of!(self.handle)) };
        std::mem::forget(self);
        handle
    }

    /// Check if the task has finished
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use gpy_agent::tests::common::fixtures::ServerGuard;
    /// # #[tokio::test]
    /// # async fn example() {
    /// # let handle = tokio::spawn(async {});
    /// let guard = ServerGuard::new(handle);
    /// if guard.is_finished() {
    ///     println!("Task completed");
    /// }
    /// # }
    /// ```
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

impl<T> Drop for ServerGuard<T> {
    fn drop(&mut self) {
        // Abort the task when guard is dropped
        // This happens automatically on test failure, panic, or normal completion
        self.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_config_builder() {
        let config = MockConfig::new()
            .with_git_enabled(false)
            .with_language_enabled(true)
            .with_timeout(60)
            .build();

        assert!(!config.git.enabled);
        assert!(config.language.enabled);
        assert_eq!(config.agent.timeout_seconds.get(), 60);
    }

    #[test]
    fn test_temp_socket() {
        let socket = TempSocket::new().unwrap();
        assert!(!socket.path().to_string_lossy().is_empty());
    }
}
