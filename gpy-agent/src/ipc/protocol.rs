//! JSON-based IPC protocol definitions
//!
//! Defines the message schemas and serialization for communication
//! between Fish shell and the agent process.

use super::{Message, Response};
use crate::{Error, Result};
use serde::Deserialize;
use serde_json;

/// Wire payload for a client-initiated config reload (newline-delimited JSON).
///
/// Shared by every client that triggers an instant reload over IPC
/// (`commands::utils::reload_agent`, `agent::lifecycle::send_config_reload_command`)
/// so the bytes on the wire can never diverge between send sites.
pub const CONFIG_RELOAD_JSON: &str = r#"{"op":"config_reload"}"#;

/// Serialize a message to JSON bytes for transmission (optimized)
///
/// # Errors
///
/// Returns an error if serialization fails or if the message exceeds size limits.
pub fn serialize_message(msg: &Message) -> Result<Vec<u8>> {
    // Pre-allocate with a reasonable size to avoid reallocations
    let mut buffer = Vec::with_capacity(256);

    // Serialize directly to the buffer
    serde_json::to_writer(&mut buffer, msg)
        .map_err(|e| Error::ipc(format!("Failed to serialize message: {e}")))?;

    validate_message_size(&buffer)?;
    Ok(buffer)
}

/// Deserialize JSON bytes into a message
///
/// # Errors
///
/// Returns an error if deserialization fails or if message validation fails.
pub fn deserialize_message(data: &[u8]) -> Result<Message> {
    let wire = parse_wire(data)?;
    let msg = resolve(wire)?;
    validate_message_content(&msg)?;
    Ok(msg)
}

/// Parse wire bytes into a [`WireMessage`]: size and UTF-8 checks, JSON-shape
/// validation, and the shell-format-vs-native-format dispatch, with zero I/O.
///
/// Every field that eventually needs path canonicalization or PID range
/// validation stays a raw `String`/`u32` here — see [`resolve`].
///
/// # Errors
///
/// Returns an error if the bytes are not valid UTF-8, exceed the message size
/// limit, or match neither the shell IPC format nor the native format.
fn parse_wire(data: &[u8]) -> Result<WireMessage> {
    validate_message_size(data)?;
    let json = std::str::from_utf8(data)
        .map_err(|e| Error::ipc(format!("Invalid UTF-8 in message: {e}")))?;

    // First try the shell IPC format: shell traffic (Fish/Bash/Zsh prompt segments)
    // dominates the hot path, so parsing it first avoids a wasted failed parse
    // attempt against the native enum on every request.
    if let Ok(shell_msg) = serde_json::from_str::<ShellIpcMessage>(json) {
        return shell_msg.into_wire_message();
    }

    // If that fails, try the native Rust enum format. This is the rare fallback,
    // used only by non-shell clients that speak the internal `Message` wire format.
    if let Ok(wire) = serde_json::from_str::<WireMessage>(json) {
        return Ok(wire);
    }

    Err(Error::ipc(
        "Failed to deserialize message: invalid format".to_owned(),
    ))
}

/// Resolve a [`WireMessage`] into a [`Message`].
///
/// This is where the parse pipeline's one impure step lives:
/// [`crate::security::SafePath::new`], which canonicalizes and so touches the
/// filesystem. [`crate::config::types::ClientPid`] is constructed here too.
///
/// `ClientPid::new` is a pure range check since #578, not a `kill(pid, 0)`
/// probe; it lives here anyway so that `WireMessage`'s field types stay
/// uniformly raw wire values and this function is the single place `Message`'s
/// stronger types get constructed.
///
/// # Errors
///
/// Returns an error if a path fails validation or a PID is out of range.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per Message variant: the single mapping from raw wire values to Message's validated types, which is only legible read as a whole"
)]
fn resolve(wire: WireMessage) -> Result<Message> {
    match wire {
        WireMessage::RepositoryStatus {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
        } => Ok(Message::RepositoryStatus {
            path: crate::security::SafePath::new(&path)
                .map_err(|e| Error::ipc(format!("Invalid path: {e}")))?,
            format,
            is_last,
            is_first,
            prev_bg,
        }),
        WireMessage::LanguageDetect {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
            virtual_env,
        } => Ok(Message::LanguageDetect {
            path: crate::security::SafePath::new(&path)
                .map_err(|e| Error::ipc(format!("Invalid path: {e}")))?,
            format,
            is_last,
            is_first,
            prev_bg,
            virtual_env,
        }),
        WireMessage::DirectoryRequest {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
        } => Ok(Message::DirectoryRequest {
            path: crate::security::SafePath::new(&path)
                .map_err(|e| Error::ipc(format!("Invalid path: {e}")))?,
            format,
            is_last,
            is_first,
            prev_bg,
        }),
        WireMessage::ClockRequest {
            shell,
            format,
            is_last,
            is_first,
            prev_bg,
        } => Ok(Message::ClockRequest {
            shell,
            format,
            is_last,
            is_first,
            prev_bg,
        }),
        WireMessage::DurationRequest {
            duration_ms,
            format,
            is_last,
            is_first,
            prev_bg,
        } => Ok(Message::DurationRequest {
            duration_ms,
            format,
            is_last,
            is_first,
            prev_bg,
        }),
        WireMessage::CharacterRequest {
            success,
            format,
            is_last,
            prev_bg,
        } => Ok(Message::CharacterRequest {
            success,
            format,
            is_last,
            prev_bg,
        }),
        WireMessage::HostnameRequest {
            hostname,
            format,
            is_last,
            prev_bg,
        } => Ok(Message::HostnameRequest {
            hostname,
            format,
            is_last,
            prev_bg,
        }),
        WireMessage::UsernameRequest {
            username,
            format,
            is_last,
            prev_bg,
        } => Ok(Message::UsernameRequest {
            username,
            format,
            is_last,
            prev_bg,
        }),
        WireMessage::RegisterClient {
            pid,
            cwd,
            shell,
            shell_version,
        } => Ok(Message::RegisterClient {
            // `cwd` is written before `pid` deliberately: struct fields are
            // evaluated in written order, and the pre-split `into_message`
            // validated the cwd before the PID, so an input that fails both
            // still reports "Invalid cwd" rather than "Invalid PID".
            cwd: cwd
                .map(|path| {
                    crate::security::SafePath::new(&path)
                        .map_err(|e| Error::ipc(format!("Invalid cwd: {e}")))
                })
                .transpose()?,
            pid: crate::config::types::ClientPid::new(pid)
                .map_err(|e| Error::ipc(format!("Invalid PID: {e}")))?,
            shell,
            shell_version,
        }),
        WireMessage::UnregisterClient { pid } => Ok(Message::UnregisterClient {
            pid: crate::config::types::ClientPid::new(pid)
                .map_err(|e| Error::ipc(format!("Invalid PID: {e}")))?,
        }),
        WireMessage::WorkspaceUpdate { pid, cwd } => Ok(Message::WorkspaceUpdate {
            pid: crate::config::types::ClientPid::new(pid)
                .map_err(|e| Error::ipc(format!("Invalid PID: {e}")))?,
            cwd: crate::security::SafePath::new(&cwd)
                .map_err(|e| Error::ipc(format!("Invalid workspace path: {e}")))?,
        }),
        WireMessage::ThemeQuery { key } => Ok(Message::ThemeQuery { key }),
        WireMessage::Ping => Ok(Message::Ping),
        WireMessage::Status => Ok(Message::Status),
        WireMessage::Shutdown => Ok(Message::Shutdown),
        WireMessage::ConfigReload => Ok(Message::ConfigReload),
        WireMessage::LatencyStats => Ok(Message::LatencyStats),
    }
}

