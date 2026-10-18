//! Unix startup path for the agent daemon: check enablement, clean up stale
//! sockets, fork, wait for readiness, and nudge registered shells after restart.

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

/// Longest a start waits for another start on the same socket.
#[cfg(unix)]
const START_LOCK_WAIT_MAX: Duration = Duration::from_secs(10);

/// Poll interval while another start holds the lock.
#[cfg(unix)]
const START_LOCK_POLL: Duration = Duration::from_millis(50);

/// Per-socket start lock path, `<socket>.lock`. Never unlinked: removing a
/// lock file races like removing the socket.
#[cfg(unix)]
fn start_lock_path(socket_path: &std::path::Path) -> PathBuf {
    let mut lock = socket_path.as_os_str().to_owned();
    lock.push(".lock");
    PathBuf::from(lock)
}

/// Take the exclusive per-socket start lock, held across check → evict → fork
/// → bind. The forked daemon inherits the flock and drops it once bound; the
/// wait is bounded so a hung daemon cannot block later starts forever.
///
/// # Errors
/// Returns an error if the lock cannot be opened or taken within [`START_LOCK_WAIT_MAX`].
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

/// Wait until daemon `agent_pid` answers a ping, stopping early if it exits.
///
/// # Errors
/// Returns an error if the daemon dies or does not answer within [`READY_WAIT_MAX`].
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
/// Notify tracked shells that a new agent is ready, so they re-register
/// instead of keeping a stale registration until the next prompt.
pub(crate) fn notify_existing_shells_of_restart() {
    nudge_all_tracked_shells_in(&default_shell_dir());
}

/// Warm the theme-export cache and, if it changed, flag every alive tracked
/// shell to reload: config changes made while no agent ran are otherwise
/// unseen, and the just-started registry is empty.
#[cfg(unix)]
pub(crate) fn warm_theme_export_cache(
    theme_manager: &crate::theme::ThemeManager,
    config: &crate::config::Config,
) {
    warm_theme_export_cache_with(&default_shell_dir(), || {
        crate::cache::write_theme_export_cache(theme_manager, config)
    });
}

/// [`warm_theme_export_cache`] with injectable export write and tracking dir.
#[cfg(unix)]
fn warm_theme_export_cache_with(
    shell_dir: &std::path::Path,
    write_export: impl FnOnce() -> Result<bool>,
) {
    match write_export() {
        Ok(true) => flag_all_tracked_shells_in(shell_dir, ShellFlag::Reload),
        Ok(false) => {}
        Err(e) => {
            debug_log!(
                "agent",
                "Warning: Failed to write initial theme export cache ({e})"
            );
        }
    }
}

/// Request re-registration from every alive tracked shell, pruning dead ones.
#[cfg(unix)]
fn nudge_all_tracked_shells_in(shell_dir: &std::path::Path) {
    flag_all_tracked_shells_in(shell_dir, ShellFlag::Reregister);
}

/// Raise `flag` for every alive tracked shell, pruning dead ones. Reuses
/// [`ShellRenudger::pids_to_nudge`] with `is_registered = false` (startup
/// notifications go to everyone) and a fresh renudger, so
/// [`MAX_RENUDGE_ATTEMPTS`] never suppresses them.
#[cfg(unix)]
fn flag_all_tracked_shells_in(shell_dir: &std::path::Path, flag: ShellFlag) {
    let mut renudger = ShellRenudger::default();
    let pids = renudger.pids_to_nudge(shell_dir, |_| false, tracked_shell_liveness());

    for pid in pids {
        debug_log!(
            "agent",
            "Raising {} for tracked shell {pid} after agent start",
            flag.suffix()
        );
        raise_shell_flag(shell_dir, pid, flag);
    }
}

/// Fork the agent daemon and return once it answers on `socket_path`.
///
/// Explicit double fork rather than `fork::daemon()`, whose processes exit
/// before anything can be awaited: the original waits for readiness holding
/// `start_lock`; the intermediate `setsid`s, forks the daemon, and pipes back
/// its PID; the daemon `chdir`s to `/`, binds, then drops the inherited lock.
///
/// # Errors
/// Returns an error if the pipe or fork fails, the daemon never reports its
/// PID, or it dies or is not ready in time. In the daemon, the agent's result.
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
            // Held until now so a daemon that never binds cannot let another start in.
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

/// Intermediate child: new session, detached stdio, fork daemon, report PID.
/// Returns only in the daemon; the intermediate exits without running
/// destructors that could unlock the inherited start lock.
#[cfg(unix)]
#[expect(clippy::exit, reason = "intermediate must not return into caller code")]
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

/// Daemon body: leave the launch directory, then run the agent.
///
/// # Errors
/// Returns an error if `chdir`, runtime construction, or the agent fails.
#[cfg(unix)]
fn run_daemon(start_lock: Flock<std::fs::File>) -> Result<()> {
    use super::write_agent_version;

    // Inputs were absolutized pre-fork; don't keep the launch dir busy.
    if let Err(e) = fork::chdir() {
        crate::warn_log!("agent", "Failed to chdir to /: {e}");
        return Err(Error::process("chdir".to_owned(), e.to_string()));
    }

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
                #[expect(clippy::exit, reason = "daemonized child; init failed fatally")]
                std::process::exit(1);
            }
        }
    });
    // Bounded: an uncancellable spawn_blocking scan must not hang exit.
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    result
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
        let socket_path = crate::paths::absolutize(&super::get_socket_path()?);
        // Absolutize a relative `GPY_AGENT_SOCKET_PATH`; the default socket stays
        // unset so it keeps the `agent.version` marker.
        if super::socket_path_override().is_some() {
            let _ = super::SOCKET_OVERRIDE.set(socket_path.clone());
        }

        // Vet the directory before the lock file, stale-socket eviction, or
        // liveness ping relies on it; the daemon checks again at bind.
        crate::ipc::server::handle::secure_socket_dir(&socket_path, super::socket_dir_repair())?;
        let start_lock = acquire_start_lock(&socket_path)?;

        if super::check_and_cleanup_socket(&socket_path) {
            return Ok(());
        }

        eprintln!("Starting GPY Agent in background...");
        fork_agent(&socket_path, start_lock)
    }

    #[cfg(not(unix))]
    {
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

        super::nudge_all_tracked_shells_in(dir.path());
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
        )
        .expect("seed export");

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn unhandled stand-in shell");
        let pid = child.id();
        track_shells(shell_dir.path(), &[pid]);
        let reload_flag = shell_dir.path().join(format!("{pid}.reload"));

        super::warm_theme_export_cache_with(shell_dir.path(), || {
            crate::cache::theme_export::write_theme_export_to_dir(
                cache.path(),
                &default_theme,
                &config,
            )
        });
        let flagged_on_unchanged = reload_flag.exists();

        super::warm_theme_export_cache_with(shell_dir.path(), || {
            crate::cache::theme_export::write_theme_export_to_dir(
                cache.path(),
                &text_theme,
                &config,
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
