//! Unix startup path for launching the agent daemon.
//!
//! The functions in this module perform the idempotent startup flow used by
//! both Fish integration and CLI commands: check whether startup is enabled,
//! clean up stale sockets, fork into the background, wait for readiness, and
//! notify already-registered shells after a restart. Shared socket and version
//! helpers live in the parent [`super`] module.

use crate::{Error, Result};
#[cfg(unix)]
use crate::{agent::Agent, debug_log};
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use crate::ipc::registry::{
    ShellFlag, default_shell_dir, process_start_time_secs, ring_doorbell, write_shell_flag,
};
#[cfg(unix)]
use fork::Fork;
#[cfg(unix)]
use nix::errno::Errno;
#[cfg(unix)]
use nix::fcntl::{Flock, FlockArg};
#[cfg(unix)]
use nix::sys::signal::kill;
#[cfg(unix)]
use nix::unistd::Pid;

/// Whether the agent is disabled via configuration (`agent.enabled = false`).
#[cfg(unix)]
fn agent_disabled() -> bool {
    crate::config::loader::load_config().is_ok_and(|config| !config.agent.enabled)
}

/// Longest a start waits for another start on the same socket (#723).
#[cfg(unix)]
const START_LOCK_WAIT_MAX: Duration = Duration::from_secs(10);

/// Poll interval while another start holds the lock.
#[cfg(unix)]
const START_LOCK_POLL: Duration = Duration::from_millis(50);

/// Path of the per-socket start lock: `<socket>.lock`, so agents on different
/// sockets never block each other. Never unlinked: removing a lock file races
/// exactly like removing the socket does.
#[cfg(unix)]
fn start_lock_path(socket_path: &std::path::Path) -> PathBuf {
    let mut lock = socket_path.as_os_str().to_owned();
    lock.push(".lock");
    PathBuf::from(lock)
}

/// Take the exclusive per-socket start lock (#723).
///
/// Held across check → evict → fork → bind: the forked daemon inherits the
/// open file description (flock locks belong to it, not to a process) and
/// drops it once its socket is bound, or by dying. The wait is bounded so a
/// daemon hung before binding cannot block every later start forever.
///
/// # Errors
///
/// Returns an error if the lock file cannot be opened or locked, or if
/// another start still holds it after [`START_LOCK_WAIT_MAX`].
#[cfg(unix)]
fn acquire_start_lock(socket_path: &std::path::Path) -> Result<Flock<std::fs::File>> {
    use std::os::unix::fs::OpenOptionsExt;

    let lock_path = start_lock_path(socket_path);
    let lock_error = |detail: String| {
        Error::process(
            "start_lock".to_owned(),
            format!("cannot take start lock {}: {detail}", lock_path.display()),
        )
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&lock_path)
        .map_err(|e| lock_error(e.to_string()))?;
    let deadline = std::time::Instant::now()
        .checked_add(START_LOCK_WAIT_MAX)
        .unwrap_or_else(std::time::Instant::now);
    loop {
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => return Ok(lock),
            Err((unlocked, Errno::EWOULDBLOCK | Errno::EINTR)) => file = unlocked,
            Err((_, errno)) => return Err(lock_error(errno.to_string())),
        }
        if std::time::Instant::now() >= deadline {
            return Err(lock_error(format!(
                "another gpy-agent start held it for more than {}s",
                START_LOCK_WAIT_MAX.as_secs()
            )));
        }
        std::thread::sleep(START_LOCK_POLL);
    }
}

/// Defensive upper bound on re-nudges sent to one tracked shell PID over a
/// single agent lifetime.
///
/// The realistic reason a shell misses its restart nudge is a circuit-breaker
/// backoff (`GPY_CIRCUIT_BREAKER_BACKOFF_SECONDS`, 60s in `fish/core/constants.fish`)
/// that the nudge landed inside, so one or two 60s pruning rounds is normally
/// enough. Capping stops a shell that can *never* register — its Fish-side gpy
/// state is disabled or broken while its tracking file survives — from being
/// signalled, and therefore repainted, once a minute forever.
#[cfg(unix)]
const MAX_RENUDGE_ATTEMPTS: u32 = 5;

