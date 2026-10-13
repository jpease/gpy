//! Tests for IPC client behavior against mock servers.

#![allow(clippy::panic)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::as_conversions)]

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::ipc::protocol::deserialize_message;
use gpy_agent::ipc::{Format, LanguageInfo, Message, Response, client::SessionHandle};
use serial_test::serial;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

/// Upper bound for `SessionHandle::connect`, whose own timeout is 5s.
const CONNECT_BOUND: Duration = Duration::from_secs(6);

/// Read one length-prefixed request frame, or `None` if the client hung up.
async fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await.ok()?;
    let mut msg_bytes = vec![0u8; u32::from_be_bytes(len_bytes) as usize];
    stream.read_exact(&mut msg_bytes).await.ok()?;
    Some(msg_bytes)
}

/// Bind `socket_path` and answer the first request with `response`.
///
/// The listener is bound before this returns, so clients may connect
/// immediately. The receiver yields the raw request bytes the client sent.
fn mock_server_normal(socket_path: &Path, response: Response) -> oneshot::Receiver<Vec<u8>> {
    let listener = UnixListener::bind(socket_path).expect("Failed to bind socket");
    let (request_tx, request_rx) = oneshot::channel();

    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let Some(request) = read_frame(&mut stream).await else {
            return;
        };
        let _ = request_tx.send(request);

        let response_bytes = serde_json::to_vec(&response).unwrap_or_default();
        let response_len = (response_bytes.len() as u32).to_be_bytes();
        let _ = stream.write_all(&response_len).await;
        let _ = stream.write_all(&response_bytes).await;
        let _ = stream.flush().await;
    });

    request_rx
}

/// Bind `socket_path` and answer the first request with `malformed_data` as the body.
fn mock_server_malformed(socket_path: &Path, malformed_data: Vec<u8>) {
    let listener = UnixListener::bind(socket_path).expect("Failed to bind socket");

    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        if read_frame(&mut stream).await.is_none() {
            return;
        }
        let response_len = (malformed_data.len() as u32).to_be_bytes();
        let _ = stream.write_all(&response_len).await;
        let _ = stream.write_all(&malformed_data).await;
        let _ = stream.flush().await;
    });
}

/// Bind `socket_path` and close the first accepted connection without replying.
fn mock_server_disconnect(socket_path: &Path) {
    let listener = UnixListener::bind(socket_path).expect("Failed to bind socket");

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
}

/// Bind `socket_path` and answer the first request with a length prefix above the client's 64KB cap.
fn mock_server_oversized(socket_path: &Path) {
    let listener = UnixListener::bind(socket_path).expect("Failed to bind socket");

    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        if read_frame(&mut stream).await.is_none() {
            return;
        }
        let _ = stream.write_all(&1_000_000_u32.to_be_bytes()).await;
        let _ = stream.flush().await;
    });
}

fn socket_in(temp_dir: &TempDir) -> PathBuf {
    temp_dir.path().join("test.sock")
}

#[tokio::test]
#[serial]
async fn test_connect_missing_socket() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("nonexistent.sock");
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);

    let Err(err) = tokio::time::timeout(CONNECT_BOUND, SessionHandle::connect())
        .await
        .expect("connect should give up within its own timeout")
    else {
        panic!("connect to a missing socket should fail");
    };

    let msg = err.to_string();
    assert!(
        msg.contains("Failed to connect") && msg.contains(&socket_path.display().to_string()),
        "error should name the unreachable socket: {msg}"
    );
}

#[tokio::test]
#[serial]
async fn test_send_request_malformed_response() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = socket_in(&temp_dir);
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);

    mock_server_malformed(&socket_path, b"not valid json".to_vec());

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.send_request(Message::Ping).await;

    assert!(
        result.is_err(),
        "malformed response body should fail to deserialize"
    );
}

#[tokio::test]
#[serial]
async fn test_send_request_oversized_response() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = socket_in(&temp_dir);
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);

    mock_server_oversized(&socket_path);

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let err = client
        .send_request(Message::Ping)
        .await
        .expect_err("oversized response should be rejected");

    assert!(
        err.to_string().contains("too large"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
#[serial]
async fn test_send_request_disconnect_during_read() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = socket_in(&temp_dir);
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);

    mock_server_disconnect(&socket_path);

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let err = client
        .send_request(Message::Ping)
        .await
        .expect_err("request on a closed connection should fail");

    let msg = err.to_string();
    assert!(
        msg.contains("read") || msg.contains("send") || msg.contains("response"),
        "error should say which step failed: {msg}"
    );
}

