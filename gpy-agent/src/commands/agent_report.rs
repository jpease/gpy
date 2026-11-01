//! What `gpy-agent start`, `stop`, `restart` and `status` print and which
//! exit code they return. `status` prints fixed `key: value` lines on stdout
//! (a documented contract); `start` and `stop` print one outcome line.

use super::exit_codes::{DISABLED, NOT_RUNNING, STATUS_UNKNOWN};
use super::output::warn_stderr;
use crate::agent::lifecycle::{
    AgentStatusInfo, ProtocolMismatch, StartNotice, StartOutcome, StatusProbe, probe_status,
    protocol_mismatch, send_shutdown_command, socket_is_stale, start_background_agent,
    wait_for_agent_shutdown,
};
use crate::config::schema;
use crate::paths::{get_socket_path, set_socket_override};
use crate::{Error, Result, VERSION};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// What `gpy-agent status` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusOutcome {
    /// The agent answered a status query.
    Responding,
    /// The agent accepts connections but is closing them without a reply.
    Busy,
    /// No socket file, or nothing listening on it.
    NotRunning,
    /// The agent did not answer, or the probe could not be made.
    Unreachable,
}

impl StatusOutcome {
    /// The process exit code for this outcome.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Responding | Self::Busy => 0,
            Self::NotRunning => NOT_RUNNING,
            Self::Unreachable => STATUS_UNKNOWN,
        }
    }
}

/// Record `--socket`, made absolute against the current directory so a
/// forked daemon (which runs in `/`) binds the path the user meant.
///
/// # Errors
/// Returns an error if the current directory cannot be read.
pub fn apply_socket_override(socket: Option<PathBuf>) -> Result<()> {
    if let Some(given) = socket {
        set_socket_override(&given)?;
    }
    Ok(())
}

/// The `state` value and outcome for one probe of an existing socket.
#[must_use]
pub const fn state_and_exit(probe: &StatusProbe) -> (&'static str, StatusOutcome) {
    match probe {
        StatusProbe::Answered(_) => ("running", StatusOutcome::Responding),
        StatusProbe::Busy => ("busy", StatusOutcome::Busy),
        StatusProbe::NotListening(_) => ("not-running", StatusOutcome::NotRunning),
        StatusProbe::NoReply(_) | StatusProbe::Failed(_) => {
            ("not-answering", StatusOutcome::Unreachable)
        }
    }
}

/// The status the agent reported, when the probe got an answer.
const fn answered(probe: Option<&StatusProbe>) -> Option<&AgentStatusInfo> {
    match probe {
        Some(StatusProbe::Answered(status)) => Some(status),
        Some(
            StatusProbe::Busy
            | StatusProbe::NotListening(_)
            | StatusProbe::NoReply(_)
            | StatusProbe::Failed(_),
        )
        | None => None,
    }
}

/// The `error:` value for a probe, when there is one to report.
fn error_reason(probe: Option<&StatusProbe>) -> Option<String> {
    match probe? {
        StatusProbe::Answered(_) => None,
        StatusProbe::Busy => Some("agent closed the connection without a reply".to_owned()),
        StatusProbe::NotListening(cause)
        | StatusProbe::NoReply(cause)
        | StatusProbe::Failed(cause) => Some(crate::error::user_message(cause)),
    }
}

/// The `config:` value: the active config file, or the built-in defaults.
fn config_line() -> String {
    let config_paths = schema::get_config_paths();
    match schema::resolve_active_config(&config_paths) {
        Some((_, Some(existing))) => existing.display().to_string(),
        _ => "none (built-in defaults)".to_owned(),
    }
}