/// Tracks re-nudge attempts per shell PID so [`ShellRenudger::pids_to_nudge`]
/// stays bounded across the agent's lifetime.
#[cfg(unix)]
#[derive(Default)]
pub(crate) struct ShellRenudger {
    attempts: std::collections::HashMap<u32, u32>,
}

#[cfg(unix)]
impl ShellRenudger {
    /// Tracked shell PIDs that are alive but absent from the client registry
    /// and still within their re-nudge budget.
    ///
    /// A shell in that state is broken: it holds a tracking file, so it
    /// registered successfully at some point, yet the agent no longer knows it
    /// and it receives no live updates at all. Re-writing its `<pid>.reregister`
    /// flag and ringing the doorbell lets it retry; the shell removes the flag
    /// before each attempt, so every re-nudge is a fresh request.
    ///
    /// Files for PIDs that are no longer alive are removed: the `<pid>`
    /// tracking file and any `<pid>.reload`/`<pid>.reregister` flags, including
    /// orphan flags whose tracking file is already gone. A PID seen
    /// registered has its attempt budget cleared, so a later restart that
    /// strands it again starts from a full budget instead of an exhausted one.
    ///
    /// `is_alive` receives the tracking file's mtime (`None` for flag files
    /// and when it cannot be read), so it can also reject a PID recycled to a
    /// process that started after the file was written (#781); such a PID's
    /// tracking file and flags are removed by name in the same pass.
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
                // Flag file: never a tracking entry, only cleaned up when its
                // shell is gone.
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

        // Drop bookkeeping for PIDs that are no longer tracked at all, so the
        // map cannot grow unbounded over a long-lived agent.
        self.attempts.retain(|pid, _| tracked.contains(pid));

        targets
    }
}

/// Production liveness for tracked shells (#781).
///
/// The PID is alive ([`crate::ipc::ClientDirectory::is_client_alive`]) and its
/// process did not start more than 1 s after the tracking file was last
/// written. Every shell rewrites its file on each registration, so a newer
/// process means the PID was recycled. A failed start-time lookup or
/// unreadable mtime counts as alive (the #432 rule). One `sysinfo::System`
/// serves the whole scan.
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

/// Whether a process that started at `started_secs` (whole seconds since the
/// epoch) started after a tracking file last written at `modified`, with one
/// second of slack for sysinfo's whole-second start time.
#[cfg(unix)]
fn started_after_tracking(started_secs: u64, modified: SystemTime) -> bool {
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .is_ok_and(|written| started_secs > written.as_secs().saturating_add(1))
}

/// Re-send the restart nudge to every tracked shell that is alive but
/// unregistered, so it retries registration instead of staying stranded for the
/// rest of this agent's lifetime.
#[cfg(unix)]
pub(crate) fn renudge_unregistered_shells(
    renudger: &mut ShellRenudger,
    registry: &crate::ipc::ClientDirectory,
) {
    renudge_unregistered_shells_in(&default_shell_dir(), renudger, registry);
}

/// [`renudge_unregistered_shells`] against an explicit tracking directory, so
/// the registry-to-signal wiring is exercisable without touching the process's
/// `XDG_*`/`HOME` environment.
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
        request_reregistration(shell_dir, pid);
    }
}

/// Write `<pid>.reregister` in `shell_dir`, then ring the doorbell. The flag
/// must exist before the signal, or the shell's handler finds nothing to do.
#[cfg(unix)]
fn request_reregistration(shell_dir: &std::path::Path, pid: u32) {
    if let Err(e) = write_shell_flag(shell_dir, pid, ShellFlag::Reregister) {
        debug_log!("agent", "Failed to write reregister flag for {pid}: {e}");
        return;
    }
    let _ = ring_doorbell(pid);
}

/// How long `gpy-agent start` waits for its daemon to answer (#741).
#[cfg(unix)]
const READY_WAIT_MAX: Duration = Duration::from_secs(5);

