//! Unix startup path for the agent daemon: check enablement, clean up stale
//! sockets, fork, wait for readiness, and nudge registered shells after restart.

#[cfg(unix)]
use crate::deadline::Deadline;
use crate::{Error, Result};
#[cfg(unix)]
use crate::{agent::Agent, debug_log};
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use crate::ipc::registry::{
    ShellFlag, default_shell_dir, process_start_time_secs, ring_doorbell, write_shell_flag,
};
#[cfg(unix)]
use fork::Fork;
#[cfg(unix)]
use nix::sys::signal::kill;
#[cfg(unix)]
use nix::unistd::Pid;

/// The progress line `gpy-agent start` prints on stderr before forking.
/// `gpy restart` filters this exact line from its own normalized output.
pub const START_PROGRESS_LINE: &str = "Starting GPY Agent in background...";

/// Whether the agent is disabled via configuration (`agent.enabled = false`).
#[cfg(unix)]
fn agent_disabled() -> bool {
    crate::config::loader::load_config().is_ok_and(|config| !config.agent.enabled)
}

/// Longest a start waits for another start on the same socket.
#[cfg(unix)]
const START_LOCK_WAIT_MAX: Duration = Duration::from_secs(10);

/// Take the exclusive per-socket start lock (`<socket>.lock`), held across
/// check → evict → fork → bind. The forked daemon inherits the lock and drops
/// it once bound; the wait is bounded so a hung daemon cannot block later
/// starts forever.
///
/// # Errors
/// Returns an error if the lock cannot be opened or taken within [`START_LOCK_WAIT_MAX`].
#[cfg(unix)]
fn acquire_start_lock(socket_path: &Path) -> Result<crate::fs_util::PathLock> {
    crate::fs_util::lock_beside(socket_path, START_LOCK_WAIT_MAX).map_err(|e| {
        Error::process(
            "start_lock",
            format!("cannot take start lock for {}: {e}", socket_path.display()),
        )
    })
}

/// Max re-nudges per tracked shell PID per agent lifetime, so a shell that can
/// never register is not repainted every minute forever. One or two rounds
/// normally clear a shell's 60s circuit-breaker backoff.
#[cfg(unix)]
const MAX_RENUDGE_ATTEMPTS: u32 = 5;

/// Bounded per-PID re-nudge attempts.
#[cfg(unix)]
#[derive(Default)]
pub(crate) struct ShellRenudger {
    attempts: std::collections::HashMap<u32, u32>,
}

#[cfg(unix)]
impl ShellRenudger {
    /// Tracked PIDs that are alive, unregistered, and within budget. Removes
    /// tracking files and flags of dead PIDs (including orphan flags), and
    /// resets the budget of registered ones. `is_alive` gets the tracking
    /// file's mtime (`None` for flags) to reject recycled PIDs.
    pub(crate) fn pids_to_nudge<R, A>(
        &mut self,
        shell_dir: &std::path::Path,
        is_registered: R,
        is_alive: A,
    ) -> Vec<u32>
    where
        R: Fn(u32) -> bool,
        A: Fn(u32, Option<SystemTime>) -> bool,
    {
        let Ok(entries) = std::fs::read_dir(shell_dir) else {
            return Vec::new();
        };

        let mut targets = Vec::new();
        let mut tracked = std::collections::HashSet::new();

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };
            if let Some((stem, suffix)) = name.split_once('.') {
                // Flag files are only cleaned up when their shell is gone.
                let is_flag = suffix == ShellFlag::Reload.suffix()
                    || suffix == ShellFlag::Reregister.suffix();
                if is_flag
                    && let Ok(flag_pid) = stem.parse::<u32>()
                    && !is_alive(flag_pid, None)
                {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            let Ok(pid) = name.parse::<u32>() else {
                continue;
            };
            tracked.insert(pid);

            let modified = entry.metadata().and_then(|m| m.modified()).ok();
            if !is_alive(pid, modified) {
                let _ = std::fs::remove_file(&path);
                for flag in [ShellFlag::Reload, ShellFlag::Reregister] {
                    let _ =
                        std::fs::remove_file(shell_dir.join(format!("{pid}.{}", flag.suffix())));
                }
                continue;
            }

            if is_registered(pid) {
                self.attempts.remove(&pid);
                continue;
            }

            let attempts = self.attempts.entry(pid).or_insert(0_u32);
            if *attempts >= MAX_RENUDGE_ATTEMPTS {
                continue;
            }
            *attempts = attempts.saturating_add(1_u32);
            targets.push(pid);
        }

        // Keep the map bounded over a long-lived agent.
        self.attempts.retain(|pid, _| tracked.contains(pid));

        targets
    }
}

