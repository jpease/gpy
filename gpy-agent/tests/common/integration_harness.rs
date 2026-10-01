//! Integration Test Harness
//!
//! Provides utilities for end-to-end integration testing of the GPY agent.
//! This harness spawns a full agent process, manages temporary git repositories,
//! and provides methods to send IPC requests and verify responses.
//!
//! # Architecture
//!
//! The harness creates:
//! - A separate agent process (daemonized via `gpy-agent start`)
//! - A temporary Unix socket for IPC communication (`TempSocket`)
//! - A temporary git repository for testing (`TempGitRepo`)
//! - Test configuration with automatic cleanup (`TestConfig`)
//!
//! # Example Usage
//!
//! ## Basic Agent Test
//!
//! ```no_run
//! use gpy_agent::ipc::{Message, Format};
//!
//! #[tokio::test]
//! async fn test_agent_responds_to_ping() {
//!     let server = IntegrationTestServer::new().await.unwrap();
//!
//!     // Send ping and verify response
//!     let response = server.ping().await.unwrap();
//!     assert!(matches!(response, Response::Ack));
//!
//!     // Verify agent is still alive
//!     server.assert_agent_alive().await.unwrap();
//! }
//! ```
//!
//! ## Git Status Test
//!
//! ```no_run
//! #[tokio::test]
//! async fn test_git_status_with_changes() {
//!     let mut server = IntegrationTestServer::new().await.unwrap();
//!
//!     // Trigger git changes
//!     server.trigger_git_change(GitChange::AddUntracked("test.txt".to_string())).unwrap();
//!
//!     // Get git status
//!     let status = server.get_git_status().await.unwrap();
//!     assert_eq!(status.untracked, 1);
//!
//!     // Stage file and verify
//!     server.trigger_git_change(GitChange::StageFile("test.txt".to_string())).unwrap();
//!     let status = server.get_git_status().await.unwrap();
//!     assert_eq!(status.staged, 1);
//! }
//! ```
//!
//! ## Custom Configuration Test
//!
//! ```no_run
//! #[tokio::test]
//! async fn test_with_custom_config() {
//!     let config = TestConfig::new()
//!         .with_git_disabled()
//!         .with_language_enabled();
//!
//!     let server = IntegrationTestServer::with_config(config).await.unwrap();
//!
//!     // Git should be disabled
//!     let status = server.get_git_status().await;
//!     assert!(status.is_err());
//! }
//! ```
//!
//! # When to Use This Harness
//!
//! **Use `IntegrationTestServer` when:**
//! - Testing the full agent binary end-to-end
//! - Testing agent startup, daemonization, and lifecycle
//! - Testing CLI flag handling and configuration loading
//! - Testing git repository interactions with a real agent process
//! - Simulating production deployment scenarios
//!
//! **Use `EndpointHandle` (from other integration tests) when:**
//! - Testing IPC protocol handling directly
//! - Testing server/client communication patterns
//! - Unit testing specific server components
//! - When faster test execution is needed (no process spawning)
//! - When testing internal server behavior without full agent
//!
//! # Cleanup
//!
//! The harness automatically cleans up resources on drop:
//! - Sends `Message::Shutdown` to the agent
//! - Removes temporary socket
//! - Removes temporary git repository
//! - Removes temporary configuration files

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::unwrap_used)]
#![allow(dead_code)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::io_other_error)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::let_unit_value)]
#![allow(clippy::derive_partial_eq_without_eq)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::as_conversions)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::shadow_reuse)]
#![allow(clippy::shadow_unrelated)]
#![allow(clippy::str_to_string)]
#![allow(clippy::needless_pass_by_ref_mut)]
#![allow(clippy::expect_used)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::cast_possible_truncation)]

#[path = "skip.rs"]
mod skip;

use gpy_agent::config::types::ClientPid;
use gpy_agent::ipc::{Format, Message, Response};
use gpy_agent::security::SafePath;
use std::io;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

// Import test fixtures from common module
mod fixtures;
use crate::fixtures::TestRepo;
use fixtures::TempSocket;

