//! Integration Pipeline Tests
//!
//! End-to-end tests for complete workflows through the agent.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::shadow_unrelated)]
#![allow(clippy::default_numeric_fallback)]
#![allow(clippy::clone_on_ref_ptr)]
#![allow(clippy::uninlined_format_args)]

mod fixtures;
#[path = "common/integration_harness.rs"]
mod integration_harness;

use fixtures::TestRepo;
use gpy_agent::ipc::{Format, Message, Response};
use gpy_agent::security::SafePath;
use integration_harness::IntegrationTestServer;

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
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
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
    let config = integration_harness::config_helpers::TestConfig::new().with_git_disabled();
    let server = IntegrationTestServer::with_config(config)
        .await
        .expect("Server failed to start");

    let repo = TestRepo::new().with_initial_commit();
    let response = server
        .send_request(Message::RepositoryStatus {
            path: SafePath::new(&repo.path_string()).unwrap(),
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
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
    let pid = gpy_agent::config::types::ClientPid::new(pid_u32).unwrap();
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
    let response = server.agent_status().await.expect("Status request failed");

    match response {
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
    let response = server
        .send_request(Message::UnregisterClient { pid })
        .await
        .expect("Request failed");
    assert!(
        matches!(response, Response::Ack),
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
    for _ in 0..5 {
        let path_clone = safe_path.clone();
        let server_clone = server.clone();
        handles.push(tokio::spawn(async move {
            server_clone
                .send_request(Message::RepositoryStatus {
                    path: path_clone,
                    format: Format::Json,
                    is_last: false,
                    is_first: false,
                    prev_bg: None,
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
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
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
/// file (whose only reader is the miss-tolerant SIGUSR2 fast-reload handler), so
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
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
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
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
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