#[tokio::test]
#[serial]
async fn test_ping_success() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = socket_in(&temp_dir);
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);

    let request = mock_server_normal(&socket_path, Response::Ack);

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    assert!(client.ping().await.expect("ping should succeed"));

    let sent = request.await.expect("server should capture the request");
    assert!(matches!(
        deserialize_message(&sent).expect("client sent a valid message"),
        Message::Ping
    ));
}

#[tokio::test]
#[serial]
async fn test_ping_unexpected_response() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = socket_in(&temp_dir);
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);

    let _request = mock_server_normal(
        &socket_path,
        Response::Error {
            message: "test error".to_owned(),
        },
    );

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let alive = client
        .ping()
        .await
        .expect("a well-formed non-Ack response is not a transport failure");

    assert!(!alive, "ping should report false for a non-Ack response");
}

#[tokio::test]
#[serial]
async fn test_ping_transport_failure_is_err() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = socket_in(&temp_dir);
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);

    mock_server_disconnect(&socket_path);

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.ping().await;

    assert!(
        result.is_err(),
        "ping over a dropped connection should be Err, got {result:?}"
    );
}

#[tokio::test]
#[serial]
async fn test_load_repository_state() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = socket_in(&temp_dir);
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);
    let repo_path = temp_dir
        .path()
        .canonicalize()
        .expect("canonicalize temp dir");

    let response = Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0_u32,
        behind: 0_u32,
        ahead_capped: false,
        behind_capped: false,
        staged: 0_u32,
        unstaged: 0_u32,
        untracked: 0_u32,
        conflicts: 0_u32,
        state: gpy_agent::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    });
    let request = mock_server_normal(&socket_path, response);

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client
        .load_repository_state(repo_path.display().to_string())
        .await;

    match result.expect("load_repository_state should succeed") {
        Response::RepositoryStatus(status) => assert_eq!(status.branch, "main"),
        other => panic!("unexpected response {other:?}"),
    }

    let sent = request.await.expect("server should capture the request");
    match deserialize_message(&sent).expect("client sent a valid message") {
        Message::RepositoryStatus { path, format, .. } => {
            assert_eq!(path.as_path(), repo_path);
            assert_eq!(format, Format::Json);
        }
        other => panic!("unexpected request {other:?}"),
    }
}

#[tokio::test]
#[serial]
async fn test_detect_languages() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = socket_in(&temp_dir);
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &socket_path);
    let repo_path = temp_dir
        .path()
        .canonicalize()
        .expect("canonicalize temp dir");

    let response = Response::Language {
        languages: vec![LanguageInfo {
            name: "Rust".to_owned(),
            version: None,
            color: gpy_agent::config::types::ColorSpec::new("#dea584").expect("valid color"),
        }],
    };
    let request = mock_server_normal(&socket_path, response);

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client
        .detect_languages(repo_path.display().to_string())
        .await;

    match result.expect("detect_languages should succeed") {
        Response::Language { languages } => {
            let names: Vec<&str> = languages.iter().map(|l| l.name.as_str()).collect();
            assert_eq!(names, ["Rust"]);
        }
        other => panic!("unexpected response {other:?}"),
    }

    let sent = request.await.expect("server should capture the request");
    match deserialize_message(&sent).expect("client sent a valid message") {
        Message::LanguageDetect { path, format, .. } => {
            assert_eq!(path.as_path(), repo_path);
            assert_eq!(format, Format::Json);
        }
        other => panic!("unexpected request {other:?}"),
    }
}

#[test]
#[serial]
fn test_socket_path_custom() {
    let _env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", "/custom/path.sock");

    assert_eq!(SessionHandle::socket_path(), "/custom/path.sock");
}

#[test]
#[serial]
fn test_socket_path_default() {
    let _env = EnvGuard::cleared(&["GPY_AGENT_SOCKET_PATH", "XDG_RUNTIME_DIR", "XDG_CACHE_HOME"]);

    let path = SessionHandle::socket_path();

    assert!(
        path.contains("gpy-test.sock"),
        "unexpected default path: {path}"
    );
}