/// Production liveness for tracked shells: alive and not started more than 1 s
/// after its tracking file was written (else the PID was recycled). Uncertain
/// probes count as alive so a live shell is never pruned.
#[cfg(unix)]
fn tracked_shell_liveness() -> impl Fn(u32, Option<SystemTime>) -> bool {
    let sys = std::cell::RefCell::new(sysinfo::System::new());
    move |pid, tracked_mtime| {
        if !crate::ipc::ClientDirectory::is_client_alive(pid) {
            return false;
        }
        let Some(written) = tracked_mtime else {
            return true;
        };
        process_start_time_secs(&mut sys.borrow_mut(), pid)
            .is_none_or(|started| !started_after_tracking(started, written))
    }
}

/// Whether a process started after a tracking file written at `modified`, with
/// one second of slack for sysinfo's whole-second start time.
#[cfg(unix)]
fn started_after_tracking(started_secs: u64, modified: SystemTime) -> bool {
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .is_ok_and(|written| started_secs > written.as_secs().saturating_add(1))
}

/// Re-send the restart nudge to every tracked shell that is alive but unregistered.
#[cfg(unix)]
pub(crate) fn renudge_unregistered_shells(
    renudger: &mut ShellRenudger,
    registry: &crate::ipc::ClientDirectory,
) {
    renudge_unregistered_shells_in(&default_shell_dir(), renudger, registry);
}

/// [`renudge_unregistered_shells`] against an explicit tracking directory.
#[cfg(unix)]
fn renudge_unregistered_shells_in(
    shell_dir: &std::path::Path,
    renudger: &mut ShellRenudger,
    registry: &crate::ipc::ClientDirectory,
) {
    let pids = renudger.pids_to_nudge(
        shell_dir,
        |pid| registry.is_registered(pid),
        tracked_shell_liveness(),
    );

    for pid in pids {
        debug_log!("agent", "Re-nudging unregistered tracked shell {pid}");
        raise_shell_flag(shell_dir, pid, ShellFlag::Reregister);
    }
}

/// Write `flag` for `pid`, then ring the doorbell; the flag must exist first.
#[cfg(unix)]
fn raise_shell_flag(shell_dir: &std::path::Path, pid: u32, flag: ShellFlag) {
    if let Err(e) = write_shell_flag(shell_dir, pid, flag) {
        debug_log!(
            "agent",
            "Failed to write {} flag for {pid}: {e}",
            flag.suffix()
        );
        return;
    }
    let _ = ring_doorbell(pid);
}

/// How long `gpy-agent start` waits for its daemon to answer.
#[cfg(unix)]
const READY_WAIT_MAX: Duration = Duration::from_secs(5);

/// Poll interval while waiting for the daemon to answer.
#[cfg(unix)]
const READY_WAIT_POLL: Duration = Duration::from_millis(10);

/// Wait until daemon `agent_pid` answers a ping, stopping early if it reports
/// a startup failure or exits.
///
/// # Errors
/// Returns an error, with the daemon's reported reason when it sent one, if
/// the daemon fails or does not answer within [`READY_WAIT_MAX`].
#[cfg(unix)]
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
        if super::ping_agent_blocking(socket_path)? {
            return Ok(());
        }
        if kill(Pid::from_raw(agent_pid), None).is_err() {
            // Gone, so every write end is closed and this drains to EOF.
            report.poll();
            return Err(startup_failed(report.failure().as_deref()));
        }
        if deadline.expired() {
            return Err(Error::process(
                "startup_timeout",
                "Agent process did not become ready before timeout".to_owned(),
            ));
        }
        std::thread::sleep(READY_WAIT_POLL);
    }
}

