//! Registry of shells that want live updates: the clients registered over IPC
//! and the broadcast that reaches them.
//!
//! Shells are notified only by the doorbell (`SIGURG`, ignored by default, so a
//! handler-less shell survives it); any payload is a flag file written first.
//! The tracking files, flag files and doorbell are the `shell_tracking` module.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::debug_log;
use crate::sync_ext::MutexExt;

#[cfg(unix)]
use nix::sys::signal::kill;
#[cfg(unix)]
use nix::unistd::Pid;

mod shell_tracking;

pub(crate) use shell_tracking::ShellFlag;
#[cfg(unix)]
use shell_tracking::{
    ProcessTable, evict_exited_processes, process_start_time_secs, ring_doorbell, write_shell_flag,
};
#[cfg(unix)]
pub(crate) use shell_tracking::{
    ShellRenudger, announce_start_in, default_shell_dir, renudge_unregistered_shells,
};

/// How long a start-time match stays trusted without another lookup. The
/// kernel hands PIDs out in sequence, so a PID is reused only after every
/// other one has been allocated (at least 32768 on Linux, 99999 on macOS),
/// which no system does within this window: a client verified this recently
/// is still the process that registered. A plain exit is caught by the ring's
/// `ESRCH` regardless.
const IDENTITY_TRUST: Duration = Duration::from_secs(2);

/// Whether a registered PID still belongs to the process that registered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PidIdentity {
    /// Start time unrecorded or equal to the recorded one.
    Matches,
    /// Recorded, but the current lookup failed. Keep the client; skip signaling.
    Unverified,
    /// Both start times known and different: the PID was reused. Prune.
    Recycled,
}

/// [`ClientInfo`] fields needed after releasing the registry lock.
struct ClientSnapshot {
    pid: u32,
    #[cfg_attr(not(unix), allow(dead_code))]
    cwd: Option<PathBuf>,
    started_at: Option<u64>,
    verified_at: Option<Instant>,
}

/// Shared registry of client PIDs.
pub struct ClientDirectory {
    inner: Mutex<HashMap<u32, ClientInfo>>,
    /// `None` resolves `default_shell_dir` at write time.
    #[cfg_attr(
        not(unix),
        expect(
            dead_code,
            reason = "flag files accompany the Unix-only doorbell signal"
        )
    )]
    shell_dir: Option<PathBuf>,
    // Reused per sysinfo guidance; recreating `System` per call re-walks process tables.
    #[cfg(unix)]
    sysinfo: Mutex<ProcessTable>,
    /// Rings one shell: [`ring_doorbell`]; tests count rings instead.
    #[cfg(unix)]
    doorbell: Doorbell,
}

/// Delivers the doorbell to one PID.
#[cfg(unix)]
type Doorbell = Box<dyn Fn(u32) -> nix::Result<()> + Send + Sync>;

impl Default for ClientDirectory {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
struct ClientInfo {
    cwd: Option<PathBuf>,
    /// Start time at registration, re-checked before a signal so a recycled
    /// PID is never signaled. `None` means "nothing to compare", not a mismatch.
    started_at: Option<u64>,
    /// When `started_at` last matched the PID's current start time
    /// (registration counts); trusted for [`IDENTITY_TRUST`]. `None` forces a
    /// lookup. Dropped with the entry when the PID leaves the registry.
    verified_at: Option<Instant>,
}

impl ClientDirectory {
    /// Get all registered repositories and their associated client PIDs.
    #[must_use]
    pub fn get_registered_repos(&self) -> Vec<(u32, PathBuf)> {
        self.inner
            .lock_or_recover()
            .iter()
            .filter_map(|(pid, info)| info.cwd.as_ref().map(|cwd| (*pid, cwd.clone())))
            .collect()
    }

