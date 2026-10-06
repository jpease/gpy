//! Test-only transport layer for IPC testing
//!
//! This module provides a simplified transport API for integration tests.
//! It is NOT used by the production server or Fish shell.
//!
//! Production IPC uses newline-delimited JSON over `gpy.sock`.
//! This test transport uses `gpy-test.sock` to avoid conflicts.

use crate::{Error, Result};
use std::path::PathBuf;
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

/// Transport connection mode
#[derive(Debug, Clone, Copy)]
pub enum ConnectionMode {
    /// Server mode - listens for connections
    Server,
    /// Client mode - connects to server
    Client,
}

/// Simple Unix domain socket transport for Fish shell
pub struct Transport {
    mode: ConnectionMode,
    socket_path: PathBuf,
    #[cfg(unix)]
    listener: Option<UnixListener>,
    #[cfg(unix)]
    stream: Option<UnixStream>,
}

impl Transport {
    /// Create a new transport for server (listening) mode
    ///
    /// # Errors
    ///
    /// Returns an error if the default socket path cannot be determined.
    pub fn new_server() -> Result<Self> {
        let socket_path = Self::get_default_path()?;
        Ok(Self {
            mode: ConnectionMode::Server,
            socket_path,
            #[cfg(unix)]
            listener: None,
            #[cfg(unix)]
            stream: None,
        })
    }

    /// Create a new transport for client (connecting) mode
    ///
    /// # Errors
    ///
    /// Returns an error if the default socket path cannot be determined.
    pub fn new_client() -> Result<Self> {
        let socket_path = Self::get_default_path()?;
        Ok(Self {
            mode: ConnectionMode::Client,
            socket_path,
            #[cfg(unix)]
            listener: None,
            #[cfg(unix)]
            stream: None,
        })
    }

    /// Bind and start listening (server mode)
    ///
    /// # Errors
    ///
    /// Returns an error if not in server mode, if the socket file cannot be removed,
    /// or if binding to the socket fails.
    #[cfg(unix)]
    pub fn bind(&mut self) -> Result<()> {
        if !matches!(self.mode, ConnectionMode::Server) {
            return Err(Error::ipc("Cannot bind in client mode".to_owned()));
        }

        // Remove existing socket file if present (ignore errors - bind will fail if socket is actually in use)
        // This avoids TOCTOU race - we don't check existence, just attempt removal
        let _ = std::fs::remove_file(&self.socket_path);

        let listener = UnixListener::bind(&self.socket_path)
            .map_err(|e| Error::ipc(format!("Failed to bind Unix socket: {e}")))?;

        self.listener = Some(listener);
        Ok(())
    }

    /// Bind and start listening (server mode) -- native Windows stub (#284).
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    #[cfg(not(unix))]
    pub fn bind(&mut self) -> Result<()> {
        Err(crate::ipc::native_windows_unsupported())
    }

    /// Connect to server (client mode)
    ///
    /// # Errors
    ///
    /// Returns an error if not in client mode or if the connection fails.
    #[cfg(unix)]
    pub async fn connect(&mut self) -> Result<()> {
        if !matches!(self.mode, ConnectionMode::Client) {
            return Err(Error::ipc("Cannot connect in server mode".to_owned()));
        }

        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| Error::ipc(format!("Failed to connect to Unix socket: {e}")))?;

        self.stream = Some(stream);
        Ok(())
    }

    /// Connect to server (client mode) -- native Windows stub (#284).
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    #[cfg(not(unix))]
    pub async fn connect(&mut self) -> Result<()> {
        Err(crate::ipc::native_windows_unsupported())
    }

    /// Send data
    ///
    /// # Errors
    ///
    /// Returns an error if no connection is active or if sending fails.
    #[cfg(unix)]
    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        match &mut self.stream {
            Some(stream) => {
                stream
                    .write_all(data)
                    .await
                    .map_err(|e| Error::ipc(format!("Failed to send data: {e}")))?;
                Ok(())
            }
            None => Err(Error::ipc("No active connection".to_owned())),
        }
    }

    /// Send data -- native Windows stub (#284).
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    #[cfg(not(unix))]
    pub async fn send(&mut self, _data: &[u8]) -> Result<()> {
        Err(crate::ipc::native_windows_unsupported())
    }

    /// Receive data
    ///
    /// # Errors
    ///
    /// Returns an error if no connection is active or if receiving fails.
    #[cfg(unix)]
    pub async fn receive(&mut self) -> Result<Vec<u8>> {
        match &mut self.stream {
            Some(stream) => {
                let mut buffer = Vec::new();
                stream
                    .read_to_end(&mut buffer)
                    .await
                    .map_err(|e| Error::ipc(format!("Failed to receive data: {e}")))?;
                Ok(buffer)
            }
            None => Err(Error::ipc("No active connection".to_owned())),
        }
    }

    /// Receive data -- native Windows stub (#284).
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    #[cfg(not(unix))]
    pub async fn receive(&mut self) -> Result<Vec<u8>> {
        Err(crate::ipc::native_windows_unsupported())
    }

    /// Get the socket path (test-only)
    #[must_use]
    pub fn address() -> String {
        Self::get_default_path().map_or_else(
            |_| "/tmp/gpy-test.sock".to_owned(),
            |path| path.to_string_lossy().to_string(),
        )
    }

    /// Get default socket path for test scenarios
    ///
    /// Uses `gpy-test.sock` to avoid conflicts with production `gpy.sock`.
    ///
    /// # Errors
    ///
    /// Returns an error if required runtime directories cannot be created or accessed.
    fn get_default_path() -> Result<PathBuf> {
        // Follow XDG Base Directory specification
        // Use gpy-test.sock to distinguish from production gpy.sock
        // Normalised like `paths::runtime_root_for` does (#626): an empty or
        // relative `XDG_*` value does not count as set.
        let xdg_runtime_raw = std::env::var("XDG_RUNTIME_DIR").ok();
        let xdg_cache_raw = std::env::var("XDG_CACHE_HOME").ok();
        let xdg_runtime =
            crate::paths::xdg_value(xdg_runtime_raw.as_deref(), crate::paths::Os::Unix);
        let xdg_cache = crate::paths::xdg_value(xdg_cache_raw.as_deref(), crate::paths::Os::Unix);
        let home = crate::paths::home_dir();

        // The `/tmp` fallback (all three unset) never created its directory
        // here -- preserve that no-I/O behaviour rather than folding it into
        // the branches below, which do.
        if xdg_runtime.is_none() && xdg_cache.is_none() && home.is_none() {
            return Ok(PathBuf::from("/tmp/gpy-test.sock"));
        }

        let runtime_path = crate::paths::runtime_root_for(xdg_runtime, xdg_cache, home.as_deref());
        // Branch-specific error text, matching the pre-#477 per-branch
        // resolvers this now replaces.
        let context = if xdg_runtime.is_some() {
            "runtime directory"
        } else {
            "cache directory"
        };
        if let Err(e) = std::fs::create_dir_all(&runtime_path) {
            // Only fail if it's not already existing - race conditions between tests are OK
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(Error::ipc(format!("Failed to create {context}: {e}")));
            }
        }
        Ok(runtime_path.join("gpy-test.sock"))
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        // Best-effort cleanup of socket file for server mode
        if matches!(self.mode, ConnectionMode::Server) && self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}
