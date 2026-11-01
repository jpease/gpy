//! Startup for `gpy-agent start`: check enablement, clean up stale sockets,
//! fork the daemon, and wait for it to answer.

use super::client::ping_agent_blocking;
use super::liveness::inspect_socket;
use super::{SocketCheck, StartNotice, StartOutcome};
use crate::Error;
use crate::Result;
use crate::deadline::Deadline;
use crate::{agent::Agent, debug_log};
use fork::Fork;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use std::path::Path;
use std::time::Duration;

/// Longest a start waits for another start on the same socket.
const START_LOCK_WAIT_MAX: Duration = Duration::from_secs(10);

/// Longest the daemon waits for its runtime's tasks at exit: an uncancellable
/// `spawn_blocking` scan must not hang exit.
const RUNTIME_SHUTDOWN_WAIT_MAX: Duration = Duration::from_secs(5);

/// Take the exclusive per-socket start lock (`<socket>.lock`), held across
/// check → evict → fork → bind. The forked daemon inherits the lock and drops
/// it once bound; the wait is bounded so a hung daemon cannot block later
/// starts forever.
///
/// # Errors
/// Returns an error if the lock cannot be opened or taken within [`START_LOCK_WAIT_MAX`].
fn acquire_start_lock(socket_path: &Path) -> Result<crate::fs_util::PathLock> {
    crate::fs_util::lock_beside(socket_path, START_LOCK_WAIT_MAX).map_err(|e| {
        Error::agent(format!(
            "cannot take start lock for {}: {e}",
            socket_path.display()
        ))
    })
}

/// How long `gpy-agent start` waits for its daemon to answer.
const READY_WAIT_MAX: Duration = Duration::from_secs(5);

/// Poll interval while waiting for the daemon to answer.
const READY_WAIT_POLL: Duration = Duration::from_millis(10);

/// Wait until daemon `agent_pid` answers a ping, stopping early if it reports
/// a startup failure or exits.
///
/// # Errors
/// Returns an error, with the daemon's reported reason when it sent one, if
/// the daemon fails or does not answer within [`READY_WAIT_MAX`].
fn wait_for_agent_ready(
    socket_path: &Path,
    agent_pid: i32,
    mut report: StartupReport,
) -> Result<()> {
    let deadline = Deadline::after(READY_WAIT_MAX);
    loop {
        report.poll();
        if let Some(reason) = report.failure() {
            return Err(startup_failed(Some(&reason)));
        }
        if ping_agent_blocking(socket_path)? {
            return Ok(());
        }
        if kill(Pid::from_raw(agent_pid), None).is_err() {
            // Gone, so every write end is closed and this drains to EOF.
            report.poll();
            return Err(startup_failed(report.failure().as_deref()));
        }
        if deadline.expired() {
            return Err(Error::agent(format!(
                "the daemon did not answer within {} s",
                READY_WAIT_MAX.as_secs()
            )));
        }
        std::thread::sleep(READY_WAIT_POLL);
    }
}

/// The daemon died before it was ready, with its reported `reason` if any.
fn startup_failed(reason: Option<&str>) -> Error {
    Error::agent(reason.map_or_else(
        || "the daemon exited before it was ready".to_owned(),
        |detail| format!("the daemon failed before it was ready: {detail}"),
    ))
}

/// Daemon half of the startup handshake with the waiting `gpy-agent start`:
/// holds the start lock and the write end of the startup pipe until the
/// socket is bound. The daemon's stderr is `/dev/null`, so a fatal error
/// before then goes down the pipe for `start` to print.
pub(crate) struct StartupHandoff {
    _start_lock: crate::fs_util::PathLock,
    report: std::io::PipeWriter,
}

impl StartupHandoff {
    /// Report `err` to the waiting `start`, then release the lock and close the pipe.
    pub(crate) fn fail(mut self, err: &Error) {
        use std::io::Write;

        let reason = crate::error::user_message(err);
        debug_log!("agent", "Fatal startup error: {reason}");
        let _ = self.report.write_all(reason.as_bytes());
    }
}

/// Parent half of the startup handshake. The daemon writes only a fatal
/// error, then closes its end; it also closes it, silently, once bound.
struct StartupReport {
    pipe: std::io::PipeReader,
    text: Vec<u8>,
    closed: bool,
}

