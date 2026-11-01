//! Shell tracking: the agent's side channel to shells, outside the IPC socket.
//!
//! # Protocol
//!
//! - **Tracking file.** A shell that wants live updates creates
//!   `<shell_dir>/<pid>` (see [`default_shell_dir`]; the shells compute the
//!   same path). The file outlives the agent, so a restarted agent finds
//!   every shell that was running.
//! - **Flag file.** The agent writes `<shell_dir>/<pid>.<suffix>` (a
//!   [`ShellFlag`]) to say what it wants: `reload` re-reads the theme and
//!   config, `reregister` forgets the current registration and registers
//!   again. The shell removes the flag after acting on it.
//! - **Doorbell.** The agent then sends `SIGURG` ([`DOORBELL`]) to the PID.
//!   The flag is written first, so a shell woken by the signal always finds
//!   it. `SIGURG` is ignored by default, so a shell with no handler survives.
//! - **Announcing a start.** Once its socket accepts, a started agent
//!   ([`announce_start_in`]) raises `reregister` (and `reload` when the theme
//!   export changed) for every alive tracked shell and rings each shell once
//!   for both flags.
//! - **Re-nudging.** Every pruning tick the agent
//!   ([`renudge_unregistered_shells`]) raises `reregister` again for tracked
//!   shells that are alive but not registered, at most
//!   [`MAX_RENUDGE_ATTEMPTS`] times per PID, so a shell that can never
//!   register is not repainted forever.
//! - **Pruning.** Tracking and flag files of dead shells are removed, and so
//!   are those of a recycled PID: a process that started more than a second
//!   after its tracking file was written is not the shell that wrote it.

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
    /// Reload theme/config.
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
/// `<runtime_root>/shells`; must match the shells' `__gpy_shell_registry_dir`.
#[must_use]
pub(crate) fn default_shell_dir() -> PathBuf {
    crate::paths::runtime_dir().join("shells")
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

/// Start time (epoch seconds) of the process occupying `pid`, if determinable.
#[cfg(unix)]
pub(super) fn process_start_time_secs(sys: &mut sysinfo::System, pid: u32) -> Option<u64> {
    let sys_pid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sys_pid]), true);
    sys.process(sys_pid).map(sysinfo::Process::start_time)
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
        shell_dir: &Path,
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
    renudge_unregistered_shells_in(&default_shell_dir(), renudger, registry);
}

/// [`renudge_unregistered_shells`] against an explicit tracking directory.
#[cfg(unix)]
fn renudge_unregistered_shells_in(
    shell_dir: &Path,
    renudger: &mut ShellRenudger,
    registry: &ClientDirectory,
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
/// pruning dead ones: `write_export` warms the theme-export cache and says
/// whether the export changed (config edited while no agent ran), which
/// raises `.reload`; `.reregister` is raised always, and each shell is rung
/// once. One ring for both flags matters because Zsh drops a ring that lands
/// while its trap for the previous one runs, and a shell from an older
/// release has no backstop for that. Call once the socket accepts.
///
/// Reuses [`ShellRenudger::pids_to_nudge`] with `is_registered = false` (the
/// announcement goes to everyone) and a fresh renudger, so
/// [`MAX_RENUDGE_ATTEMPTS`] never suppresses it.
#[cfg(unix)]
pub(crate) fn announce_start_in(
    shell_dir: &Path,
    write_export: impl FnOnce() -> crate::Result<bool>,
) {
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

#[cfg(all(test, unix))]
#[path = "shell_tracking_tests.rs"]
mod tests;