/// The `key: value` lines of `gpy-agent status`, in output order.
struct StatusReport {
    outcome: StatusOutcome,
    lines: Vec<(&'static str, String)>,
    mismatch: Option<ProtocolMismatch>,
}

/// Probe the agent and build the report. Nothing is printed here, so a bad
/// socket path produces an `Error:` line and no half-written report.
///
/// # Errors
/// Returns an error if the socket path cannot be determined.
fn status_report(socket: Option<PathBuf>) -> Result<StatusReport> {
    apply_socket_override(socket)?;
    let socket_path = get_socket_path()?;
    // No socket file is "not running" on every platform, without a probe.
    let probe = socket_path.exists().then(|| probe_status(&socket_path));
    let (state, outcome) = probe
        .as_ref()
        .map_or(("not-running", StatusOutcome::NotRunning), state_and_exit);
    let reply = answered(probe.as_ref());

    let mut lines = vec![("state", state.to_owned())];
    if let Some(status) = reply {
        if let Some(pid) = status.pid {
            lines.push(("pid", pid.to_string()));
        }
        lines.push(("version", status.version.clone()));
        lines.push(("protocol", status.protocol_version.to_string()));
    }
    lines.push(("socket", socket_path.display().to_string()));
    lines.push(("config", config_line()));
    if let Some(status) = reply {
        lines.push(("watched_repos", status.watched_repos.to_string()));
        lines.push(("registered_clients", status.registered_clients.to_string()));
        lines.push(("cache_entries", status.cache_entries.to_string()));
    }
    if let Some(reason) = error_reason(probe.as_ref()) {
        lines.push(("error", reason));
    }
    Ok(StatusReport {
        outcome,
        lines,
        mismatch: reply.and_then(|status| protocol_mismatch(status.protocol_version)),
    })
}

/// `gpy-agent status`: print the report and return its exit code. A status
/// that cannot be determined prints one `Error:` line and returns
/// [`STATUS_UNKNOWN`].
#[must_use]
pub fn status(socket: Option<PathBuf>) -> ExitCode {
    match status_report(socket) {
        Ok(report) => {
            for (key, value) in &report.lines {
                super::output::plain(format_args!("{key}: {value}"));
            }
            if let Some(mismatch) = report.mismatch {
                warn_stderr(format!(
                    "agent speaks protocol {}, this gpy-agent speaks {}; restart it: gpy-agent restart",
                    mismatch.running, mismatch.expected
                ));
            }
            ExitCode::from(report.outcome.code())
        }
        Err(err) => {
            eprintln!("Error: {}", crate::error::user_message(&err));
            ExitCode::from(STATUS_UNKNOWN)
        }
    }
}

/// Print `line` on stdout unless `quiet`.
fn say(quiet: bool, line: impl std::fmt::Display) {
    if !quiet {
        super::output::plain(line);
    }
}

/// The stderr text of a start notice that announces a wait; every other
/// notice's fact is carried by the outcome line or the error.
fn notice_text(notice: &StartNotice, socket: &str) -> Option<String> {
    match notice {
        StartNotice::Disabled => {
            Some("Agent disabled in config (agent.enabled = false); not started.".to_owned())
        }
        StartNotice::EvictingUnanswered => Some(format!(
            "Agent at {socket} accepted a connection but did not answer; replacing it."
        )),
        StartNotice::Busy => Some(format!(
            "Agent at {socket} is too busy to answer; leaving it running."
        )),
        StartNotice::Unverifiable
        | StartNotice::VersionMismatch { .. }
        | StartNotice::ProtocolMismatch { .. }
        | StartNotice::StaleSocket
        | StartNotice::AlreadyRunning
        | StartNotice::NewerRunning { .. } => None,
    }
}

/// `gpy-agent start`: start the agent and print one outcome line.
///
/// # Errors
/// Returns an error if the agent cannot be started or its socket's owner
/// cannot be verified.
pub fn start(socket: Option<PathBuf>, quiet: bool) -> Result<ExitCode> {
    apply_socket_override(socket)?;
    start_agent(quiet)
}

/// [`start`] after the socket override was applied.
fn start_agent(quiet: bool) -> Result<ExitCode> {
    let socket = get_socket_path().map_or_else(
        |_| "the agent socket".to_owned(),
        |path| path.display().to_string(),
    );
    let outcome = start_background_agent(|notice| {
        if !quiet && let Some(text) = notice_text(&notice, &socket) {
            eprintln!("{text}");
        }
    })?;
    match outcome {
        StartOutcome::Started { pid } => say(quiet, format!("Started agent (pid {pid}).")),
        StartOutcome::Replaced { previous, pid } => say(
            quiet,
            format!("Replaced agent {previous} with {VERSION} (pid {pid})."),
        ),
        StartOutcome::AlreadyRunning => say(quiet, "Agent already running."),
        StartOutcome::NewerRunning { running } => say(
            quiet,
            format!(
                "Agent {running} already running; it is newer than this gpy-agent ({VERSION}) and was left in place."
            ),
        ),
        StartOutcome::Unverifiable => {
            return Err(Error::agent(format!(
                "could not verify whether the GPY agent at {socket} is running; its socket was left alone"
            )));
        }
        StartOutcome::Disabled => return Ok(ExitCode::from(DISABLED)),
    }
    Ok(ExitCode::SUCCESS)
}

/// The error for an agent that is still running after a stop.
fn did_not_stop(pid: Option<u32>, socket_path: &Path) -> Error {
    let socket = socket_path.display();
    Error::agent(pid.map_or_else(
        || {
            format!(
                "agent at {socket} did not stop; find the process holding {socket} (for example: lsof -t {socket})"
            )
        },
        |known| format!("agent (pid {known}) at {socket} did not stop; stop it with: kill {known}"),
    ))
}

/// Stop the background agent gracefully and print one outcome line.
///
/// # Errors
/// Returns an error if the agent is still answering or cannot be reached.
fn stop_agent(quiet: bool) -> Result<()> {
    let socket_path = get_socket_path()?;
    if !socket_path.exists() {
        say(quiet, "Agent not running.");
        return Ok(());
    }

    // Learn the PID before the shutdown so the outcome can name it; any
    // answer but a status skips the lookup.
    let pid = match probe_status(&socket_path) {
        StatusProbe::Answered(status) => status.pid,
        StatusProbe::Busy
        | StatusProbe::NotListening(_)
        | StatusProbe::NoReply(_)
        | StatusProbe::Failed(_) => None,
    };

    // A current-thread runtime spawns no worker threads: once any thread has
    // existed, the forked daemon's first Objective-C use aborts on macOS, and
    // `restart` forks after this runs.
    let shutdown = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(send_shutdown_command());
    match shutdown {
        Ok(()) if wait_for_agent_shutdown(&socket_path) => {
            say(
                quiet,
                pid.map_or_else(
                    || "Stopped agent.".to_owned(),
                    |known| format!("Stopped agent (pid {known})."),
                ),
            );
            Ok(())
        }
        Ok(()) => Err(did_not_stop(pid, &socket_path)),
        // Only a refused connection proves the socket is stale; an
        // accepted-but-silent one is a live agent.
        Err(_) if socket_is_stale(&socket_path) => {
            std::fs::remove_file(&socket_path).or_else(|remove_err| {
                if remove_err.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(remove_err)
                }
            })?;
            say(
                quiet,
                format!(
                    "Agent not running; removed stale socket {}.",
                    socket_path.display()
                ),
            );
            Ok(())
        }
        Err(cause) => {
            crate::debug_log!("agent", "Shutdown request failed: {cause}");
            Err(did_not_stop(pid, &socket_path))
        }
    }
}

