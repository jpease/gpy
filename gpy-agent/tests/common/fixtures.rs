//! Shared fixtures for agent integration tests.

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(dead_code)]
#![allow(clippy::io_other_error)]
#![allow(clippy::if_not_else)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::unused_self)]

use gpy_agent::config::Config;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// Builds in-memory test configurations without touching disk.
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
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn with_git_enabled(mut self, enabled: bool) -> Self {
        self.config.git.enabled = enabled;
        self
    }

    #[must_use]
    pub const fn with_language_enabled(mut self, enabled: bool) -> Self {
        self.config.language.enabled = enabled;
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, seconds: u64) -> Self {
        self.config.agent.timeout_seconds =
            gpy_agent::config::types::AgentTimeout::new(seconds).expect("Valid timeout");
        self
    }

    #[must_use]
    pub fn build(self) -> Config {
        self.config
    }
}

/// Temporary Unix socket path, removed on drop.
pub struct TempSocket {
    path: PathBuf,
}

impl TempSocket {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            path: create_temp_socket_path()?,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempSocket {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Per-user root for all test sockets, so cleanup tooling can delete leaked
/// sockets by location rather than by name. Honors `TMPDIR`/`TMP`/`TEMP`.
pub fn gpy_test_root() -> std::io::Result<PathBuf> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".to_owned());
    let root = std::env::temp_dir().join(format!("gpy-test-{user}"));
    fs::create_dir_all(&root)?;
    Ok(root)
}

/// Socket path inside a fresh, intentionally leaked temp dir.
pub fn create_temp_socket_path() -> std::io::Result<PathBuf> {
    let root = gpy_test_root()?;
    let temp_dir = tempfile::Builder::new().prefix("sock-").tempdir_in(&root)?;
    let socket_path = temp_dir.path().join("gpy-test.sock");

    // Kept so the dir outlives this function.
    let _ = temp_dir.keep();

    Ok(socket_path)
}

/// Polls until the socket path exists or the timeout elapses.
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

/// Aborts the wrapped server task on drop, including during panic unwinding.
pub struct ServerGuard<T> {
    /// Always `Some` until `into_inner` takes it.
    handle: Option<tokio::task::JoinHandle<T>>,
}

impl<T> ServerGuard<T> {
    #[must_use]
    pub const fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    /// Consumes the guard without aborting the task.
    #[must_use]
    pub fn into_inner(mut self) -> tokio::task::JoinHandle<T> {
        self.handle
            .take()
            .expect("ServerGuard handle is present until into_inner")
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.handle
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
    }
}

impl<T> Drop for ServerGuard<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
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
