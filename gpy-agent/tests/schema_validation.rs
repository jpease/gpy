//! Schema validation tests
//!
//! This test suite validates that all JSON examples in the documentation
//! can be successfully serialized/deserialized and match the protocol schemas.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::unwrap_used)]

use gpy_agent::config::types::ClientPid;
use gpy_agent::ipc::{Message, Response};
use gpy_agent::security::SafePath;

// ── Message deserialization — Fish shell wire format ──────────────────────────

#[test]
fn test_msg_deserialize_repository_status_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let cases: &[(&[u8], &str)] = &[
        (br#"{"op":"git","cwd":"."}"#, "RepositoryStatus (minimal)"),
        (
            br#"{"op":"git","cwd":"/home/user/repo","format":"ansi","is_last":false}"#,
            "RepositoryStatus (full)",
        ),
    ];
    for (input, label) in cases {
        let result = deserialize_message(input);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_msg_deserialize_language_detect_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let cases: &[(&[u8], &str)] = &[
        (br#"{"op":"lang","cwd":"."}"#, "LanguageDetect (minimal)"),
        (
            br#"{"op":"lang","cwd":"/home/user/project","format":"fish-source","is_last":true}"#,
            "LanguageDetect (full)",
        ),
    ];
    for (input, label) in cases {
        let result = deserialize_message(input);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_msg_deserialize_client_registration_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let pid = std::process::id();

    {
        let json = format!(r#"{{"op":"register","pid":{pid},"cwd":"/home/user/project"}}"#);
        let result = deserialize_message(json.as_bytes());
        assert!(
            result.is_ok(),
            "Failed to deserialize RegisterClient (with cwd): {:?}",
            result.err()
        );
    }
    {
        let json = format!(r#"{{"op":"register","pid":{pid}}}"#);
        let result = deserialize_message(json.as_bytes());
        assert!(
            result.is_ok(),
            "Failed to deserialize RegisterClient (without cwd): {:?}",
            result.err()
        );
    }
    {
        let json = format!(r#"{{"op":"unregister","pid":{pid}}}"#);
        let result = deserialize_message(json.as_bytes());
        assert!(
            result.is_ok(),
            "Failed to deserialize UnregisterClient: {:?}",
            result.err()
        );
    }
    {
        let json = format!(r#"{{"op":"workspace","pid":{pid},"cwd":"/home/user/new-project"}}"#);
        let result = deserialize_message(json.as_bytes());
        assert!(
            result.is_ok(),
            "Failed to deserialize WorkspaceUpdate: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_msg_deserialize_simple_ops_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let cases: &[(&[u8], &str)] = &[
        (br#"{"op":"ping"}"#, "Ping"),
        (br#"{"op":"status"}"#, "Status"),
        (br#"{"op":"theme","key":"prompt_close"}"#, "ThemeQuery"),
        (br#"{"op":"shutdown"}"#, "Shutdown"),
        (br#"{"op":"config_reload"}"#, "ConfigReload"),
    ];
    for (input, label) in cases {
        let result = deserialize_message(input);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

// ── Response deserialization ──────────────────────────────────────────────────

#[test]
fn test_response_deserialize_repository_status() {
    let cases: &[(&str, &str)] = &[
        (
            r#"{"RepositoryStatus":{"branch":"main","ahead":2,"behind":0,"ahead_capped":false,"behind_capped":false,"staged":3,"unstaged":1,"untracked":2,"conflicts":0,"state":"clean"}}"#,
            "RepositoryStatus (with changes)",
        ),
        (
            r#"{"RepositoryStatus":{"branch":"feature/new-feature","ahead":0,"behind":0,"ahead_capped":false,"behind_capped":false,"staged":0,"unstaged":0,"untracked":0,"conflicts":0,"state":"clean"}}"#,
            "RepositoryStatus (clean)",
        ),
        (
            r#"{"RepositoryStatus":{"branch":"feature/complex-merge","ahead":0,"behind":0,"ahead_capped":false,"behind_capped":false,"staged":0,"unstaged":2,"untracked":0,"conflicts":1,"state":"rebasing"}}"#,
            "RepositoryStatus (rebase)",
        ),
    ];
    for (json, label) in cases {
        let result: Result<Response, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_response_deserialize_language() {
    let cases: &[(&str, &str)] = &[
        (
            r##"{"Language":{"languages":[{"name":"Rust","version":"1.70.0","color":"#dea584"}]}}"##,
            "Language (single)",
        ),
        (
            r##"{"Language":{"languages":[{"name":"JavaScript","version":"20.10.0","color":"#f1e05a"},{"name":"TypeScript","version":"5.3.3","color":"#3178c6"}]}}"##,
            "Language (multiple)",
        ),
        (r#"{"Language":{"languages":[]}}"#, "Language (none)"),
    ];
    for (json, label) in cases {
        let result: Result<Response, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_response_deserialize_simple_variants() {
    let cases: &[(&str, &str)] = &[
        (r#""Ack""#, "Ack"),
        (
            r#"{"AgentStatus":{"version":"0.1.0","protocol_version":1,"watched_repos":5,"registered_clients":3,"cache_entries":12}}"#,
            "AgentStatus",
        ),
        (r#"{"ThemeValue":{"value":""}}"#, "ThemeValue (delimiter)"),
        (
            r##"{"ThemeValue":{"value":"#ff6b6b"}}"##,
            "ThemeValue (color)",
        ),
        (
            r#"{"Error":{"message":"Failed to detect language: directory not found"}}"#,
            "Error (generic)",
        ),
        (
            r#"{"Error":{"message":"IPC connection failed: socket timeout"}}"#,
            "Error (IPC)",
        ),
    ];
    for (json, label) in cases {
        let result: Result<Response, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

// ── Round-trip tests ──────────────────────────────────────────────────────────

#[test]
fn test_message_roundtrip() {
    let pid_val = std::process::id();
    let pid = ClientPid::new(pid_val).unwrap();

    let messages = vec![
        Message::RepositoryStatus {
            path: SafePath::new("/home/user/repo").unwrap(),
            format: gpy_agent::formatter::Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        },
        Message::LanguageDetect {
            path: SafePath::new("/home/user/project").unwrap(),
            format: gpy_agent::formatter::Format::FishSource,
            is_last: true,
            is_first: false,
            prev_bg: None,
            virtual_env: None,
        },
        Message::RegisterClient {
            pid,
            cwd: Some(SafePath::new("/home/user/project").unwrap()),
            shell: None,
            shell_version: None,
        },
        Message::UnregisterClient { pid },
        Message::WorkspaceUpdate {
            pid,
            cwd: SafePath::new("/home/user/new-project").unwrap(),
        },
        Message::Ping,
        Message::Status,
        Message::ThemeQuery {
            key: "prompt_close".to_owned(),
        },
        Message::Shutdown,
        Message::ConfigReload,
    ];

    for msg in messages {
        let json = serde_json::to_string(&msg).expect("Failed to serialize message");
        let deserialized: Message =
            serde_json::from_str(&json).expect("Failed to deserialize message");
        let json2 = serde_json::to_string(&deserialized).expect("Failed to re-serialize message");
        assert_eq!(json, json2, "Roundtrip mismatch for message: {msg:?}");
    }
}

#[test]
fn test_response_roundtrip() {
    let responses = vec![
        Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 2,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 3,
            unstaged: 1,
            untracked: 2,
            conflicts: 0,
            state: gpy_agent::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }),
        Response::Language {
            languages: vec![gpy_agent::ipc::LanguageInfo {
                name: "Rust".to_owned(),
                version: Some("1.70.0".to_owned()),
                color: gpy_agent::config::types::ColorSpec::new("#dea584").unwrap(),
            }],
        },
        Response::Ack,
        Response::AgentStatus {
            version: "0.1.0".to_owned(),
            protocol_version: 1,
            watched_repos: 5,
            registered_clients: 3,
            cache_entries: 12,
        },
        Response::ThemeValue {
            value: String::new(),
        },
        Response::Error {
            message: "Test error".to_owned(),
        },
    ];

    for resp in responses {
        let json = serde_json::to_string(&resp).expect("Failed to serialize response");
        let deserialized: Response =
            serde_json::from_str(&json).expect("Failed to deserialize response");
        let json2 = serde_json::to_string(&deserialized).expect("Failed to re-serialize response");
        assert_eq!(json, json2, "Roundtrip mismatch for response: {resp:?}");
    }
}

// ── Protocol validation ───────────────────────────────────────────────────────

#[test]
fn test_protocol_validation_catches_errors() {
    use gpy_agent::ipc::protocol::deserialize_message;

    // Above ClientPid::MAX (4_194_304): rejected at parse time by range
    // validation. 2_000_000 used to fail here too, but only because
    // `ClientPid::new` ran a liveness probe at construction; #578 moved
    // liveness checking to route time (`route_request_secure`), so
    // deserialization alone no longer rejects an in-range-but-dead PID.
    let result = deserialize_message(br#"{"op":"register","pid":5000000}"#);
    assert!(result.is_err(), "Should reject out-of-range PID");

    let long_path = "a".repeat(5000);
    let json = format!(r#"{{"op":"git","cwd":"{long_path}"}}"#);
    let result_long = deserialize_message(json.as_bytes());
    assert!(result_long.is_err(), "Should reject overly long path");

    let result_missing = deserialize_message(br#"{"op":"workspace","pid":12345}"#);
    assert!(
        result_missing.is_err(),
        "Should reject missing required field"
    );

    let result_unknown = deserialize_message(br#"{"op":"invalid_operation"}"#);
    assert!(result_unknown.is_err(), "Should reject unknown operation");
}
