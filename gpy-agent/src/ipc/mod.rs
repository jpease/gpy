//! Inter-Process Communication (IPC) subsystem
//!
//! Provides JSON-based communication over Unix domain sockets
//! for Fish shell ↔ Agent communication on Linux, macOS, and WSL.

pub mod client;
pub mod handlers;
pub mod latency;
pub mod protocol;
pub mod registry;
pub mod server;
pub mod transport;

pub use crate::formatter::Format;

/// Exactly what the daemon puts on the wire for a [`Response::Ack`].
///
/// Every socket reply except `AgentStatus` is rendered by the requested
/// formatter (JSON by default), and the JSON formatter writes `Ack` as
/// `{"status":"ok"}` -- the shape the Fish, Bash and Zsh clients match on.
/// It is *not* serde's external tagging of the unit variant (`"Ack"`), which
/// is what the CLI expected after #573 and why a live agent's ping, stop and
/// config-reload acknowledgements went unrecognised. `decode_wire_reply`
/// accepts both; test fixtures that model a live agent must send this one.
pub const ACK_WIRE_JSON: &str = r#"{"status":"ok"}"#;

/// Decode a socket reply into a [`Response`].
///
/// Accepts the native serde encoding (what `AgentStatus` uses, and what a
/// [`Response`] round-trips to) and the JSON formatter's rendering of the
/// two variants a control command can receive: [`ACK_WIRE_JSON`] for
/// [`Response::Ack`] and `{"error":"..."}` for [`Response::Error`]. Anything
/// else is an error, never a silent "unacknowledged".
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
pub use latency::LatencyTracker;
pub use registry::ClientDirectory;

use serde::{Deserialize, Serialize};

/// Standard error for IPC operations that require the Unix-socket transport.
///
/// GPY's IPC transport is Unix-domain-socket based (see module docs); native
/// Windows has no implementation yet (#284). Windows users are expected to run
/// under WSL, which presents as a Unix environment and uses the real transport.
#[cfg(not(unix))]
pub(crate) fn native_windows_unsupported() -> crate::Error {
    crate::Error::ipc(
        "GPY's IPC transport is Unix-socket based and not yet implemented for native \
         Windows; run under WSL instead."
            .to_owned(),
    )
}

