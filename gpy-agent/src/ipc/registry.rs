//! Client registry for tracking shell processes interested in live updates.
//!
//! Every shell notification is a single "doorbell" signal, [`DOORBELL`]
//! (SIGURG), whose default disposition is *ignore*. A shell that has not (yet)
//! installed a handler -- e.g. one that `exec`ed itself and so kept its
//! registered PID -- is therefore never killed by a notification. Anything
//! beyond "repaint" travels in per-shell flag files under the shell tracking
//! directory (see [`ShellFlag`]), written *before* the doorbell rings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::debug_log;

#[cfg(unix)]
use nix::sys::signal::{Signal, kill};
#[cfg(unix)]
use nix::unistd::Pid;

/// The only signal the agent ever sends to a shell. Its default disposition
/// is ignore, so a process without a handler survives it.
#[cfg(unix)]
pub(crate) const DOORBELL: Signal = Signal::SIGURG;

#[cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "flag files accompany the Unix-only doorbell signal"
    )
)]
/// A message for one shell, carried by the existence of
/// `<shell_dir>/<pid>.<suffix>` and delivered by ringing [`DOORBELL`].
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
    /// File-name suffix of this flag (`<pid>.<suffix>`).
    pub(crate) const fn suffix(self) -> &'static str {
        match self {
            Self::Reload => "reload",
            Self::Reregister => "reregister",
        }
    }

    /// Path of this flag for `pid` inside `shell_dir`.
    pub(crate) fn path(self, shell_dir: &Path, pid: u32) -> PathBuf {
        shell_dir.join(format!("{pid}.{}", self.suffix()))
    }
}

#[cfg(unix)]
/// The shell tracking directory, `<runtime_root>/shells`, shared with the
/// shells' `__gpy_shell_registry_dir`.
#[must_use]
pub(crate) fn default_shell_dir() -> PathBuf {
    crate::paths::runtime_root_for(
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        std::env::var("XDG_CACHE_HOME").ok().as_deref(),
        crate::paths::home_dir().as_deref(),
    )
    .join("shells")
}

#[cfg(unix)]
/// Create `flag` for `pid` in `shell_dir` (creating the directory if needed).
///
/// # Errors
///
/// Returns an error if the directory or the flag file cannot be created.
pub(crate) fn write_shell_flag(shell_dir: &Path, pid: u32, flag: ShellFlag) -> std::io::Result<()> {
    std::fs::create_dir_all(shell_dir)?;
    std::fs::File::create(flag.path(shell_dir, pid)).map(drop)
}

/// Ring the doorbell for `pid`.
///
/// # Errors
///
/// Returns the `kill` error (`ESRCH` also for a PID that does not fit `i32`).
#[cfg(unix)]
pub(crate) fn ring_doorbell(pid: u32) -> nix::Result<()> {
    let pid_i32 = i32::try_from(pid).map_err(|_| nix::errno::Errno::ESRCH)?;
    kill(Pid::from_raw(pid_i32), DOORBELL)
}

/// Default throttle interval in milliseconds between signals to the same client
const DEFAULT_THROTTLE_MS: u64 = 150;

/// Snapshot of a registered client used while broadcasting a signal, mirroring
/// the fields of [`ClientInfo`] that `notify_with_signal` needs after releasing
/// the registry lock.
#[cfg(unix)]
struct ClientSnapshot {
    /// The client shell's process id.
    pid: u32,
    /// The client's working directory, when it reported one.
    cwd: Option<PathBuf>,
    /// Process start time, used to detect a recycled PID (#319/#432).
    started_at: Option<u64>,
}

/// Whether a broadcast bypasses the per-repo throttle.
///
/// Replaces a bare `force: bool` threaded through `notify_with_signal` and
/// `should_broadcast`, where the boolean said nothing about what it forced
/// (#600).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BroadcastMode {
    /// Respect the normal throttle/coalescing window.
    Throttled,
    /// Bypass throttle controls — used after a confirmed repository mutation.
    Force,
}

/// Per-repo throttle bookkeeping: when the last signal for that repo was delivered, plus the
/// content token it carried (#438).
///
/// The token lets the throttle coalesce genuine duplicates while still delivering a *distinct*
/// change within the window (latest-wins). A `None` token means the caller supplied no
/// content, so the throttle falls back to purely time-based suppression.
#[cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "`token` is compared only by `should_notify_repo`, which is Unix-only because the throttle exists to coalesce repaint pushes (#540)"
    )
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
    /// Shell tracking directory flag files are written to; `None` resolves
    /// `default_shell_dir` at write time.
    #[cfg_attr(
        not(unix),
        expect(
            dead_code,
            reason = "flag files accompany the Unix-only doorbell signal"
        )
    )]
    shell_dir: Option<PathBuf>,
    // Reused across identity lookups per sysinfo's own guidance: recreating
    // `System` per call is wasteful, and on some platforms re-walks process
    // tables that a targeted refresh does not need to touch.
    #[cfg(unix)]
    sysinfo: Mutex<sysinfo::System>,
    #[cfg(test)]
    notify_events: AtomicU64,
}

impl Default for ClientDirectory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "`started_at` is recorded on every platform but only read by the Unix PID-recycle check, which guards signal delivery (#319, #540)"
    )
)]
#[derive(Clone, Debug)]
struct ClientInfo {
    cwd: Option<PathBuf>,
    /// Process start time (seconds since the epoch), captured at
    /// registration. Re-checked against the current occupant of this PID
    /// before every signal: if the OS has recycled the PID to an unrelated
    /// process since the registered shell exited, the start time will
    /// differ and the signal must be skipped, so an unrelated process is
    /// never sent a doorbell nor has flag files written for it (#319).
    /// `None` means the start time couldn't be determined at
    /// registration (e.g. a transient lookup failure) and is treated as
    /// "nothing to compare", not as a mismatch.
    started_at: Option<u64>,
}

