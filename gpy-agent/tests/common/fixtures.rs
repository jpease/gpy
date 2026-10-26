//! Shared fixtures for agent integration tests.

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::io_other_error)]
#![allow(clippy::if_not_else)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::unused_self)]

use std::fs;
use std::path::{Path, PathBuf};

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
fn create_temp_socket_path() -> std::io::Result<PathBuf> {
    let root = gpy_test_root()?;
    let temp_dir = tempfile::Builder::new().prefix("sock-").tempdir_in(&root)?;
    let socket_path = temp_dir.path().join("gpy-test.sock");

    // Kept so the dir outlives this function.
    let _ = temp_dir.keep();

    Ok(socket_path)
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
}

impl<T> Drop for ServerGuard<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}