/// Poll interval while waiting for the daemon to answer.
#[cfg(unix)]
const READY_WAIT_POLL: Duration = Duration::from_millis(10);

/// Wait until the daemon `agent_pid` answers a ping on `socket_path`.
///
/// Stops early once the daemon is gone (it is not our child, so `kill(pid, 0)`
/// failing means it exited and was reaped by init).
///
/// # Errors
///
/// Returns an error if the daemon dies or does not answer within
/// [`READY_WAIT_MAX`].
#[cfg(unix)]
fn wait_for_agent_ready(socket_path: &PathBuf, agent_pid: i32) -> Result<()> {
    let deadline = std::time::Instant::now()
        .checked_add(READY_WAIT_MAX)
        .unwrap_or_else(std::time::Instant::now);
    loop {
        if super::ping_agent_blocking(socket_path)? {
            return Ok(());
        }
        if kill(Pid::from_raw(agent_pid), None).is_err() {
            return Err(Error::process(
                "startup_failed".to_owned(),
                "Agent process exited before it was ready".to_owned(),
            ));
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::process(
                "startup_timeout".to_owned(),
                "Agent process did not become ready before timeout".to_owned(),
            ));
        }
        std::thread::sleep(READY_WAIT_POLL);
    }
}

#[cfg(unix)]
/// Notify previously tracked shells that a new agent instance is ready.
///
/// The shell-side live-update registration is tied to the shell PID, not the
/// daemon PID. When the agent restarts, shells can otherwise keep a stale
/// registration until the user happens to render another prompt. Writing each
/// alive tracked shell's `<pid>.reregister` flag and ringing its doorbell makes
/// that recovery explicit and testable.
///
/// Stale shell files are removed opportunistically and flag write failures
/// are logged per shell, so there is nothing to report to the caller.
pub(crate) fn notify_existing_shells_of_restart() {
    nudge_all_tracked_shells_in(&default_shell_dir());
}

/// Request re-registration from every alive shell tracked in `shell_dir`,
/// removing the files of ones that have exited.
///
/// Shares [`ShellRenudger::pids_to_nudge`]'s scan with the periodic
/// re-nudge path (see [`renudge_unregistered_shells_in`]) instead of walking
/// the tracking directory a second time, so there is one implementation of
/// "read the shell-tracking dir, parse PIDs, drop the dead ones".
///
/// Two deliberate differences from that periodic caller:
///
/// - `is_registered` is hardwired to `false`, preserving this path's "notify
///   *everyone*" semantics. That is not just convenient here, it is the only
///   correct choice: this call's whole purpose is to notify every alive
///   tracked shell of a restart, registered or not. Wiring in the real
///   registry lookup would narrow that down to "notify only the stragglers
///   the registry doesn't know about yet" — which is
///   [`renudge_unregistered_shells_in`]'s job, `pids_to_nudge`'s other,
///   already-correct caller, not this one's.
/// - The [`ShellRenudger`] is created fresh per call, so
///   [`MAX_RENUDGE_ATTEMPTS`] can never suppress a restart nudge: every PID
///   starts at zero attempts and this is a one-shot call. The budget only
///   exists to bound the *periodic* caller, which reuses one renudger for the
///   agent's lifetime.
///
/// Liveness comes from [`tracked_shell_liveness`]: `is_client_alive` (which
/// treats `EPERM` as alive, so a live shell we merely lack permission to
/// signal keeps its tracking file) plus the recycled-PID check (#781).
#[cfg(unix)]
fn nudge_all_tracked_shells_in(shell_dir: &std::path::Path) {
    let mut renudger = ShellRenudger::default();
    let pids = renudger.pids_to_nudge(shell_dir, |_| false, tracked_shell_liveness());

    for pid in pids {
        debug_log!("agent", "Nudging tracked shell {pid} after agent restart");
        request_reregistration(shell_dir, pid);
    }
}

