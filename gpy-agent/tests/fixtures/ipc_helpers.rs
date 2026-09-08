//! IPC Test Helpers
//!
//! Utilities for testing IPC communication and protocol handling.

#![allow(dead_code)]
#![allow(clippy::expect_used)]
#![allow(clippy::str_to_string)]
#![allow(clippy::missing_const_for_fn)]

use gpy_agent::config::types::ClientPid;
use gpy_agent::ipc::{Format, Message, Response};
use gpy_agent::security::SafePath;
use std::path::PathBuf;
use tempfile::TempDir;

/// Test IPC client for simulating client interactions
pub struct TestIpcClient {
    socket_dir: TempDir,
    socket_path: PathBuf,
}

impl TestIpcClient {
    /// Create a new test IPC client with a temporary socket
    pub fn new() -> Self {
        let socket_dir = TempDir::new().expect("Failed to create temp directory for socket");
        let socket_path = socket_dir.path().join("test.sock");

        Self {
            socket_dir,
            socket_path,
        }
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Create a `RegisterClient` message
    pub fn register_message(pid: u32, cwd: Option<String>) -> Message {
        Message::RegisterClient {
            pid: ClientPid::new(pid).expect("Valid test PID"),
            cwd: cwd.map(|s| SafePath::new(&s).expect("Valid test path")),
            shell: None,
            shell_version: None,
        }
    }

    /// Create an `UnregisterClient` message
    pub fn unregister_message(pid: u32) -> Message {
        Message::UnregisterClient {
            pid: ClientPid::new(pid).expect("Valid test PID"),
        }
    }

    /// Create a `RepositoryStatus` message
    pub fn git_status_message(path: &str) -> Message {
        Message::RepositoryStatus {
            path: SafePath::new(path).expect("Valid test path"),
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        }
    }

    /// Create a `LanguageDetect` message
    pub fn language_detection_message(path: &str) -> Message {
        Message::LanguageDetect {
            path: SafePath::new(path).expect("Valid test path"),
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
            virtual_env: None,
        }
    }

    /// Create a Status message
    pub fn status_message() -> Message {
        Message::Status
    }

    /// Create a `ThemeQuery` message
    pub fn theme_query_message(key: String) -> Message {
        Message::ThemeQuery { key }
    }
}

impl Default for TestIpcClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to validate Response types
pub fn is_error_response(response: &Response) -> bool {
    matches!(response, Response::Error { .. })
}

/// Helper to validate Response types
pub fn is_ack_response(response: &Response) -> bool {
    matches!(response, Response::Ack)
}

/// Helper to validate Response types
pub fn is_repository_status_response(response: &Response) -> bool {
    matches!(response, Response::RepositoryStatus(_))
}

/// Helper to extract error message from `Response::Error`
pub fn extract_error_message(response: &Response) -> Option<&str> {
    match response {
        Response::Error { message } => Some(message),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_ipc_client() {
        let client = TestIpcClient::new();
        assert!(
            client
                .socket_path()
                .to_str()
                .unwrap()
                .ends_with("test.sock")
        );
    }

    #[test]
    fn test_register_message() {
        let current_pid = std::process::id();
        let msg = TestIpcClient::register_message(current_pid, Some("/test/path".to_string()));
        if let Message::RegisterClient {
            pid: message_pid,
            cwd,
            ..
        } = msg
        {
            assert_eq!(message_pid, std::process::id());
            assert_eq!(cwd.unwrap(), SafePath::new("/test/path").unwrap());
        } else {
            panic!("Expected RegisterClient message");
        }
    }

    #[test]
    fn test_is_error_response() {
        let error = Response::Error {
            message: "Test error".to_string(),
        };
        assert!(is_error_response(&error));

        let ack = Response::Ack;
        assert!(!is_error_response(&ack));
    }

    #[test]
    fn test_extract_error_message() {
        let error = Response::Error {
            message: "Test error".to_string(),
        };
        assert_eq!(extract_error_message(&error), Some("Test error"));

        let ack = Response::Ack;
        assert_eq!(extract_error_message(&ack), None);
    }
}