/// Serialize a response to JSON bytes (optimized for allocation reduction)
///
/// # Errors
///
/// Returns an error if serialization fails.
pub fn serialize_response(resp: &Response) -> Result<Vec<u8>> {
    // Pre-allocate with a reasonable size to avoid reallocations
    let mut buffer = Vec::with_capacity(512);

    // Serialize directly to the buffer
    serde_json::to_writer(&mut buffer, resp)
        .map_err(|e| Error::ipc(format!("Failed to serialize response: {e}")))?;

    validate_message_size(&buffer)?;
    Ok(buffer)
}

/// Deserialize JSON bytes into a response
///
/// # Errors
///
/// Returns an error if the data is not valid UTF-8, exceeds size limits,
/// or cannot be deserialized into a valid response.
pub fn deserialize_response(data: &[u8]) -> Result<Response> {
    validate_message_size(data)?;
    let json = std::str::from_utf8(data)
        .map_err(|e| Error::ipc(format!("Invalid UTF-8 in response: {e}")))?;
    serde_json::from_str(json)
        .map_err(|e| Error::ipc(format!("Failed to deserialize response: {e}")))
}

/// Validate message size limits (64KB maximum per IPC design)
///
/// # Errors
///
/// Returns an error if the message exceeds the maximum allowed size.
pub fn validate_message_size(data: &[u8]) -> Result<()> {
    // One definition of the cap, shared with `GuardSettings`'s default (#578);
    // this used to be a separate local constant that could drift from it.
    if data.len() > crate::security::MAX_MESSAGE_SIZE {
        return Err(Error::ipc(format!(
            "Message too large: {} bytes (max {})",
            data.len(),
            crate::security::MAX_MESSAGE_SIZE
        )));
    }

    Ok(())
}

/// Validate message content according to schema rules
///
/// # Errors
///
/// Returns an error if the message contains invalid data or violates schema rules.
pub fn validate_message_content(msg: &Message) -> Result<()> {
    match msg {
        Message::RepositoryStatus { prev_bg, .. }
        | Message::LanguageDetect { prev_bg, .. }
        | Message::DirectoryRequest { prev_bg, .. }
        | Message::ClockRequest { prev_bg, .. }
        | Message::DurationRequest { prev_bg, .. }
        | Message::CharacterRequest { prev_bg, .. } => {
            if let Some(bg) = prev_bg {
                validate_string_field("prev_bg", bg, 64)?;
            }
        }
        Message::HostnameRequest {
            prev_bg, hostname, ..
        } => validate_hostname_request(prev_bg.as_deref(), hostname)?,
        Message::UsernameRequest {
            prev_bg, username, ..
        } => validate_username_request(prev_bg.as_deref(), username)?,
        Message::RegisterClient { shell_version, .. } => {
            validate_register_client(shell_version.as_deref())?;
        }
        Message::ThemeQuery { key } => {
            validate_string_field("key", key, 256)?;
        }
        // No additional validation needed. `UnregisterClient` and
        // `WorkspaceUpdate` carry only a `ClientPid`, which cannot be
        // constructed out of range, so re-checking the range of an
        // already-constructed one proves nothing (#578); liveness is probed
        // once, at route time.
        Message::UnregisterClient { .. }
        | Message::WorkspaceUpdate { .. }
        | Message::Ping
        | Message::Shutdown
        | Message::Status
        | Message::ConfigReload
        | Message::LatencyStats => {}
    }
    Ok(())
}

