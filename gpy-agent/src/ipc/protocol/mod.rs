//! JSON IPC protocol between shells and the agent.

use super::{Message, Response};
use crate::{Error, Result};
use serde::Deserialize;
use serde_json;

/// Wire payload for a client-initiated config reload; shared so send sites cannot diverge.
pub const CONFIG_RELOAD_JSON: &str = r#"{"op":"config_reload"}"#;

/// Serialize a message to JSON bytes.
///
/// # Errors
/// Returns an error if serialization fails or the message exceeds size limits.
pub fn serialize_message(msg: &Message) -> Result<Vec<u8>> {
    let mut buffer = Vec::with_capacity(256);
    serde_json::to_writer(&mut buffer, msg)
        .map_err(|e| Error::ipc(format!("Failed to serialize message: {e}")))?;

    validate_message_size(&buffer)?;
    Ok(buffer)
}

/// Deserialize JSON bytes into a message.
///
/// # Errors
/// Returns an error if deserialization or validation fails.
pub fn deserialize_message(data: &[u8]) -> Result<Message> {
    let (_, message) = deserialize_message_with_format(data);
    message
}

/// Deserialize a message, also returning the requested format when parsing got far enough.
///
/// The format is read before path resolution so a request failing later (e.g. a
/// denylisted `cwd`) can still be answered in the client's format.
pub fn deserialize_message_with_format(data: &[u8]) -> (Option<super::Format>, Result<Message>) {
    let wire = match parse_wire(data) {
        Ok(wire) => wire,
        Err(e) => return (None, Err(e)),
    };
    let format = wire.format();
    let message = resolve(wire).and_then(|msg| {
        validate_message_content(&msg)?;
        Ok(msg)
    });
    (format, message)
}

/// Parse wire bytes into a [`WireMessage`] with zero I/O.
fn parse_wire(data: &[u8]) -> Result<WireMessage> {
    validate_message_size(data)?;
    let json = std::str::from_utf8(data)
        .map_err(|e| Error::ipc(format!("Invalid UTF-8 in message: {e}")))?;

    // Shell traffic dominates the hot path, so try its format first.
    if let Ok(shell_msg) = serde_json::from_str::<ShellIpcMessage>(json) {
        return shell_msg.into_wire_message();
    }

    if let Ok(wire) = serde_json::from_str::<WireMessage>(json) {
        return Ok(wire);
    }

    Err(Error::ipc(
        "Failed to deserialize message: invalid format".to_owned(),
    ))
}

/// Resolve a [`WireMessage`] into a [`Message`]; the only place the stronger
/// types (`SafePath`, which touches the filesystem, and `ClientPid`) are built.
#[expect(clippy::too_many_lines, reason = "one arm per Message variant")]
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
            display_path: path,
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
            is_ssh,
            prev_bg,
        } => Ok(Message::HostnameRequest {
            hostname,
            format,
            is_last,
            is_ssh,
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
            // `cwd` before `pid`: fields evaluate in order, so a both-invalid input reports "Invalid cwd".
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
        WireMessage::Ping => Ok(Message::Ping),
        WireMessage::Status => Ok(Message::Status),
        WireMessage::Shutdown => Ok(Message::Shutdown),
        WireMessage::ConfigReload => Ok(Message::ConfigReload),
    }
}

/// Serialize a response to JSON bytes.
///
/// # Errors
/// Returns an error if serialization fails.
pub fn serialize_response(resp: &Response) -> Result<Vec<u8>> {
    let mut buffer = Vec::with_capacity(512);
    serde_json::to_writer(&mut buffer, resp)
        .map_err(|e| Error::ipc(format!("Failed to serialize response: {e}")))?;

    validate_message_size(&buffer)?;
    Ok(buffer)
}

/// Deserialize JSON bytes into a response.
///
/// # Errors
/// Returns an error on invalid UTF-8, oversize data, or bad JSON.
pub fn deserialize_response(data: &[u8]) -> Result<Response> {
    validate_message_size(data)?;
    let json = std::str::from_utf8(data)
        .map_err(|e| Error::ipc(format!("Invalid UTF-8 in response: {e}")))?;
    serde_json::from_str(json)
        .map_err(|e| Error::ipc(format!("Failed to deserialize response: {e}")))
}

