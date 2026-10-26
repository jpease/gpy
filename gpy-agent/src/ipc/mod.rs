//! JSON IPC between shell clients and the agent over Unix domain sockets.

mod client_pid;
pub mod handlers;
pub mod protocol;
pub mod registry;
pub mod server;
#[cfg(unix)]
pub(crate) mod socket_identity;

pub use crate::formatter::Format;

/// Wire form of [`Response::Ack`] as rendered by the JSON formatter.
///
/// Not serde's `"Ack"`; shell clients match on this shape, so fixtures modelling a live agent must send it.
pub const ACK_WIRE_JSON: &str = r#"{"status":"ok"}"#;

/// Decode a socket reply into a [`Response`].
///
/// Accepts native serde encoding plus the formatter's [`ACK_WIRE_JSON`] and `{"error":"..."}`.
///
/// # Errors
///
/// Returns an error when the text is neither encoding.
pub fn decode_wire_reply(text: &str) -> crate::Result<Response> {
    let body = text.trim_end_matches(['\n', '\r']);
    if let Ok(response) = serde_json::from_str::<Response>(body) {
        return Ok(response);
    }
    let value = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|e| crate::Error::ipc(format!("Failed to parse agent response: {e}")))?;
    if value.get("status").and_then(serde_json::Value::as_str) == Some("ok") {
        return Ok(Response::Ack);
    }
    if let Some(message) = value.get("error").and_then(serde_json::Value::as_str) {
        return Ok(Response::Error {
            message: message.to_owned(),
        });
    }
    Err(crate::Error::ipc(format!(
        "Failed to parse agent response: not a Response: {body}"
    )))
}
pub use client_pid::ClientPid;
pub use registry::ClientDirectory;

use serde::{Deserialize, Serialize};

/// Error for IPC operations on native Windows, which has no transport (WSL works).
#[cfg(not(unix))]
pub(crate) fn native_windows_unsupported() -> crate::Error {
    crate::Error::ipc(
        "GPY's IPC transport is Unix-socket based and not yet implemented for native \
         Windows; run under WSL instead."
            .to_owned(),
    )
}

/// IPC message types that can be sent between Fish and Agent
#[derive(Debug)]
pub enum Message {
    /// Request git status for a directory
    RepositoryStatus {
        /// The path to the directory to check.
        path: crate::security::SafePath,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        is_first: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
    },

    /// Request language detection for a directory
    LanguageDetect {
        /// The path to the directory to check.
        path: crate::security::SafePath,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        is_first: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
        /// Client-forwarded `$VIRTUAL_ENV` for venv-aware Python version
        /// detection. Absent when no venv is activated (back-compatible).
        virtual_env: Option<String>,
    },

    /// Request directory segment render for a path.
    DirectoryRequest {
        /// The working directory to render, canonicalized and validated.
        path: crate::security::SafePath,
        /// The client's logical `$PWD`, rendered as `cwd` so symlinks match oneshot output. Never on the wire.
        display_path: String,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        is_first: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
    },

    /// Request clock segment render.
    /// Carries the shell so the reply embeds its live-time prompt token instead of a frozen timestamp.
    ClockRequest {
        /// Shell whose live-time prompt token the render should embed.
        shell: crate::shell::Shell,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        is_first: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
    },

    /// Request duration segment render for a given elapsed time.
    DurationRequest {
        /// The command duration in milliseconds.
        duration_ms: u64,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        is_first: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
    },

    /// Request character segment render (the final `❯` prompt symbol, colored by exit status).
    CharacterRequest {
        /// Whether the last command succeeded (`true`) or failed (`false`).
        success: bool,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
    },

    /// Request hostname segment render for a client-supplied hostname value.
    /// The client resolves the hostname and SSH state; the agent does not.
    HostnameRequest {
        /// The hostname string resolved by the client.
        hostname: String,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        is_first: bool,
        /// Whether the client session is over SSH; absent means `false`.
        is_ssh: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
    },

    /// Request username segment render for a client-supplied username value.
    /// The client resolves `$USER`, root state and visibility; the agent does not.
    UsernameRequest {
        /// The effective username string resolved by the client.
        username: String,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        is_first: bool,
        /// Whether the client session runs as root; absent means `false`.
        is_root: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
    },

    /// Request status segment render: the pill showing the previous command's exit status.
    /// Distinct from [`Message::Status`], the agent health check.
    ExitStatusRequest {
        /// Whether the last command succeeded (`true`) or failed (`false`).
        success: bool,
        /// Desired response format.
        format: Format,
        /// Whether this is the last segment in the prompt.
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        is_first: bool,
        /// Previous segment's background color, for chevron transitions.
        prev_bg: Option<String>,
    },

    /// Register Fish process for live updates
    RegisterClient {
        /// The process ID of the client.
        pid: crate::ipc::ClientPid,
        /// Optional current working directory for targeted updates.
        cwd: Option<crate::security::SafePath>,
        /// Optional shell; unrecognized shell names are dropped at the wire boundary
        shell: Option<crate::shell::Shell>,
        /// Optional shell version
        shell_version: Option<String>,
    },

    /// Unregister Fish process
    UnregisterClient {
        /// The process ID of the client.
        pid: crate::ipc::ClientPid,
    },

    /// Update workspace information for a registered client
    WorkspaceUpdate {
        /// The process ID of the client.
        pid: crate::ipc::ClientPid,
        /// Current working directory reported by Fish
        cwd: crate::security::SafePath,
    },

