//! Shell tracking: the agent's side channel to shells, outside the IPC socket.
//!
//! # Protocol
//!
//! - **Tracking file.** A shell that wants live updates creates
//!   `<shell_dir>/<pid>` (see [`default_shell_dir`]; the shells compute the
//!   same path). The file outlives the agent, so a restarted agent finds
//!   every shell that was running.
//! - **Flag file.** The agent writes `<shell_dir>/<pid>.<suffix>` (a
//!   [`ShellFlag`]) to say what it wants: `reload` runs the script
//!   segments again, `reregister` forgets the current registration and
//!   registers again. The shell removes the flag after acting on it.
//! - **Doorbell.** The agent then sends `SIGURG` ([`DOORBELL`]) to the PID.
//!   The flag is written first, so a shell woken by the signal always finds
//!   it. `SIGURG` is ignored by default, so a shell with no handler survives.
//! - **Announcing a start.** Once its socket accepts, a started agent
//!   ([`announce_start_in`]) raises `reload` and `reregister` for every alive
//!   tracked shell and rings each shell once for both flags.
//! - **Re-nudging.** Every pruning tick the agent
//!   ([`renudge_unregistered_shells`]) raises `reregister` again for tracked
//!   shells that are alive but not registered, at most
//!   [`MAX_RENUDGE_ATTEMPTS`] times per PID, so a shell that can never
//!   register is not repainted forever.
//! - **Pruning.** Tracking and flag files of dead shells are removed, and so
//!   are those of a recycled PID: a process that started more than a second
//!   after its tracking file was written is not the shell that wrote it. That
//!   wall-clock comparison is made once per PID (first sight); afterwards a
//!   running agent compares the start time it recorded, which a wall-clock
//!   step cannot move (see [`ShellRenudger`]).

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::SystemTime;

#[cfg(unix)]
use super::ClientDirectory;
#[cfg(unix)]
use crate::debug_log;
#[cfg(unix)]
use nix::sys::signal::{Signal, kill};
#[cfg(unix)]
use nix::unistd::Pid;

/// The only signal sent to shells; default disposition is ignore.
#[cfg(unix)]
const DOORBELL: Signal = Signal::SIGURG;

#[cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "flag files accompany the Unix-only doorbell signal"
    )
)]
/// Per-shell message: the file `<shell_dir>/<pid>.<suffix>`, followed by a [`DOORBELL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellFlag {
    /// Run the script segments again: the config may have changed.
    Reload,
    /// Forget the current registration and register again.
    Reregister,
}

#[cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "flag files accompany the Unix-only doorbell signal"
    )
)]
impl ShellFlag {
    pub(crate) const fn suffix(self) -> &'static str {
        match self {
            Self::Reload => "reload",
            Self::Reregister => "reregister",
        }
    }

    pub(crate) fn path(self, shell_dir: &Path, pid: u32) -> PathBuf {
        shell_dir.join(format!("{pid}.{}", self.suffix()))
    }
}

#[cfg(unix)]
/// `<runtime_root>/shells`, or `None` without a runtime root: where the
/// agent leaves a shell's flags for `gpy prompt` to take (`prompt::session`).
#[must_use]
pub(crate) fn default_shell_dir() -> Option<PathBuf> {
    crate::paths::runtime_dir()
        .ok()
        .map(|root| root.join("shells"))
}

#[cfg(unix)]
/// Create `flag` for `pid` in `shell_dir`, creating the directory if needed.
///
/// # Errors
/// Returns an error if the directory or file cannot be created.
pub(super) fn write_shell_flag(shell_dir: &Path, pid: u32, flag: ShellFlag) -> std::io::Result<()> {
    std::fs::create_dir_all(shell_dir)?;
    std::fs::File::create(flag.path(shell_dir, pid)).map(drop)
}

