//! Unit tests for IPC protocol serialization/deserialization

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)] // Test functions panic on assertion failures

use gpy_agent::config::types::{ClientPid, ColorSpec};
use gpy_agent::ipc::protocol::{
    deserialize_message, deserialize_response, serialize_message, serialize_response,
    validate_message_size,
};
use gpy_agent::ipc::{Format, LanguageInfo, Message, Response};
use gpy_agent::security::SafePath;

#[test]
fn test_message_serialization_git_status() {
    let msg = Message::RepositoryStatus {
        path: SafePath::new("/test/path").unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };

    let serialized_res = serialize_message(&msg);
    assert!(serialized_res.is_ok(), "serialize_message failed");
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_message(&serialized);
    assert!(deserialized_res.is_ok(), "deserialize_message failed");
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    assert!(
        matches!(deserialized, Message::RepositoryStatus { .. }),
        "Wrong message type deserialized"
    );
    if let Message::RepositoryStatus { path, format, .. } = deserialized {
        assert_eq!(path, "/test/path");
        assert_eq!(format, Format::Json);
    }
}

#[test]
fn test_message_serialization_language_detect() {
    let msg = Message::LanguageDetect {
        path: SafePath::new("/project").unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
        virtual_env: None,
    };

    let serialized_res = serialize_message(&msg);
    assert!(serialized_res.is_ok());
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_message(&serialized);
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    assert!(matches!(deserialized, Message::LanguageDetect { .. }));
    if let Message::LanguageDetect { path, format, .. } = deserialized {
        assert_eq!(path, "/project");
        assert_eq!(format, Format::Json);
    }
}

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
fn test_message_serialization_register_client() {
    let pid_val = std::process::id();
    let msg = Message::RegisterClient {
        pid: ClientPid::new(pid_val).unwrap(),
        cwd: Some(SafePath::new("/tmp").unwrap()),
        shell: None,
        shell_version: None,
    };

    let serialized_res = serialize_message(&msg);
    assert!(serialized_res.is_ok());
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_message(&serialized);
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    assert!(matches!(deserialized, Message::RegisterClient { .. }));
    if let Message::RegisterClient { pid, cwd, .. } = deserialized {
        assert_eq!(pid, pid_val);
        assert_eq!(cwd.unwrap(), SafePath::new("/tmp").unwrap());
    }
}

#[test]
fn test_message_serialization_ping() {
    let msg = Message::Ping;

    let serialized_res = serialize_message(&msg);
    assert!(serialized_res.is_ok());
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_message(&serialized);
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    assert!(matches!(deserialized, Message::Ping));
}

#[test]
fn test_response_serialization_git_status() {
    let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 3,
        unstaged: 4,
        untracked: 5,
        conflicts: 0,
        state: gpy_agent::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    });

    let serialized_res = serialize_response(&response);
    assert!(serialized_res.is_ok());
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_response(&serialized);
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Response::RepositoryStatus(s) => {
            assert_eq!(s.branch, "main");
            assert_eq!(s.ahead, 2);
            assert_eq!(s.behind, 1);
            assert!(!s.ahead_capped);
            assert!(!s.behind_capped);
            assert_eq!(s.staged, 3);
            assert_eq!(s.unstaged, 4);
            assert_eq!(s.untracked, 5);
            assert_eq!(s.conflicts, 0);
            assert_eq!(s.state, gpy_agent::git::RepositoryState::Clean);
        }
        _ => assert!(
            matches!(deserialized, Response::RepositoryStatus { .. }),
            "Wrong response type deserialized"
        ),
    }
}

#[test]
fn test_response_serialization_language() {
    let response = Response::Language {
        languages: vec![
            LanguageInfo {
                name: "Rust".to_owned(),
                version: Some("1.70.0".to_owned()),
                color: ColorSpec::new("#ff0000").unwrap(),
            },
            LanguageInfo {
                name: "Node".to_owned(),
                version: Some("18.16.1".to_owned()),
                color: ColorSpec::new("#00ff00").unwrap(),
            },
        ],
    };

    let serialized_res = serialize_response(&response);
    assert!(serialized_res.is_ok());
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_response(&serialized);
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Response::Language { languages } => {
            assert_eq!(languages.len(), 2);
            if let Some(language) = languages.first() {
                assert_eq!(language.name, "Rust");
                assert_eq!(language.version, Some("1.70.0".to_owned()));
                assert_eq!(language.color, ColorSpec::new("#ff0000").unwrap());
            }
            if let Some(language) = languages.get(1) {
                assert_eq!(language.name, "Node");
                assert_eq!(language.version, Some("18.16.1".to_owned()));
                assert_eq!(language.color, ColorSpec::new("#00ff00").unwrap());
            }
        }
        _ => assert!(
            matches!(deserialized, Response::Language { .. }),
            "Wrong response type deserialized"
        ),
    }
}

#[test]
fn test_response_serialization_ack() {
    let response = Response::Ack;

    let serialized_res = serialize_response(&response);
    assert!(serialized_res.is_ok());
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_response(&serialized);
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Response::Ack => {
            // Success - correct response type
        }
        _ => assert!(
            matches!(deserialized, Response::Ack),
            "Wrong response type deserialized"
        ),
    }
}

