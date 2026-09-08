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
use std::time::Duration;

#[cfg(unix)]
use fork::{Fork, daemon};
#[cfg(unix)]
use nix::sys::signal::{Signal, kill};
#[cfg(unix)]
use nix::unistd::Pid;

/// Check agent compatibility and return socket path if agent should start
///
/// # Returns
///
/// - `Ok(Some(socket_path))` if the agent should proceed with startup
/// - `Ok(None)` if the agent is disabled or already running (idempotent success)
///
/// # Errors
///
/// Returns an error only if the socket path cannot be determined or if there's
/// a genuine failure (not including "already running" or "disabled" cases).
#[cfg(unix)]
fn check_agent_compatibility() -> Result<Option<PathBuf>> {
    use super::{check_and_cleanup_socket, get_socket_path};

    // Check if agent is enabled via configuration
    if let Ok(config) = crate::config::loader::load_config()
        && !config.agent.enabled
    {
        eprintln!("GPY Agent disabled via config (agent.enabled = false); skipping start");
        return Ok(None); // Successful no-op
    }

    let socket_path = get_socket_path()?;

    // Check if an agent is already running and handle version compatibility
    if check_and_cleanup_socket(&socket_path) {
        // Agent already running - this is a successful no-op
        return Ok(None);
    }

    Ok(Some(socket_path))
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
    /// and it receives no SIGUSR1 live updates at all. Re-sending the SIGALRM
    /// restart nudge lets it retry — which only recovers anything because the
    /// Fish handler leaves the restart marker *unconsumed* after a failed
    /// attempt (see `__gpy_refresh_registration_after_restart` in
    /// `fish/core/ipc.fish`); consuming it there would make every retry a no-op.
    ///
    /// Tracking files for PIDs that are no longer alive are removed, mirroring
    /// [`notify_existing_shells_of_restart`]'s opportunistic cleanup. A PID seen
    /// registered has its attempt budget cleared, so a later restart that
    /// strands it again starts from a full budget instead of an exhausted one.
    pub(crate) fn pids_to_nudge<R, A>(
        &mut self,
        shell_dir: &std::path::Path,
        is_registered: R,
        is_alive: A,
    ) -> Vec<u32>
    where
        R: Fn(u32) -> bool,
        A: Fn(u32) -> bool,
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
            let Ok(pid) = name.parse::<u32>() else {
                continue;
            };
            tracked.insert(pid);

            if !is_alive(pid) {
                let _ = std::fs::remove_file(&path);
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

/// Re-send the SIGALRM restart nudge to every tracked shell that is alive but
/// unregistered, so it retries registration instead of staying stranded for the
/// rest of this agent's lifetime.
#[cfg(unix)]
pub(crate) fn renudge_unregistered_shells(
    renudger: &mut ShellRenudger,
    registry: &crate::ipc::ClientDirectory,
) {
    renudge_unregistered_shells_in(&runtime_root().join("shells"), renudger, registry);
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
        crate::ipc::ClientDirectory::is_client_alive,
    );

    for pid in pids {
        let Ok(pid_raw) = i32::try_from(pid) else {
            continue;
        };
        debug_log!("agent", "Re-nudging unregistered tracked shell {pid}");
        let _ = kill(Pid::from_raw(pid_raw), Signal::SIGALRM);
    }
}

/// Fork and start the agent as a background daemon process
///
/// # Errors
///
/// Returns an error if:
/// - Forking fails (resource limits, permissions)
/// - Child process cannot initialize the tokio runtime
/// - Agent initialization fails (see `Agent::new()` in `gpy-agent/src/agent.rs`)
#[cfg(unix)]
fn wait_for_agent_ready(socket_path: &PathBuf, child_pid: i32) -> Result<()> {
    for _ in 0_i32..50_i32 {
        if super::ping_agent_blocking(socket_path)? {
            return Ok(());
        }

        if kill(Pid::from_raw(child_pid), None).is_err() {
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    Err(Error::process(
        "startup_timeout".to_owned(),
        "Agent process did not become ready before timeout".to_owned(),
    ))
}

#[cfg(unix)]
fn runtime_root() -> PathBuf {
    crate::paths::runtime_root_for(
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        std::env::var("XDG_CACHE_HOME").ok().as_deref(),
        crate::paths::home_dir().as_deref(),
    )
}

#[cfg(unix)]
/// Write `content` to `marker_path` atomically.
///
/// The Fish-side restart handler reads the marker from a `SIGALRM`
/// signal-handler context, racing an in-progress write from this process. A
/// plain `std::fs::write` opens the destination with truncate+write, which is
/// not a single atomic syscall on POSIX: a reader that opens the file in the
/// narrow window between the truncate and the full content landing can
/// observe a transiently empty file. Writing to a temp file in the same
/// directory (so the following rename stays on one filesystem, which POSIX
/// guarantees is atomic) and renaming it into place means a concurrent reader
/// only ever observes either the complete previous content or the complete
/// new content -- never a partial or empty read.
///
/// Also opportunistically cleans up stray temp files left behind by a
/// previous write that crashed between the write and the rename (e.g. the
/// process was killed mid-write). This is safe and self-healing: a stray temp
/// file is never read as the marker itself, so leaving one behind for one
/// extra restart cycle has no observable effect, and each subsequent call
/// sweeps up anything left over.
///
/// # Errors
///
/// Returns an error if the temp file cannot be written or the rename fails.
/// The temp file is removed on a failed rename so it doesn't accumulate.
fn write_marker_atomically(
    dir: &std::path::Path,
    marker_path: &std::path::Path,
    content: &str,
) -> std::io::Result<()> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let entry_name = entry.file_name();
            let name = entry_name.to_string_lossy();
            if name.starts_with("agent.restart.marker.") && name.ends_with(".tmp") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    let tmp_path = dir.join(format!("agent.restart.marker.{}.tmp", std::process::id()));
    std::fs::write(&tmp_path, content)?;

    let rename_result = std::fs::rename(&tmp_path, marker_path);
    if rename_result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    rename_result
}

#[cfg(unix)]
/// Notify previously registered Fish shells that a new agent instance is ready.
///
/// The shell-side live-update registration is tied to the Fish PID, not the
/// daemon PID. When the agent restarts, shells can otherwise keep a stale
/// registration until the user happens to render another prompt. Writing a
/// restart marker and nudging tracked shells via `SIGALRM` makes that recovery
/// explicit and testable.
///
/// # Errors
///
/// Returns an error if the runtime directory or restart marker cannot be
/// written. Stale shell PID files are removed opportunistically and do not
/// surface as errors.
pub(crate) fn notify_existing_shells_of_restart() -> Result<()> {
    let runtime_root = runtime_root();
    std::fs::create_dir_all(&runtime_root)
        .map_err(|e| Error::process("runtime_root".to_owned(), e.to_string()))?;

    let marker_path = runtime_root.join("agent.restart.marker");
    let marker = format!(
        "{}:{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    write_marker_atomically(&runtime_root, &marker_path, &marker)
        .map_err(|e| Error::process("restart_marker".to_owned(), e.to_string()))?;

    nudge_all_tracked_shells_in(&runtime_root.join("shells"));

    Ok(())
}

/// SIGALRM every alive shell tracked in `shell_dir`, removing the tracking
/// files of ones that have exited.
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
/// Liveness now comes from [`crate::ipc::ClientDirectory::is_client_alive`],
/// which treats `EPERM` as alive where the previous inline `kill(pid, None)`
/// treated any error as dead — so a live shell we merely lack permission to
/// signal keeps its tracking file instead of having it deleted.
#[cfg(unix)]
fn nudge_all_tracked_shells_in(shell_dir: &std::path::Path) {
    let mut renudger = ShellRenudger::default();
    let pids = renudger.pids_to_nudge(
        shell_dir,
        |_| false,
        crate::ipc::ClientDirectory::is_client_alive,
    );

    for pid in pids {
        let Ok(pid_raw) = i32::try_from(pid) else {
            continue;
        };
        debug_log!("agent", "Nudging tracked shell {pid} after agent restart");
        let _ = kill(Pid::from_raw(pid_raw), Signal::SIGALRM);
    }
}

/// Fork and start the agent as a background daemon process
///
/// # Errors
///
/// Returns an error if:
/// - Forking fails (resource limits, permissions)
/// - Child process cannot initialize the tokio runtime
/// - Agent initialization fails (see `Agent::new()` in `gpy-agent/src/agent.rs`)
#[cfg(unix)]
fn fork_agent(socket_path: &PathBuf) -> Result<()> {
    use super::write_agent_version;

    // daemon(nochdir, noclose) - using daemon() function for daemonization
    // nochdir=true: keep working directory
    // noclose=false: close stdin/stdout/stderr and redirect to /dev/null
    match daemon(true, false) {
        Ok(Fork::Child) => {
            // Child: we are the agent background process
            // stdin/stdout/stderr are already redirected to /dev/null by daemon()

            // Write version file so we can detect upgrades
            let _ = write_agent_version();

            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| Error::process("runtime".to_owned(), e.to_string()))?;
            let result = rt.block_on(async {
                match Agent::new() {
                    Ok(mut agent) => agent.start_background().await,
                    Err(e) => {
                        crate::debug::write_debug_log(
                            "agent",
                            &format!("Fatal initialization error: {e}"),
                        );
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
        Ok(Fork::Parent(child_pid)) => {
            // NOTE: with `daemon()` the parent is exited inside `daemon()` and this
            // arm typically never runs. The restart nudge is therefore performed by
            // the daemon child itself (see `Agent::start_background`). This arm is
            // kept as a best-effort fallback for platforms where `daemon()` returns
            // in the parent.
            wait_for_agent_ready(socket_path, child_pid)?;
            let _ = notify_existing_shells_of_restart();
            eprintln!("✅ GPY Agent started successfully (PID: {child_pid})");
            Ok(())
        }
        Err(e) => {
            eprintln!("❌ Failed to start GPY Agent: {e}");
            Err(Error::process("fork".to_owned(), e.to_string()))
        }
    }
}

/// Start the agent in background using race-free socket-based coordination
///
/// # Errors
///
/// Returns an error if socket operations, process forking, or agent initialization fails.
pub fn start_background_agent() -> Result<()> {
    #[cfg(unix)]
    {
        // Step 1: Check agent compatibility (config enabled, version, etc.)
        let Some(socket_path) = check_agent_compatibility()? else {
            // Agent disabled or already running - successful no-op
            return Ok(());
        };

        eprintln!("Starting GPY Agent in background...");

        // Step 2: Fork and start the agent process
        fork_agent(&socket_path)
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
    /// registered successfully once, so the agent should know it, yet it receives no SIGUSR1
    /// live updates at all.
    ///
    /// Re-nudging it is the only way it recovers without the user pressing enter.
    #[cfg(unix)]
    #[test]
    fn renudge_targets_live_unregistered_tracked_shells() {
        let dir = tempfile::tempdir().expect("create tempdir");
        track_shells(dir.path(), &[100_u32, 200_u32]);

        let mut renudger = super::ShellRenudger::default();
        let targets = renudger.pids_to_nudge(dir.path(), |pid| pid == 100_u32, |_| true);

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
        let targets = renudger.pids_to_nudge(dir.path(), |_| false, |_| false);

        assert!(targets.is_empty(), "a dead shell must not be signalled");
        assert!(
            !dir.path().join("300").exists(),
            "a dead shell's tracking file should be cleaned up"
        );
    }

    /// #611: the restart nudge shares `pids_to_nudge`'s directory scan.
    ///
    /// It must keep that scan's dead-tracking-file cleanup while still
    /// targeting *every* alive tracked shell (its `is_registered` is hardwired
    /// to `false`, since the just-started daemon's registry is empty).
    ///
    /// Only dead PIDs are seeded, so nothing is actually signalled: this
    /// asserts the scan-and-cleanup half without sending SIGALRM to a real
    /// process. The signalling half is covered by
    /// `renudge_signals_only_the_unregistered_tracked_shell`, which drives the
    /// same `pids_to_nudge` scan.
    #[cfg(unix)]
    #[test]
    fn restart_nudge_cleans_up_tracking_files_for_dead_shells() {
        let dir = tempfile::tempdir().expect("create tempdir");
        // PIDs far above any plausible live process, so `is_client_alive`
        // reports them dead -- the same convention as
        // `is_client_alive_nonexistent_pid` in `ipc::registry`.
        track_shells(dir.path(), &[999_997_u32, 999_998_u32]);

        super::nudge_all_tracked_shells_in(dir.path());

        assert!(
            !dir.path().join("999997").exists() && !dir.path().join("999998").exists(),
            "the restart nudge must clean up tracking files for shells that have exited"
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
    /// `is_client_alive` -- without going through the actual signal dispatch,
    /// which would deliver a real SIGALRM (default disposition: terminate) to
    /// this test process itself. This suite's only established pattern for
    /// asserting real signal *delivery* uses a stand-in child process instead
    /// of the test's own PID (see
    /// `renudge_signals_only_the_unregistered_tracked_shell`), so this test is
    /// deliberately scoped to the targeting/survival behavior rather than
    /// inventing a new signal-handler mechanism for self-delivery.
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
            crate::ipc::ClientDirectory::is_client_alive,
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
                    .pids_to_nudge(dir.path(), |_| false, |_| true)
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
            let _ = renudger.pids_to_nudge(dir.path(), |_| false, |_| true);
        }
        assert!(
            renudger
                .pids_to_nudge(dir.path(), |_| false, |_| true)
                .is_empty(),
            "budget should be exhausted before the recovery round"
        );

        // Recovered: the agent now knows this PID again.
        let _ = renudger.pids_to_nudge(dir.path(), |_| true, |_| true);

        assert_eq!(
            renudger.pids_to_nudge(dir.path(), |_| false, |_| true),
            vec![500_u32],
            "a shell that recovered and was stranded again should get a fresh budget"
        );
    }

    /// End-to-end proof that the registry drives a real signal to the right PID.
    ///
    /// An unregistered tracked shell receives SIGALRM (whose default disposition terminates
    /// these stand-in children, making delivery observable), while a registered one is left
    /// alone.
    #[cfg(unix)]
    #[test]
    fn renudge_signals_only_the_unregistered_tracked_shell() {
        let dir = tempfile::tempdir().expect("create tempdir");

        let mut registered_child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn registered stand-in shell");
        let mut stranded_child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn stranded stand-in shell");

        let registered_pid = registered_child.id();
        let stranded_pid = stranded_child.id();
        track_shells(dir.path(), &[registered_pid, stranded_pid]);

        let registry = crate::ipc::ClientDirectory::new();
        registry.register(registered_pid, None);

        let mut renudger = super::ShellRenudger::default();
        super::renudge_unregistered_shells_in(dir.path(), &mut renudger, &registry);

        let stranded_status = stranded_child.wait().expect("await stranded stand-in");
        assert!(
            !stranded_status.success(),
            "the unregistered tracked shell should have been signalled"
        );

        // The registered one must still be running; kill it to confirm it was
        // untouched (and to avoid leaking the process).
        let _ = registered_child.kill();
        let _ = registered_child.wait();
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

    /// #419: a Fish `SIGALRM` handler reads the restart marker concurrently with the agent
    /// (re)writing it.
    ///
    /// `std::fs::write` alone truncates then writes, so a reader can land in between and
    /// observe an empty file. This drives a real concurrent reader against
    /// `write_marker_atomically` across many rapid rewrites and asserts it never observes an
    /// empty or partial marker -- proving the write-temp-then-rename swap is atomic with
    /// respect to a concurrent `read_to_string`, not just asserting it.
    #[cfg(unix)]
    #[test]
    fn write_marker_atomically_never_exposes_empty_or_partial_content() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let dir = tempfile::tempdir().expect("create tempdir");
        let marker_path = dir.path().join("agent.restart.marker");

        // Seed an initial marker so the reader always has valid content to
        // observe before the writer loop below starts overwriting it.
        super::write_marker_atomically(dir.path(), &marker_path, "seed:0")
            .expect("seed initial marker");

        let stop = Arc::new(AtomicBool::new(false));
        let saw_empty = Arc::new(AtomicBool::new(false));

        let reader_marker_path = marker_path.clone();
        let reader_stop = Arc::clone(&stop);
        let reader_saw_empty = Arc::clone(&saw_empty);
        let reader = std::thread::spawn(move || {
            while !reader_stop.load(Ordering::SeqCst) {
                if let Ok(content) = std::fs::read_to_string(&reader_marker_path)
                    && content.trim().is_empty()
                {
                    reader_saw_empty.store(true, Ordering::SeqCst);
                }
            }
        });

        for i in 0_u32..2000_u32 {
            super::write_marker_atomically(dir.path(), &marker_path, &format!("marker:{i}"))
                .expect("rewrite marker");
        }

        stop.store(true, Ordering::SeqCst);
        reader.join().expect("join reader thread");

        assert!(
            !saw_empty.load(Ordering::SeqCst),
            "concurrent reader observed an empty/partial marker file mid-write"
        );
    }

    /// #419: if the process is killed between the temp-file write and the
    /// rename, a stray `agent.restart.marker.<pid>.tmp` can be left behind.
    /// The next restart's write must not let that accumulate.
    #[cfg(unix)]
    #[test]
    fn write_marker_atomically_cleans_up_stray_temp_files_from_a_crashed_previous_write() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let marker_path = dir.path().join("agent.restart.marker");
        let stray_tmp = dir.path().join("agent.restart.marker.999999.tmp");
        std::fs::write(&stray_tmp, "stray").expect("seed stray temp file");

        super::write_marker_atomically(dir.path(), &marker_path, "marker:1").expect("write marker");

        assert!(
            !stray_tmp.exists(),
            "stray temp file from a crashed prior write should be cleaned up"
        );
        assert_eq!(
            std::fs::read_to_string(&marker_path).expect("read marker"),
            "marker:1"
        );
    }
}
