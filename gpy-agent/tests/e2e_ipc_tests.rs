//! End-to-end IPC tests: an in-process server on a real Unix socket answering
//! ping, git status (including `skip_paths`), and agent status requests.

// The agent's IPC is Unix-socket only; native Windows is CLI-only.
#![cfg(unix)]
#![allow(clippy::panic)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)] // Test functions panic on assertion failures
#![allow(clippy::missing_errors_doc)]

mod common;

use common::endpoint::{endpoint_builder_with_config, wait_for_socket};
use common::fixtures::gpy_test_root;
use common::git::{git, git_init};
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::{Error, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Start an in-process IPC server and wait for its socket; keep the `TempDir` alive.
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

    let mut server = endpoint_builder_with_config(socket_path.clone(), config_manager)
        .build()
        .expect("Server failed to build");
    let server_handle = tokio::spawn(async move {
        server.start().await.expect("Server failed to start");
    });

    wait_for_socket(&socket_path, Duration::from_secs(2)).await;

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

#[tokio::test]
async fn test_e2e_ping() -> Result<()> {
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let (server_handle, socket_path, _temp_dir) =
        start_test_ipc_server("gpy-test-ping.sock", config_manager).await;

    let mut stream = timeout(Duration::from_secs(2), UnixStream::connect(&socket_path))
        .await
        .expect("Failed to connect to server")?;

    stream.write_all(b"{\"op\":\"ping\"}\n").await?;

    let response = read_ipc_response(&mut stream).await?;
    assert!(
        response.contains(r#""status":"ok""#),
        "Ping response should contain status ok field"
    );

    server_handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_e2e_git_status() -> Result<()> {
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let (server_handle, socket_path, _temp_dir) =
        start_test_ipc_server("gpy-test-git.sock", config_manager).await;

    let repo_dir = tempdir()?;
    let repo_path = repo_dir.path();
    git_init(repo_path);
    std::fs::write(repo_path.join("README.md"), "# Test Repo\n")?;
    git(repo_path, &["add", "README.md"]);
    git(repo_path, &["commit", "-q", "-m", "Initial commit"]);

    let mut stream = timeout(Duration::from_secs(2), UnixStream::connect(&socket_path))
        .await
        .expect("Failed to connect to server")?;

    let path_str = repo_path.to_str().unwrap().replace('\\', "\\\\");
    let git_request = format!(r#"{{"op":"git","cwd":"{path_str}"}}"#);
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

/// `skip_paths` suppresses git detection in listed paths.
#[tokio::test]
async fn test_e2e_skip_paths() -> Result<()> {
    let repo_dir = tempdir()?;
    let repo_path = repo_dir.path();
    git_init(repo_path);

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

    let git_request = format!(r#"{{"op":"git","cwd":"{escaped}"}}"#);
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

#[tokio::test]
async fn test_e2e_agent_status_includes_version() -> Result<()> {
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let (server_handle, socket_path, _temp_dir) =
        start_test_ipc_server("gpy-test-status.sock", config_manager).await;

    let mut stream = timeout(Duration::from_secs(2), UnixStream::connect(&socket_path))
        .await
        .expect("Failed to connect to server")?;

    stream
        .write_all(b"{\"op\":\"status\",\"format\":\"json\"}\n")
        .await?;

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
    assert_eq!(
        response_json.as_object().map(serde_json::Map::len),
        Some(1),
        "status reply must be exactly {{\"AgentStatus\":{{…}}}}: {response_str}"
    );

    server_handle.abort();
    Ok(())
}