#[allow(clippy::duplicate_mod)]
#[path = "../fixtures/config_helpers.rs"]
pub mod config_helpers;
use config_helpers::TestConfig;

// Use config helpers from the crate's fixtures module
// use crate::fixtures::config_helpers::TestConfig; // Removed duplicate
// use crate::fixtures::TestRepo; // Removed

/// Integration test server for full end-to-end testing
///
/// Spawns a real agent process and provides methods to interact with it
/// via IPC. All resources are automatically cleaned up on drop.
pub struct IntegrationTestServer {
    agent_process: Option<Child>,
    socket: TempSocket,
    repo: TestRepo,
    config: TestConfig,
    _socket_dir: TempDir,
}

/// Git changes that can be triggered in the test repository
#[derive(Debug, Clone)]
pub enum GitChange {
    /// Add a new untracked file
    AddUntracked(String),
    /// Stage a file
    StageFile(String),
    /// Commit staged changes
    Commit(String),
    /// Create a new branch
    CreateBranch(String),
    /// Checkout a branch
    CheckoutBranch(String),
    /// Modify a tracked file (unstaged)
    ModifyFile(String, String),
}

/// Git status information
#[derive(Debug, Clone, PartialEq)]
pub struct GitStatus {
    pub branch: String,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
}

impl IntegrationTestServer {
    async fn send_request_to_socket(socket_path: &Path, msg: &Message) -> io::Result<Response> {
        let mut stream = timeout(Duration::from_secs(5), UnixStream::connect(socket_path))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Connection timeout"))?
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;

        let json = serde_json::to_string(msg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        stream.write_all(json.as_bytes()).await?;
        stream.write_all(b"\n").await?;

        let mut response_buf = vec![0_u8; 8192];
        let n = timeout(Duration::from_secs(10), stream.read(&mut response_buf))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Response timeout"))??;

        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Agent closed connection without response",
            ));
        }

        Self::parse_response_bytes(&response_buf[..n])
    }

    fn parse_response_bytes(response_buf: &[u8]) -> io::Result<Response> {
        let response_str = std::str::from_utf8(response_buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        if let Ok(resp) = serde_json::from_str::<Response>(response_str) {
            return Ok(resp);
        }

        if response_str.contains(r#""status":"ok""#) {
            return Ok(Response::Ack);
        }

        if response_str.contains(r#""error":"#) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(response_str) {
                if let Some(msg) = val.get("error").and_then(|v| v.as_str()) {
                    return Ok(Response::Error {
                        message: msg.to_owned(),
                    });
                }
            }
        }

        if response_str.contains(r#""branch":"#) && response_str.contains(r#""ahead":"#) {
            if let Ok(status_val) = serde_json::from_str::<serde_json::Value>(response_str) {
                let state =
                    std::str::FromStr::from_str(status_val["state"].as_str().unwrap_or_default())
                        .unwrap_or(gpy_agent::git::RepositoryState::InProgress);

                let status = gpy_agent::git::RepositoryStatus {
                    branch: status_val["branch"].as_str().unwrap_or_default().to_owned(),
                    ahead: status_val["ahead"].as_u64().unwrap_or(0) as u32,
                    behind: status_val["behind"].as_u64().unwrap_or(0) as u32,
                    ahead_capped: status_val["ahead_capped"].as_bool().unwrap_or(false),
                    behind_capped: status_val["behind_capped"].as_bool().unwrap_or(false),
                    staged: status_val["staged"].as_u64().unwrap_or(0) as u32,
                    unstaged: status_val["unstaged"].as_u64().unwrap_or(0) as u32,
                    untracked: status_val["untracked"].as_u64().unwrap_or(0) as u32,
                    conflicts: status_val["conflicts"].as_u64().unwrap_or(0) as u32,
                    state,
                    stash_count: 0,
                    detached: false,
                    rebase_progress: None,
                };
                return Ok(Response::RepositoryStatus(status));
            }
        }

        if response_str.contains(r#""languages":"#) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(response_str) {
                if let Some(langs_array) = val.get("languages").and_then(|v| v.as_array()) {
                    let fallback_color = gpy_agent::config::types::ColorSpec::new("#ffffff")
                        .map_err(|error| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("Failed to build fallback color spec: {error}"),
                            )
                        })?;
                    let mut languages = Vec::new();
                    for language in langs_array {
                        let color = gpy_agent::config::types::ColorSpec::new(
                            language["color"].as_str().unwrap_or("#ffffff"),
                        )
                        .unwrap_or_else(|_| fallback_color.clone());
                        languages.push(gpy_agent::ipc::LanguageInfo {
                            name: language["name"].as_str().unwrap_or_default().to_owned(),
                            version: language["version"].as_str().map(String::from),
                            color,
                        });
                    }
                    return Ok(Response::Language { languages });
                }
            }
        }

        serde_json::from_str(response_str).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Failed to deserialize response: {}. Body: {}",
                    e, response_str
                ),
            )
        })
    }

    async fn wait_until_ready(socket_path: &Path) -> io::Result<()> {
        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_secs(15))
            .ok_or_else(|| io::Error::other("Failed to compute readiness deadline"))?;
        let ping = Message::Ping;

        loop {
            match Self::send_request_to_socket(socket_path, &ping).await {
                Ok(Response::Ack) => return Ok(()),
                Ok(other) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Agent responded before ready with unexpected payload: {other:?}"),
                    ));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound
                            | io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "Agent did not respond to ping on socket {} within 15 seconds",
                                socket_path.display()
                            ),
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Create a new integration test server with default configuration
    ///
    /// This will:
    /// 1. Create a temporary git repository
    /// 2. Create a temporary Unix socket
    /// 3. Start the agent process
    /// 4. Wait for the agent to be ready
    ///
    /// # Errors
    ///
    /// Returns error if agent fails to start or socket setup fails
    pub async fn new() -> io::Result<Self> {
        Self::with_config(TestConfig::default()).await
    }

    /// Create a new integration test server with custom configuration
    ///
    /// # Arguments
    ///
    /// * `config` - Test configuration to use
    ///
    /// # Errors
    ///
    /// Returns error if agent fails to start or socket setup fails
    pub async fn with_config(config: TestConfig) -> io::Result<Self> {
        // Create temporary git repository
        let repo = TestRepo::new();

        // Create temporary socket
        let socket = TempSocket::new()?;
        let socket_dir = TempDir::new()?;

        // Write config to disk if needed
        config.write();

        // Start agent process (it will fork and daemonize)
        let agent_binary = env!("CARGO_BIN_EXE_gpy-agent");
        let status = Command::new(agent_binary)
            .arg("start")
            .arg("--socket")
            .arg(socket.path())
            .env("GPY_CONFIG_PATH", config.path())
            .status()?;

        if !status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Agent start command failed with status: {}", status),
            ));
        }

        Self::wait_until_ready(socket.path()).await?;

        Ok(Self {
            agent_process: None, // Agent is daemonized, we don't track its process
            socket,
            repo,
            config,
            _socket_dir: socket_dir,
        })
    }

    /// Send an IPC request to the agent and wait for response
    ///
    /// # Arguments
    ///
    /// * `msg` - IPC message to send
    ///
    /// # Returns
    ///
    /// Response from the agent
    ///
    /// # Errors
    ///
    /// Returns error if connection or communication fails
    pub async fn send_request(&self, msg: Message) -> io::Result<Response> {
        Self::send_request_to_socket(self.socket.path(), &msg).await
    }

    /// Trigger a git change in the test repository
    ///
    /// # Arguments
    ///
    /// * `change` - The git change to apply
    ///
    /// # Errors
    ///
    /// Returns error if git command fails
    pub fn trigger_git_change(&mut self, change: GitChange) -> io::Result<()> {
        match change {
            GitChange::AddUntracked(filename) => {
                std::fs::write(self.repo.path().join(&filename), "test content")?;
            }
            GitChange::StageFile(filename) => {
                self.git_run(&["add", &filename])?;
            }
            GitChange::Commit(message) => {
                self.git_run(&["commit", "-m", &message])?;
            }
            GitChange::CreateBranch(branch) => {
                self.git_run(&["checkout", "-b", &branch])?;
            }
            GitChange::CheckoutBranch(branch) => {
                self.git_run(&["checkout", &branch])?;
            }
            GitChange::ModifyFile(filename, content) => {
                std::fs::write(self.repo.path().join(&filename), content)?;
            }
        }
        Ok(())
    }

    /// Get current git status from the test repository
    ///
    /// # Errors
    ///
    /// Returns error if git status request fails
    pub async fn get_git_status(&self) -> io::Result<GitStatus> {
        let repo_path = self.repo.path().to_string_lossy().to_string();
        let safe_path = SafePath::new(&repo_path)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        let response = self
            .send_request(Message::RepositoryStatus {
                path: safe_path,
                format: Format::Json,
                is_last: false,
                is_first: false,
                prev_bg: None,
            })
            .await?;

        // Parse response
        match response {
            Response::RepositoryStatus(s) => Ok(GitStatus {
                branch: s.branch,
                staged: s.staged as i64,
                unstaged: s.unstaged as i64,
                untracked: s.untracked as i64,
            }),
            Response::Error { message } => Err(io::Error::new(io::ErrorKind::Other, message)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unexpected response type",
            )),
        }
    }

    /// Assert that the agent process is still alive
    ///
    /// This is an async check that can be called in async contexts.
    /// For daemonized agents, we check by attempting to connect to the socket.
    ///
    /// # Errors
    ///
    /// Returns error if agent is not responsive
    pub async fn assert_agent_alive(&self) -> io::Result<()> {
        match self.ping().await? {
            Response::Ack => Ok(()),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Agent responded to ping with unexpected payload: {other:?}"),
            )),
        }
    }

    /// Get the path to the test repository
    pub fn repo_path(&self) -> &Path {
        self.repo.path()
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &Path {
        self.socket.path()
    }

    /// Get reference to the test configuration
    pub fn config(&self) -> &TestConfig {
        &self.config
    }

    /// Send a ping request to the agent
    ///
    /// # Errors
    ///
    /// Returns error if ping fails
    pub async fn ping(&self) -> io::Result<Response> {
        self.send_request(Message::Ping).await
    }

    /// Send an agent status request
    ///
    /// # Errors
    ///
    /// Returns error if status request fails
    pub async fn agent_status(&self) -> io::Result<Response> {
        self.send_request(Message::Status).await
    }

    /// Run a git command in the test repository
    fn git_run(&self, args: &[&str]) -> io::Result<()> {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.repo.path())
            .output()?;

        if !output.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "git {:?} failed: {}",
                    args,
                    String::from_utf8_lossy(&output.stderr)
                ),
            ));
        }

        Ok(())
    }
}

