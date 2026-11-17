//! Property tests for the IPC wire: request decoding and reply codec.
//!
//! Requests arrive from any process that can reach the socket, so decoding
//! must reject bad input with an error and never panic.

#![allow(
    clippy::unwrap_used,
    reason = "tests: a failed unwrap is a test failure"
)]
#![allow(
    clippy::expect_used,
    reason = "tests: a failed expect is a test failure"
)]

use gpy_agent::ipc::protocol::{REPLY_NEWLINE, deserialize_message, encode_reply_newlines};
use gpy_agent::ipc::wire::{Op, decode_control_reply, encode_json};
use gpy_agent::ipc::{AgentStatus, Response};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use serde_json::{Value, json};

fn config() -> ProptestConfig {
    ProptestConfig {
        // A failure prints its shrunk counterexample; no regression file is
        // written into the source tree.
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

/// A JSON scalar of the kinds a client could put in any field.
fn json_scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        ".{0,40}".prop_map(Value::from),
    ]
}

/// A request object with real op names and every known field, each set to
/// an arbitrary scalar of any type or left out.
fn request_object() -> impl Strategy<Value = Value> {
    let op = prop_oneof![
        3 => prop::sample::select(Op::ALL.to_vec()).prop_map(|op| Value::from(op.as_str())),
        1 => json_scalar(),
    ];
    let fields = [
        "cwd",
        "pid",
        "format",
        "is_last",
        "is_first",
        "shell",
        "shell_version",
        "duration_ms",
        "success",
        "prev_bg",
        "virtual_env",
        "extra",
    ];
    let field_values = prop::collection::vec(prop::option::of(json_scalar()), fields.len());
    (op, field_values).prop_map(move |(op_value, values)| {
        let mut object = serde_json::Map::new();
        object.insert("op".to_owned(), op_value);
        for (name, value) in fields.iter().zip(values) {
            if let Some(field_value) = value {
                object.insert((*name).to_owned(), field_value);
            }
        }
        Value::Object(object)
    })
}

fn status_strategy() -> impl Strategy<Value = AgentStatus> {
    (
        ".{0,20}",
        any::<u8>(),
        any::<usize>(),
        any::<usize>(),
        any::<usize>(),
        prop::option::of(any::<u32>()),
    )
        .prop_map(
            |(version, protocol_version, watched_repos, registered_clients, cache_entries, pid)| {
                AgentStatus {
                    version,
                    protocol_version,
                    watched_repos,
                    registered_clients,
                    cache_entries,
                    pid,
                }
            },
        )
}

proptest! {
    #![proptest_config(config())]

    /// Arbitrary bytes decode to a message or an error.
    #[test]
    fn arbitrary_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _outcome = deserialize_message(&data);
    }

    /// Well-formed JSON with mistyped, hostile or missing fields never panics.
    #[test]
    fn structured_requests_never_panic(request in request_object()) {
        let _outcome = deserialize_message(request.to_string().as_bytes());
    }

    /// A control character in `prev_bg` is rejected, never carried into a
    /// message.
    #[test]
    fn control_characters_in_prev_bg_are_rejected(
        head in "[a-z#0-9]{0,8}",
        control in prop::sample::select(vec!['\0', '\n', '\u{1b}', '\u{7}']),
        tail in "[a-z#0-9]{0,8}",
    ) {
        let request = json!({"op": "clock", "shell": "zsh", "prev_bg": format!("{head}{control}{tail}")});
        prop_assert!(deserialize_message(request.to_string().as_bytes()).is_err());
    }

    /// Any request over the size cap is refused, whatever it holds.
    #[test]
    fn oversized_requests_are_refused(extra in 1_usize..4096) {
        let mut data = br#"{"op":"ping"}"#.to_vec();
        data.resize(extra.saturating_add(64 * 1024), b' ');
        prop_assert!(deserialize_message(&data).is_err());
    }

    /// Control replies survive encode -> decode.
    #[test]
    fn control_replies_round_trip(message in ".{0,80}", status in status_strategy()) {
        for response in [
            Response::Ack,
            Response::Error { message: message.clone() },
            Response::AgentStatus(status.clone()),
        ] {
            let line = encode_json(&response).unwrap();
            prop_assert_eq!(decode_control_reply(&format!("{line}\n")).unwrap(), response);
        }
    }

    /// Arbitrary reply text decodes or errors, never panics.
    #[test]
    fn decoding_arbitrary_replies_never_panics(text in ".{0,200}") {
        let _outcome = decode_control_reply(&text);
    }

    /// A reply payload stays one line: no raw newline survives, and no stray
    /// reply-newline marker is invented from outside text.
    #[test]
    fn reply_newlines_keep_one_line(payload in ".{0,80}") {
        let encoded = encode_reply_newlines(&payload);
        prop_assert!(!encoded.contains('\n'));
        let expected_markers = payload.matches('\n').count();
        prop_assert_eq!(encoded.matches(REPLY_NEWLINE).count(), expected_markers);
    }
}
