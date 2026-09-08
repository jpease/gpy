//! Tests for IPC client behavior with mock server

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::unused_async)]
#![allow(clippy::as_conversions)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::shadow_unrelated)]

use gpy_agent::ipc::{LanguageInfo, Message, Response, client::SessionHandle};
use serial_test::serial;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::time::sleep;

// ===== Mock Server Helpers =====

// Create a mock server that responds to length-prefixed messages
async fn mock_server_normal(socket_path: PathBuf, response: Response) {
    let listener = UnixListener::bind(&socket_path).expect("Failed to bind socket");

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Read message length
            let mut len_bytes = [0u8; 4];
            if stream.read_exact(&mut len_bytes).await.is_err() {
                return;
            }

            // Read message
            let msg_len = u32::from_be_bytes(len_bytes) as usize;
            let mut msg_bytes = vec![0u8; msg_len];
            if stream.read_exact(&mut msg_bytes).await.is_err() {
                return;
            }

            // Send response
            let response_bytes = serde_json::to_vec(&response).unwrap_or_default();
            let response_len = (response_bytes.len() as u32).to_be_bytes();

            let _ = stream.write_all(&response_len).await;
            let _ = stream.write_all(&response_bytes).await;
            let _ = stream.flush().await;
        }
    });
}

// Create a mock server that sends malformed response
async fn mock_server_malformed(socket_path: PathBuf, malformed_data: Vec<u8>) {
    let listener = UnixListener::bind(&socket_path).expect("Failed to bind socket");

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Read message length
            let mut len_bytes = [0u8; 4];
            if stream.read_exact(&mut len_bytes).await.is_err() {
                return;
            }

            // Read message
            let msg_len = u32::from_be_bytes(len_bytes) as usize;
            let mut msg_bytes = vec![0u8; msg_len];
            if stream.read_exact(&mut msg_bytes).await.is_err() {
                return;
            }

            // Send malformed response
            let response_len = (malformed_data.len() as u32).to_be_bytes();
            let _ = stream.write_all(&response_len).await;
            let _ = stream.write_all(&malformed_data).await;
            let _ = stream.flush().await;
        }
    });
}

// Create a mock server that closes connection immediately
async fn mock_server_disconnect(socket_path: PathBuf) {
    let listener = UnixListener::bind(&socket_path).expect("Failed to bind socket");

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            // Close immediately
            drop(stream);
        }
    });
}

// Create a mock server that sends oversized response
async fn mock_server_oversized(socket_path: PathBuf) {
    let listener = UnixListener::bind(&socket_path).expect("Failed to bind socket");

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            // Read message
            let mut len_bytes = [0u8; 4];
            if stream.read_exact(&mut len_bytes).await.is_err() {
                return;
            }
            let msg_len = u32::from_be_bytes(len_bytes) as usize;
            let mut msg_bytes = vec![0u8; msg_len];
            if stream.read_exact(&mut msg_bytes).await.is_err() {
                return;
            }

            // Send response claiming to be 1MB (should exceed limit)
            let fake_len = (1_000_000_u32).to_be_bytes();
            let _ = stream.write_all(&fake_len).await;
            let _ = stream.flush().await;
        }
    });
}

// ===== Connection Tests =====

#[tokio::test]
#[serial]
async fn test_connect_missing_socket() {
    // Set a socket path that doesn't exist
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("nonexistent.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    let result = tokio::time::timeout(Duration::from_secs(6), SessionHandle::connect()).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    // Should timeout or error
    assert!(result.is_err() || result.unwrap().is_err());
}

#[tokio::test]
#[serial]
async fn test_connect_success() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    // Start mock server
    mock_server_normal(socket_path.clone(), Response::Ack).await;
    sleep(Duration::from_millis(50)).await;

    let result = SessionHandle::connect().await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    assert!(result.is_ok());
}

// ===== Send Request Tests =====

#[tokio::test]
#[serial]
async fn test_send_request_ping() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    mock_server_normal(socket_path.clone(), Response::Ack).await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.send_request(Message::Ping).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), Response::Ack));
}

