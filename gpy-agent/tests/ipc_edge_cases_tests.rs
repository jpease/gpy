//! IPC protocol edge cases.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use gpy_agent::ipc::Message;
use gpy_agent::ipc::protocol::deserialize_message;
use gpy_agent::security::SafePath;

#[test]
fn test_path_with_null_bytes() {
    let path_with_nul = "/tmp/test\0/bad";

    let result = SafePath::new(path_with_nul);
    assert!(result.is_err(), "Paths with NUL bytes should be rejected");
    assert!(
        result.unwrap_err().to_string().contains("null byte"),
        "Error should mention null byte"
    );
}

#[test]
fn test_very_long_path() {
    let long_component = "a".repeat(255); // Max component length
    let path = format!("/{long_component}/{long_component}/{long_component}/{long_component}");

    let json = serde_json::json!({"op": "git", "cwd": path}).to_string();
    let parsed = deserialize_message(json.as_bytes()).expect("Should deserialize");

    match parsed {
        Message::RepositoryStatus { path: p, .. } => {
            assert_eq!(p, path, "Path should round-trip correctly");
        }
        _ => panic!("Wrong message type after deserialization"),
    }
}

#[test]
fn test_large_message_near_limit() {
    let large_path = "x".repeat(4000);

    let json = serde_json::json!({"op": "git", "cwd": large_path}).to_string();

    let parsed = deserialize_message(json.as_bytes()).expect("Should deserialize");
    match parsed {
        Message::RepositoryStatus { path: p, .. } => {
            assert_eq!(p, large_path);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_message_exceeding_size_limit() {
    let oversized_path = "x".repeat(64 * 1024);
    let json = serde_json::json!({"op": "git", "cwd": oversized_path}).to_string();

    let result = deserialize_message(json.as_bytes());
    assert!(result.is_err(), "Oversized messages should be rejected");
}

#[test]
fn test_path_exceeding_length_cap() {
    // SafePath caps paths at 4096 chars.
    let oversized_path = "x".repeat(5000);

    let result = SafePath::new(&oversized_path);
    assert!(result.is_err(), "Overly long paths should be rejected");
}

#[test]
fn test_incomplete_json_message() {
    let incomplete_json = r#"{"op":"repository_status","path":"/tmp","#;

    let result = deserialize_message(incomplete_json.as_bytes());
    assert!(
        result.is_err(),
        "Incomplete JSON should fail to deserialize"
    );
}

#[test]
fn test_json_with_unexpected_fields() {
    let json_with_extra =
        r#"{"op":"git","cwd":"/tmp","format":"json","is_last":false,"unexpected_field":"x"}"#;

    let result = deserialize_message(json_with_extra.as_bytes());
    // Extra fields are ignored for forward compatibility.
    assert!(
        result.is_ok(),
        "Should handle extra fields gracefully (forward compatibility). Error: {:?}",
        result.err()
    );
}

#[test]
fn test_all_message_types_parse_in_op_form() {
    let pid = std::process::id();
    let parse = |line: String| deserialize_message(line.as_bytes()).expect("Should deserialize");
    assert!(matches!(
        parse(format!(r#"{{"op":"register","pid":{pid},"cwd":"/tmp"}}"#)),
        Message::RegisterClient { .. }
    ));
    assert!(matches!(
        parse(format!(r#"{{"op":"unregister","pid":{pid}}}"#)),
        Message::UnregisterClient { .. }
    ));
    assert!(matches!(
        parse(format!(r#"{{"op":"workspace","pid":{pid},"cwd":"/tmp"}}"#)),
        Message::WorkspaceUpdate { .. }
    ));
    assert!(matches!(
        parse(r#"{"op":"git","cwd":"/tmp"}"#.to_owned()),
        Message::RepositoryStatus { .. }
    ));
    assert!(matches!(
        parse(r#"{"op":"lang","cwd":"/tmp"}"#.to_owned()),
        Message::LanguageDetect { .. }
    ));
    assert!(matches!(
        parse(r#"{"op":"status"}"#.to_owned()),
        Message::Status
    ));
}
