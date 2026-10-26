//! `gpy start|stop|restart|status`, delegated to the `gpy-agent` binary.

use super::utils::agent_binary;
use crate::agent::lifecycle::start::START_PROGRESS_LINE;
use crate::deadline::Deadline;
use crate::{Error, Result};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
use std::time::Duration;

/// Pause between the stop and the start of a restart, so the old agent's
/// socket is gone before the new one binds.
const RESTART_SETTLE: Duration = Duration::from_millis(500);

/// How long a restart waits for the new agent to answer.
const AGENT_READY_WAIT: Duration = Duration::from_secs(5);

/// Poll interval while waiting for the new agent to answer.
const AGENT_READY_POLL: Duration = Duration::from_millis(100);

/// `gpy-agent <subcommand>`, not yet run.
fn agent_command(subcommand: &str) -> Command {
    let mut command = Command::new(agent_binary());
    command.arg(subcommand);
    command
}

/// The error for a `gpy-agent <subcommand>` that could not be run or read.
fn agent_error(subcommand: &str, failure: impl std::fmt::Display) -> Error {
    Error::process(format!("gpy-agent {subcommand}"), failure.to_string())
}

/// Run `gpy-agent <subcommand>` with inherited stdio and wait for it.
///
/// # Errors
/// Returns an error if `gpy-agent` cannot run.
fn run_agent(subcommand: &str) -> Result<ExitStatus> {
    agent_command(subcommand)
        .status()
        .map_err(|e| agent_error(subcommand, format_args!("could not run: {e}")))
}

/// `Ok` when `gpy-agent <subcommand>` exited with `status` zero.
///
/// # Errors
/// Returns an error naming the subcommand and its exit status otherwise.
fn ensure_success(subcommand: &str, status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(agent_error(subcommand, format_args!("failed ({status})")))
    }
}

/// Start the GPY agent daemon.
///
/// # Errors
/// Returns an error if `gpy-agent start` cannot run or fails.
pub fn start() -> Result<()> {
    ensure_success("start", run_agent("start")?)
}

/// Stop the GPY agent daemon.
///
/// # Errors
/// Returns an error if `gpy-agent stop` cannot run or fails.
pub fn stop() -> Result<()> {
    ensure_success("stop", run_agent("stop")?)
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
    std::thread::sleep(RESTART_SETTLE);
    restart_start()
}

/// Run `gpy-agent stop` and report the restart's stop steps. Whether an agent
/// was running comes from pinging it, not from `stop`'s output; `stop`'s own
/// errors still reach stderr.
///
/// # Errors
/// Returns an error if the agent cannot be pinged, or `gpy-agent stop` cannot
/// run or fails.
fn restart_stop() -> Result<()> {
    let was_running =
        crate::agent::lifecycle::ping_agent_blocking(&crate::paths::get_socket_path()?)?;

    let status = agent_command("stop")
        .stdout(Stdio::null())
        .status()
        .map_err(|e| agent_error("stop", format_args!("could not run: {e}")))?;
    ensure_success("stop", status)?;

    if was_running {
        println!("Shutdown command sent");
    }
    println!("GPY stopped");
    Ok(())
}

/// Run `gpy-agent start` and wait until the daemon answers. The daemon
/// double-forks, so readiness must be polled from here.
///
/// # Errors
/// Returns an error if `gpy-agent start` fails or the agent never answers.
fn restart_start() -> Result<()> {
    let mut child = agent_command("start")
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| agent_error("start", format_args!("could not run: {e}")))?;

    let forwarded = forward_start_stderr(&mut child);
    let status = child
        .wait()
        .map_err(|e| agent_error("start", format_args!("could not wait: {e}")))?;
    forwarded?;
    ensure_success("start", status)?;

    if !agent_enabled() {
        return Ok(());
    }

    wait_for_agent_started()?;
    println!("GPY started");
    Ok(())
}

/// Forward the start's stderr, minus the expected progress line.
///
/// # Errors
/// Returns an error if the child's stderr was not captured or cannot be read.
fn forward_start_stderr(child: &mut Child) -> Result<()> {
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| agent_error("start", "could not capture its output"))?;
    for read in BufReader::new(stderr).lines() {
        let line =
            read.map_err(|e| agent_error("start", format_args!("could not read its output: {e}")))?;
        if line != START_PROGRESS_LINE {
            eprintln!("{line}");
        }
    }
    Ok(())
}

/// `agent.enabled`; `true` when the config cannot be loaded, so a failed
/// start still surfaces through the readiness wait.
fn agent_enabled() -> bool {
    crate::config::loader::load_config().map_or(true, |config| config.agent.enabled)
}

/// Poll the agent socket until it responds, for up to [`AGENT_READY_WAIT`].
///
/// # Errors
/// Returns an error if pinging fails or the agent does not answer in time.
fn wait_for_agent_started() -> Result<()> {
    let socket_path = crate::paths::get_socket_path()?;
    let deadline = Deadline::after(AGENT_READY_WAIT);
    loop {
        if crate::agent::lifecycle::ping_agent_blocking(&socket_path)? {
            return Ok(());
        }
        if deadline.expired() {
            return Err(agent_error(
                "start",
                format_args!(
                    "agent did not answer within {}s",
                    AGENT_READY_WAIT.as_secs()
                ),
            ));
        }
        std::thread::sleep(AGENT_READY_POLL);
    }
}

/// Run `gpy-agent status` and pass its exit code through unchanged (1 means
/// not running, a finding rather than an error).
///
/// # Errors
/// Returns an error if `gpy-agent` cannot run or was killed by a signal.
pub fn status() -> Result<ExitCode> {
    let raw_code = run_agent("status")?
        .code()
        .ok_or_else(|| agent_error("status", "was terminated by a signal"))?;
    let code = u8::try_from(raw_code).unwrap_or(crate::error::CLI_FAILURE_EXIT_CODE);
    Ok(ExitCode::from(code))
}
