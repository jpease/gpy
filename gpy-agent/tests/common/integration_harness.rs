//! Spawns a real daemonized agent with a temp socket, git repo, and config.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::collapsible_match)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::ignored_unit_patterns)]
#![allow(clippy::io_other_error)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::let_unit_value)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::cast_possible_truncation)]

use super::fixtures::TempSocket;
use super::git::{TestRepo, git_output};
use gpy_agent::config::Config;
use gpy_agent::ipc::{Format, Message, Response};
use gpy_agent::security::SafePath;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// A [`Config`] the harness writes to a temp `config.toml` for the agent.
pub struct TestConfig {
    _dir: TempDir,
    config_path: PathBuf,
    config: Config,
}

impl TestConfig {
    /// Default configuration.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory for config");
        let config_path = dir.path().join("config.toml");
        Self {
            _dir: dir,
            config_path,
            config: Config::default(),
        }
    }

    /// Path the configuration is written to.
    pub fn path(&self) -> &Path {
        &self.config_path
    }

    /// The configuration.
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Write the configuration to [`path`](Self::path).
    pub fn write(&self) -> &Self {
        let toml_string =
            toml::to_string(&self.config).expect("Failed to serialize config to TOML");
        fs::write(&self.config_path, toml_string).expect("Failed to write config file");
        self
    }

    /// Disable the git segment.
    #[must_use]
    pub const fn with_git_disabled(mut self) -> Self {
        self.config.git.enabled = false;
        self
    }
}

impl Default for TestConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Encode a request as one `{"op":…}` JSON line; panics on variants the harness never sends.
fn op_line(msg: &Message) -> String {
    let value = match msg {
        Message::Ping => serde_json::json!({"op": "ping"}),
        Message::Status => serde_json::json!({"op": "status"}),
        Message::RepositoryStatus {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
        } => serde_json::json!({
            "op": "git", "cwd": path.as_str(), "format": format,
            "is_last": is_last, "is_first": is_first, "prev_bg": prev_bg,
        }),
        Message::LanguageDetect {
            path,
            format,
            is_last,
            is_first,
            prev_bg,
            virtual_env,
        } => serde_json::json!({
            "op": "lang", "cwd": path.as_str(), "format": format, "is_last": is_last,
            "is_first": is_first, "prev_bg": prev_bg, "virtual_env": virtual_env,
        }),
        Message::RegisterClient {
            pid,
            cwd,
            shell,
            shell_version,
        } => serde_json::json!({
            "op": "register", "pid": pid.get(), "cwd": cwd.as_ref().map(SafePath::as_str),
            "shell": shell, "shell_version": shell_version,
        }),
        Message::UnregisterClient { pid } => {
            serde_json::json!({"op": "unregister", "pid": pid.get()})
        }
        other => panic!("op_line: no op encoding for {other:?}"),
    };
    value.to_string()
}

/// Spawns a real agent process; resources are cleaned up on drop.
pub struct IntegrationTestServer {
    socket: TempSocket,
    repo: TestRepo,
    _config: TestConfig,
    _socket_dir: TempDir,
}

/// Git changes that can be triggered in the test repository
#[derive(Debug, Clone)]
pub enum GitChange {
    /// Add a new untracked file
    AddUntracked(String),
    /// Stage a file
    StageFile(String),
}

/// Git status information
#[derive(Debug, Clone, PartialEq)]
pub struct GitStatus {
    pub branch: String,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
}

