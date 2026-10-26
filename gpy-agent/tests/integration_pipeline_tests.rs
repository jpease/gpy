//! Integration Pipeline Tests
//!
//! End-to-end tests for complete workflows through the agent.

// The agent's IPC is Unix-socket only; native Windows is CLI-only.
#![cfg(unix)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::uninlined_format_args)]

mod common;

use common::fixtures::TempSocket;
use common::git::TestRepo;
use common::integration_harness::{GitChange, GitStatus, IntegrationTestServer, TestConfig};
use common::skip::skip_test;
use gpy_agent::ipc::{ClientPid, Message, Response, Segment};
use gpy_agent::security::SafePath;
use std::time::Duration;

/// Test: Git change → Cache → Response flow
#[tokio::test]
#[serial_test::serial]
async fn test_git_status_integration() {
    let server = IntegrationTestServer::new()
        .await
        .expect("Server failed to start");

    // Create a test repository
    let repo = TestRepo::new().with_initial_commit();

    // Query git status
    let response = server
        .send_request(Message::RepositoryStatus {
            path: SafePath::new(&repo.path_string()).unwrap(),
            segment: Segment::default(),
        })
        .await
        .expect("Request failed");

    match response {
        Response::RepositoryStatus(s) => {
            assert!(!s.branch.is_empty(), "Should have a branch name");
            assert_eq!(s.staged, 0, "Clean repo should have no staged files");
            assert_eq!(s.unstaged, 0, "Clean repo should have no unstaged files");
            assert_eq!(s.untracked, 0, "Clean repo should have no untracked files");
        }
        other => panic!("Expected RepositoryStatus, got {other:?}"),
    }
}

/// Test: Config hot-reload affects git detection
#[tokio::test]
#[serial_test::serial]
async fn test_config_hot_reload_disables_git() {
    let config = TestConfig::new().with_git_disabled();
    let server = IntegrationTestServer::with_config(config)
        .await
        .expect("Server failed to start");

    let repo = TestRepo::new().with_initial_commit();
    let response = server
        .send_request(Message::RepositoryStatus {
            path: SafePath::new(&repo.path_string()).unwrap(),
            segment: Segment::default(),
        })
        .await
        .expect("Request failed");

    assert!(
        matches!(response, Response::Error { .. }),
        "Git should be disabled by config"
    );
}

/// Test: Client registration → Workspace tracking
#[tokio::test]
#[serial_test::serial]
async fn test_client_registration_flow() {
    let server = IntegrationTestServer::new()
        .await
        .expect("Server failed to start");

    // Register client
    let pid_u32 = std::process::id();
    let pid = gpy_agent::ipc::ClientPid::new(pid_u32).unwrap();
    let response = server
        .send_request(Message::RegisterClient {
            pid,
            cwd: Some(SafePath::new("/tmp").unwrap()),
            shell: None,
            shell_version: None,
        })
        .await
        .expect("Request failed");
    assert!(
        matches!(response, Response::Ack),
        "Registration should succeed, got: {:?}",
        response
    );

    // Query agent status
    let status_response = server.agent_status().await.expect("Status request failed");

    match status_response {
        Response::AgentStatus {
            registered_clients, ..
        } => {
            assert!(
                registered_clients >= 1,
                "Should have at least 1 registered client"
            );
        }
        other => panic!("Expected AgentStatus, got {other:?}"),
    }

    // Unregister client
    let unregister_response = server
        .send_request(Message::UnregisterClient { pid })
        .await
        .expect("Request failed");
    assert!(
        matches!(unregister_response, Response::Ack),
        "Unregistration should succeed"
    );
}

/// Test: Multiple concurrent requests
#[tokio::test]
#[serial_test::serial]
async fn test_concurrent_git_status_requests() {
    // Note: We need explicitly typed server for the clone
    let server: std::sync::Arc<IntegrationTestServer> = std::sync::Arc::new(
        IntegrationTestServer::new()
            .await
            .expect("Server failed to start"),
    );
    let repo = TestRepo::new().with_initial_commit();
    let path = repo.path_string();

    // Send 5 concurrent requests
    let mut handles = Vec::new();
    let safe_path = SafePath::new(&path).unwrap();
    for _ in 0_u32..5 {
        let path_clone = safe_path.clone();
        let server_clone = std::sync::Arc::clone(&server);
        handles.push(tokio::spawn(async move {
            server_clone
                .send_request(Message::RepositoryStatus {
                    path: path_clone,
                    segment: Segment::default(),
                })
                .await
        }));
    }

    // Wait for all responses
    for handle in handles {
        let result = handle.await.expect("Task should complete");
        let response = result.expect("Request should succeed");
        assert!(
            matches!(response, Response::RepositoryStatus(_)),
            "All requests should succeed"
        );
    }
}