impl Drop for IntegrationTestServer {
    fn drop(&mut self) {
        let agent_binary = env!("CARGO_BIN_EXE_gpy-agent");
        let _ = Command::new(agent_binary)
            .arg("stop")
            .arg("--socket")
            .arg(self.socket.path())
            .status();

        // Wait briefly for the daemon to tear down the socket so nextest doesn't
        // mark the test as leaky because the child is still exiting.
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(2))
            .expect("valid shutdown wait deadline");
        while self.socket.path().exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }

        // TempSocket, TempGitRepo, and TestConfig handle their own cleanup
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that the harness can start and stop cleanly
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
                // If agent binary doesn't exist or fails to start, skip test
                super::skip::skip_test(&format!("agent failed to start: {e}"));
                return;
            }
        }
    }

    /// Test sending a ping request
    #[tokio::test]
    #[serial_test::serial]
    async fn test_harness_sends_ping() {
        let result = IntegrationTestServer::new().await;

        let server = match result {
            Ok(s) => s,
            Err(e) => {
                super::skip::skip_test(&format!("agent failed to start: {e}"));
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
        match SafePath::new(path) {
            Ok(safe_path) => safe_path,
            Err(error) => panic!("Expected valid test path '{path}', got error: {error}"),
        }
    }

    fn expect_test_client_pid(pid: u32) -> ClientPid {
        match ClientPid::new(pid) {
            Ok(client_pid) => client_pid,
            Err(error) => panic!("Expected valid test pid '{pid}', got error: {error}"),
        }
    }

    async fn expect_git_status(server: &IntegrationTestServer, context: &str) -> GitStatus {
        match server.get_git_status().await {
            Ok(status) => status,
            Err(error) => panic!("{context}: {error}"),
        }
    }

    async fn expect_registration(server: &IntegrationTestServer, message: Message, context: &str) {
        match server.send_request(message).await {
            Ok(Response::Ack) => {}
            Ok(other) => {
                panic!("{context}: expected Ack, got {other:?}");
            }
            Err(error) => panic!("{context}: {error}"),
        }
    }

    /// Test getting git status through the harness
    #[tokio::test]
    #[serial_test::serial]
    async fn test_harness_git_status() {
        let result = IntegrationTestServer::new().await;

        let server = match result {
            Ok(s) => s,
            Err(e) => {
                super::skip::skip_test(&format!("agent failed to start: {e}"));
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

    /// Test triggering git changes through the harness
    ///
    /// gpy-agent#388: previously `#[ignore]`d after reproducing a deterministic
    /// (18/18) worktree-watch delivery failure, root-caused at the time to
    /// `RegisterClient` arming the `.git` watch then (since
    /// `git.watch_worktree` defaults to `true`) the worktree-root watch, with
    /// the worktree-level file-creation event never delivered. A narrower fix
    /// (skipping the redundant `.git`-dir watch when it's covered by the
    /// worktree-root watch) landed in `start_repo_watches`, and this test now
    /// passes reliably (30/30 isolated + full-nextest-contention runs) with
    /// that fix in place. The deeper `Agent::new()` watcher-startup-sequencing
    /// theory floated as a possible contributing factor did not need to be
    /// pursued -- the deterministic reproduction did not recur once retested,
    /// consistent with disk pressure (the host was at 99% capacity during the
    /// original investigation and has since been cleared) rather than pure
    /// application-level sequencing being the dominant factor. Re-added to the
    /// `watcher_fsevents_bootstrap` `file_serial` group used by #384/#385/#386
    /// as defense-in-depth: like those tests, this one bootstraps a real
    /// `FSEventStream` in a full agent process and should not do so concurrently
    /// with sibling watcher tests across nextest's per-test processes.
    #[tokio::test]
    #[serial_test::serial]
    #[serial_test::file_serial(watcher_fsevents_bootstrap)]
    async fn test_harness_git_changes() {
        let result = IntegrationTestServer::new().await;

        let mut server = match result {
            Ok(s) => s,
            Err(e) => {
                super::skip::skip_test(&format!("agent failed to start: {e}"));
                return;
            }
        };

        // Register client to enable watcher and live updates
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

        // Initial state should be clean
        let status = expect_git_status(&server, "Initial git status should succeed").await;
        assert_eq!(
            status.untracked, 0,
            "Initial repository should have 0 untracked files"
        );

        // Add an untracked file
        server
            .trigger_git_change(GitChange::AddUntracked("test.txt".to_string()))
            .expect("Adding untracked file should succeed");

        // Retry loop for status check (30 * 200ms = 6s total; widened from 2s,
        // see gpy-agent#386 above)
        let mut status = expect_git_status(&server, "Git status after adding file").await;
        for _ in 0..30 {
            if status.untracked == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            status = expect_git_status(&server, "Polling git status for untracked file").await;
        }

        // Verify untracked file appears in status
        assert_eq!(
            status.untracked, 1,
            "After adding untracked file, should have 1 untracked file"
        );

        // Stage file and verify
        server
            .trigger_git_change(GitChange::StageFile("test.txt".to_string()))
            .expect("Staging file should succeed");

        // Retry loop for status check (30 * 200ms = 6s total; widened from 2s,
        // see gpy-agent#386 above)
        status = expect_git_status(&server, "Git status after staging file").await;
        for _ in 0..30 {
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

        // Verify agent is still alive after all operations
        assert!(
            server.assert_agent_alive().await.is_ok(),
            "Agent should remain alive after git operations"
        );
    }
}
