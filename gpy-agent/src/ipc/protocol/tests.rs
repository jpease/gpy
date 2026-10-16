#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(missing_docs)]

use super::*;
use crate::config::types::ShellVariant;
use crate::ipc::Format;
use serde_json;

#[test]
fn test_config_reload_json_deserializes_to_config_reload() {
    let msg = deserialize_message(CONFIG_RELOAD_JSON.as_bytes()).expect("deserialize failed");
    assert!(matches!(msg, Message::ConfigReload));
}

#[test]
fn test_prev_bg_deserialize_missing() {
    let json = r#"{"op":"git","cwd":"."}"#;
    let msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
    assert_eq!(msg.prev_bg, None);
}

#[test]
fn test_prev_bg_deserialize_with_value() {
    let json = r#"{"op":"git","cwd":".","prev_bg":"blue"}"#;
    let msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
    assert_eq!(msg.prev_bg, Some("blue".to_owned()));
}

#[test]
fn test_prev_bg_validation_too_long() {
    let long_color = "x".repeat(65_usize);
    let result = validate_string_field("prev_bg", &long_color, 64_usize);
    assert!(result.is_err());
}

#[test]
fn test_hostname_into_message_with_is_last_and_prev_bg() {
    let json = r#"{"op":"hostname","hostname":"myhost","format":"ansi","is_last":true,"prev_bg":"blue","is_ssh":true}"#;
    let shell_msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
    let wire = shell_msg
        .into_wire_message()
        .expect("into_wire_message failed");
    let msg = resolve(wire).expect("resolve failed");
    match msg {
        Message::HostnameRequest {
            hostname,
            format,
            is_last,
            is_ssh,
            prev_bg,
        } => {
            assert_eq!(hostname, "myhost");
            assert_eq!(format, Format::Ansi);
            assert!(is_last);
            assert!(is_ssh);
            assert_eq!(prev_bg, Some("blue".to_owned()));
        }
        other => panic!("expected HostnameRequest, got {other:?}"),
    }
}

#[test]
fn test_hostname_into_message_without_is_last_and_prev_bg() {
    let json = r#"{"op":"hostname","hostname":"myhost"}"#;
    let shell_msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
    let wire = shell_msg
        .into_wire_message()
        .expect("into_wire_message failed");
    let msg = resolve(wire).expect("resolve failed");
    match msg {
        Message::HostnameRequest {
            hostname,
            is_last,
            is_ssh,
            prev_bg,
            ..
        } => {
            assert_eq!(hostname, "myhost");
            assert!(!is_last);
            assert!(!is_ssh, "an absent is_ssh must default to false");
            assert_eq!(prev_bg, None);
        }
        other => panic!("expected HostnameRequest, got {other:?}"),
    }
}

#[test]
fn test_hostname_native_round_trip_is_ssh_present_and_absent() {
    for sent in [true, false] {
        let native = Message::HostnameRequest {
            hostname: "myhost".to_owned(),
            format: Format::Ansi,
            is_last: false,
            is_ssh: sent,
            prev_bg: None,
        };
        let bytes = serialize_message(&native).expect("serialize failed");
        let text = std::str::from_utf8(&bytes).expect("utf8");
        assert_eq!(
            text.contains("is_ssh"),
            sent,
            "is_ssh must be serialized only when true: {text}"
        );
        match deserialize_message(&bytes).expect("deserialize failed") {
            Message::HostnameRequest {
                hostname, is_ssh, ..
            } => {
                assert_eq!(hostname, "myhost");
                assert_eq!(is_ssh, sent);
            }
            other => panic!("expected HostnameRequest, got {other:?}"),
        }
    }
}

#[test]
fn test_hostname_validation_accepts_valid_request() {
    let msg = Message::HostnameRequest {
        hostname: "my-host".to_owned(),
        format: Format::Ansi,
        is_last: false,
        is_ssh: false,
        prev_bg: None,
    };
    assert!(validate_message_content(&msg).is_ok());
}