impl ClientDirectory {
    /// Get all registered repositories and their associated client PIDs.
    #[must_use]
    pub fn get_registered_repos(&self) -> Vec<(u32, PathBuf)> {
        self.inner.lock().map_or_else(
            |_| Vec::new(),
            |guard| {
                guard
                    .iter()
                    .filter_map(|(pid, info)| info.cwd.as_ref().map(|cwd| (*pid, cwd.clone())))
                    .collect()
            },
        )
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
            #[cfg(test)]
            notify_events: AtomicU64::new(0),
        }
    }

    /// Create a new, empty registry that writes shell flag files into
    /// `shell_dir` instead of `<runtime_root>/shells` (tests).
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
        // Boundary invariant: store the cwd canonical so `paths_related`'s
        // correctness no longer silently depends on a far-away
        // `PathValidator::validate_path` canonicalization in the caller
        // (defense-in-depth for #432, not a live-bug fix). Done best-effort at
        // register time -- off the broadcast hot loop -- and a redundant no-op
        // for the already-canonical production path.
        let stored_cwd = cwd.map(|p| std::fs::canonicalize(&p).unwrap_or(p));
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(
                pid,
                ClientInfo {
                    cwd: stored_cwd,
                    started_at,
                },
            );
        }
    }

    /// Look up the current start time (seconds since the epoch) of the
    /// process occupying `pid`, or `None` if it can't be determined
    /// (process gone, or a transient lookup failure).
    #[cfg(unix)]
    fn process_start_time(&self, pid: u32) -> Option<u64> {
        let sys_pid = sysinfo::Pid::from_u32(pid);
        let mut sys = self.sysinfo.lock().ok()?;
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sys_pid]), true);
        sys.process(sys_pid).map(sysinfo::Process::start_time)
    }

    #[cfg(not(unix))]
    fn process_start_time(&self, _pid: u32) -> Option<u64> {
        None
    }

    /// Whether a PID's registration-time identity still matches its current
    /// occupant, given the recorded and freshly-looked-up start times.
    ///
    /// `recorded` is `None` when the start time couldn't be determined at
    /// registration (e.g. a transient lookup failure); that is deliberately
    /// treated as "nothing to compare" rather than a mismatch, so it never
    /// silently disables notifications to a legitimate, still-running
    /// client. Once a start time was recorded, though, it must match the
    /// PID's *current* occupant exactly -- including the case where the
    /// current lookup itself fails (`current: None`), which correctly
    /// refuses to signal a PID we can no longer positively identify (#319).
    #[cfg(unix)]
    fn identity_matches(recorded: Option<u64>, current: Option<u64>) -> bool {
        recorded.is_none_or(|recorded_started_at| current == Some(recorded_started_at))
    }

    /// Whether a PID's registered identity is *confirmed* to belong to a
    /// different process now -- the only case that justifies pruning it (#319).
    /// A transient lookup failure (`current` is `None`) is NOT a confirmed
    /// recycle: the PID may still be the original, live client, so it must be
    /// kept (and simply not signaled this round) rather than pruned (#432).
    #[cfg(unix)]
    const fn is_confirmed_recycle(recorded: Option<u64>, current: Option<u64>) -> bool {
        matches!((recorded, current), (Some(r), Some(c)) if r != c)
    }

    /// Whether `pid` is confirmed to belong to a different process than the one
    /// registered with `recorded` as its start time.
    #[cfg(unix)]
    fn is_recycled(&self, pid: u32, recorded: Option<u64>) -> bool {
        recorded.is_some() && Self::is_confirmed_recycle(recorded, self.process_start_time(pid))
    }

    /// Native Windows has no start-time lookup, so nothing is ever recycled.
    #[cfg(not(unix))]
    fn is_recycled(&self, _pid: u32, _recorded: Option<u64>) -> bool {
        false
    }

    /// Decide whether `pid` must be skipped this broadcast round based on its
    /// registered identity, collecting a *confirmed* PID recycle into `stale`
    /// for pruning. Returns `true` when the caller must not signal this PID.
    ///
    /// One `process_start_time` lookup drives a three-way decision (#319/#432):
    /// - confirmed recycle: push to `stale`, return `true` (prune and never
    ///   signal -- a different process now holds this PID, #319).
    /// - unconfirmed identity (transient `process_start_time` failure on a
    ///   still-registered PID): return `true` but do NOT push to `stale` --
    ///   keep the client and merely skip signaling this round (#432); it'll be
    ///   signaled on the next broadcast once the lookup recovers.
    /// - positively identified: return `false` (safe to signal).
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
    ///
    /// Used by the agent's shell re-nudge to tell a healthy shell from one
    /// stranded by a re-registration that failed during a restart nudge.
    #[must_use]
    pub fn is_registered(&self, pid: u32) -> bool {
        self.inner
            .lock()
            .is_ok_and(|guard| guard.contains_key(&pid))
    }

    /// Remove a client PID from the registry.
    pub fn unregister(&self, pid: u32) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.remove(&pid);
        }
    }

    /// Update the tracked workspace for a registered client.
    ///
    /// Returns `true` if the PID was registered and updated, `false` if not registered.
    #[must_use]
    pub fn update_workspace(&self, pid: u32, cwd: &Path) -> bool {
        // Same register-time canonical-cwd boundary invariant as `register`
        // (defense-in-depth for #432): store canonical, off the hot loop, so
        // `paths_related` compares canonical-to-canonical regardless of caller.
        let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        if let Ok(mut guard) = self.inner.lock() {
            let mut updated = false;
            guard.entry(pid).and_modify(|info| {
                info.cwd = Some(canonical_cwd);
                updated = true;
            });
            return updated;
        }
        false
    }

    /// Check if a client PID is still alive using `kill(pid, 0)`.
    ///
    /// Returns `true` if the process exists (even if we lack permission to signal it),
    /// `false` if the process doesn't exist or PID is invalid.
    #[cfg(unix)]
    #[must_use]
    pub fn is_client_alive(pid: u32) -> bool {
        let Ok(pid_i32) = i32::try_from(pid) else {
            // PID too large for i32
            return false;
        };

        // Use kill(pid, 0) to check if process exists without sending a signal
        match kill(Pid::from_raw(pid_i32), None) {
            Ok(()) | Err(nix::errno::Errno::EPERM) => {
                // Process exists (either we can signal it or we lack permission)
                true
            }
            Err(_) => {
                // Process doesn't exist (ESRCH) or other error
                false
            }
        }
    }

    /// Whether a registered client PID is still alive (native Windows stub).
    ///
    /// Native Windows has no signal-based live-update path (#284), so there is
    /// nothing to keep a liveness check honest for; assume the client is alive
    /// rather than pruning registrations the agent cannot verify.
    #[cfg(not(unix))]
    #[must_use]
    pub fn is_client_alive(_pid: u32) -> bool {
        // On non-Unix platforms, assume client is alive
        // (Windows support would require platform-specific implementation)
        true
    }

    /// Prune stale entries from the throttle map to prevent memory leaks.
    fn prune_throttle_map(&self) {
        if let Ok(mut guard) = self.last_notify.lock() {
            // Prune entries older than 1 hour
            // This prevents the map from growing unbounded over long uptime
            let expiry = Duration::from_secs(3600);
            guard.retain(|_, record| record.at.elapsed() < expiry);
        }
    }

    /// Prune dead clients from the registry.
    ///
    /// This method checks all registered PIDs and removes those that no longer
    /// exist, or whose PID now belongs to a different process (a confirmed
    /// recycle, judged like the broadcast path does: a failed start-time
    /// lookup is not a recycle, #432).
    /// Returns a vector of PIDs that were pruned (so caller can clean up watchers, etc.).
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
            if let Ok(mut guard) = self.inner.lock() {
                for pid in &dead_pids {
                    guard.remove(pid);
                }
            }
            debug_log!(
                "agent",
                "Pruned {} dead clients from registry",
                dead_pids.len()
            );
        }

        // Also prune the throttle map to prevent memory leaks
        self.prune_throttle_map();

        dead_pids
    }

    /// Ring the repaint doorbell for all (or matching) registered clients.
    /// Automatically prunes stale PIDs that no longer exist.
    ///
    /// Note: Assumes `target` and client paths are already canonical (from `PathValidator::validate_path`).
    /// This eliminates blocking filesystem calls on the async path.
    pub fn notify_repaint(&self, target: Option<&Path>) {
        self.notify_with_signal(target, BroadcastMode::Throttled, None, None);
    }

    /// Ring a throttled repaint doorbell carrying a content token (#438).
    ///
    /// Same per-repo throttle as [`Self::notify_repaint`], but a token that differs
    /// from the last delivered one for that repo is delivered even inside the
    /// throttle window (latest-wins), so a genuinely distinct rapid push is not
    /// swallowed. Derive the token from the pushed content (see
    /// `content_token`).
    pub fn notify_repaint_coalesced(&self, target: Option<&Path>, token: u64) {
        self.notify_with_signal(target, BroadcastMode::Throttled, None, Some(token));
    }

    /// Ring the repaint doorbell bypassing throttle controls.
    ///
    /// Intended for immediate updates after a confirmed repository mutation.
    pub fn notify_repaint_force(&self, target: Option<&Path>) {
        self.notify_with_signal(target, BroadcastMode::Force, None, None);
    }

    /// Ask every registered client to reload theme/config: write its
    /// `<pid>.reload` flag, then ring the doorbell.
    ///
    /// Always [`BroadcastMode::Force`]: a throttled-away doorbell would leave
    /// the flag sitting unread.
    pub fn notify_reload(&self) {
        debug_log!("agent", "Notifying clients to reload via SIGURG");
        self.notify_with_signal(None, BroadcastMode::Force, Some(ShellFlag::Reload), None);
    }

    /// Copy the registry's live clients out from under the lock, so the
    /// broadcast loop below never holds it while issuing syscalls.
    #[cfg(unix)]
    fn client_snapshots(&self) -> Vec<ClientSnapshot> {
        self.inner.lock().map_or_else(
            |_| Vec::new(),
            |guard| {
                guard
                    .iter()
                    .map(|(pid, info)| ClientSnapshot {
                        pid: *pid,
                        cwd: info.cwd.clone(),
                        started_at: info.started_at,
                    })
                    .collect()
            },
        )
    }

    fn notify_with_signal(
        &self,
        target: Option<&Path>,
        mode: BroadcastMode,
        flag: Option<ShellFlag>,
        token: Option<u64>,
    ) {
        #[cfg(test)]
        self.notify_events.fetch_add(1, Ordering::Relaxed);
        #[cfg(not(unix))]
        {
            let _ = (target, mode, flag, token);
        }
        #[cfg(unix)]
        {
            let clients = self.client_snapshots();
            let shell_dir = flag.map(|_| self.shell_dir.clone().unwrap_or_else(default_shell_dir));

            let mut stale: Vec<u32> = Vec::new();

            // Target paths come from PathValidator::validate_path which already canonicalizes
            // No blocking fs::canonicalize needed here
            let target_owned = target.map(Path::to_path_buf);

            // Decided at most once per broadcast, not per client (#565).
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

                // Skip PIDs we can't positively identify; prune only a
                // confirmed recycle (#319/#432). Same single lookup as before.
                if self.skip_signal_for_identity(started_at, pid, &mut stale) {
                    continue;
                }

                if !Self::deliver(pid, flag.zip(shell_dir.as_deref())) {
                    stale.push(pid);
                }
            }

            if !stale.is_empty()
                && let Ok(mut guard) = self.inner.lock()
            {
                for pid in stale {
                    guard.remove(&pid);
                }
            }
        }
    }

    /// Deliver one notification to `pid`: write `flag` (if any) into its shell
    /// dir, then ring the doorbell. Returns `false` when the PID is gone and
    /// must be pruned.
    ///
    /// The flag must exist before the doorbell rings, or the shell's handler
    /// would find nothing to act on. A dead PID gets no flag, so no orphan is
    /// left behind.
    #[cfg(unix)]
    fn deliver(pid: u32, flag: Option<(ShellFlag, &Path)>) -> bool {
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
        !matches!(ring_doorbell(pid), Err(nix::errno::Errno::ESRCH))
    }

    /// Override the per-repository throttle interval used when broadcasting notifications.
    pub fn set_throttle_ms(&self, ms: u64) {
        self.throttle_ms.store(ms, Ordering::Relaxed);
    }

    #[cfg(test)]
    #[must_use]
    /// Test helper: total number of notification attempts recorded.
    pub fn notify_invocations(&self) -> u64 {
        self.notify_events.load(Ordering::Relaxed)
    }

    /// Create a shared reference suitable for passing between subsystems.
    #[must_use]
    pub fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    // Not `#[cfg(unix)]`: `notify_with_signal`'s own use of this stays inside
    // its `#[cfg(unix)]` block below (signal delivery is unix-only), but
    // `has_subscriber` needs the same targeting rule on every platform, since
    // it answers "is a client subscribed", not "can we signal it".
    fn paths_related(client: &Path, target: &Path) -> bool {
        // Both client and target paths are already canonical (from PathValidator::validate_path)
        // No need to canonicalize again - avoids blocking filesystem calls
        if client == target {
            return true;
        }

        client.starts_with(target) || target.starts_with(client)
    }

    /// Whether any registered client would be a target of a signal aimed at
    /// `repo` -- i.e. whether refreshing `repo` would reach anybody.
    ///
    /// Reuses [`Self::paths_related`], the exact per-client targeting rule
    /// [`Self::notify_with_signal`] applies (plus its "a client with `cwd:
    /// None` subscribes to every target" rule), rather than reimplementing
    /// it, so a caller gating work on "would this reach a client" (#480) can
    /// never drift from what notification actually delivers. `repo` is
    /// expected to already be canonical, matching every other caller of
    /// `paths_related`.
    ///
    /// Fails OPEN (returns `true`) when the registry lock is poisoned. This
    /// predicate exists only to skip a git capture round nobody would see the
    /// result of; the failure it guards against (wasted CPU) is strictly
    /// cheaper than the failure a false negative here would risk -- silently
    /// dropping a refresh a real client needed, leaving it stuck on a stale
    /// prompt until the next event happens to fire.
    #[must_use]
    pub fn has_subscriber(&self, repo: &Path) -> bool {
        let Ok(guard) = self.inner.lock() else {
            return true;
        };
        guard.values().any(|info| {
            info.cwd
                .as_deref()
                .is_none_or(|cwd| Self::paths_related(cwd, repo))
        })
    }

    /// Get number of registered clients (for debugging/testing)
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |guard| guard.len())
    }

    /// Check if registry is empty (for debugging/testing)
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Helper used by unit tests to inspect current clients.
    #[cfg(test)]
    #[must_use]
    pub fn clients(&self) -> Vec<u32> {
        self.inner
            .lock()
            .map_or_else(|_| Vec::new(), |guard| guard.keys().copied().collect())
    }

    #[cfg(unix)]
    /// Test-only helper: read back the stored cwd for a PID, so tests can
    /// assert the register-time canonical-cwd boundary invariant (#432).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn client_cwd(&self, pid: u32) -> Option<PathBuf> {
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.get(&pid).and_then(|info| info.cwd.clone()))
    }

    #[cfg(unix)]
    /// Test-only helper: register a PID with an explicit (possibly
    /// fabricated) `started_at`, bypassing the real registration-time
    /// lookup. Lets tests exercise the PID-recycling defense in
    /// `notify_with_signal` end-to-end against a real, live PID without
    /// depending on `process_start_time`'s real value for that PID.
    #[cfg(test)]
    pub(crate) fn register_with_started_at_for_test(
        &self,
        pid: u32,
        cwd: Option<PathBuf>,
        started_at: Option<u64>,
    ) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(pid, ClientInfo { cwd, started_at });
        }
    }

    /// Whether a broadcast aimed at `target` should proceed for the *whole*
    /// broadcast -- not per client.
    ///
    /// An untargeted broadcast (`target: None`, e.g. `notify_reload`) or a
    /// forced one always proceeds. Otherwise this defers to
    /// `should_notify_repo`, which applies the per-repo throttle and mutates
    /// `last_notify` on approval. Because of that mutation, callers MUST call
    /// this at most once per broadcast and reuse the result across every
    /// client -- calling it per client lets the first approved client consume
    /// the one-shot approval and starves every later client in the same repo
    /// of its throttled signal (#565).
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

    /// Pure decision core for the per-repo throttle (#565): given the previous
    /// record for a repo (if any), the current time, the throttle window, and
    /// the push's content token, decide whether to allow the signal and what
    /// record to store afterward.
    ///
    /// - No prior record: always allow, stamping a fresh record at `now`.
    /// - Prior record present, inside `window`, and the push is a *duplicate*
    ///   (same `Some` token, or no token to compare -- time-based fallback):
    ///   suppress, leaving the record UNCHANGED.
    /// - Otherwise (outside the window, or a *distinct* `Some` token inside
    ///   it): allow and refresh the record to `now`/`token` (latest-wins,
    ///   #438).
    ///
    /// Callers decide this once per broadcast and store the result themselves
    /// (see `should_notify_repo`); this function does not touch the map.
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
        // Suppress only a *duplicate* within the window: either the same
        // content token, or a tokenless push (time-based fallback). A
        // distinct `Some` token always delivers.
        let is_duplicate = token.is_none_or(|current| record.token == Some(current));
        if within_window && is_duplicate {
            return (false, record);
        }

        (true, ThrottleRecord { at: now, token })
    }

    /// Decide whether a signal for `repo` should be delivered now, applying the
    /// per-repo throttle.
    ///
    /// The throttle coalesces a burst of *identical* pushes for the same repo,
    /// but must never swallow a genuinely different change that lands inside the
    /// window (#438). `token` is a content fingerprint of the push (see
    /// `content_token`); within the window a matching token is suppressed as a
    /// duplicate, while a *distinct* token is delivered (latest-wins) and
    /// becomes the new baseline. A `None` token carries no content to compare,
    /// so it preserves the original purely time-based suppression.
    #[cfg(unix)]
    fn should_notify_repo(&self, repo: &Path, token: Option<u64>) -> bool {
        let throttle_ms = self.throttle_ms.load(Ordering::Relaxed);
        if throttle_ms == 0 {
            return true;
        }

        let window = Duration::from_millis(throttle_ms);
        let now = Instant::now();
        let Ok(mut guard) = self.last_notify.lock() else {
            return true;
        };
        let prev = guard.get(repo).copied();
        let (allow, record) = Self::throttle_decision(prev, now, window, token);
        guard.insert(repo.to_path_buf(), record);
        allow
    }

    #[cfg(all(test, unix))]
    pub(crate) fn throttle_allows(&self, repo: &Path) -> bool {
        self.should_notify_repo(repo, None)
    }

    #[cfg(all(test, unix))]
    pub(crate) fn throttle_allows_token(&self, repo: &Path, token: u64) -> bool {
        self.should_notify_repo(repo, Some(token))
    }
}