/// IPC message types that can be sent between Fish and Agent
///
/// # JSON Protocol Examples
///
/// ## Fish Shell Format (Legacy)
///
/// Fish shell sends messages using an object format with an `op` field:
///
/// ```json
/// {"op":"git","cwd":"/home/user/repo","format":"fish-ansi"}
/// ```
///
/// ## Native Rust Format
///
/// The native format uses Rust enum serialization:
///
/// ```json
/// {"RepositoryStatus":{"path":"/home/user/repo","format":"json"}}
/// ```
///
/// Both formats are supported for backward compatibility.
///
/// **Note**: Format values use kebab-case: `"json"`, `"fish-ansi"`, `"fish-source"`
#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    /// Request git status for a directory
    ///
    /// # Examples
    ///
    /// Fish format (minimal):
    /// ```json
    /// {"op":"git","cwd":"."}
    /// ```
    ///
    /// Fish format (full):
    /// ```json
    /// {"op":"git","cwd":"/home/user/repo","format":"fish-ansi","is_last":false}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// {"RepositoryStatus":{"path":"/home/user/repo","format":"json","is_last":false}}
    /// ```
    RepositoryStatus {
        /// The path to the directory to check.
        path: crate::security::SafePath,
        /// Desired response format (defaults to JSON).
        #[serde(default)]
        #[serde(skip_serializing_if = "Format::is_json_ref")]
        format: Format,
        /// Whether this is the last segment in the prompt (for delimiter selection).
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_last: bool,
        /// Whether this is the first segment in the prompt (for opening-cap suppression).
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_first: bool,
        /// Previous segment's background color for powerline chevron transitions.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        prev_bg: Option<String>,
    },

    /// Request language detection for a directory
    ///
    /// # Examples
    ///
    /// Fish format (minimal):
    /// ```json
    /// {"op":"lang","cwd":"."}
    /// ```
    ///
    /// Fish format (full):
    /// ```json
    /// {"op":"lang","cwd":"/home/user/project","format":"fish-source","is_last":true}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// {"LanguageDetect":{"path":"/home/user/project","format":"fish-source","is_last":true}}
    /// ```
    LanguageDetect {
        /// The path to the directory to check.
        path: crate::security::SafePath,
        /// Desired response format (defaults to JSON).
        #[serde(default)]
        #[serde(skip_serializing_if = "Format::is_json_ref")]
        format: Format,
        /// Whether this is the last segment in the prompt (for delimiter selection).
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_last: bool,
        /// Whether this is the first segment in the prompt (for opening-cap suppression).
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_first: bool,
        /// Previous segment's background color for powerline chevron transitions.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        prev_bg: Option<String>,
        /// Client-forwarded `$VIRTUAL_ENV` for venv-aware Python version
        /// detection. Absent when no venv is activated (back-compatible).
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        virtual_env: Option<String>,
    },

    /// Request directory segment render for a path.
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"directory","cwd":"/home/user","format":"ansi","is_last":false}
    /// ```
    DirectoryRequest {
        /// The working directory to render.
        path: crate::security::SafePath,
        /// Desired response format.
        #[serde(default)]
        #[serde(skip_serializing_if = "Format::is_json_ref")]
        format: Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_last: bool,
        /// Whether this is the first segment in the prompt (for opening-cap suppression).
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_first: bool,
        /// Previous segment's background color for powerline chevron transitions.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        prev_bg: Option<String>,
    },

    /// Request clock segment render.
    ///
    /// Carries the requesting shell rather than a timestamp: the response
    /// embeds that shell's own live-time prompt token (`%D{…}` for Zsh,
    /// `\D{…}` for Bash) so the clock keeps ticking between prompt draws
    /// without an IPC round-trip per second. See
    /// [`crate::formatter::clock_resolver`].
    ///
    /// # Examples
    ///
    /// ```json
    /// {"op":"clock","shell":"zsh","format":"zsh-prompt","is_last":false}
    /// ```
    ClockRequest {
        /// Shell whose live-time prompt token the render should embed.
        shell: crate::shell::Shell,
        /// Desired response format.
        #[serde(default)]
        #[serde(skip_serializing_if = "Format::is_json_ref")]
        format: Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_last: bool,
        /// Whether this is the first segment in the prompt (for opening-cap suppression).
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_first: bool,
        /// Previous segment's background color for powerline chevron transitions.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        prev_bg: Option<String>,
    },

    /// Request duration segment render for a given elapsed time.
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"duration","duration_ms":65000,"format":"ansi","is_last":false}
    /// ```
    DurationRequest {
        /// The command duration in milliseconds.
        duration_ms: u64,
        /// Desired response format.
        #[serde(default)]
        #[serde(skip_serializing_if = "Format::is_json_ref")]
        format: Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_last: bool,
        /// Whether this is the first segment in the prompt (for opening-cap suppression).
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_first: bool,
        /// Previous segment's background color for powerline chevron transitions.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        prev_bg: Option<String>,
    },

    /// Request character segment render (the final `❯` prompt symbol, colored by exit status).
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"character","success":true,"format":"ansi"}
    /// ```
    CharacterRequest {
        /// Whether the last command succeeded (`true`) or failed (`false`).
        success: bool,
        /// Desired response format.
        #[serde(default)]
        #[serde(skip_serializing_if = "Format::is_json_ref")]
        format: Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_last: bool,
        /// Previous segment's background color for powerline chevron transitions.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        prev_bg: Option<String>,
    },

    /// Request hostname segment render for a client-supplied hostname value.
    ///
    /// Per #259, the client resolves and passes the hostname string (and gates
    /// SSH-only visibility) before ever sending this request; the agent does
    /// not derive the hostname itself and there is no `is_ssh` field.
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"hostname","hostname":"my-host","format":"ansi"}
    /// ```
    HostnameRequest {
        /// The hostname string resolved by the client.
        hostname: String,
        /// Desired response format.
        #[serde(default)]
        #[serde(skip_serializing_if = "Format::is_json_ref")]
        format: Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_last: bool,
        /// Previous segment's background color for powerline chevron transitions.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        prev_bg: Option<String>,
    },

    /// Request username segment render for a client-supplied username value.
    ///
    /// Mirrors [`Message::HostnameRequest`]: the client resolves the effective
    /// username (`$USER`, which is `root` under root/sudo) and gates root/sudo
    /// visibility via `segment_username_detect` before ever sending this request.
    /// The agent does not derive the username itself — it is detached from any
    /// one terminal's environment.
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"username","username":"root","format":"ansi"}
    /// ```
    UsernameRequest {
        /// The effective username string resolved by the client.
        username: String,
        /// Desired response format.
        #[serde(default)]
        #[serde(skip_serializing_if = "Format::is_json_ref")]
        format: Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        #[serde(skip_serializing_if = "is_false")]
        is_last: bool,
        /// Previous segment's background color for powerline chevron transitions.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        prev_bg: Option<String>,
    },

    /// Register Fish process for live updates
    ///
    /// # Examples
    ///
    /// Fish format (with cwd):
    /// ```json
    /// {"op":"register","pid":12345,"cwd":"/home/user/project"}
    /// ```
    ///
    /// Fish format (without cwd):
    /// ```json
    /// {"op":"register","pid":12345}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// {"RegisterClient":{"pid":12345,"cwd":"/home/user/project"}}
    /// ```
    RegisterClient {
        /// The process ID of the client.
        pid: crate::config::types::ClientPid,
        /// Optional current working directory for targeted updates.
        cwd: Option<crate::security::SafePath>,
        /// Optional shell name (e.g., "fish", "zsh")
        shell: Option<crate::config::types::ShellVariant>,
        /// Optional shell version
        shell_version: Option<String>,
    },

    /// Unregister Fish process
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"unregister","pid":12345}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// {"UnregisterClient":{"pid":12345}}
    /// ```
    UnregisterClient {
        /// The process ID of the client.
        pid: crate::config::types::ClientPid,
    },

    /// Update workspace information for a registered client
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"workspace","pid":12345,"cwd":"/home/user/new-project"}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// {"WorkspaceUpdate":{"pid":12345,"cwd":"/home/user/new-project"}}
    /// ```
    WorkspaceUpdate {
        /// The process ID of the client.
        pid: crate::config::types::ClientPid,
        /// Current working directory reported by Fish
        cwd: crate::security::SafePath,
    },

    /// Agent status health check
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"ping"}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// "Ping"
    /// ```
    Ping,

    /// Get detailed agent status (for debugging)
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"status"}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// "Status"
    /// ```
    Status,

    /// Query theme setting
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"theme","key":"prompt_close"}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// {"ThemeQuery":{"key":"prompt_close"}}
    /// ```
    ThemeQuery {
        /// The theme setting key to query (e.g., "`prompt_close`", "`prompt_open`", "prompt")
        key: String,
    },

    /// Get IPC latency statistics
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"latency_stats"}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// "LatencyStats"
    /// ```
    LatencyStats,

    /// Shutdown agent gracefully
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"shutdown"}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// "Shutdown"
    /// ```
    Shutdown,

    /// Force the agent to reload configuration from disk
    ///
    /// # Examples
    ///
    /// Fish format:
    /// ```json
    /// {"op":"config_reload"}
    /// ```
    ///
    /// Native format:
    /// ```json
    /// "ConfigReload"
    /// ```
    ConfigReload,
}