/// Reads `key` as a `u32`, saturating values that do not fit; missing keys read as 0.
fn u32_field(value: &serde_json::Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

impl IntegrationTestServer {
    async fn send_request_to_socket(socket_path: &Path, msg: &Message) -> io::Result<Response> {
        let mut stream = timeout(Duration::from_secs(5), UnixStream::connect(socket_path))
            .await
            .map_err(|e| {
                io::Error::new(io::ErrorKind::TimedOut, format!("Connection timeout: {e}"))
            })?
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;

        let json = op_line(msg);

        stream.write_all(json.as_bytes()).await?;
        stream.write_all(b"\n").await?;

        let mut response_buf = vec![0_u8; 8192];
        let n = timeout(Duration::from_secs(10), stream.read(&mut response_buf))
            .await
            .map_err(|e| {
                io::Error::new(io::ErrorKind::TimedOut, format!("Response timeout: {e}"))
            })??;

        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Agent closed connection without response",
            ));
        }

        response_buf.truncate(n);
        Self::parse_response_bytes(&response_buf)
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
                let state = std::str::FromStr::from_str(
                    status_val
                        .get("state")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                )
                .unwrap_or(gpy_agent::git::RepositoryState::InProgress);

                let status = gpy_agent::git::RepositoryStatus {
                    branch: status_val
                        .get("branch")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    ahead: u32_field(&status_val, "ahead"),
                    behind: u32_field(&status_val, "behind"),
                    ahead_capped: status_val
                        .get("ahead_capped")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    behind_capped: status_val
                        .get("behind_capped")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    staged: u32_field(&status_val, "staged"),
                    unstaged: u32_field(&status_val, "unstaged"),
                    untracked: u32_field(&status_val, "untracked"),
                    conflicts: u32_field(&status_val, "conflicts"),
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
    pub async fn new() -> io::Result<Self> {
        Self::with_config(TestConfig::default()).await
    }

    /// Create a new integration test server with custom configuration
    pub async fn with_config(config: TestConfig) -> io::Result<Self> {
        let repo = TestRepo::new();

        let socket = TempSocket::new()?;
        let socket_dir = TempDir::new()?;

        config.write();

        // XDG roots live in the temp dir so the daemon never touches ~/.config/gpy
        // or ~/.cache/gpy.
        let agent_binary = env!("CARGO_BIN_EXE_gpy-agent");
        let status = Command::new(agent_binary)
            .arg("start")
            .arg("--socket")
            .arg(socket.path())
            .env("GPY_CONFIG_PATH", config.path())
            .env("XDG_CONFIG_HOME", socket_dir.path().join("config"))
            .env("XDG_CACHE_HOME", socket_dir.path().join("cache"))
            .env("XDG_RUNTIME_DIR", socket_dir.path())
            .status()?;

        // Own the daemon now so `Drop` stops it even if readiness times out.
        let server = Self {
            socket,
            repo,
            _config: config,
            _socket_dir: socket_dir,
        };

        if !status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Agent start command failed with status: {}", status),
            ));
        }

        Self::wait_until_ready(server.socket.path()).await?;

        Ok(server)
    }

    /// Send an IPC request to the agent and wait for response
    pub async fn send_request(&self, msg: Message) -> io::Result<Response> {
        Self::send_request_to_socket(self.socket.path(), &msg).await
    }

    /// Trigger a git change in the test repository
    pub fn trigger_git_change(&mut self, change: GitChange) -> io::Result<()> {
        match change {
            GitChange::AddUntracked(filename) => {
                std::fs::write(self.repo.path().join(&filename), "test content")?;
            }
            GitChange::StageFile(filename) => {
                self.git_run(&["add", &filename])?;
            }
        }
        Ok(())
    }

    /// Get current git status from the test repository
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

        match response {
            Response::RepositoryStatus(s) => Ok(GitStatus {
                branch: s.branch,
                staged: i64::from(s.staged),
                unstaged: i64::from(s.unstaged),
                untracked: i64::from(s.untracked),
            }),
            Response::Error { message } => Err(io::Error::new(io::ErrorKind::Other, message)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unexpected response type",
            )),
        }
    }

    /// Assert that the agent process is still alive
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

    /// Send a ping request to the agent
    pub async fn ping(&self) -> io::Result<Response> {
        self.send_request(Message::Ping).await
    }

    /// Send an agent status request
    pub async fn agent_status(&self) -> io::Result<Response> {
        self.send_request(Message::Status).await
    }

    /// Run a git command in the test repository
    fn git_run(&self, args: &[&str]) -> io::Result<()> {
        let output = git_output(self.repo.path(), args);

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

        // Wait for socket teardown so nextest doesn't flag the test as leaky.
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(2))
            .expect("valid shutdown wait deadline");
        while self.socket.path().exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