impl StartupReport {
    /// Make `pipe` non-blocking so polling it never stalls the readiness wait.
    ///
    /// # Errors
    /// Returns an error if the pipe's flags cannot be set.
    fn new(pipe: std::io::PipeReader) -> Result<Self> {
        use nix::fcntl::{FcntlArg, OFlag, fcntl};

        fcntl(&pipe, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
            .map_err(|e| Error::agent(format!("cannot configure the startup pipe: {e}")))?;
        Ok(Self {
            pipe,
            text: Vec::new(),
            closed: false,
        })
    }

    /// Collect what the daemon has written so far.
    fn poll(&mut self) {
        use std::io::Read;

        if self.closed {
            return;
        }
        // On `WouldBlock`, `read_to_end` has still appended what it read.
        match self.pipe.read_to_end(&mut self.text) {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            _ => self.closed = true,
        }
    }

    /// The daemon's complete error, once it has written one and closed the pipe.
    fn failure(&self) -> Option<String> {
        (self.closed && !self.text.is_empty())
            .then(|| String::from_utf8_lossy(&self.text).trim().to_owned())
    }
}

/// Fork the agent daemon and return its PID once it answers on `socket_path`.
///
/// Explicit double fork rather than `fork::daemon()`, whose processes exit
/// before anything can be awaited: the original waits for readiness holding
/// `start_lock`; the intermediate `setsid`s and forks the daemon; the daemon
/// pipes back its PID, `chdir`s to `/`, binds, then drops the inherited lock
/// and closes the pipe ([`StartupHandoff`]).
///
/// # Errors
/// Returns an error if the pipe or fork fails, the daemon never reports its
/// PID, or it fails or is not ready in time. The daemon process itself never
/// returns: it exits after the agent stops.
fn fork_agent(socket_path: &Path, start_lock: crate::fs_util::PathLock) -> Result<u32> {
    use std::io::Read;

    let (mut reader, writer) = std::io::pipe()
        .map_err(|e| Error::agent(format!("cannot create the startup pipe: {e}")))?;

    match fork::fork() {
        Ok(Fork::Parent(intermediate_pid)) => {
            drop(writer);
            let mut pid_bytes = [0_u8; 4];
            let read = reader.read_exact(&mut pid_bytes);
            let _ = fork::waitpid(intermediate_pid);
            read.map_err(|e| Error::agent(format!("the daemon did not start: {e}")))?;
            let agent_pid = i32::from_ne_bytes(pid_bytes);
            let ready = StartupReport::new(reader)
                .and_then(|report| wait_for_agent_ready(socket_path, agent_pid, report));
            // Held until now so a daemon that never binds cannot let another start in.
            drop(start_lock);
            ready?;
            u32::try_from(agent_pid)
                .map_err(|e| Error::agent(format!("cannot fork the daemon: {e}")))
        }
        Ok(Fork::Child) => {
            drop(reader);
            fork_daemon_from_intermediate();
            exit_daemon(announce_daemon(start_lock, writer).and_then(run_daemon))
        }
        Err(e) => Err(Error::agent(format!("cannot fork the daemon: {e}"))),
    }
}

/// End the daemon process with its result. The daemon must never return into
/// the start command's outcome handling; stdio is `/dev/null`, so a failure is
/// logged.
#[expect(
    clippy::exit,
    reason = "the daemon must not return into the start command's outcome handling"
)]
fn exit_daemon(result: Result<()>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            crate::warn_log!("agent", "Daemon failed: {e}");
            std::process::exit(1)
        }
    }
}

/// Intermediate child: new session, detached stdio, fork daemon. Returns only
/// in the daemon; the intermediate exits without running destructors that
/// could unlock the inherited start lock.
#[expect(clippy::exit, reason = "intermediate must not return into caller code")]
fn fork_daemon_from_intermediate() {
    if fork::setsid().is_err() || fork::redirect_stdio().is_err() {
        std::process::exit(1);
    }
    match fork::fork() {
        Ok(Fork::Parent(_)) => std::process::exit(0),
        Ok(Fork::Child) => {}
        Err(_) => std::process::exit(1),
    }
}

/// Daemon: send `start` our PID, the first thing on the startup pipe.
///
/// # Errors
/// Returns an error if the PID cannot be written.
fn announce_daemon(
    start_lock: crate::fs_util::PathLock,
    mut report: std::io::PipeWriter,
) -> Result<StartupHandoff> {
    use std::io::Write;

    report
        .write_all(&nix::unistd::getpid().as_raw().to_ne_bytes())
        .map_err(|e| Error::agent(format!("cannot report the daemon PID: {e}")))?;
    Ok(StartupHandoff {
        _start_lock: start_lock,
        report,
    })
}

