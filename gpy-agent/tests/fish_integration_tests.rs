//! Fish integration: prompt startup with and without the agent, and agent
//! stability across repeated fish runs. Uses the cargo-built agent binary.

// Spawns the daemonised agent over its Unix socket; native Windows is CLI-only.
#![cfg(unix)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

#[path = "common/child_guard.rs"]
mod child_guard;
#[path = "common/skip.rs"]
mod skip;
#[path = "common/wait.rs"]
mod wait;

const fn get_gpy_agent_path() -> &'static str {
    env!("CARGO_BIN_EXE_gpy-agent")
}

/// Isolated socket path; every agent/fish spawn MUST use it or `stop` hits the live daemon.
fn isolated_socket() -> (TempDir, String) {
    let dir = TempDir::new().expect("create temp dir for isolated socket");
    let socket_path = dir
        .path()
        .join("gpy.sock")
        .to_str()
        .expect("temp socket path must be valid UTF-8")
        .to_owned();
    (dir, socket_path)
}

/// Points HOME and XDG config/cache/runtime into `dir` so local config cannot leak in.
fn isolate_user_dirs<'cmd>(command: &'cmd mut Command, dir: &Path) -> &'cmd mut Command {
    command
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_RUNTIME_DIR", dir)
}

/// Asserts the agent binary exists, hinting at a failed build.
fn assert_agent_binary_exists(agent_path: &str) {
    assert!(
        Path::new(agent_path).exists(),
        "Agent binary not found at {agent_path} - did the build fail?"
    );
}

/// Fish prompt works in degraded mode with `GPY_AGENT_ENABLED=0`.
#[tokio::test]
#[cfg(unix)]
async fn test_fish_prompt_works_without_agent() {
    if Command::new("fish").arg("--version").output().is_err() {
        skip::skip_test("fish shell not found in PATH");
        return;
    }

    let output = Command::new("fish")
        .arg("-c")
        .arg("echo 'Test without agent'")
        .env("GPY_AGENT_ENABLED", "0")
        .env("GPY_AGENT_SUPERVISOR_ENABLED", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run fish");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("Fish stdout: {stdout}");
        eprintln!("Fish stderr: {stderr}");
        panic!(
            "Fish should work without agent, but failed with status: {}",
            output.status
        );
    }

    assert!(
        stdout.contains("Test without agent"),
        "Fish should execute commands without agent"
    );
}

/// Agent survives rapid fish connect/disconnect cycles.
#[tokio::test]
#[cfg(unix)]
async fn test_agent_handles_rapid_fish_restarts() {
    if Command::new("fish").arg("--version").output().is_err() {
        skip::skip_test("fish shell not found in PATH");
        return;
    }

    let agent_path = get_gpy_agent_path();
    assert_agent_binary_exists(agent_path);

    let (socket_dir, socket_path) = isolated_socket();

    // Stops the daemon on every exit path, including a failed assertion.
    let mut stop_command = Command::new(agent_path);
    isolate_user_dirs(&mut stop_command, socket_dir.path())
        .env("GPY_AGENT_SOCKET_PATH", &socket_path);
    let _agent_guard =
        child_guard::AgentStopGuard::new(stop_command, std::path::PathBuf::from(&socket_path));

    let start_output = isolate_user_dirs(&mut Command::new(agent_path), socket_dir.path())
        .arg("start")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output()
        .expect("Failed to start agent");

    assert!(
        start_output.status.success(),
        "Failed to start agent: {}",
        String::from_utf8_lossy(&start_output.stderr)
    );

    let socket = Path::new(&socket_path);
    assert!(
        wait::wait_until_async(Duration::from_secs(5), Duration::from_millis(25), || {
            socket.exists()
        })
        .await,
        "agent socket {socket_path} did not appear after start"
    );

    for i in 0_u32..5 {
        let output = isolate_user_dirs(&mut Command::new("fish"), socket_dir.path())
            .arg("-c")
            .arg(format!("echo 'Iteration {i}'"))
            .env("GPY_AGENT_ENABLED", "1")
            .env("GPY_AGENT_SUPERVISOR_ENABLED", "1")
            .env("GPY_AGENT_SOCKET_PATH", &socket_path)
            .output()
            .expect("Failed to run fish");

        assert!(
            output.status.success(),
            "Fish iteration {i} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Agent must still respond.
    let status_output = isolate_user_dirs(&mut Command::new(agent_path), socket_dir.path())
        .arg("status")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output()
        .expect("Failed to check agent status");

    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        status_stdout.contains("state: running"),
        "Agent should still be running after rapid Fish restarts. Status output: {status_stdout}"
    );
}