#[test]
fn test_hostname_validation_rejects_too_long_hostname() {
    let long_hostname = "x".repeat(257_usize);
    let msg = Message::HostnameRequest {
        hostname: long_hostname,
        format: Format::Ansi,
        is_last: false,
        is_ssh: false,
        prev_bg: None,
    };
    assert!(validate_message_content(&msg).is_err());
}

#[test]
fn test_hostname_validation_rejects_control_chars() {
    let hostname_with_control = "host\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}name".to_owned();
    let msg = Message::HostnameRequest {
        hostname: hostname_with_control,
        format: Format::Ansi,
        is_last: false,
        is_ssh: false,
        prev_bg: None,
    };
    assert!(validate_message_content(&msg).is_err());
}

#[test]
fn test_username_into_message_with_is_last_and_prev_bg() {
    let json =
        r#"{"op":"username","username":"root","format":"ansi","is_last":true,"prev_bg":"blue"}"#;
    let shell_msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
    let wire = shell_msg
        .into_wire_message()
        .expect("into_wire_message failed");
    let msg = resolve(wire).expect("resolve failed");
    match msg {
        Message::UsernameRequest {
            username,
            format,
            is_last,
            prev_bg,
        } => {
            assert_eq!(username, "root");
            assert_eq!(format, Format::Ansi);
            assert!(is_last);
            assert_eq!(prev_bg, Some("blue".to_owned()));
        }
        other => panic!("expected UsernameRequest, got {other:?}"),
    }
}

#[test]
fn test_username_into_message_without_is_last_and_prev_bg() {
    let json = r#"{"op":"username","username":"root"}"#;
    let shell_msg: ShellIpcMessage = serde_json::from_str(json).expect("parse failed");
    let wire = shell_msg
        .into_wire_message()
        .expect("into_wire_message failed");
    let msg = resolve(wire).expect("resolve failed");
    match msg {
        Message::UsernameRequest {
            username,
            is_last,
            prev_bg,
            ..
        } => {
            assert_eq!(username, "root");
            assert!(!is_last);
            assert_eq!(prev_bg, None);
        }
        other => panic!("expected UsernameRequest, got {other:?}"),
    }
}

#[test]
fn test_username_validation_accepts_valid_request() {
    let msg = Message::UsernameRequest {
        username: "root".to_owned(),
        format: Format::Ansi,
        is_last: false,
        prev_bg: None,
    };
    assert!(validate_message_content(&msg).is_ok());
}

#[test]
fn test_username_validation_rejects_too_long_username() {
    let long_username = "x".repeat(257_usize);
    let msg = Message::UsernameRequest {
        username: long_username,
        format: Format::Ansi,
        is_last: false,
        prev_bg: None,
    };
    assert!(validate_message_content(&msg).is_err());
}

#[test]
fn test_username_validation_rejects_control_chars() {
    let username_with_control = "ro\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}ot".to_owned();
    let msg = Message::UsernameRequest {
        username: username_with_control,
        format: Format::Ansi,
        is_last: false,
        prev_bg: None,
    };
    assert!(validate_message_content(&msg).is_err());
}

#[test]
fn test_prev_bg_validation_null_byte() {
    let color_with_null = "blue\0red";
    let result = validate_string_field("prev_bg", color_with_null, 64_usize);
    assert!(result.is_err());
}

// The shell and native formats must be disjoint so trying the shell format first is safe.

#[test]
fn test_deserialize_message_shell_format_hot_path() {
    let json = r#"{"op":"git","cwd":"/some/path"}"#;
    let msg = deserialize_message(json.as_bytes()).expect("deserialize failed");
    match msg {
        Message::RepositoryStatus {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
        } => {
            assert_eq!(
                path,
                crate::security::SafePath::new("/some/path").expect("SafePath failed")
            );
            assert_eq!(format, Format::Json);
            assert!(!is_last);
            assert!(!is_first);
            assert_eq!(prev_bg, None);
        }
        other => panic!("expected RepositoryStatus, got {other:?}"),
    }
}