#[test]
fn test_response_serialization_error() {
    let response = Response::Error {
        message: "Test error".to_owned(),
    };

    let serialized_res = serialize_response(&response);
    assert!(serialized_res.is_ok());
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_response(&serialized);
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Response::Error { message } => {
            assert_eq!(message, "Test error");
        }
        _ => assert!(
            matches!(deserialized, Response::Error { .. }),
            "Wrong response type deserialized"
        ),
    }
}

#[test]
fn test_fish_shell_ipc_format_git() {
    let json = r#"{"op":"git","cwd":"/test/path"}"#;
    let deserialized_res = deserialize_message(json.as_bytes());
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Message::RepositoryStatus { path, format, .. } => {
            assert_eq!(path, "/test/path");
            assert_eq!(format, Format::Json);
        }
        _ => assert!(
            matches!(deserialized, Message::RepositoryStatus { .. }),
            "Wrong message type deserialized from Fish format"
        ),
    }
}

#[test]
fn test_fish_shell_ipc_format_lang() {
    let json = r#"{"op":"lang","cwd":"/project"}"#;
    let deserialized_res = deserialize_message(json.as_bytes());
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Message::LanguageDetect { path, format, .. } => {
            assert_eq!(path, "/project");
            assert_eq!(format, Format::Json);
        }
        _ => assert!(
            matches!(deserialized, Message::LanguageDetect { .. }),
            "Wrong message type deserialized from Fish format"
        ),
    }
}

#[test]
fn test_fish_shell_ipc_format_register() {
    let pid = std::process::id();
    let json = format!(r#"{{"op":"register","pid":{pid},"cwd":"/tmp"}}"#);
    let deserialized_res = deserialize_message(json.as_bytes());
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Message::RegisterClient { pid: p, cwd, .. } => {
            assert_eq!(p, pid);
            assert_eq!(cwd.unwrap(), SafePath::new("/tmp").unwrap());
        }
        _ => assert!(
            matches!(deserialized, Message::RegisterClient { .. }),
            "Wrong message type deserialized from Fish format"
        ),
    }
}

#[test]
fn test_fish_shell_ipc_format_workspace() {
    let pid = std::process::id();
    let json = format!(r#"{{"op":"workspace","pid":{pid},"cwd":"/repo"}}"#);
    let deserialized_res = deserialize_message(json.as_bytes());
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Message::WorkspaceUpdate { pid: p, cwd } => {
            assert_eq!(p, pid);
            assert_eq!(cwd, SafePath::new("/repo").unwrap());
        }
        _ => assert!(
            matches!(deserialized, Message::WorkspaceUpdate { .. }),
            "Wrong message type deserialized from Fish workspace format"
        ),
    }
}

#[test]
fn test_fish_shell_ipc_format_ping() {
    let json = r#"{"op":"ping"}"#;
    let deserialized_res = deserialize_message(json.as_bytes());
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Message::Ping => {
            // Success - correct message type
        }
        _ => assert!(
            matches!(deserialized, Message::Ping),
            "Wrong message type deserialized from Fish format"
        ),
    }
}

#[test]
fn test_message_size_validation_valid() {
    let small_data = b"hello world";
    assert!(validate_message_size(small_data).is_ok());

    let medium_data = vec![0u8; 32 * 1024]; // 32KB
    assert!(validate_message_size(&medium_data).is_ok());
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
fn test_unknown_operation_fish_format() {
    let json = r#"{"op":"unknown_operation","cwd":"/test"}"#;
    let unknown_result = deserialize_message(json.as_bytes());
    assert!(unknown_result.is_err());
}

#[test]
fn test_missing_required_fields() {
    // Missing cwd for git operation
    let git_json = r#"{"op":"git"}"#;
    let git_result = deserialize_message(git_json.as_bytes());
    assert!(matches!(&git_result, Ok(Message::RepositoryStatus { .. })));
    if let Ok(Message::RepositoryStatus { path, format, .. }) = git_result {
        assert_eq!(path, SafePath::new(".").unwrap());
        assert_eq!(format, Format::Json);
    } else {
        return;
    }

    // Missing pid for register operation must now be rejected
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
    let msg = Message::RepositoryStatus {
        path: SafePath::new(path_with_quotes).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };

    let serialized_res = serialize_message(&msg);
    assert!(serialized_res.is_ok());
    let Ok(serialized) = serialized_res else {
        return;
    };
    let deserialized_res = deserialize_message(&serialized);
    assert!(deserialized_res.is_ok());
    let Ok(deserialized) = deserialized_res else {
        return;
    };

    match deserialized {
        Message::RepositoryStatus { path, format, .. } => {
            assert_eq!(path, path_with_quotes);
            assert_eq!(format, Format::Json);
        }
        _ => assert!(
            matches!(deserialized, Message::RepositoryStatus { .. }),
            "Wrong message type deserialized"
        ),
    }
}