/// Shared per-request rendering metadata carried by every segment-render
/// message variant.
///
/// Includes the desired output format, whether this is the last prompt
/// segment, whether this is the first, and the previous segment's background
/// color for powerline chevron transitions.
///
/// Control messages (`Ping`, `RegisterClient`, `ThemeQuery`, etc.) have no
/// rendering concerns and so have no `RequestMeta`; see [`Message::request_meta`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestMeta {
    /// Desired response format.
    pub format: Format,
    /// Whether this is the last segment in the prompt.
    pub is_last: bool,
    /// Whether this is the first segment in the prompt.
    ///
    /// Only [`Message::RepositoryStatus`], [`Message::LanguageDetect`],
    /// [`Message::DirectoryRequest`], and [`Message::DurationRequest`] carry a
    /// real `is_first` field (their `format` templates support `$sep_open`);
    /// the other segment-render variants have no opening-cap concept and
    /// always report `false` here.
    pub is_first: bool,
    /// Previous segment's background color for powerline chevron transitions.
    pub prev_bg: Option<String>,
}

impl Message {
    /// Returns the shared `format`/`is_last`/`is_first`/`prev_bg` metadata for
    /// message variants that carry it, or `None` for control messages that don't.
    ///
    /// Collapses what used to be several byte-for-byte-identical match arms
    /// in the IPC server's request router into a single accessor: adding a
    /// new segment-render message only requires adding it to the pattern
    /// below, rather than duplicating a whole routing arm.
    ///
    /// Destructures to a tuple and builds the [`RequestMeta`] once at the end.
    /// The two variant groups differ only in whether they carry `is_first`, and
    /// repeating the struct literal per group pushes this past the pedantic
    /// 60-line limit.
    #[must_use]
    pub fn request_meta(&self) -> Option<RequestMeta> {
        self.positional_request_meta()
            .or_else(|| self.capless_request_meta())
    }