/// The daemon died before it was ready, with its reported `reason` if any.
#[cfg(unix)]
fn startup_failed(reason: Option<&str>) -> Error {
    let message = reason.map_or_else(
        || "Agent process exited before it was ready".to_owned(),
        |detail| format!("Agent process failed before it was ready: {detail}"),
    );
    Error::process("startup_failed", message)
}

/// Daemon half of the startup handshake with the waiting `gpy-agent start`:
/// holds the start lock and the write end of the startup pipe until the
/// socket is bound. The daemon's stderr is `/dev/null`, so a fatal error
/// before then goes down the pipe for `start` to print.
#[cfg(unix)]
pub(crate) struct StartupHandoff {
    _start_lock: crate::fs_util::PathLock,
    report: std::io::PipeWriter,
}

#[cfg(unix)]
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
#[cfg(unix)]
struct StartupReport {
    pipe: std::io::PipeReader,
    text: Vec<u8>,
    closed: bool,
}

#[cfg(unix)]
impl StartupReport {
    /// Make `pipe` non-blocking so polling it never stalls the readiness wait.
    ///
    /// # Errors
    /// Returns an error if the pipe's flags cannot be set.
    fn new(pipe: std::io::PipeReader) -> Result<Self> {
        use nix::fcntl::{FcntlArg, OFlag, fcntl};

        fcntl(&pipe, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))
            .map_err(|e| Error::process("pipe", e.to_string()))?;
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

/// Announce a started agent to every alive tracked shell, pruning dead ones:
/// warm the theme-export cache, then raise `.reload` if the export changed
/// (config edited while no agent ran) and `.reregister`, and ring each shell
/// once. One ring for both flags matters because Zsh drops a ring that lands
/// while its trap for the previous one runs, and a shell from an older
/// release has no backstop for that. Call once the socket accepts.
#[cfg(unix)]
pub(crate) fn announce_start_to_tracked_shells(
    theme_manager: &crate::theme::ThemeManager,
    config: &crate::config::Config,
    palette: &crate::template::Palette,
) {
    announce_start_in(&default_shell_dir(), || {
        crate::cache::write_theme_export_cache(theme_manager, config, palette)
    });
}

/// [`announce_start_to_tracked_shells`] with injectable export write and
/// tracking dir. Reuses [`ShellRenudger::pids_to_nudge`] with
/// `is_registered = false` (the announcement goes to everyone) and a fresh
/// renudger, so [`MAX_RENUDGE_ATTEMPTS`] never suppresses it.
#[cfg(unix)]
fn announce_start_in(shell_dir: &std::path::Path, write_export: impl FnOnce() -> Result<bool>) {
    let reload = write_export().unwrap_or_else(|e| {
        debug_log!(
            "agent",
            "Warning: Failed to write initial theme export cache ({e})"
        );
        false
    });
    let mut renudger = ShellRenudger::default();
    let pids = renudger.pids_to_nudge(shell_dir, |_| false, tracked_shell_liveness());

    for pid in pids {
        debug_log!(
            "agent",
            "Announcing agent start to tracked shell {pid} (reload: {reload})"
        );
        if reload && let Err(e) = write_shell_flag(shell_dir, pid, ShellFlag::Reload) {
            debug_log!("agent", "Failed to write reload flag for {pid}: {e}");
        }
        // Writes `.reregister`, then rings once for both flags.
        raise_shell_flag(shell_dir, pid, ShellFlag::Reregister);
    }
}

/// Fork the agent daemon and return once it answers on `socket_path`.
///
/// Explicit double fork rather than `fork::daemon()`, whose processes exit
/// before anything can be awaited: the original waits for readiness holding
/// `start_lock`; the intermediate `setsid`s and forks the daemon; the daemon
/// pipes back its PID, `chdir`s to `/`, binds, then drops the inherited lock
/// and closes the pipe ([`StartupHandoff`]).
///
/// # Errors
/// Returns an error if the pipe or fork fails, the daemon never reports its
/// PID, or it fails or is not ready in time. In the daemon, the agent's result.
#[cfg(unix)]
fn fork_agent(socket_path: &Path, start_lock: crate::fs_util::PathLock) -> Result<()> {
    use std::io::Read;

    let (mut reader, writer) =
        std::io::pipe().map_err(|e| Error::process("pipe", e.to_string()))?;

    match fork::fork() {
        Ok(Fork::Parent(intermediate_pid)) => {
            drop(writer);
            let mut pid_bytes = [0_u8; 4];
            let read = reader.read_exact(&mut pid_bytes);
            let _ = fork::waitpid(intermediate_pid);
            read.map_err(|e| Error::process("fork", format!("Agent process did not start: {e}")))?;
            let ready = StartupReport::new(reader).and_then(|report| {
                wait_for_agent_ready(socket_path, i32::from_ne_bytes(pid_bytes), report)
            });
            // Held until now so a daemon that never binds cannot let another start in.
            drop(start_lock);
            ready
        }
        Ok(Fork::Child) => {
            drop(reader);
            fork_daemon_from_intermediate();
            run_daemon(announce_daemon(start_lock, writer)?)
        }
        Err(e) => Err(Error::process("fork", e.to_string())),
    }
}

/// Intermediate child: new session, detached stdio, fork daemon. Returns only
/// in the daemon; the intermediate exits without running destructors that
/// could unlock the inherited start lock.
#[cfg(unix)]
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
#[cfg(unix)]
fn announce_daemon(
    start_lock: crate::fs_util::PathLock,
    mut report: std::io::PipeWriter,
) -> Result<StartupHandoff> {
    use std::io::Write;

    report
        .write_all(&nix::unistd::getpid().as_raw().to_ne_bytes())
        .map_err(|e| Error::process("pipe", e.to_string()))?;
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
#[cfg(unix)]
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
    // Bounded: an uncancellable spawn_blocking scan must not hang exit.
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    result
}

/// Leave the launch directory, write the version marker, build the runtime.
///
/// # Errors
/// Returns an error if `chdir` or runtime construction fails.
#[cfg(unix)]
fn enter_daemon() -> Result<tokio::runtime::Runtime> {
    // Inputs were absolutized pre-fork; don't keep the launch dir busy.
    if let Err(e) = fork::chdir() {
        crate::warn_log!("agent", "Failed to chdir to /: {e}");
        return Err(Error::process("chdir", e.to_string()));
    }

    if let Err(e) = super::write_agent_version() {
        crate::warn_log!("agent", "Failed to write agent version file: {e}");
    }

    tokio::runtime::Runtime::new().map_err(|e| Error::process("runtime", e.to_string()))
}

/// Start the agent in background with race-free socket coordination.
///
/// # Errors
/// Returns an error if socket operations, forking, or agent initialization fails.
pub fn start_background_agent() -> Result<()> {
    #[cfg(unix)]
    {
        if agent_disabled() {
            eprintln!("GPY Agent disabled via config (agent.enabled = false); skipping start");
            return Ok(());
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
        // Absolutize a relative `GPY_AGENT_SOCKET_PATH`; the default socket stays
        // unset so it keeps the `agent.version` marker.
        if crate::paths::socket_path_override().is_some() {
            crate::paths::set_socket_override(&socket_path)?;
        }

        // Vet the directory before the lock file, stale-socket eviction, or
        // liveness ping relies on it; the daemon checks again at bind.
        crate::ipc::server::socket_dir::secure_socket_dir(
            &socket_path,
            super::socket_dir_repair(),
        )?;
        let start_lock = acquire_start_lock(&socket_path)?;

        if super::check_and_cleanup_socket(&socket_path) {
            return Ok(());
        }

        eprintln!("{START_PROGRESS_LINE}");
        fork_agent(&socket_path, start_lock)
    }

    #[cfg(not(unix))]
    {
        Err(Error::process(
            "platform",
            "Background agent only supported on Unix-like systems".to_owned(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    #[cfg(unix)]
    /// Seed `dir` with one tracking file per PID, as Fish does.
    fn track_shells(dir: &std::path::Path, pids: &[u32]) {
        for pid in pids {
            std::fs::write(dir.join(pid.to_string()), pid.to_string()).expect("seed shell file");
        }
    }

    /// Tracked but unregistered shells are re-nudged.
    #[cfg(unix)]
    #[test]
    fn renudge_targets_live_unregistered_tracked_shells() {
        let dir = tempfile::tempdir().expect("create tempdir");
        track_shells(dir.path(), &[100_u32, 200_u32]);

        let mut renudger = super::ShellRenudger::default();
        let targets = renudger.pids_to_nudge(dir.path(), |pid| pid == 100_u32, |_, _| true);

        assert_eq!(
            targets,
            vec![200_u32],
            "only the tracked-but-unregistered shell should be nudged"
        );
    }

    /// Dead shells' tracking files are removed.
    #[cfg(unix)]
    #[test]
    fn renudge_removes_the_tracking_file_for_a_dead_shell() {
        let dir = tempfile::tempdir().expect("create tempdir");
        track_shells(dir.path(), &[300_u32]);

        let mut renudger = super::ShellRenudger::default();
        let targets = renudger.pids_to_nudge(dir.path(), |_| false, |_, _| false);

        assert!(targets.is_empty(), "a dead shell must not be signalled");
        assert!(
            !dir.path().join("300").exists(),
            "a dead shell's tracking file should be cleaned up"
        );
    }

    /// The restart nudge cleans up dead shells' tracking and flag files.
    #[cfg(unix)]
    #[test]
    fn restart_nudge_cleans_up_tracking_files_for_dead_shells() {
        let dir = tempfile::tempdir().expect("create tempdir");
        // PIDs far above any plausible live process.
        track_shells(dir.path(), &[999_997_u32, 999_998_u32]);
        for name in ["999997.reload", "999997.reregister", "999996.reload"] {
            std::fs::write(dir.path().join(name), "").expect("seed flag");
        }

        super::announce_start_in(dir.path(), || Ok(false));

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "the restart nudge must clean up tracking and flag files for exited shells: {leftovers:?}"
        );
    }

    /// The restart scan targets an alive tracked shell and keeps its file.
    #[cfg(unix)]
    #[test]
    fn nudge_all_tracked_shells_targets_and_preserves_an_alive_tracked_shell() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let pid = std::process::id();
        track_shells(dir.path(), &[pid]);

        let mut renudger = super::ShellRenudger::default();
        let targets = renudger.pids_to_nudge(
            dir.path(),
            |_| false,
            |tracked_pid, _| crate::ipc::ClientDirectory::is_client_alive(tracked_pid),
        );

        assert_eq!(
            targets,
            vec![pid],
            "an alive tracked shell must be targeted for a nudge"
        );
        assert!(
            dir.path().join(pid.to_string()).exists(),
            "an alive shell's tracking file must survive (only dead ones are cleaned up)"
        );
    }

    /// A tracking file older than its PID's process means a recycled PID.
    #[cfg(unix)]
    #[test]
    fn nudge_skips_and_removes_tracking_file_older_than_its_process() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let pid = std::process::id();
        track_shells(dir.path(), &[pid]);
        let tracking = dir.path().join(pid.to_string());
        std::fs::File::options()
            .write(true)
            .open(&tracking)
            .expect("open tracking file")
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000))
            .expect("backdate tracking file");

        super::announce_start_in(dir.path(), || Ok(false));

        assert!(
            !dir.path().join(format!("{pid}.reregister")).exists(),
            "a recycled PID must not get a reregister flag"
        );
        assert!(
            !tracking.exists(),
            "a recycled PID's tracking file must be removed"
        );
    }

    /// An unknown start time or mtime never makes a PID look recycled.
    #[cfg(unix)]
    #[test]
    fn started_after_tracking_needs_more_than_a_second() {
        let written = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        assert!(!super::started_after_tracking(1000, written));
        assert!(!super::started_after_tracking(1001, written));
        assert!(super::started_after_tracking(1002, written));
    }

    /// A never-registering shell is nudged at most `MAX_RENUDGE_ATTEMPTS` times.
    #[cfg(unix)]
    #[test]
    fn renudge_stops_after_the_attempt_cap() {
        let dir = tempfile::tempdir().expect("create tempdir");
        track_shells(dir.path(), &[400_u32]);

        let mut renudger = super::ShellRenudger::default();
        let mut nudges = 0_u32;
        for _ in 0_u32..(super::MAX_RENUDGE_ATTEMPTS + 3_u32) {
            nudges += u32::try_from(
                renudger
                    .pids_to_nudge(dir.path(), |_| false, |_, _| true)
                    .len(),
            )
            .expect("nudge count fits in u32");
        }

        assert_eq!(
            nudges,
            super::MAX_RENUDGE_ATTEMPTS,
            "a permanently unregisterable shell should be nudged at most MAX_RENUDGE_ATTEMPTS times"
        );
    }

    /// Re-registering resets the shell's nudge budget.
    #[cfg(unix)]
    #[test]
    fn renudge_attempt_budget_resets_once_the_shell_registers() {
        let dir = tempfile::tempdir().expect("create tempdir");
        track_shells(dir.path(), &[500_u32]);

        let mut renudger = super::ShellRenudger::default();
        for _ in 0_u32..super::MAX_RENUDGE_ATTEMPTS {
            let _ = renudger.pids_to_nudge(dir.path(), |_| false, |_, _| true);
        }
        assert!(
            renudger
                .pids_to_nudge(dir.path(), |_| false, |_, _| true)
                .is_empty(),
            "budget should be exhausted before the recovery round"
        );

        // Recovered: the agent now knows this PID again.
        let _ = renudger.pids_to_nudge(dir.path(), |_| true, |_, _| true);

        assert_eq!(
            renudger.pids_to_nudge(dir.path(), |_| false, |_, _| true),
            vec![500_u32],
            "a shell that recovered and was stranded again should get a fresh budget"
        );
    }

    /// Spawn a stand-in shell that touches `mark` when SIGURG arrives while its
    /// `<pid>.reregister` flag already exists in `dir`.
    #[cfg(unix)]
    fn spawn_reregister_observer(
        dir: &std::path::Path,
        mark: &std::path::Path,
    ) -> std::process::Child {
        let script = format!(
            "trap '[ -e \"{dir}/$$.reregister\" ] && touch \"{mark}\"' URG; while :; do sleep 0.05; done",
            dir = dir.display(),
            mark = mark.display(),
        );
        let child = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .spawn()
            .expect("spawn stand-in shell");
        // Let sh install its trap before any doorbell rings.
        std::thread::sleep(std::time::Duration::from_millis(200));
        child
    }

    #[cfg(unix)]
    fn wait_for(path: &std::path::Path) -> bool {
        for _ in 0_u32..250_u32 {
            if path.exists() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        path.exists()
    }

    /// Only the unregistered shell gets its `.reregister` flag and doorbell.
    #[cfg(unix)]
    #[test]
    fn renudge_signals_only_the_unregistered_tracked_shell() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let registered_mark = dir.path().join("registered.mark");
        let stranded_mark = dir.path().join("stranded.mark");

        let mut registered_child = spawn_reregister_observer(dir.path(), &registered_mark);
        let mut stranded_child = spawn_reregister_observer(dir.path(), &stranded_mark);

        let registered_pid = registered_child.id();
        let stranded_pid = stranded_child.id();
        track_shells(dir.path(), &[registered_pid, stranded_pid]);

        let registry = crate::ipc::ClientDirectory::new();
        registry.register(registered_pid, None);

        let mut renudger = super::ShellRenudger::default();
        super::renudge_unregistered_shells_in(dir.path(), &mut renudger, &registry);

        assert!(
            wait_for(&stranded_mark),
            "the unregistered tracked shell should get the doorbell with its reregister flag present"
        );
        assert!(
            !dir.path()
                .join(format!("{registered_pid}.reregister"))
                .exists(),
            "the registered shell must not be asked to re-register"
        );
        assert!(!registered_mark.exists());

        for child in [&mut registered_child, &mut stranded_child] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// The restart nudge writes `.reregister`, and a shell without a handler survives.
    #[cfg(unix)]
    #[test]
    fn restart_nudge_writes_reregister_flag_and_spares_unhandled_shell() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn unhandled stand-in shell");
        let pid = child.id();
        track_shells(dir.path(), &[pid]);

        super::announce_start_in(dir.path(), || Ok(false));
        // Give an unhandled SIGURG time to kill the child, if it would.
        std::thread::sleep(std::time::Duration::from_millis(200));

        assert!(
            dir.path().join(format!("{pid}.reregister")).exists(),
            "the restart nudge must write the reregister flag"
        );
        assert!(
            child.try_wait().expect("poll child").is_none(),
            "an unhandled process must survive the restart nudge"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    /// The startup warm-up raises `.reload` only when the export changed.
    #[cfg(unix)]
    #[test]
    fn warm_theme_export_flags_reload_only_when_the_export_changed() {
        let shell_dir = tempfile::tempdir().expect("create shell dir");
        let cache = tempfile::tempdir().expect("create cache dir");
        let config = crate::config::Config::default();
        let default_theme = crate::theme::ThemeManager::builtin("default").expect("default theme");
        let text_theme = crate::theme::ThemeManager::builtin("text").expect("text theme");
        // The previous agent's export, as the shell sourced it.
        crate::cache::theme_export::write_theme_export_to_dir(
            cache.path(),
            &default_theme,
            &config,
            &crate::template::Palette::default(),
        )
        .expect("seed export");

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn unhandled stand-in shell");
        let pid = child.id();
        track_shells(shell_dir.path(), &[pid]);
        let reload_flag = shell_dir.path().join(format!("{pid}.reload"));

        super::announce_start_in(shell_dir.path(), || {
            crate::cache::theme_export::write_theme_export_to_dir(
                cache.path(),
                &default_theme,
                &config,
                &crate::template::Palette::default(),
            )
        });
        let flagged_on_unchanged = reload_flag.exists();

        super::announce_start_in(shell_dir.path(), || {
            crate::cache::theme_export::write_theme_export_to_dir(
                cache.path(),
                &text_theme,
                &config,
                &crate::template::Palette::default(),
            )
        });
        let flagged_on_changed = reload_flag.exists();

        let _ = child.kill();
        let _ = child.wait();
        assert!(
            !flagged_on_unchanged,
            "an unchanged export must not ask shells to reload"
        );
        assert!(
            flagged_on_changed,
            "a changed export must write the tracked shell's reload flag"
        );
    }

    /// A start that changed the export rings each shell once, with `.reload`
    /// and `.reregister` both already written: Zsh drops a second ring that
    /// lands while its trap for the first is running.
    #[cfg(unix)]
    #[test]
    fn announce_start_rings_once_for_reload_and_reregister() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let mark = dir.path().join("rings.mark");
        let script = format!(
            "trap 'if [ -e \"{dir}/$$.reload\" ] && [ -e \"{dir}/$$.reregister\" ]; then echo both >>\"{mark}\"; else echo partial >>\"{mark}\"; fi' URG; while :; do sleep 0.05; done",
            dir = dir.path().display(),
            mark = mark.display(),
        );
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .spawn()
            .expect("spawn stand-in shell");
        // Let sh install its trap before the doorbell rings.
        std::thread::sleep(std::time::Duration::from_millis(200));
        track_shells(dir.path(), &[child.id()]);

        super::announce_start_in(dir.path(), || Ok(true));
        let rang = wait_for(&mark);
        // Room for a second ring to show up, if one were sent.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = child.kill();
        let _ = child.wait();

        assert!(rang, "the shell must be rung");
        assert_eq!(
            std::fs::read_to_string(&mark).expect("read mark"),
            "both\n",
            "one ring, after both flags were written"
        );
    }

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