#[cfg(test)]
mod tests {
    use super::ClientDirectory;
    #[cfg(unix)]
    use super::ThrottleRecord;
    use std::path::Path;
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::Duration;
    #[cfg(unix)]
    use std::time::Instant;

    #[cfg(unix)]
    #[test]
    /// # Panics
    ///
    /// Panics if throttle logic malfunctions.
    fn throttle_suppresses_within_window() {
        let directory = ClientDirectory::new();
        directory.set_throttle_ms(50);
        let repo = Path::new("/repo");

        assert!(directory.throttle_allows(repo));
        assert!(!directory.throttle_allows(repo));

        thread::sleep(Duration::from_millis(55));
        assert!(directory.throttle_allows(repo));
    }

    #[cfg(unix)]
    #[test]
    /// A genuinely distinct content token within the throttle window must still
    /// be delivered (latest-wins, #438); only a *duplicate* token is coalesced.
    ///
    /// # Panics
    ///
    /// Panics if the throttle drops a distinct push within the window.
    fn throttle_delivers_distinct_token_within_window() {
        let directory = ClientDirectory::new();
        directory.set_throttle_ms(50);
        let repo = Path::new("/repo");

        // First push for token 1 is delivered.
        assert!(directory.throttle_allows_token(repo, 1));
        // A duplicate of token 1 within the window is coalesced (suppressed).
        assert!(!directory.throttle_allows_token(repo, 1));
        // A distinct token 2 within the same window must still be delivered.
        assert!(directory.throttle_allows_token(repo, 2));
        // ...and once delivered, its own duplicate is coalesced again.
        assert!(!directory.throttle_allows_token(repo, 2));
    }

