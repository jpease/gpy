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
            Message::DirectoryRequest { path, .. } => {
                let cwd = path.to_string();
                let read_only = crate::fs_util::directory_is_read_only(std::path::Path::new(&cwd));
                Ok(Response::Directory { cwd, read_only })
            }
            Message::ClockRequest { shell, .. } => Ok(Response::Clock { shell: *shell }),
            Message::DurationRequest { duration_ms, .. } => Ok(Response::Duration {
                duration_ms: *duration_ms,
            }),
            Message::CharacterRequest { success, .. } => {
                Ok(Response::Character { success: *success })
            }
            Message::HostnameRequest { hostname, .. } => Ok(Response::Hostname {
                hostname: hostname.clone(),
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
        // canonicalizes to `\\?\D:\tmp` on Windows (#482), so the suffix
        // assertion below was unconditionally wrong there regardless of
        // whether a `D:\tmp` happened to exist on the runner image. A real,
        // platform-appropriate temp directory exercises the handler without
        // asserting anything about the host's temp-directory layout.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let temp_path = temp_dir
            .path()
            .canonicalize()
            .expect("canonicalize temp dir");
        let handler = MiscHandler::new();
        let path = crate::security::SafePath::new(
            temp_path
                .to_str()
                .expect("temp dir path must be valid UTF-8"),
        )
        .expect("valid path");
        let msg = Message::DirectoryRequest {
            path,
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        };
        let response = handler.handle(&msg).expect("handling must succeed");
        match response {
            Response::Directory { cwd, .. } => assert_eq!(
                cwd,
                temp_path.to_string_lossy(),
                "expected cwd to resolve to the temp dir's own (canonicalized) path"
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
