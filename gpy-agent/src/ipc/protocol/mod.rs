//! JSON IPC protocol between shells and the agent.

use super::{Format, Message, Response, Segment};
use crate::security::SafePath;
use crate::{Error, Result};
use serde::Deserialize;

/// Wire payload for a client-initiated config reload; shared so send sites cannot diverge.
pub const CONFIG_RELOAD_JSON: &str = r#"{"op":"config_reload"}"#;

/// Longest accepted `prev_bg` color.
const MAX_PREV_BG_LEN: usize = 64;
/// Longest accepted client-supplied hostname or username.
const MAX_IDENTITY_LEN: usize = 256;
/// Longest accepted `shell_version`.
const MAX_SHELL_VERSION_LEN: usize = 64;

/// Deserialize JSON bytes into a message.
///
/// # Errors
/// Returns an error if deserialization or validation fails.
pub fn deserialize_message(data: &[u8]) -> Result<Message> {
    let (_, message) = deserialize_message_with_format(data);
    message
}

/// Deserialize a message, also returning the requested format when it can be read.
///
/// The format is read before op dispatch and path resolution, so a request that
/// fails later (an unknown op, a missing field, a denylisted `cwd`) can still be
/// answered in the client's format.
pub fn deserialize_message_with_format(data: &[u8]) -> (Option<Format>, Result<Message>) {
    let request = match parse_request(data) {
        Ok(request) => request,
        Err(e) => return (requested_format(data), Err(e)),
    };
    let format = parse_format(request.format.as_deref());
    let message = request.into_message(format).and_then(|msg| {
        validate_message_content(&msg)?;
        Ok(msg)
    });
    (format, message)
}

/// Best-effort read of `format` from a request that failed to parse; `None`
/// when the payload is oversize, not JSON, or names no known format.
fn requested_format(data: &[u8]) -> Option<Format> {
    #[derive(Deserialize)]
    struct FormatOnly {
        format: Option<String>,
    }

    validate_message_size(data).ok()?;
    let request = serde_json::from_slice::<FormatOnly>(data).ok()?;
    parse_format(request.format.as_deref())
}

/// A wire `format` value, or `None` when absent or unknown.
fn parse_format(value: Option<&str>) -> Option<Format> {
    value?.parse().ok()
}

/// Size-check and decode request bytes into their flat wire form.
#[allow(
    clippy::map_err_ignore,
    reason = "the fixed deserialize message is the IPC error text clients see; serde's detail is kept out of it"
)]
fn parse_request(data: &[u8]) -> Result<ShellIpcMessage> {
    validate_message_size(data)?;
    let json = std::str::from_utf8(data)
        .map_err(|e| Error::ipc(format!("Invalid UTF-8 in message: {e}")))?;

    serde_json::from_str(json)
        .map_err(|_| Error::ipc("Failed to deserialize message: invalid format".to_owned()))
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
    if let Some(bg) = msg.segment().and_then(|segment| segment.prev_bg.as_deref()) {
        validate_string_field("prev_bg", bg, MAX_PREV_BG_LEN)?;
    }
    match msg {
        Message::HostnameRequest { hostname, .. } => {
            validate_string_field("hostname", hostname, MAX_IDENTITY_LEN)
        }
        Message::UsernameRequest { username, .. } => {
            validate_string_field("username", username, MAX_IDENTITY_LEN)
        }
        Message::RegisterClient {
            shell_version: Some(version),
            ..
        } => validate_string_field("shell_version", version, MAX_SHELL_VERSION_LEN),
        // Paths and `ClientPid`s are checked at construction; liveness is probed at route time.
        Message::RepositoryStatus { .. }
        | Message::LanguageDetect { .. }
        | Message::DirectoryRequest { .. }
        | Message::ClockRequest { .. }
        | Message::DurationRequest { .. }
        | Message::CharacterRequest { .. }
        | Message::ExitStatusRequest { .. }
        | Message::RegisterClient { .. }
        | Message::UnregisterClient { .. }
        | Message::WorkspaceUpdate { .. }
        | Message::Ping
        | Message::Shutdown
        | Message::Status
        | Message::ConfigReload => Ok(()),
    }
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

/// IPC protocol version, reported as `protocol_version` in the status reply.
///
/// Shells restart an agent whose version differs; `SCHEMA_EVOLUTION.md` says when to bump it.
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

/// Flat shell-side message format keyed by `op`; absent flags read as `false`.
#[derive(Debug, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is a distinct wire flag of the flat protocol"
)]
struct ShellIpcMessage {
    op: String,
    cwd: Option<String>,
    pid: Option<u32>,
    format: Option<String>,
    #[serde(default)]
    is_last: bool,
    #[serde(default)]
    is_first: bool,
    shell: Option<String>,
    shell_version: Option<String>,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    success: bool,
    prev_bg: Option<String>,
    hostname: Option<String>,
    #[serde(default)]
    is_ssh: bool,
    username: Option<String>,
    #[serde(default)]
    is_root: bool,
    virtual_env: Option<String>,
}

