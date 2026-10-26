//! Spawn-and-wait with a hard timeout and process-group termination.

use std::process::{Child, Command, Output};
use std::sync::mpsc;
use std::time::Duration;

/// Grace period after `SIGTERM` before escalating to `SIGKILL` on timeout.
#[cfg(unix)]
const TIMEOUT_KILL_GRACE: Duration = Duration::from_millis(250);

/// Why a timed-waited child process didn't produce output.
#[derive(Debug)]
pub enum WaitError {
    /// The child's I/O failed.
    Io(std::io::Error),
    /// The wait exceeded `timeout`.
    TimedOut(Duration),
    /// The wait thread exited without sending a result.
    WorkerDisconnected,
}

impl std::fmt::Display for WaitError {
    #[allow(
        clippy::use_debug,
        reason = "Duration has no Display impl; `{:?}` gives the existing readable form (e.g. 2s) in this error text"
    )]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::TimedOut(timeout) => write!(f, "timed out after {timeout:?}"),
            Self::WorkerDisconnected => write!(f, "worker exited without a result"),
        }
    }
}

impl std::error::Error for WaitError {}

/// Put the child in its own process group so `killpg` also reaches
/// descendants holding its pipes open. No-op off Unix.
pub fn spawn_in_own_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// Wait for a child with a hard timeout, then `SIGTERM`/`SIGKILL` its group.
///
/// The wait runs on a side thread: `wait_with_output` blocks until every pipe
/// writer closes, which a `SIGTERM`-ignoring grandchild could delay forever.
/// Requires piped stdout/stderr and [`spawn_in_own_process_group`].
///
/// # Errors
/// Returns [`WaitError`] if the child's I/O fails or the timeout expires.
pub fn wait_with_timeout(child: Child, timeout: Duration) -> Result<Output, WaitError> {
    let raw_pid = child.id();

    // If the child somehow survives SIGKILL this thread leaks, but never blocks the caller.
    let (result_tx, result_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = result_tx.send(child.wait_with_output());
    });

    match result_rx.recv_timeout(timeout) {
        Ok(outcome) => outcome.map_err(WaitError::Io),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            terminate_timed_out_child(raw_pid, &result_rx);
            Err(WaitError::TimedOut(timeout))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(WaitError::WorkerDisconnected),
    }
}

/// `SIGTERM`, wait up to `TIMEOUT_KILL_GRACE`, then `SIGKILL`. No-op off Unix.
fn terminate_timed_out_child(raw_pid: u32, result_rx: &mpsc::Receiver<std::io::Result<Output>>) {
    #[cfg(unix)]
    {
        signal_process_group(raw_pid, nix::sys::signal::Signal::SIGTERM);
        if result_rx.recv_timeout(TIMEOUT_KILL_GRACE).is_err() {
            signal_process_group(raw_pid, nix::sys::signal::Signal::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (raw_pid, result_rx);
    }
}

/// Signal the process group led by `raw_pid`; `ESRCH` is harmless.
#[cfg(unix)]
fn signal_process_group(raw_pid: u32, signal: nix::sys::signal::Signal) {
    if let Ok(pid) = i32::try_from(raw_pid) {
        let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), signal);
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::process::Stdio;
    use std::time::Instant;

    #[test]
    fn wait_with_timeout_times_out_on_hung_process() {
        #[cfg(unix)]
        let mut command = {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        #[cfg(not(unix))]
        let mut command = {
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
            c
        };
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        spawn_in_own_process_group(&mut command);
        let child = command.spawn().expect("spawn sleep");

        let start = Instant::now();
        let result = wait_with_timeout(child, Duration::from_millis(200));
        let elapsed = start.elapsed();

        assert!(result.is_err(), "hung command should time out");
        assert!(
            elapsed < Duration::from_secs(5),
            "watchdog should return promptly after timeout, took {elapsed:?}"
        );
    }

    /// Only process-group `SIGKILL` can reap a `TERM`-trapping shell's `sleep` child.
    #[cfg(unix)]
    #[test]
    fn wait_with_timeout_times_out_when_sigterm_is_ignored() {
        let mut command = Command::new("sh");
        command.args(["-c", "trap '' TERM; sleep 30"]);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        spawn_in_own_process_group(&mut command);
        let child = command.spawn().expect("spawn sh");

        let start = Instant::now();
        let result = wait_with_timeout(child, Duration::from_millis(200));
        let elapsed = start.elapsed();

        assert!(result.is_err(), "SIGTERM-ignoring command should time out");
        assert!(
            elapsed < Duration::from_secs(3),
            "wait must return promptly after SIGKILL escalation, took {elapsed:?}"
        );
    }
}