    /// Create a new, empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            shell_dir: None,
            #[cfg(unix)]
            sysinfo: Mutex::new(ProcessTable::default()),
            #[cfg(unix)]
            doorbell: Box::new(ring_doorbell),
        }
    }

    /// Create a registry writing flag files into `shell_dir`.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_shell_dir(shell_dir: PathBuf) -> Self {
        Self {
            shell_dir: Some(shell_dir),
            ..Self::new()
        }
    }

    /// Register a client PID for future updates.
    pub fn register(&self, pid: u32, cwd: Option<PathBuf>) {
        let started_at = self.process_start_time(pid);
        // Store canonical so `paths_related` doesn't rely on caller canonicalization.
        let stored_cwd = cwd.map(|p| std::fs::canonicalize(&p).unwrap_or(p));
        self.inner.lock_or_recover().insert(
            pid,
            ClientInfo {
                cwd: stored_cwd,
                started_at,
                verified_at: started_at.map(|_| Instant::now()),
            },
        );
    }

    /// Start time of the process occupying `pid`, if determinable.
    #[cfg(unix)]
    fn process_start_time(&self, pid: u32) -> Option<u64> {
        let mut sys = self.sysinfo.lock_or_recover();
        process_start_time_secs(&mut sys, pid)
    }

    #[cfg(not(unix))]
    fn process_start_time(&self, _pid: u32) -> Option<u64> {
        None
    }

    /// Compare the start time recorded at registration with the PID's current one.
    /// An unrecorded start time matches anything.
    const fn classify_identity(recorded: Option<u64>, current: Option<u64>) -> PidIdentity {
        match (recorded, current) {
            (None, _) => PidIdentity::Matches,
            (Some(_), None) => PidIdentity::Unverified,
            (Some(then), Some(now)) if then == now => PidIdentity::Matches,
            (Some(_), Some(_)) => PidIdentity::Recycled,
        }
    }

    /// Whether `client` still belongs to the process that registered it. Looks
    /// the start time up only when the last match is older than
    /// [`IDENTITY_TRUST`], and records a fresh match.
    fn pid_identity(&self, client: &ClientSnapshot) -> PidIdentity {
        let Some(recorded) = client.started_at else {
            return PidIdentity::Matches;
        };
        if client
            .verified_at
            .is_some_and(|at| at.elapsed() < IDENTITY_TRUST)
        {
            return PidIdentity::Matches;
        }
        let identity = Self::classify_identity(Some(recorded), self.process_start_time(client.pid));
        if identity == PidIdentity::Matches {
            self.mark_verified(client.pid, recorded);
        }
        identity
    }

    /// Note that `pid` still started at `started_at`, unless its entry was
    /// meanwhile replaced by a registration with another start time.
    fn mark_verified(&self, pid: u32, started_at: u64) {
        if let Some(info) = self.inner.lock_or_recover().get_mut(&pid)
            && info.started_at == Some(started_at)
        {
            info.verified_at = Some(Instant::now());
        }
    }

    /// Whether `pid` is currently in the registry.
    #[must_use]
    pub fn is_registered(&self, pid: u32) -> bool {
        self.inner.lock_or_recover().contains_key(&pid)
    }

    /// Remove a client PID from the registry.
    pub fn unregister(&self, pid: u32) {
        self.inner.lock_or_recover().remove(&pid);
    }

    /// Update the tracked workspace; returns `false` if `pid` is not registered.
    #[must_use]
    pub fn update_workspace(&self, pid: u32, cwd: &Path) -> bool {
        // Same canonical-cwd invariant as `register`.
        let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        let mut updated = false;
        self.inner.lock_or_recover().entry(pid).and_modify(|info| {
            info.cwd = Some(canonical_cwd);
            updated = true;
        });
        updated
    }

    /// Whether `pid` exists, via `kill(pid, 0)`; EPERM counts as alive.
    #[cfg(unix)]
    #[must_use]
    pub fn is_client_alive(pid: u32) -> bool {
        let Ok(pid_i32) = i32::try_from(pid) else {
            return false;
        };

        match kill(Pid::from_raw(pid_i32), None) {
            Ok(()) | Err(nix::errno::Errno::EPERM) => true,
            Err(_) => false,
        }
    }

    /// Always `true` on non-Unix: there is no way to verify, so never prune.
    #[cfg(not(unix))]
    #[must_use]
    pub fn is_client_alive(_pid: u32) -> bool {
        true
    }

    /// Prune dead clients from the registry.
    ///
    /// Removes dead or confirmed-recycled PIDs and returns them for watcher cleanup.
    #[must_use]
    pub fn prune_dead_clients(&self) -> Vec<u32> {
        let mut dead_pids = Vec::new();

        for client in self.client_snapshots() {
            if !Self::is_client_alive(client.pid)
                || self.pid_identity(&client) == PidIdentity::Recycled
            {
                dead_pids.push(client.pid);
            }
        }

        if !dead_pids.is_empty() {
            let mut guard = self.inner.lock_or_recover();
            for pid in &dead_pids {
                guard.remove(pid);
            }
            drop(guard);
            debug_log!(
                "agent",
                "Pruned {} dead clients from registry",
                dead_pids.len()
            );
        }

        #[cfg(unix)]
        self.evict_unregistered_processes();

        dead_pids
    }

    /// Drop exited processes that are no longer registered from the shared
    /// `System`: an unregistered PID is never looked up again, so nothing
    /// else would ever evict its entry.
    #[cfg(unix)]
    fn evict_unregistered_processes(&self) {
        let registered: std::collections::HashSet<u32> =
            self.inner.lock_or_recover().keys().copied().collect();
        evict_exited_processes(&mut self.sysinfo.lock_or_recover(), |pid| {
            registered.contains(&pid)
        });
    }

    /// Ring the repaint doorbell for all clients, or those whose cwd is related
    /// to `target`, pruning dead PIDs. `target` and client paths must already
    /// be canonical.
    pub fn notify_repaint(&self, target: Option<&Path>) {
        self.notify_with_signal(target, None);
    }

    /// Ask every client to reload: write `<pid>.reload`, then ring.
    pub fn notify_reload(&self) {
        debug_log!("agent", "Notifying clients to reload via SIGURG");
        self.notify_with_signal(None, Some(ShellFlag::Reload));
    }

    /// Copy clients out so the broadcast loop never holds the lock during syscalls.
    fn client_snapshots(&self) -> Vec<ClientSnapshot> {
        self.inner
            .lock_or_recover()
            .iter()
            .map(|(pid, info)| ClientSnapshot {
                pid: *pid,
                cwd: info.cwd.clone(),
                started_at: info.started_at,
                verified_at: info.verified_at,
            })
            .collect()
    }

    fn notify_with_signal(&self, target: Option<&Path>, flag: Option<ShellFlag>) {
        #[cfg(not(unix))]
        {
            let _ = (target, flag);
        }
        #[cfg(unix)]
        {
            let clients = self.client_snapshots();
            let shell_dir = flag.and_then(|_| self.shell_dir.clone().or_else(default_shell_dir));

            let mut stale: Vec<u32> = Vec::new();

            for client in clients {
                if let (Some(target_path), Some(client_path)) = (target, client.cwd.as_deref())
                    && !Self::paths_related(client_path, target_path)
                {
                    continue;
                }

                match self.pid_identity(&client) {
                    PidIdentity::Matches => {}
                    PidIdentity::Recycled => {
                        stale.push(client.pid);
                        continue;
                    }
                    PidIdentity::Unverified => continue,
                }

                if !self.deliver(client.pid, flag.zip(shell_dir.as_deref())) {
                    stale.push(client.pid);
                }
            }

            if !stale.is_empty() {
                let mut guard = self.inner.lock_or_recover();
                for pid in stale {
                    guard.remove(&pid);
                }
            }
        }
    }

    /// Write `flag` (if any), then ring; `false` means the PID is gone. The flag
    /// must exist before the doorbell, and dead PIDs get no orphan flag.
    #[cfg(unix)]
    fn deliver(&self, pid: u32, flag: Option<(ShellFlag, &Path)>) -> bool {
        if let Some((flag_kind, dir)) = flag {
            if !Self::is_client_alive(pid) {
                return false;
            }
            if let Err(e) = write_shell_flag(dir, pid, flag_kind) {
                debug_log!(
                    "agent",
                    "Failed to write {} flag for {pid}: {e}",
                    flag_kind.suffix()
                );
            }
        }

        // EPERM: the process exists but we lack permission -- keep it.
        !matches!((self.doorbell)(pid), Err(nix::errno::Errno::ESRCH))
    }

    /// Create a shared reference suitable for passing between subsystems.
    #[must_use]
    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    // Not `#[cfg(unix)]`: `has_subscriber` needs it on every platform. Paths are canonical.
    fn paths_related(client: &Path, target: &Path) -> bool {
        if client == target {
            return true;
        }

        client.starts_with(target) || target.starts_with(client)
    }

    /// Whether any client would be targeted by a signal for canonical `repo`;
    /// shares [`Self::paths_related`] so it cannot drift from delivery.
    #[must_use]
    pub fn has_subscriber(&self, repo: &Path) -> bool {
        let guard = self.inner.lock_or_recover();
        guard.values().any(|info| {
            info.cwd
                .as_deref()
                .is_none_or(|cwd| Self::paths_related(cwd, repo))
        })
    }

    /// Number of registered clients.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock_or_recover().len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod doorbell_tests;