/// `gpy-agent stop`.
///
/// # Errors
/// Returns an error if the agent is still answering or cannot be reached.
pub fn stop(socket: Option<PathBuf>, quiet: bool) -> Result<ExitCode> {
    apply_socket_override(socket)?;
    stop_agent(quiet)?;
    Ok(ExitCode::SUCCESS)
}

/// `gpy-agent restart`: stop, then start. A failed stop is a `warn:` line, not
/// an abort, because `start` evicts an agent that does not answer. The exit
/// code is `start`'s.
///
/// # Errors
/// Returns the start's error.
pub fn restart(socket: Option<PathBuf>, quiet: bool) -> Result<ExitCode> {
    apply_socket_override(socket)?;
    if let Err(err) = stop_agent(quiet)
        && !quiet
    {
        warn_stderr(crate::error::user_message(&err));
    }
    // `stop_agent` dropped its runtime and client state and spawned no
    // threads: nothing is live across the fork in `start`.
    start_agent(quiet)
}

#[cfg(test)]
mod tests {
    use super::{StatusOutcome, state_and_exit};
    use crate::Error;
    use crate::agent::lifecycle::StatusProbe;

    #[test]
    fn failed_probe_is_not_answering() {
        let probe = StatusProbe::Failed(Error::agent("probe failed"));
        let (state, outcome) = state_and_exit(&probe);
        assert_eq!(state, "not-answering");
        assert_eq!(outcome, StatusOutcome::Unreachable);
        assert_eq!(outcome.code(), 4_u8);
    }

    #[test]
    fn probe_states_agree_with_exit_codes() {
        let refused = StatusProbe::NotListening(Error::agent("refused"));
        let wedged = StatusProbe::NoReply(Error::agent("silent"));
        let cases = [
            (state_and_exit(&StatusProbe::Busy), "busy", 0_u8),
            (state_and_exit(&refused), "not-running", 3_u8),
            (state_and_exit(&wedged), "not-answering", 4_u8),
        ];
        for ((state, outcome), want_state, want_code) in cases {
            assert_eq!(state, want_state);
            assert_eq!(outcome.code(), want_code);
        }
    }
}