/// Validate a `HostnameRequest`'s `prev_bg` and hostname fields.
///
/// # Errors
///
/// Returns an error if `prev_bg` or `hostname` fail string validation.
fn validate_hostname_request(prev_bg: Option<&str>, hostname: &str) -> Result<()> {
    if let Some(bg) = prev_bg {
        validate_string_field("prev_bg", bg, 64)?;
    }
    validate_string_field("hostname", hostname, 256)?;
    Ok(())
}

/// Validate a `UsernameRequest`'s `prev_bg` and username fields.
///
/// # Errors
///
/// Returns an error if `prev_bg` or `username` fail string validation.
fn validate_username_request(prev_bg: Option<&str>, username: &str) -> Result<()> {
    if let Some(bg) = prev_bg {
        validate_string_field("prev_bg", bg, 64)?;
    }
    validate_string_field("username", username, 256)?;
    Ok(())
}

/// Validate a `RegisterClient` message's optional shell version.
///
/// The PID needs no check here: it arrives as a `ClientPid`, whose constructor
/// already guarantees the range invariant, and its liveness is probed once at
/// route time (#578). This function used to re-check the range of a value that
/// could not hold an out-of-range PID in the first place.
///
/// # Errors
///
/// Returns an error if `shell_version` fails string validation.
fn validate_register_client(shell_version: Option<&str>) -> Result<()> {
    if let Some(v) = shell_version {
        validate_string_field("shell_version", v, 64)?;
    }
    Ok(())
}

/// Validate string field length and content
///
/// Control characters (NUL included) are rejected outright by the crate's one
/// shared policy, [`crate::security::reject_control_chars`] (#578). This layer
/// used to run its own more permissive rule — up to five non-whitespace control
/// characters tolerated, whitespace ones unlimited — which disagreed with the
/// path validator's.
///
/// # Errors
///
/// Returns an error if the value exceeds `max_length` or contains a control
/// character.
fn validate_string_field(field_name: &str, value: &str, max_length: usize) -> Result<()> {
    // Chain validation checks with early return
    validate_length(field_name, value, max_length)?;
    crate::security::reject_control_chars(field_name, value)?;
    Ok(())
}

/// Validate string length
///
/// # Errors
///
/// Returns an error if the string exceeds the configured maximum length.
fn validate_length(field_name: &str, value: &str, max_length: usize) -> Result<()> {
    if value.len() <= max_length {
        Ok(())
    } else {
        Err(Error::ipc(format!(
            "Field '{}' too long: {} chars (max {})",
            field_name,
            value.len(),
            max_length
        )))
    }
}

/// Protocol version for compatibility checking
///
/// # Version History
///
/// - **Version 2** (current): Shell notification channel changed (#674). The
///   agent signals shells only with SIGURG (a "doorbell" whose default
///   disposition is ignore) and carries meaning in per-shell flag files under
///   `<runtime_root>/shells/` (`<pid>.reload`, `<pid>.reregister`);
///   the earlier per-purpose signals are no longer sent. Wire messages are unchanged.
/// - **Version 1**: Initial stable protocol
///   - Fish shell format (legacy) with `op` field
///   - Native Rust enum serialization
///   - Message types: `RepositoryStatus`, `LanguageDetect`, `RegisterClient`, `UnregisterClient`,
///     `WorkspaceUpdate`, `Ping`, `Status`, `ThemeQuery`, `Shutdown`, `ConfigReload`,
///     `DirectoryRequest`, `DurationRequest`, `CharacterRequest`, `HostnameRequest`,
///     `UsernameRequest`, `LatencyStats`
///   - Response types: `RepositoryStatus`, `Language`, `Ack`, `AgentStatus`, `ThemeValue`, `Error`,
///     `Directory`, `Duration`, `Character`, `Hostname`, `Username`, `LatencyStatsResult`
///   - Maximum message size: 64KB
///   - Transport: Unix domain sockets
///
/// # Compatibility Rules
///
/// This protocol follows semantic versioning principles:
///
/// ## MAJOR Version Changes (Breaking)
///
/// Increment MAJOR version when making incompatible changes:
/// - Changing message field types or semantics
/// - Removing message variants or fields
/// - Changing required field sets
/// - Modifying transport layer (e.g., switching from sockets to pipes)
/// - Changing error response format
///
/// **Migration Required**: Clients must update to handle new protocol
///
/// ## MINOR Version Changes (Backward Compatible)
///
/// Increment MINOR version when adding functionality in a backward-compatible manner:
/// - Adding new message variants
/// - Adding new optional fields to existing messages
/// - Adding new response types
/// - Extending enums with new variants (if clients handle unknown variants)
///
/// **Migration Optional**: Old clients continue to work with new agents
///
/// ## PATCH Version Changes (Internal)
///
/// For internal changes that don't affect the wire protocol:
/// - Performance improvements
/// - Bug fixes in serialization
/// - Documentation updates
/// - Internal refactoring
///
/// **No Migration Needed**: Completely transparent to clients
///
/// # Version Negotiation
///
/// Currently, version negotiation is implicit:
/// - Agent accepts both Fish format and native Rust format messages
/// - Clients should handle error responses gracefully
/// - Future versions may add explicit version negotiation via a handshake message
///
/// # Testing Compatibility
///
/// To test compatibility across versions:
///
/// ```rust,no_run
/// use gpy_agent::ipc::protocol::{deserialize_message, serialize_response};
/// use gpy_agent::ipc::Response;
///
/// // Ensure old messages can still be parsed
/// let old_message = r#"{"op":"git","cwd":"."}"#;
/// let parsed = deserialize_message(old_message.as_bytes());
/// assert!(parsed.is_ok());
///
/// // Ensure responses are forward-compatible
/// # let response = Response::Error { message: "test".to_string() };
/// let serialized = serialize_response(&response);
/// // Old clients should be able to parse this
/// ```
///
/// # Schema Validation
///
/// See `gpy-agent/schemas/` for JSON Schema definitions that can be used to
/// validate protocol compliance. These schemas are tested in
/// `tests/schema_validation.rs` to ensure documentation examples match actual
/// serialization behavior.
pub const PROTOCOL_VERSION: u8 = 2;