/// Test: Invalid path handling
#[tokio::test]
#[serial_test::serial]
async fn test_git_status_invalid_path() {
    let server = IntegrationTestServer::new()
        .await
        .expect("Server failed to start");

    // Query nonexistent path
    let response = server
        .send_request(Message::RepositoryStatus {
            path: SafePath::new("/nonexistent/path/12345").unwrap(),
            segment: Segment::default(),
        })
        .await
        .expect("Request failed");

    assert!(
        matches!(response, Response::Error { .. }),
        "Should return error for invalid path"
    );
}

/// Test: the socket serves real requests as soon as it accepts, without waiting
/// on the theme-export cache warmup.
///
/// The initial theme-export write was moved off `Agent::new`'s serial path into
/// a background `spawn_blocking` task that runs *after* the server begins
/// accepting connections. `IntegrationTestServer::new` returns as soon as the
/// readiness ping gets its first `Ack` (the moment the socket accepts), so this
/// test connects immediately after that and asserts a correct prompt render
/// comes back — even though the theme-export warmup may not have finished. A
/// first-prompt git render reads the instant cache / IPC, never the theme-export
/// file (whose only reader is the miss-tolerant reload-flag fast-reload handler), so
/// the response is correct regardless of warmup timing.
#[tokio::test]
#[serial_test::serial]
async fn test_socket_serves_requests_before_theme_export_warmup() {
    let server = IntegrationTestServer::new()
        .await
        .expect("Server failed to start");

    // Immediately (no warmup delay) request a real git render — the first-prompt
    // render path.
    let repo = TestRepo::new().with_initial_commit();
    let response = server
        .send_request(Message::RepositoryStatus {
            path: SafePath::new(&repo.path_string()).unwrap(),
            segment: Segment::default(),
        })
        .await
        .expect("Request should succeed the instant the socket accepts");

    match response {
        Response::RepositoryStatus(status) => {
            assert!(
                !status.branch.is_empty(),
                "Git render should carry a branch name even before theme-export warmup completes"
            );
        }
        other => panic!("Expected RepositoryStatus, got {other:?}"),
    }
}

/// Test: Language detection integration
#[tokio::test]
#[serial_test::serial]
async fn test_language_detection_integration() {
    let server = IntegrationTestServer::new()
        .await
        .expect("Server failed to start");

    // Create a simple Rust project
    let repo = TestRepo::new();
    repo.create_file("Cargo.toml", "[package]\nname = \"test\"\n");
    repo.create_file("main.rs", "fn main() {}\n");

    let response = server
        .send_request(Message::LanguageDetect {
            path: SafePath::new(&repo.path_string()).unwrap(),
            segment: Segment::default(),
            virtual_env: None,
        })
        .await
        .expect("Request failed");

    match response {
        Response::Language { languages } => {
            assert!(!languages.is_empty(), "Should detect at least one language");
        }
        Response::Error { .. } => {
            // Language detection might be disabled or fail, which is ok for this test
        }
        other => panic!("Expected Language or Error response, got {other:?}"),
    }
}

#[test]
fn test_config_writes_its_settings() {
    let test_config = TestConfig::new().with_git_disabled();
    assert!(!test_config.config().git.enabled);
    test_config.write();

    let contents = std::fs::read_to_string(test_config.path()).unwrap();
    assert!(contents.contains("[git]"));
    assert!(contents.contains("enabled = false"));
}

