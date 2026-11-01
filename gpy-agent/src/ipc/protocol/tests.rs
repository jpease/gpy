use super::*;
use crate::shell::Shell;

#[test]
fn test_config_reload_json_deserializes_to_config_reload() {
    let msg = deserialize_message(Op::ConfigReload.bare_request().as_bytes())
        .expect("deserialize failed");
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

fn ansi_segment() -> Segment {
    Segment {
        format: Format::Ansi,
        ..Segment::default()
    }
}

#[test]
fn test_exit_status_into_message_carries_position_and_outcome() {
    let json = r#"{"op":"exit_status","success":true,"format":"ansi","is_last":true,"is_first":true,"prev_bg":"blue"}"#;
    let msg = deserialize_message(json.as_bytes()).expect("deserialize failed");
    match msg {
        Message::ExitStatusRequest { success, segment } => {
            assert!(success);
            assert_eq!(
                segment,
                Segment {
                    format: Format::Ansi,
                    is_last: true,
                    is_first: true,
                    prev_bg: Some("blue".to_owned()),
                }
            );
        }
        other => panic!("expected ExitStatusRequest, got {other:?}"),
    }
}

#[test]
fn test_exit_status_defaults_to_failure_without_flags() {
    let msg = deserialize_message(br#"{"op":"exit_status"}"#).expect("deserialize failed");
    match msg {
        Message::ExitStatusRequest { success, segment } => {
            assert!(!success, "an absent success must default to false");
            assert_eq!(segment, Segment::default());
        }
        other => panic!("expected ExitStatusRequest, got {other:?}"),
    }
}

#[test]
fn test_exit_status_validation_rejects_long_prev_bg() {
    let msg = Message::ExitStatusRequest {
        success: true,
        segment: Segment {
            prev_bg: Some("x".repeat(MAX_PREV_BG_LEN + 1)),
            ..ansi_segment()
        },
    };
    assert!(validate_message_content(&msg).is_err());
}

/// The agent health check keeps the bare `status` op; the exit-status pill is `exit_status`.
#[test]
fn test_status_op_is_still_the_agent_health_check() {
    let msg = deserialize_message(br#"{"op":"status"}"#).expect("deserialize failed");
    assert!(matches!(msg, Message::Status));
}

#[test]
fn test_control_messages_have_no_segment() {
    for op in [Op::Ping, Op::Status, Op::ConfigReload] {
        let json = op.bare_request();
        let msg = deserialize_message(json.as_bytes()).expect("deserialize failed");
        assert!(msg.segment().is_none(), "{json}");
    }
}

#[test]
fn test_prev_bg_validation_null_byte() {
    let color_with_null = "blue\0red";
    let result = validate_string_field("prev_bg", color_with_null, 64_usize);
    assert!(result.is_err());
}

#[test]
fn test_deserialize_message_shell_format_hot_path() {
    let json = r#"{"op":"git","cwd":"/some/path"}"#;
    let msg = deserialize_message(json.as_bytes()).expect("deserialize failed");
    match msg {
        Message::RepositoryStatus { path, segment } => {
            assert_eq!(
                path,
                crate::security::SafePath::new("/some/path").expect("SafePath failed")
            );
            assert_eq!(segment, Segment::default());
        }
        other => panic!("expected RepositoryStatus, got {other:?}"),
    }
}

#[test]
fn test_deserialize_message_rejects_native_form() {
    for native in [
        r#"{"Ping":null}"#,
        r#""Ping""#,
        r#"{"RepositoryStatus":{"path":"/tmp"}}"#,
    ] {
        assert!(
            deserialize_message(native.as_bytes()).is_err(),
            "accepted {native}"
        );
    }
}

#[test]
fn test_lang_carries_render_metadata_and_virtual_env() {
    let json = br#"{"op":"lang","cwd":".","format":"ansi","is_last":true,"is_first":true,"prev_bg":"blue","virtual_env":"/venv"}"#;
    match deserialize_message(json).expect("deserialize failed") {
        Message::LanguageDetect {
            segment,
            virtual_env,
            ..
        } => {
            assert_eq!(
                segment,
                Segment {
                    format: Format::Ansi,
                    is_last: true,
                    is_first: true,
                    prev_bg: Some("blue".to_owned()),
                }
            );
            assert_eq!(virtual_env, Some("/venv".to_owned()));
        }
        other => panic!("expected LanguageDetect, got {other:?}"),
    }
}

#[test]
fn test_directory_defaults_missing_cwd_to_dot() {
    match deserialize_message(br#"{"op":"directory"}"#).expect("deserialize failed") {
        Message::DirectoryRequest { display_path, .. } => assert_eq!(display_path, "."),
        other => panic!("expected DirectoryRequest, got {other:?}"),
    }
}

#[test]
fn test_unknown_format_falls_back_to_json() {
    let (format, message) =
        deserialize_message_with_format(br#"{"op":"duration","format":"bogus"}"#);
    assert_eq!(format, None);
    let segment = message.expect("deserialize failed").segment().cloned();
    assert_eq!(segment.map(|s| s.format), Some(Format::Json));
}

#[test]
fn test_register_carries_shell_and_version() {
    let json = br#"{"op":"register","pid":1,"shell":"fish","shell_version":"3.7.1"}"#;
    match deserialize_message(json).expect("deserialize failed") {
        Message::RegisterClient {
            pid,
            cwd,
            shell,
            shell_version,
        } => {
            assert_eq!(pid.get(), 1_u32);
            assert!(cwd.is_none());
            assert_eq!(shell, Some(Shell::Fish));
            assert_eq!(shell_version, Some("3.7.1".to_owned()));
        }
        other => panic!("expected RegisterClient, got {other:?}"),
    }
}

#[test]
fn test_register_drops_unknown_shell() {
    let json = br#"{"op":"register","pid":1,"shell":"tcsh"}"#;
    match deserialize_message(json).expect("deserialize failed") {
        Message::RegisterClient { shell, .. } => assert_eq!(shell, None),
        other => panic!("expected RegisterClient, got {other:?}"),
    }
}

#[test]
fn test_out_of_range_pids_are_rejected() {
    for json in [
        // One past `ClientPid::MAX`.
        &br#"{"op":"register","pid":4194305}"#[..],
        br#"{"op":"unregister","pid":0}"#,
        br#"{"op":"workspace","pid":0,"cwd":"."}"#,
    ] {
        let err = deserialize_message(json).expect_err("out-of-range pid must fail");
        assert!(
            err.to_string().contains("Invalid PID"),
            "unexpected error text: {err}"
        );
    }
}

#[test]
fn test_encode_reply_newlines_keeps_reply_on_one_line() {
    let encoded = encode_reply_newlines("a\x1eb\nc\n\nd");
    assert_eq!(encoded, "ab\x1ec\x1e\x1ed");
    assert!(!encoded.contains('\n'));
    assert!(matches!(
        encode_reply_newlines("\x1b[31mplain\x1b[0m"),
        std::borrow::Cow::Borrowed(_)
    ));
}

#[test]
fn test_duration_is_a_plain_field_copy() {
    let json = br#"{"op":"duration","duration_ms":65000,"format":"ansi","is_last":true}"#;
    match deserialize_message(json).expect("deserialize failed") {
        Message::DurationRequest {
            duration_ms,
            segment,
        } => {
            assert_eq!(duration_ms, 65_000_u64);
            assert_eq!(
                segment,
                Segment {
                    format: Format::Ansi,
                    is_last: true,
                    ..Segment::default()
                }
            );
        }
        other => panic!("expected DurationRequest, got {other:?}"),
    }
}

#[test]
fn test_unit_ops() {
    assert!(matches!(
        deserialize_message(br#"{"op":"ping"}"#).expect("deserialize failed"),
        Message::Ping
    ));
    assert!(matches!(
        deserialize_message(br#"{"op":"shutdown"}"#).expect("deserialize failed"),
        Message::Shutdown
    ));
}

/// A missing required field is reported before the denylisted `cwd` is ever canonicalized.
#[test]
fn test_register_missing_pid_is_reported_before_path_checks() {
    let err = deserialize_message(br#"{"op":"register","cwd":"/etc/nope"}"#)
        .expect_err("register without a pid must fail");
    assert!(
        err.to_string().contains("register missing pid"),
        "unexpected error text: {err}"
    );
}

#[test]
fn test_workspace_missing_pid_and_cwd_are_reported_before_path_checks() {
    let missing_pid = deserialize_message(br#"{"op":"workspace","cwd":"/etc/nope"}"#)
        .expect_err("workspace without a pid must fail");
    assert!(
        missing_pid
            .to_string()
            .contains("Workspace update missing pid"),
        "unexpected error text for a missing pid: {missing_pid}"
    );

    let missing_cwd = deserialize_message(br#"{"op":"workspace","pid":0}"#)
        .expect_err("workspace without a cwd must fail");
    assert!(
        missing_cwd
            .to_string()
            .contains("Workspace update missing cwd"),
        "unexpected error text for a missing cwd: {missing_cwd}"
    );
}

#[test]
fn test_unknown_operation() {
    let err = deserialize_message(br#"{"op":"teleport"}"#).expect_err("unknown op must fail");
    assert!(
        err.to_string()
            .contains("Unknown shell operation: teleport"),
        "unexpected error text: {err}"
    );
}

#[test]
fn test_rejects_payload_without_op() {
    let err =
        deserialize_message(br#"{"not_an_op":true}"#).expect_err("unrecognized payload must fail");
    assert!(
        err.to_string()
            .contains("Failed to deserialize message: invalid format"),
        "unexpected error text: {err}"
    );
}

#[test]
fn test_requested_format_survives_unknown_operation() {
    let (format, message) =
        deserialize_message_with_format(br#"{"op":"teleport","format":"ansi"}"#);
    assert!(message.is_err(), "unknown op must fail");
    assert_eq!(format, Some(Format::Ansi));
}

#[test]
fn test_requested_format_survives_wire_shape_error() {
    // `pid` has the wrong type, so the full message fails to deserialize.
    let (format, message) =
        deserialize_message_with_format(br#"{"op":"register","pid":"x","format":"ansi"}"#);
    assert!(message.is_err(), "mistyped field must fail");
    assert_eq!(format, Some(Format::Ansi));
}

#[test]
fn test_requested_format_is_none_for_unparseable_requests() {
    for data in [
        &b"not json"[..],
        br#"{"op":"teleport"}"#,
        br#"{"op":"teleport","format":"bogus"}"#,
    ] {
        let (format, message) = deserialize_message_with_format(data);
        assert!(message.is_err(), "request must fail");
        assert_eq!(format, None);
    }
}

fn language_json_with_venv(venv: &str) -> String {
    serde_json::json!({"op":"lang","cwd":".","virtual_env":venv}).to_string()
}

#[test]
fn test_virtual_env_rejects_control_characters() {
    for venv in ["/tmp/venv\0x", "/tmp/venv\nx"] {
        let json = language_json_with_venv(venv);
        assert!(deserialize_message(json.as_bytes()).is_err(), "{venv:?}");
    }
}

#[test]
fn test_virtual_env_rejects_overlong_path() {
    let json = language_json_with_venv(&format!("/{}", "a".repeat(5000)));
    assert!(deserialize_message(json.as_bytes()).is_err());
}

#[test]
fn test_virtual_env_rejects_parent_dir() {
    let json = language_json_with_venv("/tmp/../etc/venv");
    assert!(deserialize_message(json.as_bytes()).is_err());
}

#[test]
fn test_virtual_env_accepts_plain_path() {
    let json = language_json_with_venv("/tmp/some-venv");
    let msg = deserialize_message(json.as_bytes()).expect("valid virtual_env");
    assert!(matches!(
        msg,
        Message::LanguageDetect { virtual_env: Some(ref v), .. } if v == "/tmp/some-venv"
    ));
}