    // --- #565: `throttle_decision` pure-core coverage ---

    #[cfg(unix)]
    #[test]
    /// No prior record: always allow, stamping a fresh record at `now`.
    ///
    /// # Panics
    ///
    /// Panics if a first-ever push is not allowed, or the stamped record is wrong.
    fn throttle_decision_no_prior_record_allows_and_stamps() {
        let now = Instant::now();
        let window = Duration::from_millis(50);

        let (allow, record) = ClientDirectory::throttle_decision(None, now, window, Some(7));

        assert!(allow, "a first push with no prior record must be allowed");
        assert_eq!(record.at, now);
        assert_eq!(record.token, Some(7));
    }

    #[cfg(unix)]
    #[test]
    /// Inside the window with a matching token: suppress, and leave the record
    /// UNCHANGED (not bumped to `now`).
    ///
    /// # Panics
    ///
    /// Panics if a same-token duplicate inside the window is delivered, or the
    /// record is refreshed despite being suppressed.
    fn throttle_decision_inside_window_same_token_suppresses_and_keeps_record() {
        let recorded_at = Instant::now();
        let prev = ThrottleRecord {
            at: recorded_at,
            token: Some(3),
        };
        let now = recorded_at + Duration::from_millis(10);
        let window = Duration::from_millis(50);

        let (allow, record) = ClientDirectory::throttle_decision(Some(prev), now, window, Some(3));

        assert!(
            !allow,
            "a same-token duplicate inside the window must be suppressed"
        );
        assert_eq!(
            record.at, recorded_at,
            "a suppressed decision must not refresh the record's timestamp"
        );
        assert_eq!(record.token, Some(3));
    }