/// Validate the message size limit.
///
/// # Errors
/// Returns an error if the message is too large.
pub fn validate_message_size(data: &[u8]) -> Result<()> {
    if data.len() > crate::security::MAX_MESSAGE_SIZE {
        return Err(Error::ipc(format!(
            "Message too large: {} bytes (max {})",
            data.len(),
            crate::security::MAX_MESSAGE_SIZE
        )));
    }

    Ok(())
}

/// Validate message content according to schema rules.
///
/// # Errors
/// Returns an error if a field violates schema rules.
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
        // `ClientPid` is range-checked at construction; liveness is probed at route time.
        Message::UnregisterClient { .. }
        | Message::WorkspaceUpdate { .. }
        | Message::Ping
        | Message::Shutdown
        | Message::Status
        | Message::ConfigReload => {}
    }
    Ok(())
}

fn validate_hostname_request(prev_bg: Option<&str>, hostname: &str) -> Result<()> {
    if let Some(bg) = prev_bg {
        validate_string_field("prev_bg", bg, 64)?;
    }
    validate_string_field("hostname", hostname, 256)?;
    Ok(())
}

fn validate_username_request(prev_bg: Option<&str>, username: &str) -> Result<()> {
    if let Some(bg) = prev_bg {
        validate_string_field("prev_bg", bg, 64)?;
    }
    validate_string_field("username", username, 256)?;
    Ok(())
}

/// Validate a `RegisterClient` message's optional shell version.
fn validate_register_client(shell_version: Option<&str>) -> Result<()> {
    if let Some(v) = shell_version {
        validate_string_field("shell_version", v, 64)?;
    }
    Ok(())
}

/// Validate string length and reject control characters via the shared
/// [`crate::security::reject_control_chars`] policy.
fn validate_string_field(field_name: &str, value: &str, max_length: usize) -> Result<()> {
    validate_length(field_name, value, max_length)?;
    crate::security::reject_control_chars(field_name, value)?;
    Ok(())
}

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

/// IPC protocol version.
///
/// v2: shells are notified only via SIGURG (ignored by default) plus per-shell
/// flag files under `<runtime_root>/shells/`; wire messages unchanged from v1.
pub const PROTOCOL_VERSION: u8 = 2;

/// Stands in for `\n` inside a reply, which must stay one newline-terminated
/// line. Shells map it back to `\n`. Backward compatible both ways: an older
/// shell shows the (non-printing) byte, an older agent sends the raw newline.
pub const REPLY_NEWLINE: char = '\x1e';

/// Encode a reply body for the newline-terminated wire: drop any stray
/// [`REPLY_NEWLINE`], then replace `\n` with it. Borrows when neither occurs.
#[must_use]
pub fn encode_reply_newlines(payload: &str) -> std::borrow::Cow<'_, str> {
    if !payload.contains(['\n', REPLY_NEWLINE]) {
        return std::borrow::Cow::Borrowed(payload);
    }
    std::borrow::Cow::Owned(
        payload
            .chars()
            .filter(|&c| c != REPLY_NEWLINE)
            .map(|c| if c == '\n' { REPLY_NEWLINE } else { c })
            .collect(),
    )
}

