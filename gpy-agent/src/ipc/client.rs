//! Test-only IPC client: length-prefixed framing on `gpy-test.sock`, incompatible with production.

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

/// Test-only IPC client; see the module docs for its framing.
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

    /// Connect to the agent -- native Windows stub.
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    #[cfg(not(unix))]
    pub async fn connect() -> Result<Self> {
        Err(crate::ipc::native_windows_unsupported())
    }

    /// Socket path, honouring the shells' override (empty means unset).
    fn default_socket_path() -> PathBuf {
        if let Some(custom_path) = crate::agent::lifecycle::socket_path_override() {
            return custom_path;
        }

        let runtime_root = Self::get_runtime_root();
        runtime_root.join("gpy-test.sock")
    }

    fn get_runtime_root() -> std::path::PathBuf {
        crate::paths::runtime_root_for(
            crate::paths::root_var("XDG_RUNTIME_DIR").as_deref(),
            crate::paths::root_var("XDG_CACHE_HOME").as_deref(),
            crate::paths::home_dir().as_deref(),
        )
    }

    /// Send a message and wait for response
    ///
    /// # Errors
    ///
    /// Returns an error if serialization, I/O, or deserialization fails.
    #[cfg(unix)]
    pub async fn send_request(&mut self, msg: Message) -> Result<Response> {
        let message_bytes = protocol::serialize_message(&msg)?;
        let message_len = u32::try_from(message_bytes.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes();

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

        let mut len_bytes = [0u8; 4];
        self.stream
            .read_exact(&mut len_bytes)
            .await
            .map_err(|e| Error::ipc(format!("Failed to read response length: {e}")))?;

        let response_len = usize::try_from(u32::from_be_bytes(len_bytes)).unwrap_or(usize::MAX);
        if response_len > 65536 {
            return Err(Error::ipc(format!(
                "Response too large: {response_len} bytes"
            )));
        }

        let mut response_bytes = vec![0u8; response_len];
        self.stream
            .read_exact(&mut response_bytes)
            .await
            .map_err(|e| Error::ipc(format!("Failed to read response: {e}")))?;

        protocol::deserialize_response(&response_bytes)
    }

    /// Send a message and wait for response -- native Windows stub.
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    #[cfg(not(unix))]
    pub async fn send_request(&mut self, _msg: Message) -> Result<Response> {
        Err(crate::ipc::native_windows_unsupported())
    }

    /// `Ok(true)` if the agent answers a ping with `Ack`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request or response fails.
    pub async fn ping(&mut self) -> Result<bool> {
        Ok(matches!(
            self.send_request(Message::Ping).await?,
            Response::Ack
        ))
    }

    /// Close the connection
    pub const fn disconnect() {
        // The stream closes on drop.
    }

    /// Get the socket path being used
    #[must_use]
    pub fn socket_path() -> String {
        Self::default_socket_path()
            .into_os_string()
            .into_string()
            .unwrap_or_else(|raw| raw.to_string_lossy().into_owned())
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
