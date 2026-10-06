//! End-to-end integration tests for the IPC server.
//!
//! These tests start a real IPC server in a background task and connect to it
//! with a client to test the full request-response cycle.
//!
//! # Test Navigation Map
//!
//! **Total Tests**: 4
//! **Last Updated**: 2026-01-20
//!
//! ## Test Categories
//!
//! ### ✅ Basic Operations (2 tests)
//! Tests covering normal/happy path scenarios:
//! - `test_e2e_ping` - Full IPC server ping request/response cycle
//! - `test_e2e_agent_status_includes_version` - Agent status request includes protocol version and metadata
//!
//! ### ❌ Error Cases (0 tests)
//! Tests covering error scenarios and failure modes:
//! - (None currently)
//!
//! ### 🔀 Edge Cases (1 test)
//! Tests covering boundary conditions:
//! - `test_e2e_skip_paths` - Skip paths configuration prevents git detection
//!
//! ### 🔗 Integration (1 test)
//! Tests covering cross-module interactions:
//! - `test_e2e_git_status` - Full E2E git status through IPC with temp repo
//!
//! ## What's NOT Tested (TODO)
//! Known gaps for this module (from the original planning notes, since removed):
//! - Error responses for invalid message formats
//! - Server behavior when client disconnects mid-request
//! - Multiple sequential requests on same connection
//! - Language detection E2E through IPC
//! - Workspace update E2E flow
//! - Client registration and unregistration E2E
//! - Config hot-reload E2E scenarios
//! - Skip paths E2E (currently ignored, needs redesign)
//! - Theme changes propagation E2E
//! - Git cache invalidation E2E
//!
//! ## Related Code
//! - **Production**: `src/ipc/server.rs`, `src/ipc/mod.rs`, `src/git/status.rs`, `src/config/manager.rs`
//! - **Other Tests**: `integration_tests.rs`, `agent_daemon_tests.rs`, `git_status_tests.rs`
//! - **Fixtures**: None (uses tempfile for temp directories)
//!
//! ## Quick Reference
//! ```bash
//! cargo test --test e2e_ipc_tests
//! cargo test --test e2e_ipc_tests -- --nocapture
//! cargo test --test e2e_ipc_tests --ignored  # Run ignored tests
//! ```

#![allow(clippy::panic)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)] // Test functions panic on assertion failures
#![allow(clippy::missing_errors_doc)]

mod common;

