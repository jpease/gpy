//! Fish shell integration tests
//!
//! These tests verify that the GPY agent correctly integrates with Fish shell,
//! including auto-start behavior and prompt initialization.
//!
//! **Note**: These tests use the `gpy-agent` binary from the build output
//! (via `env!("CARGO_BIN_EXE_gpy-agent")`) rather than expecting it in PATH.
//! This ensures tests work reliably in CI and on dev machines.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::default_numeric_fallback)]

mod test_harness;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

#[path = "common/skip.rs"]
mod skip;
use test_harness::EventFlag;

/// Get the path to the gpy-agent binary built by cargo
const fn get_gpy_agent_path() -> &'static str {
    env!("CARGO_BIN_EXE_gpy-agent")
}

/// A fresh, isolated socket path for one test.
///
/// Every `gpy-agent stop`/`start`/`status` invocation and every `fish -c`
/// spawn in this file MUST carry `GPY_AGENT_SOCKET_PATH` pointing at this
/// path. Without it, each of those subprocesses falls through to the
/// production default socket (`get_socket_path()`'s unset-env-var fallback)
/// -- on a machine with a live dogfooding daemon (the normal state for any
/// contributor actually using gpy), `stop` sends a real shutdown to that
/// daemon. Confirmed as the root cause of a daemon that repeatedly died
/// during `cargo nextest run`, reproduced with `GPY_DEBUG_LOG` enabled on the
/// real daemon: its log showed `[server] Received shutdown signal` at the
/// exact second a test run using this file's old, unisolated `stop` calls
/// was executing.
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

/// Assert the cargo-built agent binary exists, with a message pointing at
/// the likely cause (a failed build) rather than a bare path-not-found.
fn assert_agent_binary_exists(agent_path: &str) {
    assert!(
        Path::new(agent_path).exists(),
        "Agent binary not found at {agent_path} - did the build fail?"
    );
}

/// Test that the agent auto-starts when Fish initializes
///
/// This end-to-end test verifies:
/// 1. Fish can be launched with GPY enabled
/// 2. The agent automatically starts on the first prompt
/// 3. The agent process is running after Fish initialization
#[tokio::test]
#[cfg(unix)]
#[ignore = "fish -c never renders a prompt, and the agent auto-starts from fish_prompt, so this \
            can only pass vacuously (it did, until #636 made `status` exit non-zero for a \
            stopped agent). #645 replaces it with a PTY-driven interactive session."]
async fn test_agent_auto_starts_with_fish() {
    // Check if fish is available
    if Command::new("fish").arg("--version").output().is_err() {
        skip::skip_test("fish shell not found in PATH");
        return;
    }

    let agent_path = get_gpy_agent_path();
    assert_agent_binary_exists(agent_path);

    let (_socket_dir, socket_path) = isolated_socket();

    // Stop any existing agent -- scoped to this test's own isolated socket,
    // never the ambient default (see `isolated_socket`'s doc comment).
    let _ = Command::new(agent_path)
        .arg("stop")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output();

    // Wait for agent to fully stop
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create a flag to track if the test script completed
    let test_completed = EventFlag::new();
    let test_completed_clone = test_completed.clone();

    // Spawn fish with a command that triggers the prompt
    let fish_task = tokio::spawn({
        let task_socket_path = socket_path.clone();
        async move {
            let output = Command::new("fish")
                .arg("-c")
                .arg("echo 'Test initialized'")
                .env("GPY_AGENT_ENABLED", "1")
                .env("GPY_AGENT_SUPERVISOR_ENABLED", "1")
                .env("GPY_AGENT_SOCKET_PATH", &task_socket_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();

            test_completed_clone.set();
            output
        }
    });

    // Wait for fish command to complete (should be quick)
    let completed = test_completed.wait_until_set(Duration::from_secs(5)).await;
    assert!(completed, "Fish command should complete within 5 seconds");

    let output = fish_task
        .await
        .expect("Fish task panicked")
        .expect("Fish command failed");

    // Verify fish ran successfully
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("Fish stdout: {stdout}");
        eprintln!("Fish stderr: {stderr}");
        panic!("Fish command failed with status: {}", output.status);
    }

    // Give the agent a moment to start (it runs in background via disown)
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check if agent is now running
    let status_output = Command::new(agent_path)
        .arg("status")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output()
        .expect("Failed to check agent status");

    let status_stdout = String::from_utf8_lossy(&status_output.stdout);

    // Assert on the report line, not on the exit code: before #636 `status`
    // exited 0 whether or not anything was running, so a success check was vacuous.
    assert!(
        status_stdout.contains("Status: Running and Responding"),
        "Agent should be running after Fish initialization. Status output: {status_stdout}"
    );

    // Clean up: stop the agent
    let _ = Command::new(agent_path)
        .arg("stop")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output();
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

    let (_socket_dir, socket_path) = isolated_socket();

    // Stop any existing agent -- scoped to this test's own isolated socket,
    // never the ambient default (see `isolated_socket`'s doc comment).
    let _ = Command::new(agent_path)
        .arg("stop")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Start the agent manually
    let start_output = Command::new(agent_path)
        .arg("start")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output()
        .expect("Failed to start agent");

    if !start_output.status.success() {
        eprintln!(
            "Agent start output: {}",
            String::from_utf8_lossy(&start_output.stderr)
        );
        panic!("Failed to start agent");
    }

    // Give agent time to fully start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Run multiple fish instances in rapid succession
    for i in 0..5 {
        let output = Command::new("fish")
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

        // Small delay between iterations
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Agent should still be responsive
    let status_output = Command::new(agent_path)
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
    let _ = Command::new(agent_path)
        .arg("stop")
        .env("GPY_AGENT_SOCKET_PATH", &socket_path)
        .output();
}
