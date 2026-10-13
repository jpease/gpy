//! Fish shell integration tests: prompt startup with and without the agent,
//! and agent stability across repeated fish invocations.
//!
//! The agent under test is the cargo-built binary
//! (`env!("CARGO_BIN_EXE_gpy-agent")`), not whatever is on `PATH`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::default_numeric_fallback)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

#[path = "common/skip.rs"]
mod skip;
#[path = "common/wait.rs"]
mod wait;

/// Get the path to the gpy-agent binary built by cargo
const fn get_gpy_agent_path() -> &'static str {
    env!("CARGO_BIN_EXE_gpy-agent")
}

/// A fresh, isolated socket path for one test.
///
/// Every `gpy-agent stop`/`start`/`status` invocation and every `fish -c`
/// spawn in this file MUST carry `GPY_AGENT_SOCKET_PATH` pointing at this
/// path. Without it, each of those subprocesses falls through to the
/// production default socket (`get_socket_path()`'s unset-env-var fallback),
/// and `stop` would shut down a developer's live daemon.
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

/// Point a child's home and its config, cache and runtime roots inside `dir`.
///
/// `gpy-agent start` reads `config.toml` (a developer's `agent.enabled =
/// false` turns it into a no-op), and a running agent writes theme exports
/// into the cache root, so without this the developer's `~/.config/gpy` and
/// `~/.cache/gpy` take part in the test. `HOME` goes too: config lookup falls
/// back to `$HOME/.config/gpy/config.toml` when the `$XDG_CONFIG_HOME` one is
/// missing.
fn isolate_user_dirs<'cmd>(command: &'cmd mut Command, dir: &Path) -> &'cmd mut Command {
    command
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_RUNTIME_DIR", dir)
}

/// Assert the cargo-built agent binary exists, with a message pointing at
/// the likely cause (a failed build) rather than a bare path-not-found.
fn assert_agent_binary_exists(agent_path: &str) {
    assert!(
        Path::new(agent_path).exists(),
        "Agent binary not found at {agent_path} - did the build fail?"
    );
}

/// Test that Fish prompt initialization works without agent
///
/// Verifies that the prompt can function in degraded mode when the agent
/// is not available (e.g., `GPY_AGENT_ENABLED=0`).
#[tokio::test]
#[cfg(unix)]
async fn test_fish_prompt_works_without_agent() {
    // Check if fish is available
    if Command::new("fish").arg("--version").output().is_err() {
        skip::skip_test("fish shell not found in PATH");
        return;
    }

    // Run fish with agent disabled
    let output = Command::new("fish")
        .arg("-c")
        .arg("echo 'Test without agent'")
        .env("GPY_AGENT_ENABLED", "0")
        .env("GPY_AGENT_SUPERVISOR_ENABLED", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run fish");

    // Verify fish ran successfully even without the agent
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

/// Test that the agent can handle rapid Fish restarts
///
/// Verifies that the agent doesn't crash or leak resources when Fish
/// processes connect and disconnect rapidly.
#[tokio::test]
#[cfg(unix)]
async fn test_agent_handles_rapid_fish_restarts() {
    // Check if fish is available
    if Command::new("fish").arg("--version").output().is_err() {
        skip::skip_test("fish shell not found in PATH");
        return;
    }

    let agent_path = get_gpy_agent_path();
    assert_agent_binary_exists(agent_path);

    let (socket_dir, socket_path) = isolated_socket();

    // Start the agent on this test's isolated socket.
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

    // Run multiple fish instances in rapid succession
    for i in 0..5 {
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

    // Agent should still be responsive
    let status_output = isolate_user_dirs(&mut Command::new(agent_path), socket_dir.path())
        .arg("status")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output()
        .expect("Failed to check agent status");

    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        status_stdout.contains("Status: Running and Responding"),
        "Agent should still be running after rapid Fish restarts. Status output: {status_stdout}"
    );

    // Clean up
    let _ = isolate_user_dirs(&mut Command::new(agent_path), socket_dir.path())
        .arg("stop")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output();
}