/// Ring the doorbell for `pid`.
///
/// # Errors
/// Returns the `kill` error (`ESRCH` for a PID that does not fit `i32`).
#[cfg(unix)]
#[allow(
    clippy::map_err_ignore,
    reason = "the documented contract maps a PID that does not fit i32 to ESRCH; Errno has no source slot"
)]
pub(super) fn ring_doorbell(pid: u32) -> nix::Result<()> {
    let pid_i32 = i32::try_from(pid).map_err(|_| nix::errno::Errno::ESRCH)?;
    kill(Pid::from_raw(pid_i32), DOORBELL)
}

/// A `sysinfo::System` made on first use. Creating one can fail (sysinfo
/// 0.40), and then it stays unmade and is tried again on the next use: every
/// start time is unknown meanwhile, which callers treat as uncertain.
#[cfg(unix)]
#[derive(Default)]
pub(super) struct ProcessTable(Option<sysinfo::System>);

#[cfg(unix)]
impl ProcessTable {
    fn system(&mut self) -> Option<&mut sysinfo::System> {
        if self.0.is_none() {
            self.0 = sysinfo::System::new().ok();
        }
        self.0.as_mut()
    }

    /// Whether `pid` has an entry, made or kept by a refresh.
    #[cfg(test)]
    pub(super) fn holds(&self, pid: u32) -> bool {
        self.0
            .as_ref()
            .is_some_and(|sys| sys.process(sysinfo::Pid::from_u32(pid)).is_some())
    }
}

/// Start time (epoch seconds) of the process occupying `pid`, if determinable.
/// Refreshes only that PID's base entry: no memory, CPU, disk, exe or task
/// reads, which a plain `refresh_processes` adds.
#[cfg(unix)]
pub(super) fn process_start_time_secs(table: &mut ProcessTable, pid: u32) -> Option<u64> {
    let sys = table.system()?;
    let sys_pid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[sys_pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    sys.process(sys_pid).map(sysinfo::Process::start_time)
}

/// Drop from `table` every process `keep` rejects once it has exited. A
/// long-lived `System` otherwise holds an entry (and on Linux an open
/// `/proc/<pid>/stat`) for every PID it was ever asked about.
#[cfg(unix)]
pub(super) fn evict_exited_processes(table: &mut ProcessTable, keep: impl Fn(u32) -> bool) {
    // Nothing was ever recorded in a table not yet made.
    let Some(sys) = table.0.as_mut() else {
        return;
    };
    let candidates: Vec<sysinfo::Pid> = sys
        .processes()
        .keys()
        .filter(|pid| !keep(pid.as_u32()))
        .copied()
        .collect();
    if candidates.is_empty() {
        return;
    }
    // Refreshing with `remove_dead_processes` removes exactly the dead ones.
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&candidates),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
}

/// Max re-nudges per tracked shell PID per agent lifetime, so a shell that can
/// never register is not repainted every minute forever. One or two rounds
/// normally clear a shell's 60s circuit-breaker backoff.
#[cfg(unix)]
const MAX_RENUDGE_ATTEMPTS: u32 = 5;

/// Bounded per-PID re-nudge attempts, and the identity of every tracked shell
/// verified so far.
#[cfg(unix)]
#[derive(Default)]
pub(crate) struct ShellRenudger {
    attempts: std::collections::HashMap<u32, u32>,
    identities: TrackedShellIdentities,
}

/// Start times of tracked shells already verified against their tracking
/// file, read through one long-lived `System`.
///
/// On Linux sysinfo derives a start time from the boot time it read when the
/// `System` was created, and that boot time moves with the wall clock. Two
/// reads through one `System` therefore agree across a clock step, while a
/// fresh read compared with a tracking file's mtime makes every older shell
/// look recycled after a forward step.
#[cfg(unix)]
#[derive(Default)]
struct TrackedShellIdentities {
    sys: ProcessTable,
    verified: std::collections::HashMap<u32, u64>,
    /// A table made during the current scan for first-sight checks, so their
    /// start times are in the wall-clock scale of now; reset by
    /// [`Self::end_scan`].
    fresh: ProcessTable,
}