/// Raw wire mirror of [`Message`], carrying unvalidated field values.
///
/// Every field `Message` holds as a type whose constructor touches the
/// filesystem or enforces a domain invariant — `SafePath`, `ClientPid` — is a
/// plain `String`/`u32` here, so a payload can be parsed into this enum with no
/// I/O at all (#601). [`resolve`] converts it into a `Message` and is the only
/// place those stronger types get constructed.
///
/// Variant names, field names and the remaining field types mirror `Message`
/// exactly, including which fields carry `#[serde(default)]`, so the native
/// wire format this parses is byte-for-byte the one `Message`'s own
/// `Deserialize` accepts. `shell` keeps `Message`'s `ShellVariant` rather than a
/// raw string: its `Deserialize` is pure, and holding the parsed type preserves
/// the native format's rejection of an unknown shell name (the shell format's
/// looser `FromStr`-with-`Unknown`-fallback happens in
/// [`ShellIpcMessage::into_wire_message`], which is equally pure).
#[derive(Debug, Deserialize)]
enum WireMessage {
    /// Raw form of [`Message::RepositoryStatus`].
    RepositoryStatus {
        /// Unvalidated path string.
        path: String,
        /// Desired response format (defaults to JSON).
        #[serde(default)]
        format: super::Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        #[serde(default)]
        is_first: bool,
        /// Previous segment's background color.
        #[serde(default)]
        prev_bg: Option<String>,
    },
    /// Raw form of [`Message::LanguageDetect`].
    LanguageDetect {
        /// Unvalidated path string.
        path: String,
        /// Desired response format (defaults to JSON).
        #[serde(default)]
        format: super::Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        #[serde(default)]
        is_first: bool,
        /// Previous segment's background color.
        #[serde(default)]
        prev_bg: Option<String>,
        /// Client-forwarded `$VIRTUAL_ENV`.
        #[serde(default)]
        virtual_env: Option<String>,
    },
    /// Raw form of [`Message::DirectoryRequest`].
    DirectoryRequest {
        /// Unvalidated path string.
        path: String,
        /// Desired response format.
        #[serde(default)]
        format: super::Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        #[serde(default)]
        is_first: bool,
        /// Previous segment's background color.
        #[serde(default)]
        prev_bg: Option<String>,
    },
    /// Raw form of [`Message::ClockRequest`].
    ClockRequest {
        /// Shell whose live-time prompt token the render should embed.
        shell: crate::shell::Shell,
        /// Desired response format.
        #[serde(default)]
        format: super::Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        #[serde(default)]
        is_first: bool,
        /// Previous segment's background color.
        #[serde(default)]
        prev_bg: Option<String>,
    },
    /// Raw form of [`Message::DurationRequest`].
    DurationRequest {
        /// The command duration in milliseconds.
        duration_ms: u64,
        /// Desired response format.
        #[serde(default)]
        format: super::Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        is_last: bool,
        /// Whether this is the first segment in the prompt.
        #[serde(default)]
        is_first: bool,
        /// Previous segment's background color.
        #[serde(default)]
        prev_bg: Option<String>,
    },
    /// Raw form of [`Message::CharacterRequest`].
    CharacterRequest {
        /// Whether the last command succeeded.
        success: bool,
        /// Desired response format.
        #[serde(default)]
        format: super::Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        is_last: bool,
        /// Previous segment's background color.
        #[serde(default)]
        prev_bg: Option<String>,
    },
    /// Raw form of [`Message::HostnameRequest`].
    HostnameRequest {
        /// The hostname string resolved by the client.
        hostname: String,
        /// Desired response format.
        #[serde(default)]
        format: super::Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        is_last: bool,
        /// Previous segment's background color.
        #[serde(default)]
        prev_bg: Option<String>,
    },
    /// Raw form of [`Message::UsernameRequest`].
    UsernameRequest {
        /// The effective username string resolved by the client.
        username: String,
        /// Desired response format.
        #[serde(default)]
        format: super::Format,
        /// Whether this is the last segment in the prompt.
        #[serde(default)]
        is_last: bool,
        /// Previous segment's background color.
        #[serde(default)]
        prev_bg: Option<String>,
    },
    /// Raw form of [`Message::RegisterClient`].
    RegisterClient {
        /// Unvalidated process ID.
        pid: u32,
        /// Unvalidated current working directory.
        cwd: Option<String>,
        /// Optional shell name.
        shell: Option<crate::config::types::ShellVariant>,
        /// Optional shell version.
        shell_version: Option<String>,
    },
    /// Raw form of [`Message::UnregisterClient`].
    UnregisterClient {
        /// Unvalidated process ID.
        pid: u32,
    },
    /// Raw form of [`Message::WorkspaceUpdate`].
    WorkspaceUpdate {
        /// Unvalidated process ID.
        pid: u32,
        /// Unvalidated current working directory.
        cwd: String,
    },
    /// Raw form of [`Message::Ping`].
    Ping,
    /// Raw form of [`Message::Status`].
    Status,
    /// Raw form of [`Message::ThemeQuery`].
    ThemeQuery {
        /// The theme setting key to query.
        key: String,
    },
    /// Raw form of [`Message::LatencyStats`].
    LatencyStats,
    /// Raw form of [`Message::Shutdown`].
    Shutdown,
    /// Raw form of [`Message::ConfigReload`].
    ConfigReload,
}