/// Fork the agent daemon and return once it answers on `socket_path` (#741).
///
/// An explicit double fork instead of `fork::daemon()`, whose original and
/// intermediate processes `_exit(0)` before anything can be awaited:
///
/// 1. The original forks, reads the daemon's PID from a pipe, reaps the
///    intermediate and waits for readiness, holding `start_lock` throughout.
/// 2. The intermediate calls `setsid`, redirects stdio to `/dev/null`, forks
///    the daemon, writes its PID into the pipe and exits.
/// 3. The daemon `chdir`s to `/` (#724), so it never keeps the launch
///    directory's mount busy, binds its socket and only then drops the start
///    lock it inherited (#723).
///
/// The restart nudge stays in the daemon (`Agent::start_background`), so
/// shells are nudged once per start.
///
/// # Errors
///
/// Returns an error if the pipe or fork fails, if the daemon never reports
/// its PID, or if it dies or does not answer before the readiness timeout.
/// In the daemon itself, returns the agent's own exit result.
#[cfg(unix)]
fn fork_agent(socket_path: &PathBuf, start_lock: Flock<std::fs::File>) -> Result<()> {
    use std::io::Read;

    let (mut pid_reader, pid_writer) =
        std::io::pipe().map_err(|e| Error::process("pipe".to_owned(), e.to_string()))?;

    match fork::fork() {
        Ok(Fork::Parent(intermediate_pid)) => {
            drop(pid_writer);
            let mut pid_bytes = [0_u8; 4];
            let read = pid_reader.read_exact(&mut pid_bytes);
            let _ = fork::waitpid(intermediate_pid);
            read.map_err(|e| {
                Error::process(
                    "fork".to_owned(),
                    format!("Agent process did not start: {e}"),
                )
            })?;
            let ready = wait_for_agent_ready(socket_path, i32::from_ne_bytes(pid_bytes));
            // Held until now so a daemon that never binds cannot let another
            // start in early; unlocking a lock the daemon already released is
            // harmless.
            drop(start_lock);
            ready
        }
        Ok(Fork::Child) => {
            drop(pid_reader);
            fork_daemon_from_intermediate(pid_writer);
            run_daemon(start_lock)
        }
        Err(e) => Err(Error::process("fork".to_owned(), e.to_string())),
    }
}

/// Intermediate child: start a new session, detach stdio, fork the daemon and
/// report its PID.
///
/// Returns only in the daemon; the intermediate always exits here, without
/// running destructors that could unlock the inherited start lock.
#[cfg(unix)]
#[expect(
    clippy::exit,
    reason = "the intermediate process of the double fork must end right after handing the daemon's PID to the original process; returning would run the caller's code twice"
)]
fn fork_daemon_from_intermediate(mut pid_writer: std::io::PipeWriter) {
    use std::io::Write;

    if fork::setsid().is_err() || fork::redirect_stdio().is_err() {
        std::process::exit(1);
    }
    match fork::fork() {
        Ok(Fork::Parent(daemon_pid)) => {
            let code = i32::from(pid_writer.write_all(&daemon_pid.to_ne_bytes()).is_err());
            std::process::exit(code);
        }
        Ok(Fork::Child) => drop(pid_writer),
        Err(_) => std::process::exit(1),
    }
}

/// Daemon body: detach from the launch directory, then run the agent until
/// it shuts down.
///
/// # Errors
///
/// Returns an error if the tokio runtime cannot be built or the agent fails.
#[cfg(unix)]
fn run_daemon(start_lock: Flock<std::fs::File>) -> Result<()> {
    use super::write_agent_version;

    // Every relative input was made absolute before the fork (#724).
    if let Err(e) = fork::chdir() {
        debug_log!("agent", "Failed to chdir to /: {e}");
    }

    // Write version file so we can detect upgrades
    let _ = write_agent_version();

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| Error::process("runtime".to_owned(), e.to_string()))?;
    let result = rt.block_on(async {
        match Agent::new() {
            Ok(mut agent) => {
                agent.server.hold_start_lock_until_bound(start_lock);
                agent.start_background().await
            }
            Err(e) => {
                crate::debug::write_debug_log("agent", &format!("Fatal initialization error: {e}"));
                #[expect(
                    clippy::exit,
                    reason = "this is the daemonized child process after stdio has been redirected to /dev/null; agent init failed fatally, so exiting is the only way to end the process"
                )]
                std::process::exit(1);
            }
        }
    });
    // Bound runtime teardown instead of letting `rt`'s Drop block until
    // every spawn_blocking task finishes. Without this, an in-flight
    // scan with no cancellation hook (e.g. a slow language-detection
    // walk, #391) can hang process exit indefinitely after a clean
    // shutdown signal. Anything still running past the deadline is
    // detached, not joined; the process exiting when this function
    // returns forcibly terminates it regardless.
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    result
}

