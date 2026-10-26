//! Registry of shells that want live updates.
//!
//! Shells are notified only by [`DOORBELL`] (SIGURG, ignored by default, so a
//! handler-less shell survives it); any payload is a [`ShellFlag`] file written first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::debug_log;
use crate::sync_ext::MutexExt;

#[cfg(unix)]
use nix::sys::signal::{Signal, kill};
#[cfg(unix)]
use nix::unistd::Pid;

/// The only signal sent to shells; default disposition is ignore.
#[cfg(unix)]
pub(crate) const DOORBELL: Signal = Signal::SIGURG;

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
    crate::paths::runtime_root_for(
        crate::paths::root_var("XDG_RUNTIME_DIR").as_deref(),
        crate::paths::root_var("XDG_CACHE_HOME").as_deref(),
        crate::paths::home_dir().as_deref(),
    )
    .join("shells")
}

#[cfg(unix)]
/// Create `flag` for `pid` in `shell_dir`, creating the directory if needed.
///
/// # Errors
/// Returns an error if the directory or file cannot be created.
pub(crate) fn write_shell_flag(shell_dir: &Path, pid: u32, flag: ShellFlag) -> std::io::Result<()> {
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
pub(crate) fn ring_doorbell(pid: u32) -> nix::Result<()> {
    let pid_i32 = i32::try_from(pid).map_err(|_| nix::errno::Errno::ESRCH)?;
    kill(Pid::from_raw(pid_i32), DOORBELL)
}

/// Start time (epoch seconds) of the process occupying `pid`, if determinable.
#[cfg(unix)]
pub(crate) fn process_start_time_secs(sys: &mut sysinfo::System, pid: u32) -> Option<u64> {
    let sys_pid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sys_pid]), true);
    sys.process(sys_pid).map(sysinfo::Process::start_time)
}

const DEFAULT_THROTTLE_MS: u64 = 150;

/// [`ClientInfo`] fields needed after releasing the registry lock.
struct ClientSnapshot {
    pid: u32,
    #[cfg_attr(not(unix), allow(dead_code))]
    cwd: Option<PathBuf>,
    started_at: Option<u64>,
}

/// Whether a broadcast bypasses the per-repo throttle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BroadcastMode {
    Throttled,
    /// Used after a confirmed repository mutation.
    Force,
}

/// Last signal time and content token per repo; the token lets a *distinct*
/// change through within the window. `None` means time-only suppression.
#[cfg_attr(
    not(unix),
    expect(dead_code, reason = "only read by the Unix-only `should_notify_repo`")
)]
#[derive(Clone, Copy)]
struct ThrottleRecord {
    at: Instant,
    token: Option<u64>,
}

/// Shared registry of client PIDs.
pub struct ClientDirectory {
    inner: Mutex<HashMap<u32, ClientInfo>>,
    last_notify: Mutex<HashMap<PathBuf, ThrottleRecord>>,
    throttle_ms: AtomicU64,
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
    sysinfo: Mutex<sysinfo::System>,
    /// Rings one shell: [`ring_doorbell`]; tests count rings instead.
    #[cfg(unix)]
    doorbell: Doorbell,
}

