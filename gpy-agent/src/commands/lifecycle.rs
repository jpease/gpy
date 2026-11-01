//! `gpy start|stop|restart|status`, delegated to the `gpy-agent` binary. The
//! wrappers pass `gpy-agent`'s exit code through unchanged (see
//! [`super::exit_codes`]).

use super::exit_codes::STATUS_UNKNOWN;
use super::utils::agent_binary;
use crate::error::CLI_FAILURE_EXIT_CODE;
use crate::{Error, Result};
use std::process::{Command, ExitCode};

/// The error for a `gpy-agent <subcommand>` that could not be run or read.
fn agent_error(subcommand: &str, failure: impl std::fmt::Display) -> Error {
    Error::process(format!("gpy-agent {subcommand}"), failure.to_string())
}

/// Run `gpy-agent <subcommand>` with inherited stdio, wait for it, and return
/// its exit code unchanged.
///
/// # Errors
/// Returns an error if `gpy-agent` cannot run or was killed by a signal.
fn run_agent(subcommand: &str) -> Result<ExitCode> {
    let raw_code = Command::new(agent_binary())
        .arg(subcommand)
        .status()
        .map_err(|e| agent_error(subcommand, format_args!("could not run: {e}")))?
        .code()
        .ok_or_else(|| agent_error(subcommand, "was terminated by a signal"))?;
    Ok(ExitCode::from(
        u8::try_from(raw_code).unwrap_or(CLI_FAILURE_EXIT_CODE),
    ))
}

/// Start the GPY agent daemon.
///
/// # Errors
/// Returns an error if `gpy-agent start` cannot run.
pub fn start() -> Result<ExitCode> {
    run_agent("start")
}

/// Stop the GPY agent daemon.
///
/// # Errors
/// Returns an error if `gpy-agent stop` cannot run.
pub fn stop() -> Result<ExitCode> {
    run_agent("stop")
}

/// Restart the GPY agent daemon.
///
/// # Errors
/// Returns an error if `gpy-agent restart` cannot run.
pub fn restart() -> Result<ExitCode> {
    run_agent("restart")
}

/// Run `gpy-agent status`. A `gpy-agent` that cannot be run leaves the status
/// unknown: print the error and return [`STATUS_UNKNOWN`], so `status` never
/// reports a failure as 1.
#[must_use]
pub fn status() -> ExitCode {
    run_agent("status").unwrap_or_else(|err| {
        eprintln!("Error: {}", crate::error::user_message(&err));
        ExitCode::from(STATUS_UNKNOWN)
    })
}