use common::gpy_test_root;
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::ipc::ClientDirectory;
use gpy_agent::ipc::LatencyTracker;
use gpy_agent::ipc::server::EndpointHandle;
use gpy_agent::theme::ThemeManager;
use gpy_agent::{Error, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Start an in-process IPC server for testing, wait until the socket is ready, and return
/// the join handle, socket path, and the `TempDir` guard (must stay alive for the duration).
async fn start_test_ipc_server(
    socket_name: &str,
    config_manager: Arc<ConfigManager>,
) -> (tokio::task::JoinHandle<()>, PathBuf, TempDir) {
    let root = gpy_test_root().expect("failed to create gpy test root");
    let temp_dir = tempfile::Builder::new()
        .prefix("sock-")
        .tempdir_in(&root)
        .expect("failed to create temp dir");
    let socket_path = temp_dir.path().join(socket_name);
    let server_socket_path = socket_path.clone();

    let registry = ClientDirectory::new().shared();
    let server_registry = Arc::clone(&registry);
    let git_cache = Arc::new(GitStatusCache::new());
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let latency_tracker = Arc::new(LatencyTracker::new(100));

    let server_handle = tokio::spawn(async move {
        let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
        let mut server = EndpointHandle::builder()
            .socket_path(server_socket_path)
            .client_registry(server_registry)
            .git_cache(git_cache)
            .config_manager(config_manager)
            .watcher_slot(Arc::new(std::sync::Mutex::new(None)))
            .theme_manager(theme_manager)
            .instant_cache(instant_cache)
            .latency_tracker(latency_tracker)
            .language_cache(gpy_agent::language::DetectionCache::new())
            .build()
            .expect("Server failed to start");
        server.start().await.expect("Server failed to start");
    });

    timeout(Duration::from_secs(2), async {
        while !socket_path.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Server socket was not created in time");

    (server_handle, socket_path, temp_dir)
}

/// Read a single IPC response line from a `UnixStream`.
async fn read_ipc_response(stream: &mut UnixStream) -> Result<String> {
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).await?;
    buf.get(..n)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(str::to_owned)
        .ok_or_else(|| Error::ipc("Invalid UTF-8 in IPC response".to_owned()))
}

/// Test that the server can be started and a client can successfully ping it.
#[tokio::test]
async fn test_e2e_ping() -> Result<()> {
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let (server_handle, socket_path, _temp_dir) =
        start_test_ipc_server("gpy-test-ping.sock", config_manager).await;

    let mut stream = timeout(Duration::from_secs(2), UnixStream::connect(&socket_path))
        .await
        .expect("Failed to connect to server")?;

    stream.write_all(b"\"Ping\"\n").await?;

    let response = read_ipc_response(&mut stream).await?;
    assert!(
        response.contains(r#""status":"ok""#),
        "Ping response should contain status ok field"
    );

    server_handle.abort();
    Ok(())
}

/// Test a git status request through the running IPC server.
#[tokio::test]
async fn test_e2e_git_status() -> Result<()> {
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let (server_handle, socket_path, _temp_dir) =
        start_test_ipc_server("gpy-test-git.sock", config_manager).await;

    // Set up a temporary committed git repository with a clean working tree
    let repo_dir = tempdir()?;
    let repo_path = repo_dir.path();
    for args in [
        vec!["init"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
    ] {
        std::process::Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .output()?;
    }
    std::fs::write(repo_path.join("README.md"), "# Test Repo\n")?;
    std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(repo_path)
        .output()?;
    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(repo_path)
        .output()?;

    let mut stream = timeout(Duration::from_secs(2), UnixStream::connect(&socket_path))
        .await
        .expect("Failed to connect to server")?;

    let path_str = repo_path.to_str().unwrap().replace('\\', "\\\\");
    let git_request = format!(r#"{{"RepositoryStatus":{{"path":"{path_str}"}}}}"#);
    stream.write_all(git_request.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let response_str = read_ipc_response(&mut stream).await?;
    let response_json: serde_json::Value = serde_json::from_str(&response_str)?;

    let branch = response_json
        .get("branch")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        !branch.is_empty(),
        "Git status response should include a non-empty branch name"
    );

    let staged = response_json
        .get("staged")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    assert_eq!(
        staged, 0_i64,
        "Clean test repository should have 0 staged files"
    );

    let untracked = response_json
        .get("untracked")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    assert_eq!(
        untracked, 0_i64,
        "Committed test repository should have 0 untracked files"
    );

    server_handle.abort();
    Ok(())
}

/// Test that `skip_paths` configuration prevents git detection in specified paths.
#[tokio::test]
async fn test_e2e_skip_paths() -> Result<()> {
    let repo_dir = tempdir()?;
    let repo_path = repo_dir.path();
    std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .output()?;

    let canonical_repo_path = repo_path.canonicalize()?;
    let skip_path_str = canonical_repo_path.to_string_lossy().into_owned();

    let config_dir = tempdir()?;
    let config_path = config_dir.path().join("config.toml");
    let escaped = skip_path_str.replace('\\', "\\\\");
    std::fs::write(
        &config_path,
        format!("[git]\nenabled = true\nskip_paths = [\"{escaped}\"]\n"),
    )?;

    let config_manager =
        Arc::new(ConfigManager::from_path(&config_path).expect("config should load"));
    let (server_handle, socket_path, _temp_dir) =
        start_test_ipc_server("gpy-test-skip.sock", config_manager).await;

    let mut stream = timeout(Duration::from_secs(2), UnixStream::connect(&socket_path))
        .await
        .expect("Failed to connect to server")?;

    let git_request = format!(r#"{{"RepositoryStatus":{{"path":"{escaped}"}}}}"#);
    stream.write_all(git_request.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let response = read_ipc_response(&mut stream).await?;
    assert!(
        response.contains("skipped") || response.contains("disabled"),
        "Expected error message about skipped path, got: {response}"
    );

    server_handle.abort();
    Ok(())
}

/// Test agent status request returns protocol version information.
#[tokio::test]
async fn test_e2e_agent_status_includes_version() -> Result<()> {
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let (server_handle, socket_path, _temp_dir) =
        start_test_ipc_server("gpy-test-status.sock", config_manager).await;

    let mut stream = timeout(Duration::from_secs(2), UnixStream::connect(&socket_path))
        .await
        .expect("Failed to connect to server")?;

    stream.write_all(b"\"Status\"\n").await?;

    let response_str = read_ipc_response(&mut stream).await?;
    let response_json: serde_json::Value = serde_json::from_str(&response_str)?;

    let agent_status = response_json
        .get("AgentStatus")
        .ok_or_else(|| Error::ipc("Response missing AgentStatus field".to_owned()))?;

    let version = agent_status
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::ipc("AgentStatus missing version field".to_owned()))?;

    assert!(
        !version.is_empty(),
        "Agent version should not be empty string"
    );
    assert!(
        version.contains('.'),
        "Agent version should follow semver format with dots: {version}"
    );
    assert!(
        agent_status.get("watched_repos").is_some(),
        "AgentStatus should include watched_repos count field"
    );
    assert!(
        agent_status.get("registered_clients").is_some(),
        "AgentStatus should include registered_clients count field"
    );
    assert!(
        agent_status.get("cache_entries").is_some(),
        "AgentStatus should include cache_entries count field"
    );

    server_handle.abort();
    Ok(())
}