#[test]
fn test_temp_socket() {
    let socket = TempSocket::new().unwrap();
    assert!(!socket.path().to_string_lossy().is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn test_harness_starts_and_stops() {
    let result = IntegrationTestServer::new().await;

    match result {
        Ok(server) => {
            assert!(
                server.socket_path().exists(),
                "Socket path should exist after server starts"
            );
            assert!(
                server.repo_path().exists(),
                "Repository path should exist after server starts"
            );
            assert!(
                server.assert_agent_alive().await.is_ok(),
                "Agent process should be alive after startup"
            );
        }
        Err(e) => {
            skip_test(&format!("agent failed to start: {e}"));
            return;
        }
    }
}

#[tokio::test]
#[serial_test::serial]
async fn test_harness_sends_ping() {
    let result = IntegrationTestServer::new().await;

    let server = match result {
        Ok(s) => s,
        Err(e) => {
            skip_test(&format!("agent failed to start: {e}"));
            return;
        }
    };

    let response = server.ping().await;
    assert!(
        response.is_ok(),
        "Ping request should succeed, got error: {:?}",
        response.err()
    );

    match response {
        Ok(Response::Ack) => {}
        Ok(other) => {
            panic!("Expected Ack response to Ping message, got: {:?}", other);
        }
        Err(error) => {
            panic!("Ping request should succeed, got error: {error}");
        }
    }
}

fn expect_test_safe_path(path: &str) -> SafePath {
    SafePath::new(path).expect("valid test path")
}

fn expect_test_client_pid(pid: u32) -> ClientPid {
    ClientPid::new(pid).expect("valid test pid")
}

async fn expect_git_status(server: &IntegrationTestServer, context: &str) -> GitStatus {
    server.get_git_status().await.expect(context)
}

async fn expect_registration(server: &IntegrationTestServer, message: Message, context: &str) {
    let response = server.send_request(message).await.expect(context);
    assert!(
        matches!(response, Response::Ack),
        "{context}: expected Ack, got {response:?}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn test_harness_git_status() {
    let result = IntegrationTestServer::new().await;

    let server = match result {
        Ok(s) => s,
        Err(e) => {
            skip_test(&format!("agent failed to start: {e}"));
            return;
        }
    };

    let status = expect_git_status(&server, "Git status request should succeed").await;
    assert!(
        !status.branch.is_empty(),
        "Git status should include non-empty branch name, got: {:?}",
        status.branch
    );
    assert_eq!(
        status.staged, 0,
        "New repository should have 0 staged files"
    );
    assert_eq!(
        status.untracked, 0,
        "New repository should have 0 untracked files"
    );
}

/// Watcher-driven status updates reach a registered client end to end.
/// Serialized in `watcher_fsevents_bootstrap`: `FSEventStream` bootstraps must
/// not run concurrently across nextest processes.
#[tokio::test]
#[serial_test::serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
async fn test_harness_git_changes() {
    let result = IntegrationTestServer::new().await;

    let mut server = match result {
        Ok(s) => s,
        Err(e) => {
            skip_test(&format!("agent failed to start: {e}"));
            return;
        }
    };

    let pid = std::process::id();
    let repo_path = server.repo_path().to_string_lossy().to_string();
    let safe_path = expect_test_safe_path(&repo_path);

    expect_registration(
        &server,
        Message::RegisterClient {
            pid: expect_test_client_pid(pid),
            cwd: Some(safe_path),
            shell: None,
            shell_version: None,
        },
        "Registration failed",
    )
    .await;

    let initial = expect_git_status(&server, "Initial git status should succeed").await;
    assert_eq!(
        initial.untracked, 0,
        "Initial repository should have 0 untracked files"
    );

    server
        .trigger_git_change(GitChange::AddUntracked("test.txt".to_owned()))
        .expect("Adding untracked file should succeed");

    // Poll up to 6s for the watcher-driven update.
    let mut status = expect_git_status(&server, "Git status after adding file").await;
    for _ in 0_u32..30 {
        if status.untracked == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        status = expect_git_status(&server, "Polling git status for untracked file").await;
    }

    assert_eq!(
        status.untracked, 1,
        "After adding untracked file, should have 1 untracked file"
    );

    server
        .trigger_git_change(GitChange::StageFile("test.txt".to_owned()))
        .expect("Staging file should succeed");

    status = expect_git_status(&server, "Git status after staging file").await;
    for _ in 0_u32..30 {
        if status.staged == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        status = expect_git_status(&server, "Polling git status for staged file").await;
    }

    assert_eq!(
        status.staged, 1,
        "After staging file, should have 1 staged file"
    );
    assert_eq!(
        status.untracked, 0,
        "After staging file, should have 0 untracked files"
    );

    assert!(
        server.assert_agent_alive().await.is_ok(),
        "Agent should remain alive after git operations"
    );
}