    #[cfg(unix)]
    #[test]
    /// Inside the window with a distinct `Some` token: allow (latest-wins, #438)
    /// and refresh the record to `now`/the new token.
    ///
    /// # Panics
    ///
    /// Panics if a distinct token inside the window is suppressed, or the
    /// refreshed record is wrong.
    fn throttle_decision_inside_window_distinct_token_allows_and_refreshes() {
        let recorded_at = Instant::now();
        let prev = ThrottleRecord {
            at: recorded_at,
            token: Some(1),
        };
        let now = recorded_at + Duration::from_millis(10);
        let window = Duration::from_millis(50);

        let (allow, record) = ClientDirectory::throttle_decision(Some(prev), now, window, Some(2));

        assert!(
            allow,
            "a distinct token inside the window must still be delivered"
        );
        assert_eq!(
            record.at, now,
            "an allowed decision must refresh the record's timestamp"
        );
        assert_eq!(record.token, Some(2));
    }

    #[cfg(unix)]
    #[test]
    /// A tokenless push (`token: None`) inside the window has no content to
    /// compare, so it falls back to purely time-based suppression.
    ///
    /// # Panics
    ///
    /// Panics if a tokenless push inside the window is delivered.
    fn throttle_decision_tokenless_inside_window_suppresses() {
        let recorded_at = Instant::now();
        let prev = ThrottleRecord {
            at: recorded_at,
            token: Some(1),
        };
        let now = recorded_at + Duration::from_millis(10);
        let window = Duration::from_millis(50);

        let (allow, record) = ClientDirectory::throttle_decision(Some(prev), now, window, None);

        assert!(
            !allow,
            "a tokenless push inside the window must be suppressed"
        );
        assert_eq!(record.at, recorded_at);
        assert_eq!(record.token, Some(1));
    }

    #[cfg(unix)]
    #[test]
    /// Outside the window: allow regardless of token, and refresh the record.
    ///
    /// # Panics
    ///
    /// Panics if a push outside the window is suppressed, or the refreshed
    /// record is wrong.
    fn throttle_decision_outside_window_allows_and_refreshes() {
        let recorded_at = Instant::now();
        let prev = ThrottleRecord {
            at: recorded_at,
            token: Some(1),
        };
        let now = recorded_at + Duration::from_millis(60);
        let window = Duration::from_millis(50);

        let (allow, record) = ClientDirectory::throttle_decision(Some(prev), now, window, Some(1));

        assert!(
            allow,
            "a push outside the window must be delivered even with the same token"
        );
        assert_eq!(
            record.at, now,
            "an allowed decision must refresh the record's timestamp"
        );
        assert_eq!(record.token, Some(1));
    }

