//! IPC Protocol Edge Case Tests
//!
//! Tests for unusual IPC messages and edge cases in protocol handling.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::str_to_string)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

mod fixtures;

use fixtures::ipc_helpers::*;
use gpy_agent::config::types::ClientPid;
use gpy_agent::ipc::{Format, Message, Response};
use gpy_agent::security::SafePath;

/// Test: Message with path containing NUL bytes (should be rejected)
#[test]
fn test_path_with_null_bytes() {
    let path_with_nul = "/tmp/test\0/bad";

    // Validation should reject this during construction
    let result = SafePath::new(path_with_nul);
    assert!(result.is_err(), "Paths with NUL bytes should be rejected");
    assert!(
        result.unwrap_err().to_string().contains("null byte"),
        "Error should mention null byte"
    );
}

/// Test: Very long path (but valid)
#[test]
fn test_very_long_path() {
    // Create a path near the system limit (usually 4096 bytes)
    let long_component = "a".repeat(255); // Max component length
    let path = format!("/{long_component}/{long_component}/{long_component}/{long_component}");

    let msg = Message::RepositoryStatus {
        path: SafePath::new(&path).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };

    // Should be able to serialize/deserialize without error
    let json = serde_json::to_string(&msg).expect("Should serialize");
    let parsed: Message = serde_json::from_str(&json).expect("Should deserialize");

    match parsed {
        Message::RepositoryStatus { path: p, .. } => {
            assert_eq!(p, path, "Path should round-trip correctly");
        }
        _ => panic!("Wrong message type after deserialization"),
    }
}

/// Test: Message size approaching path limit (4KB)
#[test]
fn test_large_message_near_limit() {
    // Create a path that is large (near the 4096 byte limit)
    let large_path = "x".repeat(4000); // 4KB path

    let msg = Message::RepositoryStatus {
        path: SafePath::new(&large_path).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };

    let json = serde_json::to_string(&msg).expect("Should serialize");
    assert!(
        json.len() < 65536,
        "Message should be under 64KB limit, was {} bytes",
        json.len()
    );

    // Should deserialize correctly
    let parsed: Message = serde_json::from_str(&json).expect("Should deserialize");
    match parsed {
        Message::RepositoryStatus { path: p, .. } => {
            // SafePath doesn't have .len(), use .as_path().to_string_lossy().len() or similar
            assert_eq!(p.to_string().len(), large_path.len());
        }
        _ => panic!("Wrong message type"),
    }
}

/// Test: Message exceeding size limit (should fail gracefully)
#[test]
fn test_message_exceeding_size_limit() {
    // Create a path that would exceed the 4096 char limit for SafePath
    let oversized_path = "x".repeat(5000);

    let result = SafePath::new(&oversized_path);
    assert!(result.is_err(), "Overly long paths should be rejected");
}

/// Test: Malformed JSON (incomplete message)
#[test]
fn test_incomplete_json_message() {
    let incomplete_json = r#"{"op":"repository_status","path":"/tmp","#;

    let result = serde_json::from_str::<Message>(incomplete_json);
    assert!(
        result.is_err(),
        "Incomplete JSON should fail to deserialize"
    );
}

/// Test: JSON with unexpected fields (should ignore or error)
#[test]
fn test_json_with_unexpected_fields() {
    let json_with_extra = r#"{
        "RepositoryStatus": {
            "path": "/tmp",
            "format": "json",
            "is_last": false,
            "unexpected_field": "should be ignored"
        }
    }"#;

    // Serde should either ignore extra fields or error (depending on settings)
    let result = serde_json::from_str::<Message>(json_with_extra);
    // We want to ignore extra fields for forward compatibility
    assert!(
        result.is_ok(),
        "Should handle extra fields gracefully (forward compatibility). Error: {:?}",
        result.err()
    );
}

/// Test: Valid message types
#[test]
fn test_all_message_types_serialize() {
    let pid_val = std::process::id();
    let pid = ClientPid::new(pid_val).unwrap();
    let messages = vec![
        Message::RegisterClient {
            pid,
            cwd: Some(SafePath::new("/tmp").unwrap()),
            shell: None,
            shell_version: None,
        },
        Message::UnregisterClient { pid },
        Message::WorkspaceUpdate {
            pid,
            cwd: SafePath::new("/tmp").unwrap(),
        },
        Message::RepositoryStatus {
            path: SafePath::new("/tmp").unwrap(),
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        },
        Message::LanguageDetect {
            path: SafePath::new("/tmp").unwrap(),
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
            virtual_env: None,
        },
        Message::Status,
        Message::ThemeQuery {
            key: "git.branch".to_string(),
        },
    ];

    for msg in messages {
        let json = serde_json::to_string(&msg).expect("Should serialize");
        let parsed: Message = serde_json::from_str(&json).expect("Should deserialize");

        // Verify round-trip
        let json2 = serde_json::to_string(&parsed).expect("Should re-serialize");
        assert_eq!(json, json2, "Messages should round-trip identically");
    }
}

/// Test: Response error message validation
#[test]
fn test_response_error_messages() {
    let error_response = Response::Error {
        message: "Test error message".to_string(),
    };

    assert!(is_error_response(&error_response));
    assert_eq!(
        extract_error_message(&error_response),
        Some("Test error message")
    );
}

/// Test: `Response::Ack` validation
#[test]
fn test_response_ack_validation() {
    let ack = Response::Ack;
    assert!(is_ack_response(&ack));
    assert!(!is_error_response(&ack));
}

/// Test: Protocol version mismatch handling
#[test]
#[allow(clippy::assertions_on_constants)]
fn test_protocol_version_field() {
    use gpy_agent::ipc::protocol::PROTOCOL_VERSION;

    // Protocol version should be a positive integer
    assert!(PROTOCOL_VERSION > 0, "Protocol version should be positive");

    // Agent status response should include protocol version
    let status = Response::AgentStatus {
        version: "1.0.0".to_string(),
        protocol_version: PROTOCOL_VERSION,
        watched_repos: 0,
        registered_clients: 0,
        cache_entries: 0,
    };

    let json = serde_json::to_string(&status).expect("Should serialize");
    assert!(
        json.contains("protocol_version"),
        "Status response should include protocol version"
    );
}