    /// Agent status health check
    Ping,

    /// Get detailed agent status (for debugging)
    Status,

    /// Shutdown agent gracefully
    Shutdown,

    /// Force the agent to reload configuration from disk
    ConfigReload,
}

/// Per-request rendering metadata shared by segment-render messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestMeta {
    /// Desired response format.
    pub format: Format,
    /// Whether this is the last segment in the prompt.
    pub is_last: bool,
    /// Whether this is the first segment in the prompt.
    /// Always `false` for variants without an opening cap.
    pub is_first: bool,
    /// Previous segment's background color, for chevron transitions.
    pub prev_bg: Option<String>,
}

impl Message {
    /// Shared render metadata, or `None` for control messages.
    #[must_use]
    pub fn request_meta(&self) -> Option<RequestMeta> {
        self.positional_request_meta()
            .or_else(|| self.capless_request_meta())
    }

    /// [`RequestMeta`] for the segment variants that carry `is_first`.
    fn positional_request_meta(&self) -> Option<RequestMeta> {
        let (format, is_last, is_first, prev_bg) = match self {
            Self::RepositoryStatus {
                format,
                is_last,
                is_first,
                prev_bg,
                ..
            }
            | Self::LanguageDetect {
                format,
                is_last,
                is_first,
                prev_bg,
                ..
            }
            | Self::DirectoryRequest {
                format,
                is_last,
                is_first,
                prev_bg,
                ..
            }
            | Self::ClockRequest {
                format,
                is_last,
                is_first,
                prev_bg,
                ..
            }
            | Self::DurationRequest {
                format,
                is_last,
                is_first,
                prev_bg,
                ..
            }
            | Self::HostnameRequest {
                format,
                is_last,
                is_first,
                prev_bg,
                ..
            }
            | Self::UsernameRequest {
                format,
                is_last,
                is_first,
                prev_bg,
                ..
            }
            | Self::ExitStatusRequest {
                format,
                is_last,
                is_first,
                prev_bg,
                ..
            } => (*format, *is_last, *is_first, prev_bg),
            _ => return None,
        };

        Some(RequestMeta {
            format,
            is_last,
            is_first,
            prev_bg: prev_bg.clone(),
        })
    }

    /// [`RequestMeta`] for the variant that carries no `is_first` (the prompt character).
    fn capless_request_meta(&self) -> Option<RequestMeta> {
        let Self::CharacterRequest {
            format,
            is_last,
            prev_bg,
            ..
        } = self
        else {
            return None;
        };

        Some(RequestMeta {
            format: *format,
            is_last: *is_last,
            is_first: false,
            prev_bg: prev_bg.clone(),
        })
    }

    /// Path that live-update notifications are scoped to, if the message carries one.
    #[must_use]
    pub fn notify_path(&self) -> Option<String> {
        match self {
            Self::RepositoryStatus { path, .. }
            | Self::LanguageDetect { path, .. }
            | Self::DirectoryRequest { path, .. } => Some(path.to_string()),
            _ => None,
        }
    }
}

/// IPC response types
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    /// Git status information
    RepositoryStatus(crate::git::RepositoryStatus),

    /// Language detection information
    Language {
        /// A list of detected languages.
        languages: Vec<LanguageInfo>,
    },

    /// Directory segment data returned by the agent when format is set.
    Directory {
        /// The working directory the prompt is rendering for.
        cwd: String,
        /// Whether `cwd` is read-only (not writable by the current process).
        read_only: bool,
    },

    /// Duration segment data returned by the agent when format is set.
    Duration {
        /// The command duration in milliseconds.
        duration_ms: u64,
    },

    /// Clock segment data returned by the agent when a clock format is set.
    Clock {
        /// Shell whose live-time prompt token the render embeds.
        shell: crate::shell::Shell,
    },

    /// Character segment data returned by the agent when format is set.
    Character {
        /// Whether the last command succeeded (`true`) or failed (`false`).
        success: bool,
    },

    /// Status segment data returned by the agent when a status format is set.
    ExitStatus {
        /// Whether the last command succeeded (`true`) or failed (`false`).
        success: bool,
    },

    /// Hostname segment data returned by the agent when format is set.
    Hostname {
        /// The hostname string, as supplied by the requesting client.
        hostname: String,
        /// Whether the requesting client's session is over SSH.
        #[serde(default)]
        is_ssh: bool,
    },

    /// Username segment data returned by the agent when format is set.
    Username {
        /// The username string, as supplied by the requesting client.
        username: String,
        /// Whether the requesting client's session runs as root.
        #[serde(default)]
        is_root: bool,
    },

    /// Simple acknowledgment
    Ack,

    /// Agent status information (for debugging)
    AgentStatus {
        /// Agent version (e.g., "0.1.0")
        version: String,
        /// Protocol version (e.g., 1)
        protocol_version: u8,
        /// Number of watched repositories
        watched_repos: usize,
        /// Number of registered clients
        registered_clients: usize,
        /// Number of cache entries
        cache_entries: usize,
    },

    /// Error response
    Error {
        /// The error message.
        message: String,
    },
}

/// Individual language information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageInfo {
    /// Language name (e.g., `Rust`, `JavaScript`)
    pub name: String,
    /// Version string (e.g., "1.70.0")
    pub version: Option<String>,
    /// Display color for the language (hex format)
    pub color: crate::config::types::ColorSpec,
}