    // --- #565: broadcast throttle must be decided once per broadcast, not per client ---

    #[cfg(unix)]
    #[test]
    /// Regression test for #565: two clients registered in the same repo must
    /// BOTH receive a throttled signal in one broadcast.
    ///
    /// Before the fix, `should_notify_repo` was called (and mutated
    /// `last_notify`) inside the per-client loop, so whichever client the
    /// (nondeterministic) `HashMap` iteration order visited first consumed the
    /// one-shot throttle approval; the second client then saw
    /// `within_window && is_duplicate == true` and was skipped via `continue`
    /// *before* ever reaching `kill()` -- so it was never marked stale and
    /// stayed registered (`len() == 1`, not `0`).
    ///
    /// Uses two guaranteed-nonexistent PIDs with no recorded `started_at` (same
    /// pattern as `notify_does_not_block_signal_when_identity_unknown`) so both
    /// bypass the identity check and reach `kill()`, get `ESRCH`, and get
    /// pruned -- proving both were actually signaled in this one broadcast.
    ///
    /// # Panics
    ///
    /// Panics if both clients were not signaled (i.e. not both pruned) in one
    /// broadcast.
    fn notify_delivers_to_both_clients_in_same_repo_within_one_broadcast() {
        let directory = ClientDirectory::new();
        let repo = PathBuf::from("/repo");

        directory.register_with_started_at_for_test(999_990, Some(repo.clone()), None);
        directory.register_with_started_at_for_test(999_991, Some(repo.clone()), None);
        assert_eq!(directory.len(), 2, "Should have 2 clients registered");

        directory.notify_repaint(Some(&repo));

        assert_eq!(
            directory.len(),
            0,
            "both clients in the same repo must be signaled (and pruned via ESRCH) \
             within one broadcast, not just the first one HashMap iteration visits"
        );
    }

    #[test]
    /// Test that `update_workspace` returns false when PID is not registered
    ///
    /// This is critical for auto-reconnection: when the agent restarts,
    /// Fish shells need to detect that their workspace update failed
    /// so they can re-register.
    ///
    /// # Panics
    ///
    /// Panics if workspace update behavior is incorrect.
    fn update_workspace_fails_when_not_registered() {
        let directory = ClientDirectory::new();
        let unregistered_pid = 99999_u32;
        let cwd = Path::new("/test/path");

        // Attempting to update workspace for unregistered PID should return false
        let result = directory.update_workspace(unregistered_pid, cwd);
        assert!(
            !result,
            "update_workspace should return false for unregistered PID"
        );
    }

    #[test]
    /// Test that `update_workspace` returns true when PID is registered
    ///
    /// # Panics
    ///
    /// Panics if workspace update behavior is incorrect.
    fn update_workspace_succeeds_when_registered() {
        let directory = ClientDirectory::new();
        let pid = 12345_u32;
        let initial_cwd = Path::new("/initial/path");
        let new_cwd = Path::new("/new/path");

        // Register the PID
        directory.register(pid, Some(initial_cwd.to_path_buf()));

        // Updating workspace for registered PID should return true
        let result = directory.update_workspace(pid, new_cwd);
        assert!(
            result,
            "update_workspace should return true for registered PID"
        );
    }

    #[test]
    /// Test that workspace update after unregister fails
    ///
    /// This simulates the agent restart scenario where registration is lost.
    ///
    /// # Panics
    ///
    /// Panics if unregister or workspace update behavior is incorrect.
    fn update_workspace_fails_after_unregister() {
        let directory = ClientDirectory::new();
        let pid = 12345_u32;
        let cwd = Path::new("/test/path");

        // Register and then unregister
        directory.register(pid, Some(cwd.to_path_buf()));
        directory.unregister(pid);

        // Workspace update should now fail
        let result = directory.update_workspace(pid, cwd);
        assert!(
            !result,
            "update_workspace should return false after unregister"
        );
    }

    #[test]
    /// The agent's shell re-nudge (`ShellRenudger`) needs to ask "does the
    /// registry still know this PID?" to tell a healthy shell from one stranded
    /// by a failed post-restart re-registration.
    ///
    /// # Panics
    ///
    /// Panics if registration membership is not reported accurately.
    fn is_registered_tracks_registration_membership() {
        let directory = ClientDirectory::new();
        let pid = 4242_u32;

        assert!(
            !directory.is_registered(pid),
            "an unknown PID must not report as registered"
        );

        directory.register(pid, Some(Path::new("/test/path").to_path_buf()));
        assert!(
            directory.is_registered(pid),
            "a registered PID must report as registered"
        );

        directory.unregister(pid);
        assert!(
            !directory.is_registered(pid),
            "an unregistered PID must not report as registered"
        );
    }

    #[test]
    #[cfg(unix)]
    /// Test that `is_client_alive` returns true for the current process
    ///
    /// # Panics
    ///
    /// Panics if liveness check fails for current process.
    fn is_client_alive_current_process() {
        let pid = std::process::id();
        assert!(
            ClientDirectory::is_client_alive(pid),
            "Current process should be detected as alive"
        );
    }

    #[test]
    #[cfg(unix)]
    /// Test that `is_client_alive` returns false for a PID that doesn't exist
    ///
    /// # Panics
    ///
    /// Panics if liveness check incorrectly reports non-existent PID as alive.
    fn is_client_alive_nonexistent_pid() {
        // Use a very high PID that's unlikely to exist
        let fake_pid = 999_999_u32;
        assert!(
            !ClientDirectory::is_client_alive(fake_pid),
            "Non-existent PID should be detected as dead"
        );
    }