/// Shell IPC message format
#[derive(Debug, Deserialize)]
struct ShellIpcMessage {
    op: String,
    cwd: Option<String>,
    pid: Option<u32>,
    format: Option<String>,
    key: Option<String>,
    is_last: Option<bool>,
    /// Whether this is the first segment in the prompt (for opening-cap suppression).
    #[serde(default)]
    is_first: Option<bool>,
    shell: Option<String>,
    shell_version: Option<String>,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    success: Option<bool>,
    /// Previous segment's background color for powerline chevron transitions.
    #[serde(default)]
    prev_bg: Option<String>,
    /// Client-resolved hostname string for the `hostname` op (#259).
    #[serde(default)]
    hostname: Option<String>,
    /// Client-resolved effective username string for the `username` op (#252).
    #[serde(default)]
    username: Option<String>,
    /// Client-forwarded `$VIRTUAL_ENV` for the `lang` op (venv-aware Python).
    #[serde(default)]
    virtual_env: Option<String>,
}

impl ShellIpcMessage {
    /// Convert shell message format to the raw [`WireMessage`] form.
    ///
    /// Pure: this checks only the wire shape (which fields an op requires, and
    /// whether the op is known). Path canonicalization and PID range checking
    /// happen later, in [`resolve`].
    ///
    /// # Errors
    ///
    /// Returns an error if mandatory fields are missing for the requested operation or if the
    /// operation identifier is unrecognized.
    #[expect(
        clippy::too_many_lines,
        reason = "dispatch table mapping every shell-side op string to its Message variant, validating each op's required fields; splitting per-op would scatter the single source of truth for the wire protocol"
    )]
    fn into_wire_message(self) -> Result<WireMessage> {
        // Unlike Format::from_str's hard error, an unrecognized/missing format
        // here silently defaults to JSON: shell-side IPC messages are
        // best-effort, and a malformed format string shouldn't fail the whole
        // request.
        let format = self
            .format
            .as_deref()
            .and_then(|value| value.parse::<super::Format>().ok())
            .unwrap_or(super::Format::Json);

        match self.op.as_str() {
            "git" => Ok(WireMessage::RepositoryStatus {
                path: self.cwd.unwrap_or_else(|| ".".to_owned()),
                format,
                is_last: self.is_last.unwrap_or(false),
                is_first: self.is_first.unwrap_or(false),
                prev_bg: self.prev_bg,
            }),
            "lang" => Ok(WireMessage::LanguageDetect {
                path: self.cwd.unwrap_or_else(|| ".".to_owned()),
                format,
                is_last: self.is_last.unwrap_or(false),
                is_first: self.is_first.unwrap_or(false),
                prev_bg: self.prev_bg,
                virtual_env: self.virtual_env,
            }),
            "directory" => Ok(WireMessage::DirectoryRequest {
                path: self.cwd.unwrap_or_else(|| ".".to_owned()),
                format,
                is_last: self.is_last.unwrap_or(false),
                is_first: self.is_first.unwrap_or(false),
                prev_bg: self.prev_bg,
            }),
            "clock" => Ok(WireMessage::ClockRequest {
                // Reuses the flat wire struct's existing `shell` string field.
                // An absent or unrecognized shell falls back to Fish, whose
                // render is a no-op (Fish draws the clock locally), so a
                // malformed request degrades to "agent emits nothing" rather
                // than to some other shell's prompt-escape syntax leaking
                // through as literal text.
                shell: self
                    .shell
                    .as_deref()
                    .and_then(|s| s.parse::<crate::shell::Shell>().ok())
                    .unwrap_or(crate::shell::Shell::Fish),
                format,
                is_last: self.is_last.unwrap_or(false),
                is_first: self.is_first.unwrap_or(false),
                prev_bg: self.prev_bg,
            }),
            "duration" => Ok(WireMessage::DurationRequest {
                duration_ms: self.duration_ms,
                format,
                is_last: self.is_last.unwrap_or(false),
                is_first: self.is_first.unwrap_or(false),
                prev_bg: self.prev_bg,
            }),
            "character" => Ok(WireMessage::CharacterRequest {
                success: self.success.unwrap_or(false),
                format,
                is_last: self.is_last.unwrap_or(false),
                prev_bg: self.prev_bg,
            }),
            "hostname" => Ok(WireMessage::HostnameRequest {
                hostname: self.hostname.unwrap_or_default(),
                format,
                is_last: self.is_last.unwrap_or(false),
                prev_bg: self.prev_bg,
            }),
            "username" => Ok(WireMessage::UsernameRequest {
                username: self.username.unwrap_or_default(),
                format,
                is_last: self.is_last.unwrap_or(false),
                prev_bg: self.prev_bg,
            }),
            "ping" => Ok(WireMessage::Ping),
            "status" => Ok(WireMessage::Status),
            "shutdown" => Ok(WireMessage::Shutdown),
            "config_reload" => Ok(WireMessage::ConfigReload),
            "register" => {
                let pid = self
                    .pid
                    .ok_or_else(|| Error::ipc("register missing pid".to_owned()))?;
                let shell = self.shell.as_deref().map(|s| {
                    std::str::FromStr::from_str(s)
                        .unwrap_or(crate::config::types::ShellVariant::Unknown)
                });

                Ok(WireMessage::RegisterClient {
                    pid,
                    cwd: self.cwd,
                    shell,
                    shell_version: self.shell_version,
                })
            }
            "unregister" => Ok(WireMessage::UnregisterClient {
                pid: self
                    .pid
                    .ok_or_else(|| Error::ipc("unregister missing pid".to_owned()))?,
            }),
            "workspace" => {
                let pid = self
                    .pid
                    .ok_or_else(|| Error::ipc("Workspace update missing pid".to_owned()))?;
                let cwd = self
                    .cwd
                    .ok_or_else(|| Error::ipc("Workspace update missing cwd".to_owned()))?;
                Ok(WireMessage::WorkspaceUpdate { pid, cwd })
            }
            "theme" => Ok(WireMessage::ThemeQuery {
                key: self
                    .key
                    .ok_or_else(|| Error::ipc("Theme query missing key".to_owned()))?,
            }),
            "latency_stats" => Ok(WireMessage::LatencyStats),
            _ => Err(Error::ipc(format!("Unknown shell operation: {}", self.op))),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]
    #![allow(missing_docs)]

    use super::*;
    use crate::config::types::ShellVariant;
    use crate::ipc::Format;
    use serde_json;

    #[test]
    fn test_config_reload_json_deserializes_to_config_reload() {
        let msg = deserialize_message(CONFIG_RELOAD_JSON.as_bytes()).expect("deserialize failed");
        assert!(matches!(msg, Message::ConfigReload));
    }

    #[test]
    fn test_prev_bg_deserialize_missing() {
        let json = r#"{"op":"git","cwd":"."}"#;
        let msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
        assert_eq!(msg.prev_bg, None);
    }

    #[test]
    fn test_prev_bg_deserialize_with_value() {
        let json = r#"{"op":"git","cwd":".","prev_bg":"blue"}"#;
        let msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
        assert_eq!(msg.prev_bg, Some("blue".to_owned()));
    }

    #[test]
    fn test_prev_bg_validation_too_long() {
        let long_color = "x".repeat(65_usize);
        let result = validate_string_field("prev_bg", &long_color, 64_usize);
        assert!(result.is_err());
    }

    #[test]
    fn test_hostname_into_message_with_is_last_and_prev_bg() {
        let json = r#"{"op":"hostname","hostname":"myhost","format":"ansi","is_last":true,"prev_bg":"blue"}"#;
        let shell_msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
        let wire = shell_msg
            .into_wire_message()
            .expect("into_wire_message failed");
        let msg = resolve(wire).expect("resolve failed");
        match msg {
            Message::HostnameRequest {
                hostname,
                format,
                is_last,
                prev_bg,
            } => {
                assert_eq!(hostname, "myhost");
                assert_eq!(format, Format::Ansi);
                assert!(is_last);
                assert_eq!(prev_bg, Some("blue".to_owned()));
            }
            other => panic!("expected HostnameRequest, got {other:?}"),
        }
    }

    #[test]
    fn test_hostname_into_message_without_is_last_and_prev_bg() {
        let json = r#"{"op":"hostname","hostname":"myhost"}"#;
        let shell_msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
        let wire = shell_msg
            .into_wire_message()
            .expect("into_wire_message failed");
        let msg = resolve(wire).expect("resolve failed");
        match msg {
            Message::HostnameRequest {
                hostname,
                is_last,
                prev_bg,
                ..
            } => {
                assert_eq!(hostname, "myhost");
                assert!(!is_last);
                assert_eq!(prev_bg, None);
            }
            other => panic!("expected HostnameRequest, got {other:?}"),
        }
    }

    #[test]
    fn test_hostname_validation_accepts_valid_request() {
        let msg = Message::HostnameRequest {
            hostname: "my-host".to_owned(),
            format: Format::Ansi,
            is_last: false,
            prev_bg: None,
        };
        assert!(validate_message_content(&msg).is_ok());
    }

    #[test]
    fn test_hostname_validation_rejects_too_long_hostname() {
        let long_hostname = "x".repeat(257_usize);
        let msg = Message::HostnameRequest {
            hostname: long_hostname,
            format: Format::Ansi,
            is_last: false,
            prev_bg: None,
        };
        assert!(validate_message_content(&msg).is_err());
    }

    #[test]
    fn test_hostname_validation_rejects_control_chars() {
        let hostname_with_control = "host\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}name".to_owned();
        let msg = Message::HostnameRequest {
            hostname: hostname_with_control,
            format: Format::Ansi,
            is_last: false,
            prev_bg: None,
        };
        assert!(validate_message_content(&msg).is_err());
    }

    #[test]
    fn test_username_into_message_with_is_last_and_prev_bg() {
        let json = r#"{"op":"username","username":"root","format":"ansi","is_last":true,"prev_bg":"blue"}"#;
        let shell_msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
        let wire = shell_msg
            .into_wire_message()
            .expect("into_wire_message failed");
        let msg = resolve(wire).expect("resolve failed");
        match msg {
            Message::UsernameRequest {
                username,
                format,
                is_last,
                prev_bg,
            } => {
                assert_eq!(username, "root");
                assert_eq!(format, Format::Ansi);
                assert!(is_last);
                assert_eq!(prev_bg, Some("blue".to_owned()));
            }
            other => panic!("expected UsernameRequest, got {other:?}"),
        }
    }

    #[test]
    fn test_username_into_message_without_is_last_and_prev_bg() {
        let json = r#"{"op":"username","username":"root"}"#;
        let shell_msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
        let wire = shell_msg
            .into_wire_message()
            .expect("into_wire_message failed");
        let msg = resolve(wire).expect("resolve failed");
        match msg {
            Message::UsernameRequest {
                username,
                is_last,
                prev_bg,
                ..
            } => {
                assert_eq!(username, "root");
                assert!(!is_last);
                assert_eq!(prev_bg, None);
            }
            other => panic!("expected UsernameRequest, got {other:?}"),
        }
    }

    #[test]
    fn test_username_validation_accepts_valid_request() {
        let msg = Message::UsernameRequest {
            username: "root".to_owned(),
            format: Format::Ansi,
            is_last: false,
            prev_bg: None,
        };
        assert!(validate_message_content(&msg).is_ok());
    }

    #[test]
    fn test_username_validation_rejects_too_long_username() {
        let long_username = "x".repeat(257_usize);
        let msg = Message::UsernameRequest {
            username: long_username,
            format: Format::Ansi,
            is_last: false,
            prev_bg: None,
        };
        assert!(validate_message_content(&msg).is_err());
    }

    #[test]
    fn test_username_validation_rejects_control_chars() {
        let username_with_control = "ro\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}ot".to_owned();
        let msg = Message::UsernameRequest {
            username: username_with_control,
            format: Format::Ansi,
            is_last: false,
            prev_bg: None,
        };
        assert!(validate_message_content(&msg).is_err());
    }

    #[test]
    fn test_prev_bg_validation_null_byte() {
        let color_with_null = "blue\0red";
        let result = validate_string_field("prev_bg", color_with_null, 64_usize);
        assert!(result.is_err());
    }

    // Regression tests for #339: `deserialize_message` tries `ShellIpcMessage`
    // first (hot path for shell traffic) and falls back to the native `Message`
    // enum. These tests pin that both directions still work after the reorder,
    // and that the two wire formats are provably disjoint, which is exactly what
    // makes trying `ShellIpcMessage` first safe (it can never shadow a native
    // `Message` payload, or vice versa).

    #[test]
    fn test_deserialize_message_shell_format_hot_path() {
        let json = r#"{"op":"git","cwd":"/some/path"}"#;
        let msg = deserialize_message(json.as_bytes()).expect("deserialize failed");
        match msg {
            Message::RepositoryStatus {
                path,
                format,
                is_last,
                is_first,
                prev_bg,
            } => {
                assert_eq!(
                    path,
                    crate::security::SafePath::new("/some/path").expect("SafePath failed")
                );
                assert_eq!(format, Format::Json);
                assert!(!is_last);
                assert!(!is_first);
                assert_eq!(prev_bg, None);
            }
            other => panic!("expected RepositoryStatus, got {other:?}"),
        }
    }

    #[test]
    fn test_deserialize_message_native_format_fallback() {
        let native = Message::Ping;
        let serialized = serialize_message(&native).expect("serialize failed");
        let msg = deserialize_message(&serialized).expect("deserialize failed");
        assert!(matches!(msg, Message::Ping));
    }

    #[test]
    fn test_shell_and_native_message_formats_are_disjoint() {
        // Shell-format JSON must not parse as the externally-tagged native
        // `Message` enum: `{"op":"git",...}` has no key matching any variant name.
        let shell_json = r#"{"op":"git","cwd":"/some/path"}"#;
        let native_parse: std::result::Result<Message, serde_json::Error> =
            serde_json::from_str(shell_json);
        assert!(
            native_parse.is_err(),
            "shell-format JSON unexpectedly parsed as native Message"
        );

        // Native-format JSON must not parse as `ShellIpcMessage`: it has no
        // required `op` field, so the required-field check fails.
        let native = Message::Ping;
        let native_bytes = serialize_message(&native).expect("serialize failed");
        let native_str = std::str::from_utf8(&native_bytes).expect("valid utf-8");
        let shell_parse: std::result::Result<ShellIpcMessage, serde_json::Error> =
            serde_json::from_str(native_str);
        assert!(
            shell_parse.is_err(),
            "native-format JSON unexpectedly parsed as ShellIpcMessage"
        );
    }

    // #601: `parse_wire` is the pure half of the parse pipeline. Every test
    // below goes from raw JSON bytes to a `WireMessage` without constructing a
    // `SafePath` or a `ClientPid`, without touching the filesystem, and without
    // referencing any real process. Several deliberately use paths and PIDs
    // that `resolve` would reject — `/etc/...` is denylisted by
    // `PathValidator`, `0` and `4_194_305` are outside `ClientPid`'s range — so
    // a passing assertion is positive evidence that no validation ran at this
    // layer.

    #[test]
    fn test_parse_wire_git_keeps_path_raw() {
        // `/etc` is denylisted by `PathValidator`, so `SafePath::new` would
        // reject this; `parse_wire` must hand it back untouched.
        let json = br#"{"op":"git","cwd":"/etc/never-canonicalized"}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::RepositoryStatus {
                path,
                format,
                is_last,
                is_first,
                prev_bg,
            } => {
                assert_eq!(path, "/etc/never-canonicalized");
                assert_eq!(format, Format::Json);
                assert!(!is_last);
                assert!(!is_first);
                assert_eq!(prev_bg, None);
            }
            _ => panic!("expected WireMessage::RepositoryStatus"),
        }
    }

    #[test]
    fn test_parse_wire_lang_carries_render_metadata_and_virtual_env() {
        let json = br#"{"op":"lang","cwd":"/etc/nope","format":"ansi","is_last":true,"is_first":true,"prev_bg":"blue","virtual_env":"/venv"}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::LanguageDetect {
                path,
                format,
                is_last,
                is_first,
                prev_bg,
                virtual_env,
            } => {
                assert_eq!(path, "/etc/nope");
                assert_eq!(format, Format::Ansi);
                assert!(is_last);
                assert!(is_first);
                assert_eq!(prev_bg, Some("blue".to_owned()));
                assert_eq!(virtual_env, Some("/venv".to_owned()));
            }
            _ => panic!("expected WireMessage::LanguageDetect"),
        }
    }

    #[test]
    fn test_parse_wire_directory_defaults_missing_cwd_to_dot() {
        let json = br#"{"op":"directory"}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::DirectoryRequest { path, .. } => assert_eq!(path, "."),
            _ => panic!("expected WireMessage::DirectoryRequest"),
        }
    }

    #[test]
    fn test_parse_wire_register_keeps_pid_and_cwd_raw() {
        // 4_194_305 is one past `ClientPid::MAX`: it must survive wire parsing
        // and only be rejected later, by `resolve`.
        let json = br#"{"op":"register","pid":4194305,"cwd":"/etc/nope","shell":"fish","shell_version":"3.7.1"}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::RegisterClient {
                pid,
                cwd,
                shell,
                shell_version,
            } => {
                assert_eq!(pid, 4_194_305_u32);
                assert_eq!(cwd, Some("/etc/nope".to_owned()));
                assert_eq!(shell, Some(ShellVariant::Fish));
                assert_eq!(shell_version, Some("3.7.1".to_owned()));
            }
            _ => panic!("expected WireMessage::RegisterClient"),
        }
    }

    #[test]
    fn test_parse_wire_unregister_keeps_out_of_range_pid() {
        let json = br#"{"op":"unregister","pid":0}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::UnregisterClient { pid } => assert_eq!(pid, 0_u32),
            _ => panic!("expected WireMessage::UnregisterClient"),
        }
    }

    #[test]
    fn test_parse_wire_workspace_keeps_pid_and_cwd_raw() {
        let json = br#"{"op":"workspace","pid":0,"cwd":"/etc/nope"}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::WorkspaceUpdate { pid, cwd } => {
                assert_eq!(pid, 0_u32);
                assert_eq!(cwd, "/etc/nope");
            }
            _ => panic!("expected WireMessage::WorkspaceUpdate"),
        }
    }

    #[test]
    fn test_parse_wire_duration_is_a_plain_field_copy() {
        let json = br#"{"op":"duration","duration_ms":65000,"format":"ansi","is_last":true}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::DurationRequest {
                duration_ms,
                format,
                is_last,
                is_first,
                prev_bg,
            } => {
                assert_eq!(duration_ms, 65_000_u64);
                assert_eq!(format, Format::Ansi);
                assert!(is_last);
                assert!(!is_first);
                assert_eq!(prev_bg, None);
            }
            _ => panic!("expected WireMessage::DurationRequest"),
        }
    }

    #[test]
    fn test_parse_wire_theme_query_carries_key() {
        let json = br#"{"op":"theme","key":"prompt_close"}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::ThemeQuery { key } => assert_eq!(key, "prompt_close"),
            _ => panic!("expected WireMessage::ThemeQuery"),
        }
    }

    #[test]
    fn test_parse_wire_unit_ops() {
        assert!(matches!(
            parse_wire(br#"{"op":"ping"}"#).expect("parse_wire failed"),
            WireMessage::Ping
        ));
        assert!(matches!(
            parse_wire(br#"{"op":"status"}"#).expect("parse_wire failed"),
            WireMessage::Status
        ));
        assert!(matches!(
            parse_wire(br#"{"op":"latency_stats"}"#).expect("parse_wire failed"),
            WireMessage::LatencyStats
        ));
    }

    #[test]
    fn test_parse_wire_register_missing_pid_is_a_wire_shape_error() {
        let err = parse_wire(br#"{"op":"register","cwd":"/etc/nope"}"#)
            .expect_err("register without a pid must fail");
        assert!(
            err.to_string().contains("register missing pid"),
            "unexpected error text"
        );
    }

    #[test]
    fn test_parse_wire_workspace_missing_pid_and_cwd_are_wire_shape_errors() {
        let missing_pid = parse_wire(br#"{"op":"workspace","cwd":"/etc/nope"}"#)
            .expect_err("workspace without a pid must fail");
        assert!(
            missing_pid
                .to_string()
                .contains("Workspace update missing pid"),
            "unexpected error text for a missing pid"
        );

        let missing_cwd = parse_wire(br#"{"op":"workspace","pid":1}"#)
            .expect_err("workspace without a cwd must fail");
        assert!(
            missing_cwd
                .to_string()
                .contains("Workspace update missing cwd"),
            "unexpected error text for a missing cwd"
        );
    }

    #[test]
    fn test_parse_wire_unknown_operation() {
        let err = parse_wire(br#"{"op":"teleport"}"#).expect_err("unknown op must fail");
        assert!(
            err.to_string()
                .contains("Unknown shell operation: teleport"),
            "unexpected error text"
        );
    }

    #[test]
    fn test_parse_wire_rejects_neither_format() {
        let err = parse_wire(br#"{"not_an_op":true}"#).expect_err("unrecognized payload must fail");
        assert!(
            err.to_string()
                .contains("Failed to deserialize message: invalid format"),
            "unexpected error text"
        );
    }

    #[test]
    fn test_parse_wire_native_format_struct_variant() {
        let json =
            br#"{"RepositoryStatus":{"path":"/etc/nope","format":"fish-source","is_last":true}}"#;
        let wire = parse_wire(json).expect("parse_wire failed");
        match wire {
            WireMessage::RepositoryStatus {
                path,
                format,
                is_last,
                is_first,
                prev_bg,
            } => {
                assert_eq!(path, "/etc/nope");
                assert_eq!(format, Format::FishSource);
                assert!(is_last);
                assert!(!is_first);
                assert_eq!(prev_bg, None);
            }
            _ => panic!("expected WireMessage::RepositoryStatus"),
        }
    }

    #[test]
    fn test_parse_wire_native_format_unit_variant() {
        let wire = parse_wire(br#""Ping""#).expect("parse_wire failed");
        assert!(matches!(wire, WireMessage::Ping));
    }
}