#[cfg(unix)]
impl TrackedShellIdentities {
    /// Whether `pid` is alive and still the shell that wrote its tracking file
    /// (`tracked_mtime`; `None` for flag files, which only need liveness).
    /// A PID seen before must keep its recorded start time; a new one gets the
    /// mtime check of [`tracked_shell_liveness`]. Uncertain probes count as
    /// alive.
    fn is_same_shell(&mut self, pid: u32, tracked_mtime: Option<SystemTime>) -> bool {
        if !ClientDirectory::is_client_alive(pid) {
            self.verified.remove(&pid);
            return false;
        }
        let Some(written) = tracked_mtime else {
            return true;
        };
        let Some(started) = process_start_time_secs(&mut self.sys, pid) else {
            return true;
        };
        if let Some(&seen) = self.verified.get(&pid) {
            if seen == started {
                return true;
            }
            self.verified.remove(&pid);
            return false;
        }
        let same = process_start_time_secs(&mut self.fresh, pid)
            .is_none_or(|now_scale| !started_after_tracking(now_scale, written));
        if same {
            self.verified.insert(pid, started);
        }
        same
    }

    /// Forget every PID that no longer has a tracking file, and the scan's
    /// fresh table.
    fn end_scan(&mut self, tracked: &std::collections::HashSet<u32>) {
        self.fresh = ProcessTable::default();
        self.verified.retain(|pid, _| tracked.contains(pid));
        evict_exited_processes(&mut self.sys, |pid| tracked.contains(&pid));
    }
}

/// One scan of the tracking directory.
#[cfg(unix)]
struct NudgeScan {
    /// Alive, unregistered PIDs within their re-nudge budget.
    targets: Vec<u32>,
    /// Every PID whose tracking file survived the scan.
    tracked: std::collections::HashSet<u32>,
}

#[cfg(unix)]
impl ShellRenudger {
    /// Tracked PIDs that are alive, unregistered, and within budget. Removes
    /// tracking files and flags of dead PIDs (including orphan flags), and
    /// resets the budget of registered ones. `is_alive` gets the tracking
    /// file's mtime (`None` for flags) to reject recycled PIDs.
    pub(crate) fn pids_to_nudge<R, A>(
        &mut self,
        shell_dir: &Path,
        is_registered: R,
        is_alive: A,
    ) -> Vec<u32>
    where
        R: Fn(u32) -> bool,
        A: FnMut(u32, Option<SystemTime>) -> bool,
    {
        scan_tracked_shells(&mut self.attempts, shell_dir, is_registered, is_alive).targets
    }
}