/// Delivers [`DOORBELL`] to one PID.
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
    /// Start time at registration, re-checked before every signal so a recycled
    /// PID is never signaled. `None` means "nothing to compare", not a mismatch.
    started_at: Option<u64>,
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
            last_notify: Mutex::new(HashMap::new()),
            throttle_ms: AtomicU64::new(DEFAULT_THROTTLE_MS),
            shell_dir: None,
            #[cfg(unix)]
            sysinfo: Mutex::new(sysinfo::System::new()),
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

    /// Whether the recorded identity still matches. An unrecorded start time
    /// matches anything; a recorded one requires a successful, equal lookup.
    #[cfg(unix)]
    fn identity_matches(recorded: Option<u64>, current: Option<u64>) -> bool {
        recorded.is_none_or(|recorded_started_at| current == Some(recorded_started_at))
    }

    /// Whether the PID is *confirmed* to belong to another process. A failed
    /// lookup is not confirmation: keep the client, just skip signaling.
    #[cfg(unix)]
    const fn is_confirmed_recycle(recorded: Option<u64>, current: Option<u64>) -> bool {
        matches!((recorded, current), (Some(r), Some(c)) if r != c)
    }

    /// Whether `pid` is confirmed recycled since registration.
    #[cfg(unix)]
    fn is_recycled(&self, pid: u32, recorded: Option<u64>) -> bool {
        recorded.is_some() && Self::is_confirmed_recycle(recorded, self.process_start_time(pid))
    }

    #[cfg(not(unix))]
    fn is_recycled(&self, _pid: u32, _recorded: Option<u64>) -> bool {
        false
    }

    /// Whether to skip signaling `pid` this round; confirmed recycles are also
    /// pushed to `stale` for pruning, while unconfirmed identities are kept.
    #[cfg(unix)]
    fn skip_signal_for_identity(
        &self,
        recorded: Option<u64>,
        pid: u32,
        stale: &mut Vec<u32>,
    ) -> bool {
        let current = self.process_start_time(pid);
        if Self::is_confirmed_recycle(recorded, current) {
            stale.push(pid);
            return true;
        }
        !Self::identity_matches(recorded, current)
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

    /// Drop throttle entries older than an hour so the map stays bounded.
    fn prune_throttle_map(&self) {
        let expiry = Duration::from_secs(3600);
        self.last_notify
            .lock_or_recover()
            .retain(|_, record| record.at.elapsed() < expiry);
    }

    /// Prune dead clients from the registry.
    ///
    /// Removes dead or confirmed-recycled PIDs and returns them for watcher cleanup.
    #[must_use]
    pub fn prune_dead_clients(&self) -> Vec<u32> {
        let mut dead_pids = Vec::new();

        for client in self.client_snapshots() {
            if !Self::is_client_alive(client.pid) || self.is_recycled(client.pid, client.started_at)
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

        self.prune_throttle_map();

        dead_pids
    }

    /// Ring the repaint doorbell for all (or matching) clients, pruning dead PIDs.
    /// `target` and client paths must already be canonical.
    pub fn notify_repaint(&self, target: Option<&Path>) {
        self.notify_with_signal(target, BroadcastMode::Throttled, None, None);
    }

    /// Ring a throttled repaint doorbell carrying a content token.
    ///
    /// Like [`Self::notify_repaint`], but a token differing from the last one
    /// for that repo bypasses the throttle window (latest-wins).
    pub fn notify_repaint_coalesced(&self, target: Option<&Path>, token: u64) {
        self.notify_with_signal(target, BroadcastMode::Throttled, None, Some(token));
    }

    /// Ring the repaint doorbell bypassing the throttle, after a confirmed mutation.
    pub fn notify_repaint_force(&self, target: Option<&Path>) {
        self.notify_with_signal(target, BroadcastMode::Force, None, None);
    }

    /// Ask every client to reload: write `<pid>.reload`, then ring. Forced, so
    /// the flag is never left unread by a throttled-away doorbell.
    pub fn notify_reload(&self) {
        debug_log!("agent", "Notifying clients to reload via SIGURG");
        self.notify_with_signal(None, BroadcastMode::Force, Some(ShellFlag::Reload), None);
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
            })
            .collect()
    }

    fn notify_with_signal(
        &self,
        target: Option<&Path>,
        mode: BroadcastMode,
        flag: Option<ShellFlag>,
        token: Option<u64>,
    ) {
        #[cfg(not(unix))]
        {
            let _ = (target, mode, flag, token);
        }
        #[cfg(unix)]
        {
            let clients = self.client_snapshots();
            let shell_dir = flag.map(|_| self.shell_dir.clone().unwrap_or_else(default_shell_dir));

            let mut stale: Vec<u32> = Vec::new();

            // Already canonical (`PathValidator::validate_path`); no blocking canonicalize.
            let target_owned = target.map(Path::to_path_buf);

            // Decided at most once per broadcast, not per client.
            let mut should_notify: Option<bool> = None;

            for ClientSnapshot {
                pid,
                cwd,
                started_at,
            } in clients
            {
                if matches!(
                    (target_owned.as_ref(), cwd.as_ref()),
                    (Some(target_path), Some(client_path))
                        if !Self::paths_related(client_path, target_path)
                ) {
                    continue;
                }

                let allow = *should_notify.get_or_insert_with(|| {
                    self.should_broadcast(target_owned.as_deref(), mode, token)
                });
                if !allow {
                    continue;
                }

                if self.skip_signal_for_identity(started_at, pid, &mut stale) {
                    continue;
                }

                if !self.deliver(pid, flag.zip(shell_dir.as_deref())) {
                    stale.push(pid);
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

    /// Override the per-repository throttle interval used when broadcasting notifications.
    pub fn set_throttle_ms(&self, ms: u64) {
        self.throttle_ms.store(ms, Ordering::Relaxed);
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

    /// Whether the whole broadcast proceeds. Mutates `last_notify` on approval,
    /// so call at most once per broadcast or later clients get starved.
    #[cfg(unix)]
    fn should_broadcast(
        &self,
        target: Option<&Path>,
        mode: BroadcastMode,
        token: Option<u64>,
    ) -> bool {
        target.is_none_or(|repo_path| match mode {
            BroadcastMode::Force => true,
            BroadcastMode::Throttled => self.should_notify_repo(repo_path, token),
        })
    }

    /// Pure per-repo throttle decision: suppress (record unchanged) only a
    /// duplicate (same or no token) inside `window`; otherwise allow and refresh.
    #[cfg(unix)]
    fn throttle_decision(
        prev: Option<ThrottleRecord>,
        now: Instant,
        window: Duration,
        token: Option<u64>,
    ) -> (bool, ThrottleRecord) {
        let Some(record) = prev else {
            return (true, ThrottleRecord { at: now, token });
        };

        let within_window = now.saturating_duration_since(record.at) < window;
        let is_duplicate = token.is_none_or(|current| record.token == Some(current));
        if within_window && is_duplicate {
            return (false, record);
        }

        (true, ThrottleRecord { at: now, token })
    }

    /// Apply the per-repo throttle for `repo` and record the outcome.
    #[cfg(unix)]
    fn should_notify_repo(&self, repo: &Path, token: Option<u64>) -> bool {
        let throttle_ms = self.throttle_ms.load(Ordering::Relaxed);
        if throttle_ms == 0 {
            return true;
        }

        let window = Duration::from_millis(throttle_ms);
        let now = Instant::now();
        let mut guard = self.last_notify.lock_or_recover();
        let prev = guard.get(repo).copied();
        let (allow, record) = Self::throttle_decision(prev, now, window, token);
        guard.insert(repo.to_path_buf(), record);
        allow
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod doorbell_tests;