/// Daemon body: leave the launch directory, then run the agent; a failure
/// before the socket is bound is reported through `startup`.
///
/// # Errors
/// Returns an error if `chdir`, runtime construction, or the agent fails.
fn run_daemon(startup: StartupHandoff) -> Result<()> {
    let rt = match enter_daemon() {
        Ok(rt) => rt,
        Err(e) => {
            startup.fail(&e);
            return Err(e);
        }
    };
    let result = rt.block_on(async move {
        let mut agent = match Agent::new() {
            Ok(agent) => agent,
            Err(e) => {
                startup.fail(&e);
                return Err(e);
            }
        };
        agent.server.hold_startup_until_bound(startup);
        agent.start_background().await
    });
    rt.shutdown_timeout(RUNTIME_SHUTDOWN_WAIT_MAX);
    result
}

/// Leave the launch directory and build the runtime.
///
/// # Errors
/// Returns an error if `chdir` or runtime construction fails.
fn enter_daemon() -> Result<tokio::runtime::Runtime> {
    // Inputs were absolutized pre-fork; don't keep the launch dir busy.
    if let Err(e) = fork::chdir() {
        crate::warn_log!("agent", "Failed to chdir to /: {e}");
        return Err(Error::agent(format!(
            "cannot leave the launch directory: {e}"
        )));
    }

    tokio::runtime::Runtime::new()
        .map_err(|e| Error::agent(format!("cannot create the async runtime: {e}")))
}

/// Start the agent in background with race-free socket coordination. Progress
/// and advisories go to `on_notice`; nothing is printed here.
///
/// # Errors
/// Returns an error if socket operations, forking, or agent initialization fails.
pub fn start_background_agent(mut on_notice: impl FnMut(StartNotice)) -> Result<StartOutcome> {
    if !crate::config::loader::agent_enabled() {
        on_notice(StartNotice::Disabled);
        return Ok(StartOutcome::Disabled);
    }

    // The daemon runs in `/`: anchor relative inputs before the fork.
    if let Ok(cwd) = std::env::current_dir()
        && let Err(rejected) = crate::paths::LAUNCH_DIR.set(cwd)
    {
        debug_assert!(
            crate::paths::LAUNCH_DIR.get() == Some(&rejected),
            "launch dir set twice with a different value: new={}",
            rejected.display()
        );
    }
    crate::debug::init_path();
    super::ensure_runtime_dir()?;
    let socket_path = crate::paths::absolutize(&crate::paths::get_socket_path()?);
    // Absolutize a relative `GPY_AGENT_SOCKET_PATH` so the daemon, which
    // runs in `/`, binds the same path.
    if crate::paths::socket_path_override().is_some() {
        crate::paths::set_socket_override(&socket_path)?;
    }

    // Vet the directory before the lock file, stale-socket eviction, or
    // liveness ping relies on it; the daemon checks again at bind.
    crate::ipc::server::socket_dir::secure_socket_dir(&socket_path, super::socket_dir_repair())?;
    let start_lock = acquire_start_lock(&socket_path)?;

    let previous = match inspect_socket(&socket_path, &mut on_notice) {
        SocketCheck::Done(outcome) => return Ok(outcome),
        SocketCheck::Proceed { previous } => previous,
    };

    let pid = fork_agent(&socket_path, start_lock)?;
    Ok(previous.map_or(StartOutcome::Started { pid }, |prior| {
        StartOutcome::Replaced {
            previous: prior,
            pid,
        }
    }))
}

#[cfg(test)]
mod tests {
    /// `shutdown_timeout` returns promptly despite a stuck `spawn_blocking` job.
    #[test]
    fn shutdown_timeout_returns_promptly_despite_stuck_blocking_task() {
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        let started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_flag = std::sync::Arc::clone(&started);

        rt.spawn_blocking(move || {
            started_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_secs(30));
        });

        // Ensure the task started so only shutdown_timeout is measured.
        while !started.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let deadline = std::time::Duration::from_millis(200);
        let before = std::time::Instant::now();
        rt.shutdown_timeout(deadline);
        let elapsed = before.elapsed();

        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "shutdown_timeout must not block for anywhere near the stuck task's \
             30s sleep; took {elapsed:?}"
        );
    }
}
