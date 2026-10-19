//! `gpy start|stop|restart|status`, delegated to the `gpy-agent` binary.

use super::utils::agent_binary;
use crate::{Error, Result};
use std::io::{BufRead, BufReader};
use std::process::{Command, ExitCode, Stdio};

/// Start the GPY agent daemon.
///
/// # Errors
/// Returns an error if `gpy-agent start` cannot run or fails.
pub fn start() -> Result<()> {
    let status = Command::new(agent_binary())
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

/// Stop the GPY agent daemon.
///
/// # Errors
/// Returns an error if `gpy-agent stop` cannot run or fails.
pub fn stop() -> Result<()> {
    let status = Command::new(agent_binary())
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

/// Restart the GPY agent daemon, reporting each step as it completes.
///
/// # Errors
/// Returns an error if the start fails.
pub fn restart() -> Result<()> {
    println!("Restarting GPY...");
    // A failed stop must not abort: start evicts a wedged agent, and its
    // result decides the exit code.
    if restart_stop().is_err() {
        println!("Stop failed; attempting to start anyway");
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    restart_start()
}

/// Run `gpy-agent stop`, translating its progress lines into restart steps.
///
/// # Errors
/// Returns an error if `gpy-agent stop` cannot run or fails.
fn restart_stop() -> Result<()> {
    let mut child = Command::new(agent_binary())
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
            "Shutdown command sent successfully" => println!("Shutdown command sent"),
            "Agent stopped successfully"
            | "Agent is not running (socket not found)"
            | "Agent is not running (stale socket removed)" => {
                println!("GPY stopped");
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

/// Run `gpy-agent start` and wait until the daemon answers. The daemon
/// double-forks, so readiness must be polled from here.
///
/// # Errors
/// Returns an error if `gpy-agent start` fails or the agent never answers.
fn restart_start() -> Result<()> {
    let mut child = Command::new(agent_binary())
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

    // Forward everything except the expected progress line.
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
    println!("GPY started");
    Ok(())
}

/// `agent.enabled`; `true` when the config cannot be loaded, so a failed
/// start still surfaces through the readiness wait.
fn agent_enabled() -> bool {
    crate::config::loader::load_config().map_or(true, |config| config.agent.enabled)
}

/// Poll the agent socket for up to 5 s until it responds.
///
/// # Errors
/// Returns an error if pinging fails or the agent does not answer in time.
fn wait_for_agent_started() -> Result<()> {
    use crate::agent::lifecycle::ping_agent_blocking;
    use crate::paths::get_socket_path;

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

/// Run `gpy-agent status` and pass its exit code through unchanged (1 means
/// not running, a finding rather than an error).
///
/// # Errors
/// Returns an error if `gpy-agent` cannot run or was killed by a signal.
pub fn status() -> Result<ExitCode> {
    let status = Command::new(agent_binary())
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