impl ShellIpcMessage {
    /// Build the [`Message`] for `op`. Missing required fields are reported
    /// before any path is canonicalized or PID range-checked.
    fn into_message(self, format: Option<Format>) -> Result<Message> {
        // Unlike `Format::from_str`, a bad format defaults to JSON: shell IPC is best-effort.
        let segment = Segment {
            format: format.unwrap_or(Format::Json),
            is_last: self.is_last,
            is_first: self.is_first,
            prev_bg: self.prev_bg,
        };
        let cwd = self.cwd.as_deref().unwrap_or(".");

        match self.op.as_str() {
            "git" => Ok(Message::RepositoryStatus {
                path: safe_path(cwd, "path")?,
                segment,
            }),
            "lang" => Ok(Message::LanguageDetect {
                path: safe_path(cwd, "path")?,
                segment,
                virtual_env: self.virtual_env.map(validate_virtual_env).transpose()?,
            }),
            "directory" => Ok(Message::DirectoryRequest {
                path: safe_path(cwd, "path")?,
                display_path: self.cwd.unwrap_or_else(|| ".".to_owned()),
                segment,
            }),
            "clock" => Ok(Message::ClockRequest {
                // Unknown shell falls back to Fish, whose render is a no-op, rather
                // than leaking another shell's prompt escapes as literal text.
                shell: parse_shell(self.shell.as_deref()).unwrap_or(crate::shell::Shell::Fish),
                segment,
            }),
            "duration" => Ok(Message::DurationRequest {
                duration_ms: self.duration_ms,
                segment,
            }),
            "character" => Ok(Message::CharacterRequest {
                success: self.success,
                segment,
            }),
            "hostname" => Ok(Message::HostnameRequest {
                hostname: self.hostname.unwrap_or_default(),
                is_ssh: self.is_ssh,
                segment,
            }),
            "username" => Ok(Message::UsernameRequest {
                username: self.username.unwrap_or_default(),
                is_root: self.is_root,
                segment,
            }),
            // Not "status": that op is the agent health check.
            "exit_status" => Ok(Message::ExitStatusRequest {
                success: self.success,
                segment,
            }),
            "ping" => Ok(Message::Ping),
            "status" => Ok(Message::Status),
            "shutdown" => Ok(Message::Shutdown),
            "config_reload" => Ok(Message::ConfigReload),
            "register" => {
                let pid = self
                    .pid
                    .ok_or_else(|| Error::ipc("register missing pid".to_owned()))?;
                // `cwd` before `pid`, so a both-invalid input reports "Invalid cwd".
                let register_cwd = self
                    .cwd
                    .as_deref()
                    .map(|path| safe_path(path, "cwd"))
                    .transpose()?;
                Ok(Message::RegisterClient {
                    pid: client_pid(pid)?,
                    cwd: register_cwd,
                    shell: parse_shell(self.shell.as_deref()),
                    shell_version: self.shell_version,
                })
            }
            "unregister" => {
                let pid = self
                    .pid
                    .ok_or_else(|| Error::ipc("unregister missing pid".to_owned()))?;
                Ok(Message::UnregisterClient {
                    pid: client_pid(pid)?,
                })
            }
            "workspace" => {
                let pid = self
                    .pid
                    .ok_or_else(|| Error::ipc("Workspace update missing pid".to_owned()))?;
                let workspace_cwd = self
                    .cwd
                    .as_deref()
                    .ok_or_else(|| Error::ipc("Workspace update missing cwd".to_owned()))?;
                Ok(Message::WorkspaceUpdate {
                    pid: client_pid(pid)?,
                    cwd: safe_path(workspace_cwd, "workspace path")?,
                })
            }
            _ => Err(Error::ipc(format!("Unknown shell operation: {}", self.op))),
        }
    }
}

/// Canonicalize and validate a client path; `field` names it in the error.
fn safe_path(path: &str, field: &str) -> Result<SafePath> {
    SafePath::new(path).map_err(|e| Error::ipc(format!("Invalid {field}: {e}")))
}

fn client_pid(pid: u32) -> Result<super::ClientPid> {
    super::ClientPid::new(pid).map_err(|e| Error::ipc(format!("Invalid PID: {e}")))
}

/// A shell name, or `None` when unrecognized.
fn parse_shell(name: Option<&str>) -> Option<crate::shell::Shell> {
    name?.parse().ok()
}

fn validate_virtual_env(venv: String) -> Result<String> {
    crate::security::PathValidator::validate_path(&venv)
        .map_err(|e| Error::ipc(format!("Invalid virtual_env: {e}")))?;
    Ok(venv)
}

#[cfg(test)]
mod tests;