    /// [`RequestMeta`] for the segment variants that carry `is_first`.
    ///
    /// Split from its `capless` sibling purely to keep each match under the
    /// pedantic 60-line limit; together they cover every render request.
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

    /// [`RequestMeta`] for the variants that carry no `is_first`.
    ///
    /// These can never open the segment chain, so the opening-cap suppression
    /// `is_first` drives does not apply and it is reported as `false`.
    fn capless_request_meta(&self) -> Option<RequestMeta> {
        let (format, is_last, prev_bg) = match self {
            Self::CharacterRequest {
                format,
                is_last,
                prev_bg,
                ..
            }
            | Self::HostnameRequest {
                format,
                is_last,
                prev_bg,
                ..
            }
            | Self::UsernameRequest {
                format,
                is_last,
                prev_bg,
                ..
            } => (*format, *is_last, prev_bg),
            _ => return None,
        };

        Some(RequestMeta {
            format,
            is_last,
            is_first: false,
            prev_bg: prev_bg.clone(),
        })
    }

    /// Returns the path that live-update notifications should be scoped to
    /// for this message, or `None` for messages that don't carry a path.
    ///
    /// Only `RepositoryStatus`, `LanguageDetect`, and `DirectoryRequest`
    /// resolve a filesystem path; all other variants return `None`.
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

/// Helper function for serde `skip_serializing_if` to skip false values.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if requires a fn(&T) -> bool signature, not fn(T) -> bool"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

/// IPC response types
///
/// All responses use the native Rust enum serialization format.
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    /// Git status information
    ///
    /// # Example
    ///
    /// Success response with changes:
    /// ```json
    /// {
    ///   "RepositoryStatus": {
    ///     "branch": "main",
    ///     "ahead": 2,
    ///     "behind": 0,
    ///     "staged": 3,
    ///     "unstaged": 1,
    ///     "untracked": 2,
    ///     "conflicts": 0,
    ///     "state": "clean"
    ///   }
    /// }
    /// ```
    RepositoryStatus(crate::git::RepositoryStatus),

    /// Language detection information
    ///
    /// # Examples
    ///
    /// Single language:
    /// ```json
    /// {
    ///   "Language": {
    ///     "languages": [
    ///       {
    ///         "name": "Rust",
    ///         "version": "1.70.0",
    ///         "color": "#dea584"
    ///       }
    ///     ]
    ///   }
    /// }
    /// ```
    ///
    /// Multiple languages:
    /// ```json
    /// {
    ///   "Language": {
    ///     "languages": [
    ///       {
    ///         "name": "JavaScript",
    ///         "version": "20.10.0",
    ///         "color": "#f1e05a"
    ///       },
    ///       {
    ///         "name": "TypeScript",
    ///         "version": "5.3.3",
    ///         "color": "#3178c6"
    ///       }
    ///     ]
    ///   }
    /// }
    /// ```
    ///
    /// No language detected:
    /// ```json
    /// {
    ///   "Language": {
    ///     "languages": []
    ///   }
    /// }
    /// ```
    Language {
        /// A list of detected languages.
        languages: Vec<LanguageInfo>,
    },

    /// Directory segment data returned by the agent when format is set.
    ///
    /// # Example
    ///
    /// ```json
    /// {"Directory":{"cwd":"/home/user/project","read_only":false}}
    /// ```
    Directory {
        /// The working directory the prompt is rendering for.
        cwd: String,
        /// Whether `cwd` is read-only (not writable by the current process).
        read_only: bool,
    },

    /// Duration segment data returned by the agent when format is set.
    ///
    /// # Example
    ///
    /// ```json
    /// {"Duration":{"duration_ms":65000}}
    /// ```
    Duration {
        /// The command duration in milliseconds.
        duration_ms: u64,
    },

