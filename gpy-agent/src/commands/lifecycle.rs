//! `gpy agent` lifecycle command handlers.
//!
//! The public CLI delegates start, stop, restart, and status operations to the
//! internal `gpy-agent` binary so daemon behavior stays centralized in the
//! agent executable. This module is intentionally thin glue between Clap
//! dispatch and process execution.

use crate::{Error, Result};
use std::io::{BufRead, BufReader};
use std::process::{Command, ExitCode, Stdio};

/// Start the GPY agent daemon
///
/// # Errors
///
/// Returns an error if the gpy-agent binary cannot be executed or fails to start.
pub fn start() -> Result<()> {
    let status = Command::new("gpy-agent")
        .arg("start")
        .status()
        .map_err(|e| Error::process("gpy-agent".to_owned(), e.to_string()))?;

    if !status.success() {
        return Err(Error::process(
            "start".to_owned(),
            "Failed to start agent".to_owned(),
        ));
    }
    Ok(())
}

/// Stop the GPY agent daemon
///
/// # Errors
///
/// Returns an error if the gpy-agent binary cannot be executed or fails to stop.
pub fn stop() -> Result<()> {
    let status = Command::new("gpy-agent")
        .arg("stop")
        .status()
        .map_err(|e| Error::process("gpy-agent".to_owned(), e.to_string()))?;

    if !status.success() {
        return Err(Error::process(
            "stop".to_owned(),
            "Failed to stop agent".to_owned(),
        ));
    }
    Ok(())
}

/// Restart the GPY agent daemon
///
/// Reports each step (shutdown sent, stopped, started) as it completes,
/// rather than letting the underlying `gpy-agent stop`/`start` output pass
/// through verbatim.
///
/// # Errors
///
/// Returns an error if stop or start operations fail.
pub fn restart() -> Result<()> {
    println!("Restarting GPY...");
    restart_stop()?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    restart_start()
}

/// Run `gpy-agent stop`, translating its progress lines into the
/// restart's normalized "✅ ..." step output as they arrive.
///
/// # Errors
///
/// Returns an error if the gpy-agent binary cannot be executed or fails to stop.
fn restart_stop() -> Result<()> {
    let mut child = Command::new("gpy-agent")
        .arg("stop")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| Error::process("gpy-agent".to_owned(), e.to_string()))?;

    let stdout = child.stdout.take().ok_or_else(|| {
        Error::process(
            "gpy-agent".to_owned(),
            "Failed to capture stop output".to_owned(),
        )
    })?;

    for line_result in BufReader::new(stdout).lines() {
        let Ok(line) = line_result else { continue };
        match line.as_str() {
            "Shutdown command sent successfully" => println!("✅ Shutdown command sent"),
            "Agent stopped successfully" | "Agent is not running (socket not found)" => {
                println!("✅ GPY stopped");
            }
            other => println!("{other}"),
        }
    }

    let status = child
        .wait()
        .map_err(|e| Error::process("gpy-agent".to_owned(), e.to_string()))?;

    if !status.success() {
        return Err(Error::process(
            "stop".to_owned(),
            "Failed to stop agent".to_owned(),
        ));
    }
    Ok(())
}

/// Run `gpy-agent start`, then print the restart's normalized "✅ GPY
/// started" line once the daemon is confirmed responsive.
///
/// The daemon double-forks and detaches (see `fork_agent` in
/// `agent::lifecycle::start`), so the spawned `gpy-agent start` process
/// typically exits before the real daemon finishes initializing and can
/// never print its own success line to our pipe -- readiness has to be
/// polled from out here instead.
///
/// # Errors
///
/// Returns an error if the gpy-agent binary cannot be executed, fails to
/// start, or does not become responsive before the readiness timeout.
fn restart_start() -> Result<()> {
    let mut child = Command::new("gpy-agent")
        .arg("start")
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::process("gpy-agent".to_owned(), e.to_string()))?;

    let stderr = child.stderr.take().ok_or_else(|| {
        Error::process(
            "gpy-agent".to_owned(),
            "Failed to capture start output".to_owned(),
        )
    })?;

    // Suppress the one expected progress line; forward anything else
    // (disabled-via-config notices, fork/init errors) so it stays visible.
    for line_result in BufReader::new(stderr).lines() {
        let Ok(line) = line_result else { continue };
        if line != "Starting GPY Agent in background..." {
            eprintln!("{line}");
        }
    }

    let status = child
        .wait()
        .map_err(|e| Error::process("gpy-agent".to_owned(), e.to_string()))?;

    if !status.success() {
        return Err(Error::process(
            "start".to_owned(),
            "Failed to start agent".to_owned(),
        ));
    }

    if !agent_enabled() {
        return Ok(());
    }

    wait_for_agent_started()?;
    println!("✅ GPY started");
    Ok(())
}

/// Whether the agent is enabled per config, defaulting to `true` if the
/// config can't be loaded so a start failure surfaces via the readiness
/// wait instead of being silently skipped.
fn agent_enabled() -> bool {
    crate::config::loader::load_config().map_or(true, |config| config.agent.enabled)
}

/// Poll the agent socket until it responds, mirroring the timeout used by
/// `agent::lifecycle::start::wait_for_agent_ready`.
///
/// # Errors
///
/// Returns an error if the socket path cannot be determined, pinging the
/// agent fails, or it does not become responsive before the timeout.
fn wait_for_agent_started() -> Result<()> {
    use crate::agent::lifecycle::{get_socket_path, ping_agent_blocking};

    let socket_path = get_socket_path()?;
    for _ in 0_i32..50_i32 {
        if ping_agent_blocking(&socket_path)? {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Err(Error::process(
        "start".to_owned(),
        "Agent did not become ready before timeout".to_owned(),
    ))
}

/// Show GPY agent status.
///
/// Prints `gpy-agent status`'s report and returns its exit code unchanged:
/// `0` when the agent responded, `1` when it is not running or unreachable
/// (#636). A non-zero code from the agent is a finding, not a failure of
/// this command, so it is passed through without an `Error:` line.
///
/// # Errors
///
/// Returns an error if the gpy-agent binary cannot be executed or was
/// killed by a signal.
pub fn status() -> Result<ExitCode> {
    let status = Command::new("gpy-agent")
        .arg("status")
        .status()
        .map_err(|e| Error::process("gpy-agent".to_owned(), e.to_string()))?;

    let raw_code = status.code().ok_or_else(|| {
        Error::process(
            "status".to_owned(),
            "gpy-agent status was terminated by a signal".to_owned(),
        )
    })?;
    let code = u8::try_from(raw_code).unwrap_or(crate::error::CLI_FAILURE_EXIT_CODE);
    Ok(ExitCode::from(code))
}
