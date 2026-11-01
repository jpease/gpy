//! Unit tests for IPC protocol serialization/deserialization

use gpy_agent::ipc::protocol::{deserialize_message, validate_message_size};
use gpy_agent::ipc::{Format, Message};
use gpy_agent::security::SafePath;

#[test]
fn lang_request_without_virtual_env_still_parses() {
    // Back-compat: old shell clients omit the field entirely.
    let msg = deserialize_message(br#"{"op":"lang","cwd":"."}"#).unwrap();
    match msg {
        Message::LanguageDetect { virtual_env, .. } => assert!(virtual_env.is_none()),
        other => panic!("expected LanguageDetect, got {other:?}"),
    }
}

#[test]
fn lang_request_carries_virtual_env() {
    let msg =
        deserialize_message(br#"{"op":"lang","cwd":".","virtual_env":"/proj/.venv"}"#).unwrap();
    match msg {
        Message::LanguageDetect { virtual_env, .. } => {
            assert_eq!(virtual_env.as_deref(), Some("/proj/.venv"));
        }
        other => panic!("expected LanguageDetect, got {other:?}"),
    }
}

#[test]
fn test_shell_ipc_format_git() {
    let json = r#"{"op":"git","cwd":"/test/path"}"#;
    let deserialized = deserialize_message(json.as_bytes()).expect("deserialize_message failed");

    match deserialized {
        Message::RepositoryStatus { path, segment } => {
            assert_eq!(path, "/test/path");
            assert_eq!(segment.format, Format::Json);
        }
        _ => assert!(
            matches!(deserialized, Message::RepositoryStatus { .. }),
            "Wrong message type deserialized from wire format"
        ),
    }
}

#[test]
fn test_shell_ipc_format_lang() {
    let json = r#"{"op":"lang","cwd":"/project"}"#;
    let deserialized = deserialize_message(json.as_bytes()).expect("deserialize_message failed");

    match deserialized {
        Message::LanguageDetect { path, segment, .. } => {
            assert_eq!(path, "/project");
            assert_eq!(segment.format, Format::Json);
        }
        _ => assert!(
            matches!(deserialized, Message::LanguageDetect { .. }),
            "Wrong message type deserialized from wire format"
        ),
    }
}

#[test]
fn test_shell_ipc_format_register() {
    let pid = std::process::id();
    let json = format!(r#"{{"op":"register","pid":{pid},"cwd":"/tmp"}}"#);
    let deserialized = deserialize_message(json.as_bytes()).expect("deserialize_message failed");

    match deserialized {
        Message::RegisterClient { pid: p, cwd, .. } => {
            assert_eq!(p, pid);
            assert_eq!(cwd.unwrap(), SafePath::new("/tmp").unwrap());
        }
        _ => assert!(
            matches!(deserialized, Message::RegisterClient { .. }),
            "Wrong message type deserialized from wire format"
        ),
    }
}

#[test]
fn test_shell_ipc_format_workspace() {
    let pid = std::process::id();
    let json = format!(r#"{{"op":"workspace","pid":{pid},"cwd":"/repo"}}"#);
    let deserialized = deserialize_message(json.as_bytes()).expect("deserialize_message failed");

    match deserialized {
        Message::WorkspaceUpdate { pid: p, cwd } => {
            assert_eq!(p, pid);
            assert_eq!(cwd, SafePath::new("/repo").unwrap());
        }
        _ => assert!(
            matches!(deserialized, Message::WorkspaceUpdate { .. }),
            "Wrong message type deserialized from wire workspace format"
        ),
    }
}

#[test]
fn test_shell_ipc_format_ping() {
    let json = r#"{"op":"ping"}"#;
    let deserialized = deserialize_message(json.as_bytes()).expect("deserialize_message failed");

    match deserialized {
        Message::Ping => {
            // Success - correct message type
        }
        _ => assert!(
            matches!(deserialized, Message::Ping),
            "Wrong message type deserialized from wire format"
        ),
    }
}

#[test]
fn test_message_size_validation_valid() {
    let small_data = b"hello world";
    validate_message_size(small_data).expect("small message should be valid size");

    let medium_data = vec![0u8; 32 * 1024]; // 32KB
    validate_message_size(&medium_data).expect("medium message should be valid size");
}

#[test]
fn test_message_size_validation_too_large() {
    let large_data = vec![0u8; 128 * 1024]; // 128KB - exceeds 64KB limit
    assert!(validate_message_size(&large_data).is_err());
}

#[test]
fn test_invalid_json_deserialization() {
    let invalid_json = b"not valid json at all";
    let invalid_result = deserialize_message(invalid_json);
    assert!(invalid_result.is_err());

    let partial_json = br#"{"op":"git","cwd""#;
    let partial_result = deserialize_message(partial_json);
    assert!(partial_result.is_err());
}

#[test]
fn test_unknown_operation_wire_format() {
    let json = r#"{"op":"unknown_operation","cwd":"/test"}"#;
    let unknown_result = deserialize_message(json.as_bytes());
    assert!(unknown_result.is_err());
}

#[test]
fn test_missing_required_fields() {
    // Missing cwd for git operation
    let git_json = r#"{"op":"git"}"#;
    let git_result = deserialize_message(git_json.as_bytes());
    let Ok(Message::RepositoryStatus { path, segment }) = git_result else {
        panic!("missing cwd must deserialize to RepositoryStatus with default path");
    };
    assert_eq!(path, SafePath::new(".").unwrap());
    assert_eq!(segment.format, Format::Json);

    // Missing pid for register operation must be rejected
    let register_json = r#"{"op":"register"}"#;
    let register_result = deserialize_message(register_json.as_bytes());
    assert!(
        register_result.is_err(),
        "register without pid should be rejected"
    );
}

#[test]
fn test_json_string_escaping() {
    let path_with_quotes = r#"/path/with"quotes"/and\backslashes"#;
    let line = serde_json::json!({"op": "git", "cwd": path_with_quotes}).to_string();
    let deserialized = deserialize_message(line.as_bytes()).expect("deserialize_message failed");

    match deserialized {
        Message::RepositoryStatus { path, segment } => {
            assert_eq!(path, path_with_quotes);
            assert_eq!(segment.format, Format::Json);
        }
        _ => assert!(
            matches!(deserialized, Message::RepositoryStatus { .. }),
            "Wrong message type deserialized"
        ),
    }
}
