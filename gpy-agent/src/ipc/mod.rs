//! JSON IPC between shell clients and the agent over Unix domain sockets.

mod client_pid;
#[cfg(test)]
pub(crate) mod fixtures;
pub mod handlers;
pub mod protocol;
pub mod registry;
pub mod server;
pub(crate) mod single_flight;
#[cfg(unix)]
pub(crate) mod socket_identity;
pub mod wire;

pub use crate::formatter::Format;

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

/// Rendering position and format shared by every segment request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Segment {
    /// Desired response format.
    pub format: Format,
    /// Whether this is the last segment in the prompt.
    pub is_last: bool,
    /// Whether this is the first segment in the prompt (opening-cap suppression).
    pub is_first: bool,
    /// Previous segment's background color, for chevron transitions.
    pub prev_bg: Option<String>,
}

/// Requests a shell client sends to the agent.
#[derive(Debug)]
pub enum Message {
    /// Request git status for a directory
    RepositoryStatus {
        /// The path to the directory to check.
        path: crate::security::SafePath,
        /// Render position and format.
        segment: Segment,
    },

    /// Request language detection for a directory
    LanguageDetect {
        /// The path to the directory to check.
        path: crate::security::SafePath,
        /// Render position and format.
        segment: Segment,
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
        /// Render position and format.
        segment: Segment,
    },

    /// Request clock segment render.
    /// Carries the shell so the reply embeds its live-time prompt token instead of a frozen timestamp.
    ClockRequest {
        /// Shell whose live-time prompt token the render should embed.
        shell: crate::shell::Shell,
        /// Render position and format.
        segment: Segment,
    },

    /// Request duration segment render for a given elapsed time.
    DurationRequest {
        /// The command duration in milliseconds.
        duration_ms: u64,
        /// Render position and format.
        segment: Segment,
    },

    /// Request character segment render (the final `❯` prompt symbol, colored by exit status).
    CharacterRequest {
        /// Whether the last command succeeded (`true`) or failed (`false`).
        success: bool,
        /// Render position and format; the prompt symbol never draws an opening cap, so `is_first` has no effect.
        segment: Segment,
    },

    /// Request status segment render: the pill showing the previous command's exit status.
    /// Distinct from [`Message::Status`], the agent health check.
    ExitStatusRequest {
        /// Whether the last command succeeded (`true`) or failed (`false`).
        success: bool,
        /// Render position and format.
        segment: Segment,
    },

    /// Register a shell process for live updates
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

    /// Unregister a shell process
    UnregisterClient {
        /// The process ID of the client.
        pid: crate::ipc::ClientPid,
    },

    /// Update workspace information for a registered client
    WorkspaceUpdate {
        /// The process ID of the client.
        pid: crate::ipc::ClientPid,
        /// Current working directory reported by the client
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

impl Message {
    /// Render position and format, or `None` for control messages.
    #[must_use]
    pub const fn segment(&self) -> Option<&Segment> {
        match self {
            Self::RepositoryStatus { segment, .. }
            | Self::LanguageDetect { segment, .. }
            | Self::DirectoryRequest { segment, .. }
            | Self::ClockRequest { segment, .. }
            | Self::DurationRequest { segment, .. }
            | Self::CharacterRequest { segment, .. }
            | Self::ExitStatusRequest { segment, .. } => Some(segment),
            Self::RegisterClient { .. }
            | Self::UnregisterClient { .. }
            | Self::WorkspaceUpdate { .. }
            | Self::Ping
            | Self::Status
            | Self::Shutdown
            | Self::ConfigReload => None,
        }
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

/// Replies the agent sends; [`wire::encode_json`] is their JSON form.
#[derive(Debug, PartialEq)]
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

    /// Simple acknowledgment
    Ack,

    /// Agent status information (for debugging)
    AgentStatus(AgentStatus),

    /// Error response
    Error {
        /// The error message.
        message: String,
    },
}

/// Status payload of the `status` reply, shared by the agent and the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatus {
    /// Agent version (e.g., "0.1.0")
    pub version: String,
    /// The answering agent's [`protocol::PROTOCOL_VERSION`].
    pub protocol_version: u8,
    /// Number of watched repositories
    pub watched_repos: usize,
    /// Number of registered clients
    pub registered_clients: usize,
    /// Number of cache entries
    pub cache_entries: usize,
    /// Process ID of the answering agent; absent from older agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// Individual language information
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageInfo {
    /// Language name (e.g., `Rust`, `JavaScript`)
    pub name: String,
    /// Version string (e.g., "1.70.0")
    pub version: Option<String>,
    /// Display color for the language (hex format)
    pub color: crate::config::types::ColorSpec,
}