    /// Clock segment data returned by the agent when a clock format is set.
    ///
    /// Carries the requesting shell so the formatter can embed that shell's
    /// live-time prompt token rather than a timestamp frozen at render time.
    ///
    /// # Example
    ///
    /// ```json
    /// {"Clock":{"shell":"zsh"}}
    /// ```
    Clock {
        /// Shell whose live-time prompt token the render embeds.
        shell: crate::shell::Shell,
    },

    /// Character segment data returned by the agent when format is set.
    ///
    /// # Example
    ///
    /// ```json
    /// {"Character":{"success":true}}
    /// ```
    Character {
        /// Whether the last command succeeded (`true`) or failed (`false`).
        success: bool,
    },

    /// Hostname segment data returned by the agent when format is set.
    ///
    /// The agent echoes back the hostname the client supplied on the request;
    /// it does not resolve or validate hostname semantics itself (#259).
    ///
    /// # Example
    ///
    /// ```json
    /// {"Hostname":{"hostname":"my-host"}}
    /// ```
    Hostname {
        /// The hostname string, as supplied by the requesting client.
        hostname: String,
    },

    /// Username segment data returned by the agent when format is set.
    ///
    /// The agent echoes back the username the client supplied on the request;
    /// it does not resolve or validate username semantics itself.
    ///
    /// # Example
    ///
    /// ```json
    /// {"Username":{"username":"root"}}
    /// ```
    Username {
        /// The username string, as supplied by the requesting client.
        username: String,
    },

    /// Simple acknowledgment
    ///
    /// # Example
    ///
    /// ```json
    /// "Ack"
    /// ```
    Ack,

    /// Agent status information (for debugging)
    ///
    /// # Example
    ///
    /// ```json
    /// {
    ///   "AgentStatus": {
    ///     "version": "0.1.0",
    ///     "watched_repos": 5,
    ///     "registered_clients": 3,
    ///     "cache_entries": 12
    ///   }
    /// }
    /// ```
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

    /// Theme setting value
    ///
    /// # Examples
    ///
    /// Prompt delimiter:
    /// ```json
    /// {
    ///   "ThemeValue": {
    ///     "value": ""
    ///   }
    /// }
    /// ```
    ///
    /// Color code:
    /// ```json
    /// {
    ///   "ThemeValue": {
    ///     "value": "#ff6b6b"
    ///   }
    /// }
    /// ```
    ThemeValue {
        /// The theme setting value
        value: String,
    },

    /// IPC latency statistics
    ///
    /// # Example
    ///
    /// ```json
    /// {
    ///   "LatencyStatsResult": {
    ///     "min_ms": 1,
    ///     "max_ms": 15,
    ///     "avg_ms": 3,
    ///     "sample_count": 42
    ///   }
    /// }
    /// ```
    LatencyStatsResult {
        /// Minimum response time in milliseconds
        min_ms: u64,
        /// Maximum response time in milliseconds
        max_ms: u64,
        /// Average response time in milliseconds
        avg_ms: u64,
        /// Number of samples in statistics (up to 100)
        sample_count: usize,
    },

    /// Error response
    ///
    /// # Examples
    ///
    /// Generic error:
    /// ```json
    /// {
    ///   "Error": {
    ///     "message": "Failed to detect language: directory not found"
    ///   }
    /// }
    /// ```
    ///
    /// IPC error:
    /// ```json
    /// {
    ///   "Error": {
    ///     "message": "IPC connection failed: socket timeout"
    ///   }
    /// }
    /// ```
    Error {
        /// The error message.
        message: String,
    },
}

/// Individual language information
///
/// # Example
///
/// With version:
/// ```json
/// {
///   "name": "Python",
///   "version": "3.11.4",
///   "color": "#3572A5"
/// }
/// ```
///
/// Without version:
/// ```json
/// {
///   "name": "Go",
///   "version": null,
///   "color": "#00ADD8"
/// }
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct LanguageInfo {
    /// Language name (e.g., `Rust`, `JavaScript`)
    pub name: String,
    /// Version string (e.g., "1.70.0")
    pub version: Option<String>,
    /// Display color for the language (hex format)
    pub color: crate::config::types::ColorSpec,
}