#[test]
fn test_deserialize_message_native_format_fallback() {
    let native = Message::Ping;
    let serialized = serialize_message(&native).expect("serialize failed");
    let msg = deserialize_message(&serialized).expect("deserialize failed");
    assert!(matches!(msg, Message::Ping));
}

#[test]
fn test_shell_and_native_message_formats_are_disjoint() {
    let shell_json = r#"{"op":"git","cwd":"/some/path"}"#;
    let native_parse: std::result::Result<Message, serde_json::Error> =
        serde_json::from_str(shell_json);
    assert!(
        native_parse.is_err(),
        "shell-format JSON unexpectedly parsed as native Message"
    );

    let native = Message::Ping;
    let native_bytes = serialize_message(&native).expect("serialize failed");
    let native_str = std::str::from_utf8(&native_bytes).expect("valid utf-8");
    let shell_parse: std::result::Result<ShellIpcMessage, serde_json::Error> =
        serde_json::from_str(native_str);
    assert!(
        shell_parse.is_err(),
        "native-format JSON unexpectedly parsed as ShellIpcMessage"
    );
}

// `parse_wire` must not validate: these use denylisted paths and out-of-range PIDs.

#[test]
fn test_parse_wire_git_keeps_path_raw() {
    let json = br#"{"op":"git","cwd":"/etc/never-canonicalized"}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::RepositoryStatus {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
        } => {
            assert_eq!(path, "/etc/never-canonicalized");
            assert_eq!(format, Format::Json);
            assert!(!is_last);
            assert!(!is_first);
            assert_eq!(prev_bg, None);
        }
        _ => panic!("expected WireMessage::RepositoryStatus"),
    }
}

#[test]
fn test_parse_wire_lang_carries_render_metadata_and_virtual_env() {
    let json = br#"{"op":"lang","cwd":"/etc/nope","format":"ansi","is_last":true,"is_first":true,"prev_bg":"blue","virtual_env":"/venv"}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::LanguageDetect {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
            virtual_env,
        } => {
            assert_eq!(path, "/etc/nope");
            assert_eq!(format, Format::Ansi);
            assert!(is_last);
            assert!(is_first);
            assert_eq!(prev_bg, Some("blue".to_owned()));
            assert_eq!(virtual_env, Some("/venv".to_owned()));
        }
        _ => panic!("expected WireMessage::LanguageDetect"),
    }
}

#[test]
fn test_parse_wire_directory_defaults_missing_cwd_to_dot() {
    let json = br#"{"op":"directory"}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::DirectoryRequest { path, .. } => assert_eq!(path, "."),
        _ => panic!("expected WireMessage::DirectoryRequest"),
    }
}

#[test]
fn test_parse_wire_register_keeps_pid_and_cwd_raw() {
    // One past `ClientPid::MAX`; only `resolve` rejects it.
    let json = br#"{"op":"register","pid":4194305,"cwd":"/etc/nope","shell":"fish","shell_version":"3.7.1"}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::RegisterClient {
            pid,
            cwd,
            shell,
            shell_version,
        } => {
            assert_eq!(pid, 4_194_305_u32);
            assert_eq!(cwd, Some("/etc/nope".to_owned()));
            assert_eq!(shell, Some(ShellVariant::Fish));
            assert_eq!(shell_version, Some("3.7.1".to_owned()));
        }
        _ => panic!("expected WireMessage::RegisterClient"),
    }
}

#[test]
fn test_parse_wire_unregister_keeps_out_of_range_pid() {
    let json = br#"{"op":"unregister","pid":0}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::UnregisterClient { pid } => assert_eq!(pid, 0_u32),
        _ => panic!("expected WireMessage::UnregisterClient"),
    }
}

#[test]
fn test_parse_wire_workspace_keeps_pid_and_cwd_raw() {
    let json = br#"{"op":"workspace","pid":0,"cwd":"/etc/nope"}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::WorkspaceUpdate { pid, cwd } => {
            assert_eq!(pid, 0_u32);
            assert_eq!(cwd, "/etc/nope");
        }
        _ => panic!("expected WireMessage::WorkspaceUpdate"),
    }
}