    #[test]
    #[cfg(unix)]
    /// Test that `prune_dead_clients` removes only dead clients
    ///
    /// # Panics
    ///
    /// Panics if pruning doesn't correctly remove dead clients.
    fn prune_dead_clients_removes_dead() {
        let directory = ClientDirectory::new();
        let current_pid = std::process::id();
        let fake_pid = 999_999_u32;

        // Register both alive and dead PIDs
        directory.register(current_pid, Some(Path::new("/test1").to_path_buf()));
        directory.register(fake_pid, Some(Path::new("/test2").to_path_buf()));

        assert_eq!(directory.len(), 2, "Should have 2 clients registered");

        // Prune dead clients
        let pruned_pids = directory.prune_dead_clients();
        assert_eq!(pruned_pids.len(), 1, "Should have pruned 1 dead client");
        assert!(
            pruned_pids.contains(&fake_pid),
            "Pruned list should contain fake PID"
        );
        assert_eq!(directory.len(), 1, "Should have 1 client remaining");

        // Verify the alive client is still registered
        let clients = directory.clients();
        assert!(
            clients.contains(&current_pid),
            "Current process should still be registered"
        );
        assert!(
            !clients.contains(&fake_pid),
            "Fake PID should have been removed"
        );
    }

    #[test]
    #[cfg(unix)]
    /// Test that `prune_dead_clients` returns empty vec when all clients are alive
    ///
    /// # Panics
    ///
    /// Panics if pruning incorrectly removes alive clients.
    fn prune_dead_clients_no_pruning_needed() {
        let directory = ClientDirectory::new();
        let current_pid = std::process::id();

        // Register only alive PID
        directory.register(current_pid, Some(Path::new("/test").to_path_buf()));

        assert_eq!(directory.len(), 1, "Should have 1 client registered");

        // Prune dead clients
        let pruned_pids = directory.prune_dead_clients();
        assert_eq!(pruned_pids.len(), 0, "Should have pruned 0 clients");
        assert!(pruned_pids.is_empty(), "Pruned list should be empty");
        assert_eq!(directory.len(), 1, "Should still have 1 client");
    }

    #[test]
    /// Test that `prune_dead_clients` handles empty registry
    ///
    /// # Panics
    ///
    /// Panics if pruning fails on empty registry.
    fn prune_dead_clients_empty_registry() {
        let directory = ClientDirectory::new();

        assert_eq!(directory.len(), 0, "Should start with 0 clients");

        // Prune when empty
        let pruned_pids = directory.prune_dead_clients();
        assert_eq!(pruned_pids.len(), 0, "Should have pruned 0 clients");
        assert!(pruned_pids.is_empty(), "Pruned list should be empty");
        assert_eq!(directory.len(), 0, "Should still have 0 clients");
    }

    // --- #319: PID-recycling defense ---

    #[cfg(unix)]
    #[test]
    /// A registration where the start time couldn't be determined must not
    /// block future signals.
    ///
    /// Otherwise a transient lookup failure would silently and permanently
    /// disable notifications to a legitimate client.
    ///
    /// # Panics
    ///
    /// Panics if an unknown recorded start time is treated as a mismatch.
    fn identity_matches_true_when_recorded_is_none() {
        assert!(ClientDirectory::identity_matches(None, Some(123)));
        assert!(ClientDirectory::identity_matches(None, None));
    }

    #[cfg(unix)]
    #[test]
    /// # Panics
    ///
    /// Panics if matching start times are treated as a mismatch.
    fn identity_matches_true_when_start_times_match() {
        assert!(ClientDirectory::identity_matches(Some(100), Some(100)));
    }

    #[cfg(unix)]
    #[test]
    /// The core PID-recycling case: the registered PID now belongs to a
    /// process with a different start time, so it must not match.
    ///
    /// # Panics
    ///
    /// Panics if differing start times are treated as a match.
    fn identity_matches_false_when_start_times_differ() {
        assert!(!ClientDirectory::identity_matches(Some(100), Some(200)));
    }

    #[cfg(unix)]
    #[test]
    /// Once a start time was recorded, a lookup that now fails (process
    /// gone, or otherwise unidentifiable) must not fall back to "assume
    /// it's fine" -- that would defeat the whole check.
    ///
    /// # Panics
    ///
    /// Panics if an unresolvable current lookup is treated as a match.
    fn identity_matches_false_when_current_lookup_fails() {
        assert!(!ClientDirectory::identity_matches(Some(100), None));
    }

    #[test]
    #[cfg(unix)]
    /// End-to-end regression test for #319.
    ///
    /// A registered PID that is very much alive (the test process itself) but
    /// whose recorded start time no longer matches its real one -- simulating
    /// the PID having been recycled to a different process since registration
    /// -- must be treated as stale and pruned, not signaled.
    ///
    /// This never risks sending a real signal to the test process: the
    /// identity check runs and `continue`s before `notify_with_signal`
    /// reaches its `kill()` call.
    ///
    /// # Panics
    ///
    /// Panics if the mismatched-identity PID is not pruned.
    fn notify_prunes_pid_whose_identity_no_longer_matches() {
        let directory = ClientDirectory::new();
        let current_pid = std::process::id();

        // A real, live PID, but tagged with a start time that cannot be its
        // real one (process start times are seconds since the epoch; no
        // process created in 1970 is still running).
        directory.register_with_started_at_for_test(current_pid, None, Some(1));
        assert_eq!(directory.len(), 1, "Should have 1 client registered");

        directory.notify_repaint(None);

        assert!(
            !directory.clients().contains(&current_pid),
            "PID with mismatched identity must be pruned, not signaled"
        );
    }

    #[test]
    #[cfg(unix)]
    /// A PID with no recorded start time must not be blocked by the identity
    /// check.
    ///
    /// It falls through to the existing liveness handling unchanged. Uses a
    /// PID that doesn't exist so this never risks signaling a real process: the
    /// existing ESRCH-based staleness pruning removes it, proving the new
    /// identity check didn't get in the way of that path.
    ///
    /// # Panics
    ///
    /// Panics if a PID with no recorded identity is not pruned via the
    /// normal liveness path.
    fn notify_does_not_block_signal_when_identity_unknown() {
        let directory = ClientDirectory::new();
        let fake_pid = 999_999_u32;

        directory.register_with_started_at_for_test(fake_pid, None, None);
        assert_eq!(directory.len(), 1, "Should have 1 client registered");

        directory.notify_repaint(None);

        assert!(
            !directory.clients().contains(&fake_pid),
            "Nonexistent PID should still be pruned via normal ESRCH handling"
        );
    }