/// Raw wire mirror of [`Message`] with unvalidated `String`/`u32` fields, so
/// parsing needs no I/O; [`resolve`] builds the validated types. Must stay
/// byte-compatible with `Message`'s own `Deserialize` (including `#[serde(default)]`).
#[derive(Debug, Deserialize)]
enum WireMessage {
    RepositoryStatus {
        path: String,
        #[serde(default)]
        format: super::Format,
        #[serde(default)]
        is_last: bool,
        #[serde(default)]
        is_first: bool,
        #[serde(default)]
        prev_bg: Option<String>,
    },
    LanguageDetect {
        path: String,
        #[serde(default)]
        format: super::Format,
        #[serde(default)]
        is_last: bool,
        #[serde(default)]
        is_first: bool,
        #[serde(default)]
        prev_bg: Option<String>,
        #[serde(default)]
        virtual_env: Option<String>,
    },
    DirectoryRequest {
        path: String,
        #[serde(default)]
        format: super::Format,
        #[serde(default)]
        is_last: bool,
        #[serde(default)]
        is_first: bool,
        #[serde(default)]
        prev_bg: Option<String>,
    },
    ClockRequest {
        shell: crate::shell::Shell,
        #[serde(default)]
        format: super::Format,
        #[serde(default)]
        is_last: bool,
        #[serde(default)]
        is_first: bool,
        #[serde(default)]
        prev_bg: Option<String>,
    },
    DurationRequest {
        duration_ms: u64,
        #[serde(default)]
        format: super::Format,
        #[serde(default)]
        is_last: bool,
        #[serde(default)]
        is_first: bool,
        #[serde(default)]
        prev_bg: Option<String>,
    },
    CharacterRequest {
        success: bool,
        #[serde(default)]
        format: super::Format,
        #[serde(default)]
        is_last: bool,
        #[serde(default)]
        prev_bg: Option<String>,
    },
    HostnameRequest {
        hostname: String,
        #[serde(default)]
        format: super::Format,
        #[serde(default)]
        is_last: bool,
        #[serde(default)]
        is_ssh: bool,
        #[serde(default)]
        prev_bg: Option<String>,
    },
    UsernameRequest {
        username: String,
        #[serde(default)]
        format: super::Format,
        #[serde(default)]
        is_last: bool,
        #[serde(default)]
        prev_bg: Option<String>,
    },
    RegisterClient {
        pid: u32,
        cwd: Option<String>,
        shell: Option<crate::config::types::ShellVariant>,
        shell_version: Option<String>,
    },
    UnregisterClient {
        pid: u32,
    },
    WorkspaceUpdate {
        pid: u32,
        cwd: String,
    },
    Ping,
    Status,
    Shutdown,
    ConfigReload,
}

impl WireMessage {
    /// Requested response format, or `None` for ops without one.
    const fn format(&self) -> Option<super::Format> {
        match self {
            Self::RepositoryStatus { format, .. }
            | Self::LanguageDetect { format, .. }
            | Self::DirectoryRequest { format, .. }
            | Self::ClockRequest { format, .. }
            | Self::DurationRequest { format, .. }
            | Self::CharacterRequest { format, .. }
            | Self::HostnameRequest { format, .. }
            | Self::UsernameRequest { format, .. } => Some(*format),
            Self::RegisterClient { .. }
            | Self::UnregisterClient { .. }
            | Self::WorkspaceUpdate { .. }
            | Self::Ping
            | Self::Status
            | Self::Shutdown
            | Self::ConfigReload => None,
        }
    }
}

/// Flat shell-side message format keyed by `op`.
#[derive(Debug, Deserialize)]
struct ShellIpcMessage {
    op: String,
    cwd: Option<String>,
    pid: Option<u32>,
    format: Option<String>,
    is_last: Option<bool>,
    #[serde(default)]
    is_first: Option<bool>,
    shell: Option<String>,
    shell_version: Option<String>,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    prev_bg: Option<String>,
    #[serde(default)]
    hostname: Option<String>,
    /// Absent means `false`.
    #[serde(default)]
    is_ssh: Option<bool>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    virtual_env: Option<String>,
}

impl ShellIpcMessage {
    /// Convert to [`WireMessage`], checking only wire shape; validation happens in [`resolve`].
    fn into_wire_message(self) -> Result<WireMessage> {
        // Unlike `Format::from_str`, a bad format defaults to JSON: shell IPC is best-effort.
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
                // Unknown shell falls back to Fish, whose render is a no-op, rather
                // than leaking another shell's prompt escapes as literal text.
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
                is_ssh: self.is_ssh.unwrap_or(false),
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
            _ => Err(Error::ipc(format!("Unknown shell operation: {}", self.op))),
        }
    }
}

#[cfg(test)]
mod tests;