/// [`ShellRenudger::pids_to_nudge`] over the attempt budget alone, so the
/// caller can lend the liveness probe the renudger's identities.
#[cfg(unix)]
fn scan_tracked_shells<R, A>(
    attempts: &mut std::collections::HashMap<u32, u32>,
    shell_dir: &Path,
    is_registered: R,
    mut is_alive: A,
) -> NudgeScan
where
    R: Fn(u32) -> bool,
    A: FnMut(u32, Option<SystemTime>) -> bool,
{
    let mut targets = Vec::new();
    let mut tracked = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir(shell_dir) else {
        return NudgeScan { targets, tracked };
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if let Some((stem, suffix)) = name.split_once('.') {
            // Flag files are only cleaned up when their shell is gone.
            let is_flag =
                suffix == ShellFlag::Reload.suffix() || suffix == ShellFlag::Reregister.suffix();
            if is_flag
                && let Ok(flag_pid) = stem.parse::<u32>()
                && crate::ipc::ClientPid::pid_in_range(flag_pid)
                && !is_alive(flag_pid, None)
            {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        // `kill` reads PID 0 as the caller's own process group, so probing or
        // ringing it would signal the agent's group: only plausible PIDs count.
        let Some(pid) = name
            .parse::<u32>()
            .ok()
            .filter(|&pid| crate::ipc::ClientPid::pid_in_range(pid))
        else {
            continue;
        };

        let modified = entry.metadata().and_then(|m| m.modified()).ok();
        if !is_alive(pid, modified) {
            let _ = std::fs::remove_file(&path);
            for flag in [ShellFlag::Reload, ShellFlag::Reregister] {
                let _ = std::fs::remove_file(shell_dir.join(format!("{pid}.{}", flag.suffix())));
            }
            continue;
        }
        tracked.insert(pid);

        if is_registered(pid) {
            attempts.remove(&pid);
            continue;
        }

        let count = attempts.entry(pid).or_insert(0_u32);
        if *count >= MAX_RENUDGE_ATTEMPTS {
            continue;
        }
        *count = count.saturating_add(1_u32);
        targets.push(pid);
    }

    // Keep the map bounded over a long-lived agent.
    attempts.retain(|pid, _| tracked.contains(pid));

    NudgeScan { targets, tracked }
}

/// Production liveness for tracked shells: alive and not started more than 1 s
/// after its tracking file was written (else the PID was recycled). Uncertain
/// probes count as alive so a live shell is never pruned.
#[cfg(unix)]
fn tracked_shell_liveness() -> impl Fn(u32, Option<SystemTime>) -> bool {
    let sys = std::cell::RefCell::new(ProcessTable::default());
    move |pid, tracked_mtime| {
        if !ClientDirectory::is_client_alive(pid) {
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
    registry: &ClientDirectory,
) {
    if let Some(shell_dir) = default_shell_dir() {
        renudge_unregistered_shells_in(&shell_dir, renudger, registry);
    }
}

/// [`renudge_unregistered_shells`] against an explicit tracking directory.
/// Unlike [`announce_start_in`], liveness goes through the renudger's
/// identities, so a shell verified on an earlier tick is never pruned by a
/// wall-clock step.
#[cfg(unix)]
fn renudge_unregistered_shells_in(
    shell_dir: &Path,
    renudger: &mut ShellRenudger,
    registry: &ClientDirectory,
) {
    let ShellRenudger {
        attempts,
        identities,
    } = renudger;
    let scan = scan_tracked_shells(
        attempts,
        shell_dir,
        |pid| registry.is_registered(pid),
        |pid, tracked_mtime| identities.is_same_shell(pid, tracked_mtime),
    );
    identities.end_scan(&scan.tracked);

    for pid in scan.targets {
        debug_log!("agent", "Re-nudging unregistered tracked shell {pid}");
        raise_shell_flag(shell_dir, pid, ShellFlag::Reregister);
    }
}

/// Write `flag` for `pid`, then ring the doorbell; the flag must exist first.
#[cfg(unix)]
fn raise_shell_flag(shell_dir: &Path, pid: u32, flag: ShellFlag) {
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

/// Announce a started agent to every alive tracked shell in `shell_dir`,
/// pruning dead ones: `.reload` and `.reregister` are raised for each, and
/// each shell is rung once. `.reload` makes the next prompt run the script
/// segments again and reset their circuit breakers: the config may have
/// changed while no agent ran. One ring for both flags matters because Zsh
/// drops a ring that lands while its trap for the previous one runs. Call
/// once the socket accepts.
///
/// Reuses [`ShellRenudger::pids_to_nudge`] with `is_registered = false` (the
/// announcement goes to everyone) and a fresh renudger, so
/// [`MAX_RENUDGE_ATTEMPTS`] never suppresses it.
#[cfg(unix)]
pub(crate) fn announce_start_in(shell_dir: &Path) {
    let mut renudger = ShellRenudger::default();
    let pids = renudger.pids_to_nudge(shell_dir, |_| false, tracked_shell_liveness());

    for pid in pids {
        debug_log!("agent", "Announcing agent start to tracked shell {pid}");
        if let Err(e) = write_shell_flag(shell_dir, pid, ShellFlag::Reload) {
            debug_log!("agent", "Failed to write reload flag for {pid}: {e}");
        }
        // Writes `.reregister`, then rings once for both flags.
        raise_shell_flag(shell_dir, pid, ShellFlag::Reregister);
    }
}

#[cfg(all(test, unix))]
#[path = "shell_tracking_tests.rs"]
mod tests;
