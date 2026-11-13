//! IPC protocol edge cases.

#![allow(clippy::expect_used, reason = "test code may panic on failure")]
#![allow(clippy::unwrap_used, reason = "test code may panic on failure")]

mod common;

use gpy_agent::ipc::Message;
use gpy_agent::ipc::protocol::deserialize_message;
use gpy_agent::security::SafePath;
use std::assert_matches;

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

    let message = deserialize_message(json_with_extra.as_bytes())
        .expect("Should handle extra fields gracefully (forward compatibility)");
    assert_matches!(message, Message::RepositoryStatus { .. });
}

#[test]
fn test_all_message_types_parse_in_op_form() {
    let pid = std::process::id();
    let parse = |line: String| deserialize_message(line.as_bytes()).expect("Should deserialize");
    assert_matches!(
        parse(format!(r#"{{"op":"register","pid":{pid},"cwd":"/tmp"}}"#)),
        Message::RegisterClient { .. }
    );
    assert_matches!(
        parse(format!(r#"{{"op":"unregister","pid":{pid}}}"#)),
        Message::UnregisterClient { .. }
    );
    assert_matches!(
        parse(format!(r#"{{"op":"workspace","pid":{pid},"cwd":"/tmp"}}"#)),
        Message::WorkspaceUpdate { .. }
    );
    assert_matches!(
        parse(r#"{"op":"git","cwd":"/tmp"}"#.to_owned()),
        Message::RepositoryStatus { .. }
    );
    assert_matches!(
        parse(r#"{"op":"lang","cwd":"/tmp"}"#.to_owned()),
        Message::LanguageDetect { .. }
    );
    assert_matches!(parse(r#"{"op":"status"}"#.to_owned()), Message::Status);
}

/// A `{"op":"ping","pad":"xxx"}` request body of exactly `len` bytes.
#[cfg(unix)]
fn padded_ping(len: usize) -> Vec<u8> {
    let prefix = br#"{"op":"ping","pad":""#;
    let suffix = br#""}"#;
    let mut body = prefix.to_vec();
    body.resize(len.saturating_sub(suffix.len()), b'x');
    body.extend_from_slice(suffix);
    assert_eq!(body.len(), len);
    body
}

/// Start a server, connect, send `request` plus a newline, and return the
/// buffered stream positioned before the first reply line.
#[cfg(unix)]
async fn send_line(
    socket_path: &std::path::Path,
    request: &[u8],
) -> tokio::io::BufReader<tokio::net::UnixStream> {
    use tokio::io::AsyncWriteExt;

    let mut stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .expect("connect to test server");
    // One buffer, so the newline is never sent after a gap in which the server
    // could read an unterminated line over the limit and drop the connection.
    stream
        .write_all(&[request, b"\n"].concat())
        .await
        .expect("send request");
    tokio::io::BufReader::new(stream)
}

#[cfg(unix)]
async fn read_reply(stream: &mut tokio::io::BufReader<tokio::net::UnixStream>) -> String {
    use tokio::io::AsyncBufReadExt;

    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.read_line(&mut line),
    )
    .await
    .expect("reply within 5s")
    .expect("read reply");
    line
}

#[cfg(unix)]
#[tokio::test]
async fn a_request_line_of_exactly_the_size_limit_is_answered() {
    let dir = tempfile::TempDir::new().unwrap();
    let socket_path = dir.path().join("limit.sock");
    let mut server = common::endpoint::endpoint(socket_path.clone());
    let handle = tokio::spawn(async move {
        let _ = server.start().await;
    });
    common::endpoint::wait_for_socket(&socket_path, std::time::Duration::from_secs(5)).await;

    let mut stream = send_line(&socket_path, &padded_ping(65_536)).await;

    assert_eq!(
        read_reply(&mut stream).await.trim_end(),
        r#"{"status":"ok"}"#
    );
    handle.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn a_request_line_one_byte_over_the_limit_gets_the_too_large_reply() {
    use tokio::io::AsyncWriteExt;

    let dir = tempfile::TempDir::new().unwrap();
    let socket_path = dir.path().join("over.sock");
    let mut server = common::endpoint::endpoint(socket_path.clone());
    let handle = tokio::spawn(async move {
        let _ = server.start().await;
    });
    common::endpoint::wait_for_socket(&socket_path, std::time::Duration::from_secs(5)).await;

    let mut stream = send_line(&socket_path, &padded_ping(65_537)).await;

    let reply = read_reply(&mut stream).await;
    assert!(
        reply.contains("Message too large: 65537 bytes (max 65536)"),
        "{reply}"
    );
    stream
        .get_mut()
        .write_all(b"{\"op\":\"ping\"}\n")
        .await
        .unwrap();
    assert_eq!(
        read_reply(&mut stream).await.trim_end(),
        r#"{"status":"ok"}"#
    );
    handle.abort();
}