/// Start the agent in background using race-free socket-based coordination
///
/// # Errors
///
/// Returns an error if socket operations, process forking, or agent initialization fails.
pub fn start_background_agent() -> Result<()> {
    #[cfg(unix)]
    {
        if agent_disabled() {
            eprintln!("GPY Agent disabled via config (agent.enabled = false); skipping start");
            return Ok(()); // Successful no-op
        }

        // The daemon runs in `/` (#724): anchor every relative input to the
        // launch directory now, before the fork, so it inherits the result.
        if let Ok(cwd) = std::env::current_dir() {
            let _ = crate::paths::LAUNCH_DIR.set(cwd);
        }
        crate::debug::init_path();
        let socket_path = crate::paths::absolutize(&super::get_socket_path()?);
        // `--socket` already stored an absolute path; this covers a relative
        // `GPY_AGENT_SOCKET_PATH`, which the daemon would otherwise re-read
        // against `/`. The default socket is left unset so it keeps the
        // `agent.version` marker.
        if super::socket_path_override().is_some() {
            let _ = super::SOCKET_OVERRIDE.set(socket_path.clone());
        }

        // Serialize check → evict → fork → bind per socket (#723). The forked
        // daemon inherits the lock and releases it once its socket is bound.
        let start_lock = acquire_start_lock(&socket_path)?;

        // An agent already running (or one we just found replaced by a
        // responsive agent) is a successful no-op.
        if super::check_and_cleanup_socket(&socket_path) {
            return Ok(());
        }

        eprintln!("Starting GPY Agent in background...");
        // Exits non-zero (via `main`'s `Error:` line) unless the daemon
        // answers on its socket (#741).
        fork_agent(&socket_path, start_lock)
    }

    #[cfg(not(unix))]
    {
        eprintln!("❌ Background agent not supported on this platform");
        Err(Error::process(
            "platform".to_owned(),
            "Background agent only supported on Unix-like systems".to_owned(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    #[cfg(unix)]
    /// Seed `dir` with one tracking file per PID, mirroring the Fish side's
    /// `__gpy_track_shell_for_agent_recovery` (filename is the PID).
    fn track_shells(dir: &std::path::Path, pids: &[u32]) {
        for pid in pids {
            std::fs::write(dir.join(pid.to_string()), pid.to_string()).expect("seed shell file");
        }
    }

    /// A shell holding a tracking file but missing from the client registry is broken: it
    /// registered successfully once, so the agent should know it, yet it receives no
    /// live updates at all.
    ///
    /// Re-nudging it is the only way it recovers without the user pressing enter.
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

    /// Mirrors `notify_existing_shells_of_restart`'s opportunistic cleanup so
    /// tracking files for exited shells don't accumulate forever.
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

    /// #611: the restart nudge shares `pids_to_nudge`'s directory scan.
    ///
    /// It must keep that scan's dead-shell cleanup (tracking file and both
    /// flag kinds, including orphan flags) while still targeting *every*
    /// alive tracked shell (its `is_registered` is hardwired to `false`, since
    /// the just-started daemon's registry is empty).
    #[cfg(unix)]
    #[test]
    fn restart_nudge_cleans_up_tracking_files_for_dead_shells() {
        let dir = tempfile::tempdir().expect("create tempdir");
        // PIDs far above any plausible live process, so `is_client_alive`
        // reports them dead -- the same convention as
        // `is_client_alive_nonexistent_pid` in `ipc::registry`.
        track_shells(dir.path(), &[999_997_u32, 999_998_u32]);
        for name in ["999997.reload", "999997.reregister", "999996.reload"] {
            std::fs::write(dir.path().join(name), "").expect("seed flag");
        }

        super::nudge_all_tracked_shells_in(dir.path());

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

    /// #611: covers the half of `nudge_all_tracked_shells_in`'s scan that
    /// `restart_nudge_cleans_up_tracking_files_for_dead_shells` doesn't.
    ///
    /// That test covers the dead-PID cleanup half; this covers the half the
    /// function actually exists for -- an alive, tracked shell must be
    /// targeted for a nudge, and (unlike a dead shell's) its tracking file
    /// must survive.
    ///
    /// Uses the current test process's own PID as the "alive" shell,
    /// mirroring `is_client_alive_current_process` in `ipc::registry`, and
    /// drives the exact scan `nudge_all_tracked_shells_in` runs --
    /// `pids_to_nudge` with `is_registered` hardwired to `false` and the real
    /// `is_client_alive` -- without the flag write and doorbell, which are
    /// covered by `restart_nudge_writes_reregister_flag_and_spares_unhandled_shell`.
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

    /// #781: a tracking file written long before its PID's current process
    /// started belongs to a recycled PID. It is removed and never nudged.
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

        super::nudge_all_tracked_shells_in(dir.path());

        assert!(
            !dir.path().join(format!("{pid}.reregister")).exists(),
            "a recycled PID must not get a reregister flag"
        );
        assert!(
            !tracking.exists(),
            "a recycled PID's tracking file must be removed"
        );
    }

    /// #781: an unknown start time or mtime never makes a PID look recycled.
    #[cfg(unix)]
    #[test]
    fn started_after_tracking_needs_more_than_a_second() {
        let written = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        assert!(!super::started_after_tracking(1000, written));
        assert!(!super::started_after_tracking(1001, written));
        assert!(super::started_after_tracking(1002, written));
    }

    /// A shell that can never register (Fish-side gpy disabled or broken, but
    /// its tracking file survives) must not be signalled -- and repainted --
    /// once a minute for the agent's whole lifetime.
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

    /// Once a shell is registered again its budget must reset, so a *later*
    /// agent restart that strands it again still gets a full set of retries
    /// rather than inheriting an exhausted counter.
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

    /// End-to-end proof that the registry drives the re-nudge to the right PID:
    /// the unregistered tracked shell gets its `.reregister` flag written before
    /// the doorbell, while the registered one gets neither.
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

    /// #674 regression: the restart nudge writes `.reregister` and rings the
    /// doorbell, and a tracked process with NO handler (a shell mid-`exec`)
    /// survives it.
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

        super::nudge_all_tracked_shells_in(dir.path());
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

    /// #391: `fork_agent` bounds runtime teardown with `shutdown_timeout`.
    ///
    /// Instead of relying on `Runtime`'s Drop (which blocks until every
    /// `spawn_blocking` task finishes, with no deadline). This test isolates
    /// that mechanism -- not `fork_agent` itself, which forks/daemonizes and
    /// isn't practical to exercise in a unit test -- and proves
    /// `shutdown_timeout` returns promptly even while a `spawn_blocking` job
    /// with no cancellation hook is still running, mirroring an in-flight
    /// language-detection scan that outlives a shutdown signal.
    #[test]
    fn shutdown_timeout_returns_promptly_despite_stuck_blocking_task() {
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        let started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_flag = std::sync::Arc::clone(&started);

        rt.spawn_blocking(move || {
            started_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            // Simulate a scan with no cancellation hook that outlives shutdown.
            std::thread::sleep(std::time::Duration::from_secs(30));
        });

        // Wait for the blocking task to actually start before timing shutdown,
        // so the assertion below measures shutdown_timeout's own deadline
        // rather than incidental scheduling delay.
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