#[tokio::test]
#[serial]
async fn test_send_request_malformed_response() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    // Send invalid JSON
    let malformed = b"not valid json".to_vec();
    mock_server_malformed(socket_path.clone(), malformed).await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.send_request(Message::Ping).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    // Should fail with deserialization error
    assert!(result.is_err());
}

#[tokio::test]
#[serial]
async fn test_send_request_oversized_response() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    mock_server_oversized(socket_path.clone()).await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.send_request(Message::Ping).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    // Should reject oversized response
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("too large"));
}

#[tokio::test]
#[serial]
async fn test_send_request_disconnect_during_read() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    mock_server_disconnect(socket_path.clone()).await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.send_request(Message::Ping).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    // Should fail when trying to read response
    assert!(result.is_err());
}

// ===== Ping Tests =====

#[tokio::test]
#[serial]
async fn test_ping_success() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    mock_server_normal(socket_path.clone(), Response::Ack).await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.ping().await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    assert!(result.is_ok());
    assert!(result.unwrap());
}

#[tokio::test]
#[serial]
async fn test_ping_unexpected_response() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    // Send wrong response type
    mock_server_normal(
        socket_path.clone(),
        Response::Error {
            message: "test error".to_owned(),
        },
    )
    .await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.ping().await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    // Ping should return false for unexpected responses
    assert!(result.is_ok());
    assert!(!result.unwrap());
}

// ===== Helper Function Tests =====

#[tokio::test]
#[serial]
async fn test_load_repository_state() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

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

    mock_server_normal(socket_path.clone(), response).await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.load_repository_state(".".to_owned()).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    assert!(result.is_ok());
    if let Ok(Response::RepositoryStatus(s)) = result {
        assert_eq!(s.branch, "main");
    } else {
        panic!("Unexpected response type");
    }
}

#[tokio::test]
#[serial]
async fn test_detect_languages() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    let response = Response::Language {
        languages: vec![LanguageInfo {
            name: "Rust".to_owned(),
            version: None,
            color: gpy_agent::config::types::ColorSpec::new("#dea584").unwrap(),
        }],
    };

    mock_server_normal(socket_path.clone(), response).await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.detect_languages(".".to_owned()).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    assert!(result.is_ok());
    if let Ok(Response::Language { languages }) = result {
        assert_eq!(languages.len(), 1);
        assert_eq!(languages[0].name, "Rust");
    } else {
        panic!("Unexpected response type");
    }
}

// ===== Socket Path Tests =====

#[test]
#[serial]
fn test_socket_path_custom() {
    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", "/custom/path.sock");
    }

    let path = SessionHandle::socket_path();

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    assert_eq!(path, "/custom/path.sock");
}

#[test]
#[serial]
fn test_socket_path_default() {
    // Clear all related env vars
    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("XDG_CACHE_HOME");
    }

    let path = SessionHandle::socket_path();

    // Should include gpy-test.sock
    assert!(path.contains("gpy-test.sock"));
}

// ===== Error Message Tests =====

#[tokio::test]
#[serial]
async fn test_connection_error_message() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("missing.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    let result = tokio::time::timeout(Duration::from_secs(6), SessionHandle::connect()).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    // Should have descriptive error message
    if let Ok(Err(err)) = result {
        let msg = err.to_string();
        assert!(msg.contains("connect") || msg.contains("timeout") || msg.contains("IPC"));
    }
}

#[tokio::test]
#[serial]
async fn test_send_error_message() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test.sock");

    unsafe {
        std::env::set_var("GPY_AGENT_SOCKET_PATH", socket_path.to_str().unwrap());
    }

    mock_server_disconnect(socket_path.clone()).await;
    sleep(Duration::from_millis(50)).await;

    let mut client = SessionHandle::connect().await.expect("Failed to connect");
    let result = client.send_request(Message::Ping).await;

    unsafe {
        std::env::remove_var("GPY_AGENT_SOCKET_PATH");
    }

    if let Err(err) = result {
        let msg = err.to_string();
        // Error should mention what failed
        assert!(
            msg.contains("read") || msg.contains("response") || msg.contains("IPC"),
            "Error message should be descriptive: {msg}"
        );
    }
}
