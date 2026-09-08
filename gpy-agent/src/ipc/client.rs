//! Test-only IPC client with length-prefixed framing
//!
//! **WARNING:** This client uses length-prefixed message framing and is NOT
//! compatible with the production server or Fish shell, which use newline-delimited JSON.
//!
//! This module is intended for Rust integration tests only.
//! Production IPC from Fish uses simple newline-delimited JSON over `gpy.sock`.

#[cfg(unix)]
use super::protocol;
use super::{Message, Response};
#[cfg(unix)]
use crate::Error;
use crate::Result;
use crate::formatter::Format;
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::UnixStream;

/// Test-only IPC client with length-prefixed framing
///
/// **Not compatible with production server!**
/// Uses length-prefixed messages instead of newline-delimited JSON.
pub struct SessionHandle {
    #[cfg(unix)]
    stream: UnixStream,
}

impl SessionHandle {
    /// Connect to the agent
    ///
    /// # Errors
    ///
    /// Returns an error if the connection times out or if the socket cannot be connected to.
    #[cfg(unix)]
    pub async fn connect() -> Result<Self> {
        let socket_path = Self::default_socket_path();

        // Try to connect with timeout
        let stream =
            tokio::time::timeout(Duration::from_secs(5), UnixStream::connect(&socket_path))
                .await
                .map_err(|_| Error::ipc("Connection timeout".to_owned()))?
                .map_err(|e| {
                    Error::ipc(format!(
                        "Failed to connect to agent at '{}': {}",
                        socket_path.display(),
                        e
                    ))
                })?;

        Ok(Self { stream })
    }

    /// Connect to the agent -- native Windows stub (#284).
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    #[cfg(not(unix))]
    pub async fn connect() -> Result<Self> {
        Err(crate::ipc::native_windows_unsupported())
    }

    /// Get the default socket path (matches server implementation)
    fn default_socket_path() -> PathBuf {
        // Check for custom socket path in environment (matches Fish shell logic).
        // Empty means unset, exactly as every shell reads it (#626).
        if let Some(custom_path) = crate::agent::lifecycle::socket_path_override() {
            return custom_path;
        }

        // Unix socket path for tests - uses gpy-test.sock to avoid conflicts
        // Production uses gpy.sock with newline-delimited protocol
        let runtime_root = Self::get_runtime_root();
        runtime_root.join("gpy-test.sock")
    }

    /// Get GPY runtime root directory (matches server logic)
    fn get_runtime_root() -> std::path::PathBuf {
        // Follow same precedence as server and Fish shell
        crate::paths::runtime_root_for(
            std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
            std::env::var("XDG_CACHE_HOME").ok().as_deref(),
            crate::paths::home_dir().as_deref(),
        )
    }

    /// Send a message and wait for response
    ///
    /// # Errors
    ///
    /// Returns an error if message serialization fails, if sending fails,
    /// or if the response cannot be received or deserialized.
    #[cfg(unix)]
    pub async fn send_request(&mut self, msg: Message) -> Result<Response> {
        // Serialize the message
        let message_bytes = protocol::serialize_message(&msg)?;
        let message_len = u32::try_from(message_bytes.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes();

        // Send message length first, then message
        self.stream
            .write_all(&message_len)
            .await
            .map_err(|e| Error::ipc(format!("Failed to send message length: {e}")))?;
        self.stream
            .write_all(&message_bytes)
            .await
            .map_err(|e| Error::ipc(format!("Failed to send message: {e}")))?;
        self.stream
            .flush()
            .await
            .map_err(|e| Error::ipc(format!("Failed to flush message: {e}")))?;

        // Read response length
        let mut len_bytes = [0u8; 4];
        self.stream
            .read_exact(&mut len_bytes)
            .await
            .map_err(|e| Error::ipc(format!("Failed to read response length: {e}")))?;

        let response_len = usize::try_from(u32::from_be_bytes(len_bytes)).unwrap_or(usize::MAX);
        if response_len > 65536 {
            // 64KB max
            return Err(Error::ipc(format!(
                "Response too large: {response_len} bytes"
            )));
        }

        // Read response data
        let mut response_bytes = vec![0u8; response_len];
        self.stream
            .read_exact(&mut response_bytes)
            .await
            .map_err(|e| Error::ipc(format!("Failed to read response: {e}")))?;

        // Deserialize response
        protocol::deserialize_response(&response_bytes)
    }

    /// Send a message and wait for response -- native Windows stub (#284).
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    #[cfg(not(unix))]
    pub async fn send_request(&mut self, _msg: Message) -> Result<Response> {
        Err(crate::ipc::native_windows_unsupported())
    }

    /// Check if agent is reachable
    ///
    /// # Errors
    ///
    /// Returns an error if the ping request fails to send or receive.
    pub async fn ping(&mut self) -> Result<bool> {
        match self.send_request(Message::Ping).await {
            Ok(Response::Ack) => Ok(true),
            Ok(_) | Err(_) => Ok(false), // Unexpected response or connection failed
        }
    }

    /// Close the connection
    pub const fn disconnect() {
        // The connection will be closed when the stream is dropped
        // No explicit action needed for Unix sockets
    }

    /// Get the socket path being used
    #[must_use]
    pub fn socket_path() -> String {
        Self::default_socket_path().to_string_lossy().to_string()
    }

    /// Convenience method for sending git status request
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails to send or receive.
    pub async fn load_repository_state(&mut self, path: String) -> Result<Response> {
        let safe_path = crate::security::SafePath::new(&path)?;
        self.send_request(Message::RepositoryStatus {
            path: safe_path,
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        })
        .await
    }

    /// Convenience method for sending language detection request
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails to send or receive.
    pub async fn detect_languages(&mut self, path: String) -> Result<Response> {
        let safe_path = crate::security::SafePath::new(&path)?;
        self.send_request(Message::LanguageDetect {
            path: safe_path,
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
            virtual_env: None,
        })
        .await
    }
}