    #[test]
    #[cfg(unix)]
    /// The timer's `prune_dead_clients` must prune a live PID whose recorded
    /// start time proves the PID now belongs to a different process (#782).
    ///
    /// # Panics
    ///
    /// Panics if the recycled PID is not returned and removed.
    fn prune_dead_clients_prunes_confirmed_recycle() {
        let directory = ClientDirectory::new();
        let pid = std::process::id();

        directory.register_with_started_at_for_test(pid, None, Some(1));

        assert_eq!(directory.prune_dead_clients(), vec![pid]);
        assert_eq!(directory.len(), 0);
    }

    #[test]
    #[cfg(unix)]
    /// A live PID with no recorded start time is never pruned by identity.
    ///
    /// # Panics
    ///
    /// Panics if the unidentified live PID is pruned.
    fn prune_dead_clients_keeps_live_pid_without_recorded_start_time() {
        let directory = ClientDirectory::new();
        let pid = std::process::id();

        directory.register_with_started_at_for_test(pid, None, None);

        assert_eq!(directory.prune_dead_clients(), Vec::<u32>::new());
        assert_eq!(directory.len(), 1);
    }

    // --- #432: false-positive PID-identity prune (keep + skip-signal) ---

    #[cfg(unix)]
    #[test]
    /// `is_confirmed_recycle` is true only when both start times are known and
    /// they differ -- the only case that justifies pruning (#319).
    ///
    /// A transient current-lookup failure (`current` is `None`) is the crux of
    /// #432: it is NOT a confirmed recycle, so the PID must be kept (and merely
    /// not signaled this round), not pruned. Forcing real sysinfo to return
    /// `None` for a live PID isn't feasible, so this pure-function coverage
    /// plus the three-way branch in `notify_with_signal` IS the deterministic
    /// proof that a transient failure won't prune a live client.
    ///
    /// # Panics
    ///
    /// Panics if any confirmed/unconfirmed classification is wrong.
    fn is_confirmed_recycle_only_when_both_known_and_differ() {
        assert!(!ClientDirectory::is_confirmed_recycle(None, None));
        assert!(!ClientDirectory::is_confirmed_recycle(None, Some(5)));
        // The crux: transient current-lookup failure on a still-registered PID.
        assert!(!ClientDirectory::is_confirmed_recycle(Some(5), None));
        assert!(!ClientDirectory::is_confirmed_recycle(Some(5), Some(5)));
        // Confirmed recycle: a different process now holds this PID.
        assert!(ClientDirectory::is_confirmed_recycle(Some(5), Some(9)));
    }

    // --- #432: register-time canonical-cwd boundary invariant ---

    #[test]
    #[cfg(unix)]
    /// A cwd handed to `register` through a symlink must be stored canonical, so
    /// `paths_related` (which assumes canonical inputs) matches a canonical notify target for
    /// the real directory.
    ///
    /// Locks the register-time boundary invariant added for #432.
    ///
    /// # Panics
    ///
    /// Panics if the stored cwd is not canonicalized at registration.
    fn register_canonicalizes_symlinked_cwd() {
        use std::fs;

        let base = std::env::temp_dir().join(format!("gpy_reg_canon_{}", std::process::id()));
        let real_dir = base.join("real");
        let link = base.join("link");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&real_dir).expect("create real dir");
        std::os::unix::fs::symlink(&real_dir, &link).expect("create symlink");

        let canonical = fs::canonicalize(&real_dir).expect("canonicalize real dir");

        let directory = ClientDirectory::new();
        let pid = 4242_u32;
        directory.register(pid, Some(link));

        let stored = directory.client_cwd(pid).expect("cwd should be stored");
        assert_eq!(
            stored, canonical,
            "register must store the canonical cwd, not the raw symlink path"
        );
        assert!(
            ClientDirectory::paths_related(&stored, &canonical),
            "a canonical notify target for the real dir must match the stored cwd"
        );

        let _ = fs::remove_dir_all(&base);
    }
}

#[cfg(all(test, unix))]
mod doorbell_tests {
    #![allow(clippy::expect_used, clippy::missing_panics_doc)]

    use super::{ClientDirectory, ShellFlag};
    use std::path::Path;
    use std::process::{Child, Command};
    use std::time::Duration;

    /// Wait up to ~5s for `path` to exist.
    fn wait_for(path: &Path) -> bool {
        for _ in 0_u32..250_u32 {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        path.exists()
    }

    fn kill_child(mut child: Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    /// #674 regression: a registered process with NO handler (a shell that
    /// just `exec`ed itself) must survive every agent notification.
    #[test]
    fn unhandled_registered_process_survives_repaint_and_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();

        let registry = ClientDirectory::with_shell_dir(dir.path().to_path_buf());
        registry.register(pid, None);
        registry.notify_repaint_force(None);
        registry.notify_repaint(None);
        registry.notify_reload();
        std::thread::sleep(Duration::from_millis(200));

        assert!(
            child.try_wait().expect("poll child").is_none(),
            "an unhandled process must survive the doorbell"
        );
        assert!(registry.is_registered(pid));
        kill_child(child);
    }

    /// `notify_reload` writes `<pid>.reload` before ringing the doorbell: the
    /// stand-in's URG trap only records delivery if the flag already exists.
    #[test]
    fn notify_reload_writes_flag_before_signalling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mark = dir.path().join("delivered");
        let flag_glob = dir.path().join("$$.reload");
        let script = format!(
            "trap '[ -e \"{flag}\" ] && touch \"{mark}\"' URG; while :; do sleep 0.05; done",
            flag = flag_glob.display(),
            mark = mark.display(),
        );
        let child = Command::new("sh")
            .arg("-c")
            .arg(script)
            .spawn()
            .expect("spawn sh");
        let pid = child.id();
        // Let sh install its trap before the doorbell rings.
        std::thread::sleep(Duration::from_millis(200));

        let registry = ClientDirectory::with_shell_dir(dir.path().to_path_buf());
        registry.register(pid, None);
        registry.notify_reload();

        assert!(
            ShellFlag::Reload.path(dir.path(), pid).exists(),
            "reload flag must be written"
        );
        assert!(
            wait_for(&mark),
            "doorbell must arrive with the reload flag already present"
        );
        kill_child(child);
    }
}
