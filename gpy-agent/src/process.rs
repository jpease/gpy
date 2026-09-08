//! Shared subprocess wait-with-timeout primitive (#590).
//!
//! `git::native` and `language::version` each grew a byte-for-byte identical
//! ~100-line implementation of "spawn a child in its own process group, wait
//! for it on a side thread, and escalate `SIGTERM` → `SIGKILL` on timeout" —
//! only the error type each wrapped the result in, and whether an extra
//! caller-supplied label decorated the error message, ever differed. This
//! module is the one implementation; callers (git subprocess execution,
//! language version detection, `fc-list`, ...) map [`WaitError`] to whatever
//! error type and message wording is meaningful to them.

use std::process::{Child, Command, Output};
use std::sync::mpsc;
use std::time::Duration;

/// Grace period after `SIGTERM` before escalating to `SIGKILL` on timeout.
#[cfg(unix)]
const TIMEOUT_KILL_GRACE: Duration = Duration::from_millis(250);

/// Why a timed-waited child process didn't produce output.
///
/// Domain-agnostic on purpose (#590): callers (git subprocess execution,
/// language version detection, `fc-list`) each map this to their own error
/// type (`Error::git`, `Error::language`, ...) with whatever label/context is
/// meaningful to them.
#[derive(Debug)]
pub enum WaitError {
    /// The child's I/O failed (see the wrapped `std::io::Error`).
    Io(std::io::Error),
    /// The wait exceeded `timeout` (plus, on Unix, up to `TIMEOUT_KILL_GRACE`
    /// while the process group was terminated).
    TimedOut(Duration),
    /// The dedicated wait thread exited without sending a result -- should not
    /// happen in practice; treated as a hard failure rather than panicking.
    WorkerDisconnected,
}

#[expect(
    clippy::use_debug,
    reason = "Duration has no Display impl; `{:?}` is the standard way to print one (matches src/commands/debug.rs's `prompt` precedent)"
)]
impl std::fmt::Display for WaitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::TimedOut(timeout) => write!(f, "timed out after {timeout:?}"),
            Self::WorkerDisconnected => write!(f, "worker exited without a result"),
        }
    }
}

impl std::error::Error for WaitError {}

/// Place a spawned command in its own process group so a timeout watchdog can
/// signal the whole group.
///
/// The group id equals the child's PID, so `killpg` reaps the child *and* any
/// descendants it spawned (which may hold output pipes open) without ever
/// touching the agent's own process group. No-op on non-Unix platforms.
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

/// Wait for a spawned child, enforcing a hard timeout on the wait itself.
///
/// The wait runs on a dedicated thread because `Child::wait_with_output` blocks
/// until the child exits *and* every write-end of its pipes is closed. A child
/// that ignores `SIGTERM`, or a grandchild that inherits and holds the pipes
/// open, could otherwise wedge the calling thread indefinitely. Owning the
/// wait in a side thread lets this function return after at most `timeout +
/// TIMEOUT_KILL_GRACE`, regardless of the child's behavior.
///
/// On timeout it escalates termination of the child's process group:
/// `SIGTERM`, a short grace period for cooperative shutdown, then `SIGKILL`.
/// The child must have been spawned with piped stdout/stderr and (on Unix) its
/// own process group via [`spawn_in_own_process_group`].
///
/// # Errors
///
/// Returns [`WaitError`] if the child's I/O fails or the timeout expires.
pub fn wait_with_timeout(child: Child, timeout: Duration) -> Result<Output, WaitError> {
    // Grab the PID before moving the child so the watchdog can signal it.
    let raw_pid = child.id();

    // Drain the pipes and reap the child on a dedicated thread. If the child
    // outlives the timeout escalation (it should not, after a process-group
    // SIGKILL) this thread is left to finish on its own -- a bounded leak that
    // never blocks the caller, since this function returns regardless.
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

/// Escalate termination of a timed-out child's process group: `SIGTERM`, a short
/// grace period for cooperative shutdown, then `SIGKILL`.
///
/// Bounded by `TIMEOUT_KILL_GRACE`; never waits on the reader thread beyond it.
/// No-op (best effort) on non-Unix platforms, where the reader thread is left to
/// finish on its own once the OS tears the child down.
fn terminate_timed_out_child(raw_pid: u32, result_rx: &mpsc::Receiver<std::io::Result<Output>>) {
    #[cfg(unix)]
    {
        signal_process_group(raw_pid, nix::sys::signal::Signal::SIGTERM);
        // A cooperative child exits within the grace period and the reader
        // delivers (closing the pipes); anything still alive is force-killed.
        if result_rx.recv_timeout(TIMEOUT_KILL_GRACE).is_err() {
            signal_process_group(raw_pid, nix::sys::signal::Signal::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (raw_pid, result_rx);
    }
}

/// Send `signal` to the process group led by `raw_pid`.
///
/// Children are spawned via [`spawn_in_own_process_group`], so the group id
/// equals the child PID and the signal reaches any grandchildren holding the
/// output pipes open. A non-leader PID resolves to no such group (`ESRCH`),
/// which is harmless.
#[cfg(unix)]
fn signal_process_group(raw_pid: u32, signal: nix::sys::signal::Signal) {
    if let Ok(pid) = i32::try_from(raw_pid) {
        let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), signal);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use std::process::Stdio;
    use std::time::Instant;

    #[test]
    fn wait_with_timeout_times_out_on_hung_process() {
        // A command that would otherwise block far longer than the timeout.
        // `sleep` is Unix-only; Windows has no equivalent on PATH by default.
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

    /// Regression for #151/#177: a `SIGTERM`-ignoring child must not wedge the
    /// wait.
    ///
    /// The shell traps `TERM` and leaves a `sleep` child inheriting
    /// stdout/stderr, so only process-group `SIGKILL` escalation can reap it and
    /// close the pipes -- proving the wait path is bounded regardless of whether
    /// the child cooperates.
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
        // timeout (200ms) + kill grace (250ms) plus slack; nowhere near `sleep 30`.
        assert!(
            elapsed < Duration::from_secs(3),
            "wait must return promptly after SIGKILL escalation, took {elapsed:?}"
        );
    }
}