#[test]
fn test_parse_wire_duration_is_a_plain_field_copy() {
    let json = br#"{"op":"duration","duration_ms":65000,"format":"ansi","is_last":true}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::DurationRequest {
            duration_ms,
            format,
            is_last,
            is_first,
            prev_bg,
        } => {
            assert_eq!(duration_ms, 65_000_u64);
            assert_eq!(format, Format::Ansi);
            assert!(is_last);
            assert!(!is_first);
            assert_eq!(prev_bg, None);
        }
        _ => panic!("expected WireMessage::DurationRequest"),
    }
}

#[test]
fn test_parse_wire_theme_query_carries_key() {
    let json = br#"{"op":"theme","key":"prompt_close"}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::ThemeQuery { key } => assert_eq!(key, "prompt_close"),
        _ => panic!("expected WireMessage::ThemeQuery"),
    }
}

#[test]
fn test_parse_wire_unit_ops() {
    assert!(matches!(
        parse_wire(br#"{"op":"ping"}"#).expect("parse_wire failed"),
        WireMessage::Ping
    ));
    assert!(matches!(
        parse_wire(br#"{"op":"status"}"#).expect("parse_wire failed"),
        WireMessage::Status
    ));
    assert!(matches!(
        parse_wire(br#"{"op":"latency_stats"}"#).expect("parse_wire failed"),
        WireMessage::LatencyStats
    ));
}

#[test]
fn test_parse_wire_register_missing_pid_is_a_wire_shape_error() {
    let err = parse_wire(br#"{"op":"register","cwd":"/etc/nope"}"#)
        .expect_err("register without a pid must fail");
    assert!(
        err.to_string().contains("register missing pid"),
        "unexpected error text"
    );
}

#[test]
fn test_parse_wire_workspace_missing_pid_and_cwd_are_wire_shape_errors() {
    let missing_pid = parse_wire(br#"{"op":"workspace","cwd":"/etc/nope"}"#)
        .expect_err("workspace without a pid must fail");
    assert!(
        missing_pid
            .to_string()
            .contains("Workspace update missing pid"),
        "unexpected error text for a missing pid"
    );

    let missing_cwd = parse_wire(br#"{"op":"workspace","pid":1}"#)
        .expect_err("workspace without a cwd must fail");
    assert!(
        missing_cwd
            .to_string()
            .contains("Workspace update missing cwd"),
        "unexpected error text for a missing cwd"
    );
}

#[test]
fn test_parse_wire_unknown_operation() {
    let err = parse_wire(br#"{"op":"teleport"}"#).expect_err("unknown op must fail");
    assert!(
        err.to_string()
            .contains("Unknown shell operation: teleport"),
        "unexpected error text"
    );
}

#[test]
fn test_parse_wire_rejects_neither_format() {
    let err = parse_wire(br#"{"not_an_op":true}"#).expect_err("unrecognized payload must fail");
    assert!(
        err.to_string()
            .contains("Failed to deserialize message: invalid format"),
        "unexpected error text"
    );
}

#[test]
fn test_parse_wire_native_format_struct_variant() {
    let json =
        br#"{"RepositoryStatus":{"path":"/etc/nope","format":"fish-source","is_last":true}}"#;
    let wire = parse_wire(json).expect("parse_wire failed");
    match wire {
        WireMessage::RepositoryStatus {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
        } => {
            assert_eq!(path, "/etc/nope");
            assert_eq!(format, Format::FishSource);
            assert!(is_last);
            assert!(!is_first);
            assert_eq!(prev_bg, None);
        }
        _ => panic!("expected WireMessage::RepositoryStatus"),
    }
}

#[test]
fn test_parse_wire_native_format_unit_variant() {
    let wire = parse_wire(br#""Ping""#).expect("parse_wire failed");
    assert!(matches!(wire, WireMessage::Ping));
}
