//! IPC handler for small, self-contained segment requests.
//!
//! `DirectoryRequest`, `DurationRequest`, `CharacterRequest`, `HostnameRequest`,
//! and `UsernameRequest` all carry the data their response needs directly on
//! the request (the client already resolved cwd/duration/exit-status/hostname/
//! username); the agent's only job is to echo it back as the matching
//! [`Response`] variant. `DirectoryRequest` additionally checks whether the
//! path is writable via a `stat` call.
//!
//! These previously lived inline in [`super::HandlerRegistry::route`], sitting
//! directly on the routing hot path alongside the `Arc<dyn RequestHandler>`
//! dispatch for git/language/theme/client. Giving them their own handler keeps
//! `route` a pure variant-to-handler dispatch table.

use super::{HandlerError, RequestHandler};
use crate::ipc::{Message, Response};

/// Handler for echo-like segment requests that carry their own response data.
pub struct MiscHandler;

impl MiscHandler {
    /// Create a new `MiscHandler`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for MiscHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestHandler for MiscHandler {
    fn handle(&self, message: &Message) -> Result<Response, HandlerError> {
        match message {
            Message::DirectoryRequest {
                path, display_path, ..
            } => {
                let canonical = path.to_string();
                let read_only =
                    crate::fs_util::directory_is_read_only(std::path::Path::new(&canonical));
                Ok(Response::Directory {
                    cwd: display_path.clone(),
                    read_only,
                })
            }
            Message::ClockRequest { shell, .. } => Ok(Response::Clock { shell: *shell }),
            Message::DurationRequest { duration_ms, .. } => Ok(Response::Duration {
                duration_ms: *duration_ms,
            }),
            Message::CharacterRequest { success, .. } => {
                Ok(Response::Character { success: *success })
            }
            Message::HostnameRequest {
                hostname, is_ssh, ..
            } => Ok(Response::Hostname {
                hostname: hostname.clone(),
                is_ssh: *is_ssh,
            }),
            Message::UsernameRequest { username, .. } => Ok(Response::Username {
                username: username.clone(),
            }),
            _ => Err(HandlerError::UnexpectedMessage {
                handler: "MiscHandler",
                qualifier: "unsupported",
                message: format!("{message:?}"),
            }),
        }
    }

    fn name(&self) -> &'static str {
        "MiscHandler"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use crate::formatter::Format;

    #[test]
    fn handles_duration_request() {
        let handler = MiscHandler::new();
        let msg = Message::DurationRequest {
            duration_ms: 65_000,
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        };
        let response = handler.handle(&msg).expect("handling must succeed");
        match response {
            Response::Duration { duration_ms } => assert_eq!(duration_ms, 65_000),
            other => panic!("expected Response::Duration, got {other:?}"),
        }
    }

    #[test]
    fn handles_character_request() {
        let handler = MiscHandler::new();
        let msg = Message::CharacterRequest {
            success: false,
            format: Format::Json,
            is_last: true,
            prev_bg: None,
        };
        let response = handler.handle(&msg).expect("handling must succeed");
        match response {
            Response::Character { success } => assert!(!success),
            other => panic!("expected Response::Character, got {other:?}"),
        }
    }

    #[test]
    fn handles_directory_request() {
        // A `tempfile::TempDir` rather than a hardcoded "/tmp": that literal
        // canonicalizes to `\\?\D:\tmp` on Windows (#482). The response
        // `cwd` is the requested path, not its canonical form (#697).
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let requested = temp_dir
            .path()
            .to_str()
            .expect("temp dir path must be valid UTF-8")
            .to_owned();
        let handler = MiscHandler::new();
        let path = crate::security::SafePath::new(&requested).expect("valid path");
        let msg = Message::DirectoryRequest {
            path,
            display_path: requested.clone(),
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        };
        let response = handler.handle(&msg).expect("handling must succeed");
        match response {
            Response::Directory { cwd, .. } => {
                assert_eq!(cwd, requested, "expected cwd to echo the requested path");
            }
            other => panic!("expected Response::Directory, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn directory_request_preserves_logical_symlink_path() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let target = temp_dir.path().join("real").join("target");
        std::fs::create_dir_all(&target).expect("create target");
        let link = temp_dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");
        let link_str = link.to_str().expect("link path must be valid UTF-8");
        let msg = crate::ipc::protocol::deserialize_message(
            format!(r#"{{"op":"directory","cwd":"{link_str}","format":"json"}}"#).as_bytes(),
        )
        .expect("directory request must parse");
        let response = MiscHandler::new()
            .handle(&msg)
            .expect("handling must succeed");
        match response {
            Response::Directory { cwd, .. } => assert_eq!(
                cwd, link_str,
                "cwd must be the logical (symlink) path, not the resolved target"
            ),
            other => panic!("expected Response::Directory, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unsupported_message() {
        let handler = MiscHandler::new();
        let response = handler.handle(&Message::Ping);
        assert!(response.is_err(), "MiscHandler must reject Message::Ping");
    }
}
