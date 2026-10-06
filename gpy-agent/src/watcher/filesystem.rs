//! File system watching implementation using notify crate
//!
//! Integrates with the notify crate to provide efficient file system
//! monitoring with proper event filtering and error handling.

use super::watch_set::{OsOp, WatchMode, WatchSet};
use super::{
    DelayedEventScheduler, PendingCallback, PendingEvent, WatchRegistry,
    multi_repo::MultiRepoWatcher, should_trigger_update,
};
use crate::cache::bounded::IGNORE_CACHE_CAPACITY;
use crate::git::native::NativeGitBackend;
use crate::{Error, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::event::{CreateKind, EventKind, Flag, MetadataKind, ModifyKind, RemoveKind};
use notify::{Event, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// How long the one-shot health probe waits for the native backend to deliver
/// an event for its own sentinel file before declaring it silent (#442).
///
/// Two seconds is a deliberate compromise: long enough that a merely slow
/// `FSEvents`/inotify delivery is not mistaken for a dead one (the observed
/// healthy latency is well under 250ms), short enough that a genuinely silent
/// backend is replaced roughly 20x sooner than the ~45s reconcile
/// (`RECONCILE_INTERVAL_SECS`) would have papered over it.
const DEFAULT_WATCH_PROBE_MS: u64 = 2_000;

/// Interval used by the `notify::PollWatcher` fallback once it is armed
/// (#442).
///
/// `PollWatcher` recursively stat-walks every watched root on each tick, so
/// this is the one knob that decides how expensive a degraded machine is.
/// Two seconds is chosen because:
///
/// - the debouncer already coalesces bursts (default 100ms window), so
///   sub-second polling adds tree walks without adding user-visible
///   responsiveness;
/// - a repo with a large untracked `target/`/`node_modules` tree makes each
///   walk cost real I/O, and this only ever runs on machines whose native
///   backend is already broken; and
/// - 2s is still ~20x better than the ~45s reconcile that was previously the
///   only recovery.
///
/// Override with `GPY_WATCH_POLL_MS` (milliseconds).
const DEFAULT_WATCH_POLL_MS: u64 = 2_000;

/// How long each probe iteration blocks waiting for an event before re-touching the sentinel
/// file.
///
/// Re-touching (rather than writing once and waiting for the full timeout) guards against a
/// write that lands before the watch is fully armed, which would otherwise look identical to a
/// dead backend.
const PROBE_TOUCH_INTERVAL: Duration = Duration::from_millis(250);

/// Shared read-only context for a single `process_event` call, bundled so per-path helpers
/// (e.g.
///
/// `FileSystemWatcher::emit_classified_event`) stay under clippy's `too_many_arguments` limit
/// without duplicating the same three values as separate parameters (#443).
struct WorktreeEventContext<'a> {
    event: &'a Event,
    registry: &'a Arc<WatchRegistry>,
    worktree_enabled: bool,
}

/// Which notify backend is currently armed behind a [`FileSystemWatcher`] (#442).
///
/// Reported in the debug log on construction and on every swap so a degraded machine is
/// diagnosable from `GPY_DEBUG` output alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BackendKind {
    /// `notify::RecommendedWatcher` — the OS-native backend (`FSEvents`,
    /// inotify, `ReadDirectoryChangesW`, kqueue).
    Native,
    /// `notify::PollWatcher` — periodic recursive stat walk, used only when
    /// the native backend proved silent.
    Poll,
}

impl BackendKind {
    /// Stable label for debug-log lines.
    const fn label(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Poll => "poll",
        }
    }

    /// Whether removing one watch on this backend can also remove watches on
    /// other, overlapping paths (#687). See [`WatchSet`].
    const fn removal_cascades(self) -> bool {
        match self {
            Self::Native => NATIVE_REMOVAL_CASCADES,
            Self::Poll => false,
        }
    }
}

/// Whether the platform's native `notify` backend shares OS watches between
/// overlapping registrations, so that removing one path also removes watches
/// another registration depends on (#687).
///
/// inotify keeps one descriptor per directory, shared by every registration
/// whose walk reached it, and `notify` removes every descriptor under a
/// recursive entry; kqueue likewise walks and removes the whole subtree.
/// `FSEvents` (macOS, built with `macos_fsevent`) and `ReadDirectoryChangesW`
/// (Windows) keep one independent entry per watched path.
const NATIVE_REMOVAL_CASCADES: bool = !cfg!(any(target_os = "macos", target_os = "windows"));

/// One logical watch registration: a `(path, mode)` pair as passed to
/// `watch`/`watch_shallow`, held by `owners` callers (#687).
#[derive(Debug)]
struct Registration {
    /// How many `watch` calls for this pair have not yet been matched by an
    /// `unwatch`.
    owners: u32,
    /// The OS-level `(path, mode)` pairs this registration holds in
    /// [`WatcherBackend::os`]: the path itself on the native backend, the
    /// `.gitignore`-pruned directory expansion of a recursive directory watch
    /// on the poll backend (#463, see [`FileSystemWatcher::poll_watch_targets`]).
    /// A poll expansion follows the tree as directories are created and
    /// removed under it (#721), so this is a set: ordered, so a removed
    /// directory's subtree is one contiguous range.
    targets: BTreeSet<(PathBuf, WatchMode)>,
}

/// The armed notify backend plus the set of paths it must be watching.
///
/// These two live behind ONE mutex on purpose (#442): swapping the backend
/// has to re-arm every path that was registered against the old one, and a
/// concurrent `MultiRepoWatcher::register_client` calling `watch()` must
/// either be fully before the swap (so the swap re-arms its path) or fully
/// after it (so it lands on the new backend). Splitting them into two locks
/// would open exactly that window.
///
/// Lock discipline: this is a LEAF lock. Nothing taken while it is held ever
/// reaches back into `MultiRepoWatcher`'s `ops_lock` / `watched_repos` /
/// `watcher` locks, so it cannot participate in a cycle with them. The event
/// thread takes it only to keep the poll expansion in step with the tree
/// (#721): never while walking it, never while invoking a callback.
struct WatcherBackend {
    /// `None` once [`FileSystemWatcher::stop`] has torn the backend down.
    watcher: Option<Box<dyn Watcher + Send>>,
    /// Every logical registration, keyed by `(path, mode)`. The single source
    /// of truth for what callers asked to watch (#687): `covers`, unwatch
    /// bookkeeping and the backend-swap re-arm all read it. A shallow and a
    /// recursive registration of one path are separate keys, each with its own
    /// owner count.
    registrations: BTreeMap<(PathBuf, WatchMode), Registration>,
    /// Reference-counted OS-level watches, shared by every registration whose
    /// targets overlap (the poll expansions of nested roots, a common
    /// directory armed shallow by a worktree and recursively by its main
    /// checkout). Only this set talks to `watcher`, so one owner's unwatch
    /// never removes a watch another owner still holds.
    os: WatchSet,
    kind: BackendKind,
}

impl WatcherBackend {
    /// A freshly armed backend of `kind` with no paths registered yet.
    fn armed(watcher: Box<dyn Watcher + Send>, kind: BackendKind) -> Self {
        Self {
            watcher: Some(watcher),
            registrations: BTreeMap::new(),
            os: WatchSet::new(kind.removal_cascades()),
            kind,
        }
    }

    /// The OS-level targets a `(path, mode)` registration arms on a `kind`
    /// backend. `poll_targets` is the precomputed directory expansion for a
    /// recursive directory watch on the poll backend; without one the path
    /// itself is armed.
    fn targets_for(
        kind: BackendKind,
        path: &Path,
        mode: WatchMode,
        poll_targets: Option<Vec<PathBuf>>,
    ) -> BTreeSet<(PathBuf, WatchMode)> {
        match (kind, mode, poll_targets) {
            // The descent already happened in `poll_watch_targets`, so each
            // directory is armed shallow: `PollWatcher` never walks deeper than
            // one level per registered directory (#463).
            (BackendKind::Poll, WatchMode::Recursive, Some(targets)) => targets
                .into_iter()
                .map(|target| (target, WatchMode::Shallow))
                .collect(),
            _ => BTreeSet::from([(path.to_path_buf(), mode)]),
        }
    }

    /// Add one owner of `(path, mode)`, arming whatever OS watches that newly
    /// requires. A no-op once the backend has been stopped.
    ///
    /// # Errors
    ///
    /// Returns an error, with every OS change rolled back, if the backend
    /// rejects one of this registration's own targets.
    fn acquire(
        &mut self,
        path: &Path,
        mode: WatchMode,
        poll_targets: Option<Vec<PathBuf>>,
    ) -> Result<()> {
        if self.watcher.is_none() {
            return Ok(());
        }
        let key = (path.to_path_buf(), mode);
        if let Some(registration) = self.registrations.get_mut(&key) {
            registration.owners = registration.owners.saturating_add(1);
            return Ok(());
        }

        let targets = Self::targets_for(self.kind, path, mode, poll_targets);
        let mut ops = Vec::new();
        for (target, target_mode) in &targets {
            ops.extend(self.os.acquire(target, *target_mode));
        }
        let failures = self.apply(ops);
        let own_failure = failures
            .into_iter()
            .find(|(failed, _)| targets.iter().any(|(target, _)| target == failed));
        if let Some((_, error)) = own_failure {
            let mut rollback = Vec::new();
            for (target, target_mode) in &targets {
                rollback.extend(self.os.release(target, *target_mode).unwrap_or_default());
            }
            self.apply(rollback);
            return Err(Error::watcher(format!("Failed to watch path: {error}")));
        }

        self.registrations
            .insert(key, Registration { owners: 1, targets });
        Ok(())
    }

    /// Drop one owner of `(path, mode)`. OS watches are removed only once no
    /// registration targets them any more; everything another owner still
    /// holds stays armed. A no-op once the backend has been stopped.
    ///
    /// # Errors
    ///
    /// Returns an error if no owner holds `(path, mode)`.
    fn release(&mut self, path: &Path, mode: WatchMode) -> Result<()> {
        if self.watcher.is_none() {
            return Ok(());
        }
        let key = (path.to_path_buf(), mode);
        let Some(registration) = self.registrations.get_mut(&key) else {
            return Err(Error::watcher(format!(
                "Failed to unwatch path: {} is not watched ({mode:?})",
                path.display()
            )));
        };
        if registration.owners > 1 {
            registration.owners = registration.owners.saturating_sub(1);
            return Ok(());
        }
        let Some(released) = self.registrations.remove(&key) else {
            return Ok(());
        };
        let mut ops = Vec::new();
        for (target, target_mode) in &released.targets {
            ops.extend(self.os.release(target, *target_mode).unwrap_or_default());
        }
        self.apply(ops);
        Ok(())
    }

    /// Apply `ops` to the backend as one batch -- one `FSEventStream` rebuild
    /// on macOS however many paths change (#388) -- returning every path whose
    /// add failed. Those are also marked unarmed in [`WatcherBackend::os`].
    /// Removal failures are only logged: the watch is gone either way.
    fn apply(&mut self, ops: Vec<OsOp>) -> Vec<(PathBuf, notify::Error)> {
        let mut failures = Vec::new();
        if ops.is_empty() {
            return failures;
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return failures;
        };
        let mut batch = watcher.paths_mut();
        for op in ops {
            match op {
                OsOp::Add(path, mode) => {
                    if let Err(error) = batch.add(&path, mode.recursive_mode()) {
                        crate::debug::write_debug_log(
                            "watcher",
                            &format!("failed to watch {} ({mode:?}): {error}", path.display()),
                        );
                        failures.push((path, error));
                    }
                }
                OsOp::Remove(path) => {
                    if let Err(error) = batch.remove(&path) {
                        crate::debug::write_debug_log(
                            "watcher",
                            &format!("failed to unwatch {}: {error}", path.display()),
                        );
                    }
                }
            }
        }
        if let Err(error) = batch.commit() {
            crate::debug::write_debug_log(
                "watcher",
                &format!("failed to commit watch changes: {error}"),
            );
        }
        for (path, _) in &failures {
            self.os.mark_unarmed(path);
        }
        failures
    }

    /// Whether a registration delivers events for `target`: a recursive one
    /// covers itself and every descendant, a shallow one only itself.
    fn covers(&self, target: &Path) -> bool {
        self.registrations.keys().any(|(watched, mode)| match mode {
            WatchMode::Recursive => target.starts_with(watched),
            WatchMode::Shallow => watched == target,
        })
    }

    /// The recursive registrations whose poll expansion (#463) the directory
    /// `dir`, created after they were armed, belongs in (#721): every one
    /// whose expansion holds `dir`'s parent but not yet `dir`. Overlapping
    /// roots each take it, so the reference counts (#687) keep it armed until
    /// the last of them releases it. Empty off the poll backend.
    fn poll_expansion_owners(&self, dir: &Path) -> Vec<PathBuf> {
        if self.kind != BackendKind::Poll || self.watcher.is_none() {
            return Vec::new();
        }
        let Some(parent) = dir.parent() else {
            return Vec::new();
        };
        let parent_target = (parent.to_path_buf(), WatchMode::Shallow);
        let dir_target = (dir.to_path_buf(), WatchMode::Shallow);
        self.registrations
            .iter()
            .filter(|((_, mode), registration)| {
                *mode == WatchMode::Recursive
                    && registration.targets.contains(&parent_target)
                    && !registration.targets.contains(&dir_target)
            })
            .map(|((root, _), _)| root.clone())
            .collect()
    }

    /// Add each root's newly found directories to its poll expansion, arming
    /// whichever no other registration already holds (#721). `expansions`
    /// pairs a root from [`WatcherBackend::poll_expansion_owners`] with
    /// [`FileSystemWatcher::poll_watch_targets_within`]'s walk for it. A root
    /// released since then is skipped, and a directory a root already holds
    /// is not counted twice.
    fn extend_poll_expansion(&mut self, expansions: Vec<(PathBuf, Vec<PathBuf>)>) {
        if self.kind != BackendKind::Poll || self.watcher.is_none() {
            return;
        }
        let mut ops = Vec::new();
        for (root, dirs) in expansions {
            let Some(registration) = self.registrations.get_mut(&(root, WatchMode::Recursive))
            else {
                continue;
            };
            for dir in dirs {
                let target = (dir, WatchMode::Shallow);
                if registration.targets.contains(&target) {
                    continue;
                }
                ops.extend(self.os.acquire(&target.0, WatchMode::Shallow));
                registration.targets.insert(target);
            }
        }
        // Kept in the expansion either way, as `rearm_poll_paths` does: the
        // parent's scan reports the directory's removal, which drops it.
        for (path, error) in self.apply(ops) {
            crate::debug::write_debug_log(
                "watcher",
                &format!("poll backend could not arm new {}: {error}", path.display()),
            );
        }
    }

    /// Drop `removed` and everything under it from every recursive
    /// registration's poll expansion, unwatching whatever no other
    /// registration still holds (#721), so the expansion does not grow across
    /// mkdir/rmdir cycles. A registration's own root stays: `PollWatcher`
    /// watches by path, so it reports the root's entries again if the
    /// directory is recreated.
    fn drop_poll_expansion(&mut self, removed: &Path) {
        if self.kind != BackendKind::Poll || self.watcher.is_none() {
            return;
        }
        let mut ops = Vec::new();
        for ((root, mode), registration) in &mut self.registrations {
            if *mode != WatchMode::Recursive {
                continue;
            }
            // Paths order component-wise, so `removed`'s subtree is the
            // contiguous run of keys starting at `removed` itself.
            let gone: Vec<(PathBuf, WatchMode)> = registration
                .targets
                .range((removed.to_path_buf(), WatchMode::Shallow)..)
                .take_while(|(target, _)| target.starts_with(removed))
                .filter(|(target, _)| target != root)
                .cloned()
                .collect();
            for gone_target in &gone {
                registration.targets.remove(gone_target);
                let (target, target_mode) = gone_target;
                ops.extend(self.os.release(target, *target_mode).unwrap_or_default());
            }
        }
        self.apply(ops);
    }
}

/// One raw event path already attributed to a repository root by the third arm
/// of [`FileSystemWatcher::process_event`].
///
/// Bundled rather than passed as four parameters so
/// [`FileSystemWatcher::emit_attributed_worktree_event`] stays under clippy's
/// argument-count limit.
struct AttributedWorktreeEvent<'a> {
    event: &'a Event,
    path: &'a Path,
    repo: PathBuf,
    registry: &'a Arc<WatchRegistry>,
}

/// The directory-create follow-up wiring (#416, #569).
///
/// How long to hold the follow-up back, and where to hand it off.
///
/// Bundled so [`FileSystemWatcher::spawn_event_thread`] stays under clippy's
/// argument-count limit; the two always travel together.
struct FollowupHandoff {
    delay: Duration,
    schedule: DelayedEventScheduler,
}

/// The watcher state the event thread shares beyond its channel and stop flag.
///
/// The registry `process_event` attributes against, and the backend whose
/// poll expansion it keeps in step with the tree (#721). Bundled so
/// [`FileSystemWatcher::spawn_event_thread`] stays under clippy's
/// argument-count limit.
struct EventThreadState {
    registry: Arc<WatchRegistry>,
    backend: Arc<Mutex<WatcherBackend>>,
}

/// The two durations [`FallbackPolicy::ProbeThenFallback`] carries.
///
/// Bundled so [`FileSystemWatcher::spawn_health_probe`] stays under clippy's
/// argument-count limit now that it also takes the watcher's registry (#617).
#[derive(Clone, Copy, Debug)]
struct ProbeTiming {
    probe_timeout: Duration,
    poll_interval: Duration,
}

/// A poll watcher built and ready to install, plus its interval.
///
/// Only the interval is used after installation (for the debug log). Bundled
/// so [`FileSystemWatcher::apply_poll_swap`] stays under clippy's
/// argument-count limit.
struct ArmedPollWatcher {
    watcher: Box<dyn Watcher + Send>,
    poll_interval: Duration,
}

/// How this watcher should react to a native backend that accepts watches and
/// then delivers nothing (#442).
///
/// Resolved once, at construction, by [`FallbackPolicy::from_env`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FallbackPolicy {
    /// Use the native backend and never probe or swap — pre-#442 behavior,
    /// reachable only by explicitly setting `GPY_WATCH_PROBE_MS=0`.
    NativeOnly,
    /// Run one bounded health probe in the background; if the native backend
    /// is silent, swap in `PollWatcher`. The default for EVERY watcher.
    ///
    /// #442 originally scoped this to the repo watcher, on the reasoning that
    /// the theme/config watchers each watch one small file and already ship
    /// opt-in `GPY_THEME_WATCH_POLL_MS` / `GPY_CONFIG_WATCH_POLL_MS` pollers
    /// from #354. #447 showed that reasoning was wrong: during an `FSEvents`
    /// outage the config watcher silently stopped applying config edits until
    /// the agent restarted, with no diagnostic. An opt-in remedy cannot help
    /// there, because the failure it remedies is invisible — which is the
    /// same argument that justified auto-detection for the repo watcher in
    /// the first place. The probe is one-shot and exits as soon as the first
    /// event lands, so a healthy machine pays a short-lived thread per
    /// watcher and nothing else.
    ProbeThenFallback {
        probe_timeout: Duration,
        poll_interval: Duration,
    },
    /// Skip detection entirely and start on `PollWatcher`. Opt-in escape
    /// hatch (`GPY_WATCH_FORCE_POLL`) for a machine already known to be
    /// broken, and the policy the fallback tests use.
    ForcePoll { poll_interval: Duration },
}

impl FallbackPolicy {
    /// Resolve the policy from the process environment.
    ///
    /// - `GPY_WATCH_FORCE_POLL` (`1`/`true`/`yes`/`on`): start on
    ///   `PollWatcher`, no probe.
    /// - `GPY_WATCH_PROBE_MS`: probe timeout in milliseconds; `0` disables
    ///   auto-detection entirely (back to pre-#442 behavior).
    /// - `GPY_WATCH_POLL_MS`: interval for the `PollWatcher` fallback.
    fn from_env() -> Self {
        let force_raw = std::env::var("GPY_WATCH_FORCE_POLL").ok();
        let probe_raw = std::env::var("GPY_WATCH_PROBE_MS").ok();
        let poll_raw = std::env::var("GPY_WATCH_POLL_MS").ok();
        Self::resolve(
            force_raw.as_deref(),
            probe_raw.as_deref(),
            poll_raw.as_deref(),
        )
    }

    /// Pure resolution of the three env values, split out from
    /// [`FallbackPolicy::from_env`] so it is directly unit-testable without
    /// mutating the real process environment (this crate forbids
    /// `unsafe_code`, and `std::env::set_var` requires it). Mirrors
    /// `ConfigManager::parse_poll_interval_ms` /
    /// `parse_reconcile_interval_secs` (#354).
    fn resolve(force_raw: Option<&str>, probe_raw: Option<&str>, poll_raw: Option<&str>) -> Self {
        let poll_interval = Duration::from_millis(
            parse_millis(poll_raw)
                .filter(|value| *value > 0_u64)
                .unwrap_or(DEFAULT_WATCH_POLL_MS),
        );

        if matches!(force_raw.map(str::trim), Some("1" | "true" | "yes" | "on")) {
            return Self::ForcePoll { poll_interval };
        }

        let probe_ms = parse_millis(probe_raw).unwrap_or(DEFAULT_WATCH_PROBE_MS);
        if probe_ms == 0_u64 {
            return Self::NativeOnly;
        }

        Self::ProbeThenFallback {
            probe_timeout: Duration::from_millis(probe_ms),
            poll_interval,
        }
    }
}

/// Parse a raw millisecond env value. `None` when unset or non-numeric, so
/// the caller can apply its own default (and, where `0` is meaningful, act on
/// it explicitly).
fn parse_millis(raw: Option<&str>) -> Option<u64> {
    raw?.trim().parse::<u64>().ok()
}

/// Registration owner counts and the armed OS watches, as returned by
/// [`FileSystemWatcher::held_watches`].
#[cfg(test)]
pub(crate) type HeldWatches = (
    BTreeMap<(PathBuf, WatchMode), u32>,
    BTreeMap<PathBuf, WatchMode>,
);

/// File system watcher using notify crate
pub struct FileSystemWatcher {
    backend: Arc<Mutex<WatcherBackend>>,
    event_tx: Option<Sender<Event>>,
    handle: Option<thread::JoinHandle<()>>,
    probe_handle: Option<thread::JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
    /// Shared with the owning [`super::WatchCoordinator`] and the event
    /// thread; also read by `watch`'s poll-backend branch, whose
    /// `poll_watch_targets` descent consults the ignore caches (#617).
    registry: Arc<WatchRegistry>,
}

impl FileSystemWatcher {
    /// Create a new file system watcher with a callback.
    ///
    /// `registry` is the [`WatchRegistry`] this watcher shares with its owning
    /// coordinator: the watched git roots `process_event` uses for
    /// longest-prefix event attribution (#344), the gitdir attribution maps,
    /// the registered config paths, the worktree-watch flag and the ignore
    /// caches. `None` allocates a private registry, whose empty watched-root
    /// set makes attribution fall back to the ancestor `.git` stat walk — what
    /// a standalone (theme/config) watcher gets.
    ///
    /// `followup_delay` is how long to wait before firing the single
    /// coalesced follow-up refresh scheduled after a directory-create event
    /// (#416). It is threaded down from the owning coordinator's debounce
    /// window so `GPY_DEBOUNCE_MS` tuning applies uniformly.
    ///
    /// `schedule_delayed` is where that follow-up is handed off: it records the
    /// event with a due time and returns immediately, leaving the wait to the
    /// owning coordinator's flush thread (#569). See [`DelayedEventScheduler`].
    ///
    /// # Errors
    ///
    /// Returns an error if the file watcher cannot be created.
    pub fn new(
        callback: PendingCallback,
        registry: Option<Arc<WatchRegistry>>,
        followup_delay: Duration,
        schedule_delayed: DelayedEventScheduler,
    ) -> Result<Self> {
        let policy = FallbackPolicy::from_env();
        Self::with_fallback_policy(callback, registry, followup_delay, schedule_delayed, policy)
    }

    /// Construction with an explicit [`FallbackPolicy`], split out from
    /// [`FileSystemWatcher::new`] so tests can drive the fallback path
    /// directly instead of mutating the process environment (#442).
    ///
    /// # Errors
    ///
    /// Returns an error if the selected notify backend cannot be created.
    fn with_fallback_policy(
        callback: PendingCallback,
        registry: Option<Arc<WatchRegistry>>,
        followup_delay: Duration,
        schedule_delayed: DelayedEventScheduler,
        policy: FallbackPolicy,
    ) -> Result<Self> {
        let shared_registry = registry.unwrap_or_else(|| Arc::new(WatchRegistry::new()));
        let (event_tx, event_rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();

        let (watcher, kind) = match policy {
            FallbackPolicy::ForcePoll { poll_interval } => (
                Self::build_poll_watcher(&event_tx, poll_interval)?,
                BackendKind::Poll,
            ),
            FallbackPolicy::NativeOnly | FallbackPolicy::ProbeThenFallback { .. } => {
                (Self::build_native_watcher(&event_tx)?, BackendKind::Native)
            }
        };
        crate::debug::write_debug_log(
            "watcher",
            &format!("backend armed: {} (policy: {policy:?})", kind.label()),
        );

        let backend = Arc::new(Mutex::new(WatcherBackend::armed(watcher, kind)));

        // Start background thread to process events
        let stop_flag = Arc::new(AtomicBool::new(false));
        let handle = Self::spawn_event_thread(
            event_rx,
            callback,
            &stop_flag,
            EventThreadState {
                registry: Arc::clone(&shared_registry),
                backend: Arc::clone(&backend),
            },
            FollowupHandoff {
                delay: followup_delay,
                schedule: schedule_delayed,
            },
        );

        let probe_handle = match policy {
            FallbackPolicy::ProbeThenFallback {
                probe_timeout,
                poll_interval,
            } => Some(Self::spawn_health_probe(
                &backend,
                &event_tx,
                &stop_flag,
                ProbeTiming {
                    probe_timeout,
                    poll_interval,
                },
                &shared_registry,
            )),
            FallbackPolicy::NativeOnly | FallbackPolicy::ForcePoll { .. } => None,
        };

        Ok(Self {
            backend,
            event_tx: Some(event_tx),
            handle: Some(handle),
            probe_handle,
            stop_flag,
            registry: shared_registry,
        })
    }

    /// Spawn the thread that drains the shared event channel into
    /// [`FileSystemWatcher::process_event`], first keeping the poll
    /// expansion in step with any directory the event creates or removes
    /// (#721).
    ///
    /// Split out of [`FileSystemWatcher::with_fallback_policy`] purely to keep
    /// that constructor under clippy's line limit; it owns nothing the
    /// constructor still needs.
    fn spawn_event_thread(
        event_rx: Receiver<Event>,
        callback: PendingCallback,
        stop_flag: &Arc<AtomicBool>,
        state: EventThreadState,
        followup: FollowupHandoff,
    ) -> thread::JoinHandle<()> {
        let callback_arc = Arc::new(callback);
        let stop_flag_thread = Arc::clone(stop_flag);
        thread::spawn(move || {
            loop {
                if stop_flag_thread.load(Ordering::Relaxed) {
                    break;
                }

                match event_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(event) => {
                        // Before classification, so a new directory is armed
                        // before any refresh it triggers -- its
                        // directory-create follow-up included -- scans it.
                        Self::track_poll_expansion(&event, &state.backend, &state.registry);
                        Self::process_event(
                            &event,
                            &callback_arc,
                            &state.registry,
                            followup.delay,
                            &followup.schedule,
                        );
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
    }

    /// Forward one raw notify result into the shared event channel.
    ///
    /// Every backend — the native one, the `PollWatcher` that may replace it,
    /// and any future addition — hands events to this same `Sender<Event>`
    /// clone, so a swap needs no change to the event-processing thread or to
    /// callback ownership (#442).
    fn forward_event(event_tx: &Sender<Event>, res: notify::Result<Event>) {
        match res {
            Ok(event) => {
                let _ = event_tx.send(event);
            }
            Err(error) => {
                crate::debug::write_debug_log("watcher", &format!("notify backend error: {error}"));
            }
        }
    }

    /// Build the OS-native backend (`notify::RecommendedWatcher`).
    ///
    /// # Errors
    ///
    /// Returns an error if the platform backend cannot be initialized.
    fn build_native_watcher(event_tx: &Sender<Event>) -> Result<Box<dyn Watcher + Send>> {
        let tx_clone = event_tx.clone();
        let watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| Self::forward_event(&tx_clone, res),
            notify::Config::default(),
        )
        .map_err(|e| Error::watcher(format!("Failed to create file watcher: {e}")))?;
        Ok(Box::new(watcher))
    }

    /// Build the polling fallback backend (`notify::PollWatcher`).
    ///
    /// `with_compare_contents(true)`: notify's poll compares only whole-second
    /// mtimes (not size), so a rewrite landing in the same second as the
    /// previous write -- a branch ref moved to another commit, a `git config`
    /// edit, two saves in quick succession -- is otherwise never reported
    /// (#817). With content comparison a same-second change is caught by its
    /// hash. The cost is one read of every watched file per tick, accepted
    /// because this backend only runs on a machine whose native watcher is
    /// already broken or forced off, the expansion excludes ignored trees
    /// (#463/#617), and `GPY_WATCH_POLL_MS` raises the interval.
    ///
    /// Remaining poll limitation: a rewrite that leaves both the content and
    /// the mtime second unchanged (a `HEAD` rewritten with the value it
    /// already holds) is invisible to any stat/hash comparison; the periodic
    /// reconcile (`RECONCILE_INTERVAL_SECS`) bounds it, and it changes no state.
    ///
    /// # Errors
    ///
    /// Returns an error if the poll watcher cannot be initialized.
    fn build_poll_watcher(
        event_tx: &Sender<Event>,
        poll_interval: Duration,
    ) -> Result<Box<dyn Watcher + Send>> {
        let tx_clone = event_tx.clone();
        let watcher = PollWatcher::new(
            move |res: notify::Result<Event>| match res {
                Ok(event) if is_poll_directory_mtime_noise(&event) => {}
                other => Self::forward_event(&tx_clone, other),
            },
            notify::Config::default()
                .with_poll_interval(poll_interval)
                .with_compare_contents(true),
        )
        .map_err(|e| Error::watcher(format!("Failed to create poll-fallback watcher: {e}")))?;
        Ok(Box::new(watcher))
    }

    /// Enumerate the directories under `root` that a `BackendKind::Poll`
    /// watch should register individually (#463).
    ///
    /// `notify::PollWatcher` re-stat-walks whatever directory it is told to
    /// watch on every tick, with no `.gitignore` awareness of its own (its
    /// `scan_all_path_data` is a flat `WalkDir` over the watched root). Handing
    /// it one `Recursive` watch on a worktree root therefore re-walks every
    /// gitignored subtree too — a vendored/bundled directory, say — on every
    /// tick, for no benefit: `path_is_gitignored` already keeps those paths
    /// from ever triggering a refresh once notify reports them. On a large
    /// ignored subtree that walk cost dominates and can pin a CPU core
    /// indefinitely.
    ///
    /// This does the recursive descent itself instead, pruning any directory
    /// `path_is_gitignored` reports as ignored (skipping it and everything
    /// under it) — except inside `.git`, which #443/#447 already pin as never
    /// subject to `.gitignore` filtering on the event path; `is_in_git_dir`
    /// short-circuits the same way here so a `.gitignore` rule that happens to
    /// match something under `.git` (e.g. a broad `*.pack` pattern) can never
    /// blind the watcher to real git-internal changes.
    ///
    /// `root` is always the first returned entry, even when it contains no
    /// other watchable directory, so a leaf/empty worktree still gets one
    /// watch registered. The caller arms every returned directory
    /// `NonRecursive`: the recursive descent already happened here, so
    /// `PollWatcher` never needs to walk deeper than one level per registered
    /// directory.
    fn poll_watch_targets(root: &Path, registry: &Arc<WatchRegistry>) -> Vec<PathBuf> {
        Self::poll_targets_from(root, root, registry)
    }

    /// What a directory `dir`, created under `root` after `root` was armed,
    /// adds to `root`'s poll expansion (#721): `dir` and every directory
    /// below it, pruned by `root`'s ignore rules exactly as
    /// [`FileSystemWatcher::poll_watch_targets`] would prune them. Empty when
    /// `dir` is pruned itself, or is not a real directory by now -- gone
    /// again, or a symlink, which the arm-time walk does not follow either.
    fn poll_watch_targets_within(
        root: &Path,
        dir: &Path,
        registry: &Arc<WatchRegistry>,
    ) -> Vec<PathBuf> {
        let is_real_dir = std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir());
        if !is_real_dir || Self::poll_prunes(root, dir, registry) {
            return Vec::new();
        }
        Self::poll_targets_from(root, dir, registry)
    }

    /// `start` followed by every directory below it that `root`'s poll
    /// expansion keeps: the descent [`FileSystemWatcher::poll_watch_targets`]
    /// describes.
    fn poll_targets_from(root: &Path, start: &Path, registry: &Arc<WatchRegistry>) -> Vec<PathBuf> {
        let mut targets = vec![start.to_path_buf()];
        let mut stack = vec![start.to_path_buf()];

        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_dir() {
                    continue;
                }
                let path = entry.path();
                if Self::poll_prunes(root, &path, registry) {
                    continue;
                }
                targets.push(path.clone());
                stack.push(path);
            }
        }

        targets
    }

    /// Whether `root`'s poll expansion skips the directory `path` and
    /// everything under it: gitignored, and not inside `.git`.
    fn poll_prunes(root: &Path, path: &Path, registry: &Arc<WatchRegistry>) -> bool {
        !is_in_git_dir(path) && path_is_gitignored(root, path, registry)
    }

    /// Keep the poll backend's directory expansion (#463) in step with the
    /// tree (#721).
    ///
    /// `PollWatcher` scans each expanded directory one level deep, so a
    /// directory created after arming appears only as an entry in its
    /// parent's scan, and nothing inside it would ever be scanned. A created
    /// directory therefore joins the expansion of every root whose expansion
    /// holds its parent, and a removed one leaves it, subtree and all, so the
    /// expansion does not grow across mkdir/rmdir cycles. `PollWatcher`
    /// reports a rename as a remove plus a create, so those two kinds cover
    /// it; native backends watch recursively and make both a no-op.
    fn track_poll_expansion(
        event: &Event,
        backend: &Mutex<WatcherBackend>,
        registry: &Arc<WatchRegistry>,
    ) {
        match event.kind {
            EventKind::Create(_) => {
                for path in &event.paths {
                    if is_directory_create(event.kind, path) {
                        Self::expand_poll_into(path, backend, registry);
                    }
                }
            }
            // A removed file was never an expansion target.
            EventKind::Remove(kind) if kind != RemoveKind::File => {
                let Ok(mut backend_guard) = backend.lock() else {
                    return;
                };
                for path in &event.paths {
                    backend_guard.drop_poll_expansion(path);
                }
            }
            _ => {}
        }
    }

    /// Add the newly created directory `dir`, and whatever it already
    /// contains, to the poll expansion of every root it belongs to (#721).
    ///
    /// The walk runs with the backend lock released, as `watch` does (#618):
    /// it can transitively run `git ls-files`, and the lock is shared by
    /// every repo this watcher serves. Anything written into `dir` before its
    /// watch takes its baseline is caught by the refresh `dir`'s own create
    /// event triggers once `process_event` handles it after this returns:
    /// the directory-create follow-up for a worktree directory (#416), the
    /// Git classification itself for a ref directory.
    fn expand_poll_into(
        dir: &Path,
        backend: &Mutex<WatcherBackend>,
        registry: &Arc<WatchRegistry>,
    ) {
        let owners = {
            let Ok(backend_guard) = backend.lock() else {
                return;
            };
            backend_guard.poll_expansion_owners(dir)
        };
        if owners.is_empty() {
            return;
        }
        let expansions: Vec<(PathBuf, Vec<PathBuf>)> = owners
            .into_iter()
            .map(|root| {
                let dirs = Self::poll_watch_targets_within(&root, dir, registry);
                (root, dirs)
            })
            .collect();
        let Ok(mut backend_guard) = backend.lock() else {
            return;
        };
        backend_guard.extend_poll_expansion(expansions);
    }

    /// Spawn the one-shot background health probe (#442).
    ///
    /// Deliberately asynchronous: agent startup — and therefore the first
    /// prompt — must not wait on a multi-second filesystem probe, so
    /// [`FileSystemWatcher::new`] returns as soon as the native backend is
    /// armed and this thread does the waiting. Worst-case added startup
    /// latency is one `thread::spawn`.
    ///
    /// One-shot rather than periodic: a silent `FSEvents` daemon is a
    /// machine-level condition that does not spontaneously heal within a
    /// shell session, while re-probing forever would cost a permanent timer
    /// thread and risk flapping between backends — and every flap drops the
    /// events that occur during re-arming. Polling stays correct, just
    /// slower, so the swap is permanent for the process lifetime; a restarted
    /// agent re-probes.
    fn spawn_health_probe(
        backend: &Arc<Mutex<WatcherBackend>>,
        event_tx: &Sender<Event>,
        stop_flag: &Arc<AtomicBool>,
        timing: ProbeTiming,
        registry: &Arc<WatchRegistry>,
    ) -> thread::JoinHandle<()> {
        let backend_for_probe = Arc::clone(backend);
        let tx_for_probe = event_tx.clone();
        let stop_for_probe = Arc::clone(stop_flag);
        // An owned clone, not a borrow: this thread outlives the constructor.
        let registry_for_probe = Arc::clone(registry);
        thread::spawn(move || {
            if native_backend_delivers_events(timing.probe_timeout, &stop_for_probe) {
                crate::debug::write_debug_log(
                    "watcher",
                    "native backend health probe passed; staying on native",
                );
                return;
            }
            crate::debug::write_debug_log(
                "watcher",
                "native backend accepted the watch but delivered no events \
                 within the probe window; falling back to polling (#442)",
            );
            Self::engage_poll_fallback(
                &backend_for_probe,
                &tx_for_probe,
                &stop_for_probe,
                timing.poll_interval,
                &registry_for_probe,
            );
        })
    }

    /// Phase 1 of [`FileSystemWatcher::engage_poll_fallback`] (#618): bail out
    /// under a short lock if the swap is no longer needed or safe, otherwise
    /// return the paths of every recursive registration, whose poll-target
    /// expansion the caller computes with no lock held. Nothing is mutated
    /// here -- the actual swap happens in
    /// [`FileSystemWatcher::apply_poll_swap`], atomically with the re-arm.
    /// Split out to keep `engage_poll_fallback` under clippy's line-count
    /// limit.
    fn poll_swap_snapshot(
        backend: &Arc<Mutex<WatcherBackend>>,
        stop_flag: &AtomicBool,
    ) -> Option<Vec<PathBuf>> {
        let Ok(snapshot_guard) = backend.lock() else {
            crate::debug::write_debug_log(
                "watcher",
                "poll fallback aborted: watcher backend lock poisoned",
            );
            return None;
        };
        if stop_flag.load(Ordering::Relaxed) || snapshot_guard.watcher.is_none() {
            return None;
        }
        if snapshot_guard.kind == BackendKind::Poll {
            return None;
        }
        Some(
            snapshot_guard
                .registrations
                .keys()
                .filter(|(_, mode)| *mode == WatchMode::Recursive)
                .map(|(path, _)| path.clone())
                .collect(),
        )
    }

    /// Phase 3 of [`FileSystemWatcher::engage_poll_fallback`] (#618): re-lock,
    /// re-check (a concurrent `stop()` may have torn the backend down while
    /// the poll-target expansion was being computed with no lock held, and
    /// re-arming watches after that would leak a live watcher past shutdown),
    /// then swap in `armed.watcher` and re-arm every currently watched path
    /// together, atomically, using `precomputed`. Returns the re-armed paths
    /// for the caller to push synthetic rescans for, or `None` if the swap
    /// was aborted. Split out to keep `engage_poll_fallback` under clippy's
    /// line-count limit.
    fn apply_poll_swap(
        backend: &Arc<Mutex<WatcherBackend>>,
        stop_flag: &AtomicBool,
        armed: ArmedPollWatcher,
        registry: &Arc<WatchRegistry>,
        precomputed: &HashMap<PathBuf, Vec<PathBuf>>,
    ) -> Option<Vec<PathBuf>> {
        let Ok(mut backend_guard) = backend.lock() else {
            crate::debug::write_debug_log(
                "watcher",
                "poll fallback aborted: watcher backend lock poisoned",
            );
            return None;
        };
        if stop_flag.load(Ordering::Relaxed) || backend_guard.watcher.is_none() {
            return None;
        }
        if backend_guard.kind == BackendKind::Poll {
            return None;
        }

        backend_guard.watcher = None;
        backend_guard.watcher = Some(armed.watcher);
        backend_guard.kind = BackendKind::Poll;

        let paths = Self::rearm_poll_paths(&mut backend_guard, registry, precomputed);
        crate::debug::write_debug_log(
            "watcher",
            &format!(
                "backend swapped: native -> poll ({}ms interval), {} path(s) re-armed",
                armed.poll_interval.as_millis(),
                paths.len()
            ),
        );
        Some(paths)
    }

    /// Replace the silent native backend with a `PollWatcher` and re-arm every
    /// path the old backend was watching (#442).
    ///
    /// The old backend is dropped before the new one is armed, so the two
    /// recursive mechanisms never run side by side. The swap and the re-arm
    /// happen together in one lock ([`FileSystemWatcher::apply_poll_swap`]),
    /// so that pair is atomic: no caller can observe `kind == Poll` before
    /// the new backend actually has every path registered.
    ///
    /// The expensive part -- computing each directory path's poll-target
    /// expansion, which transitively walks the tree and can run `git
    /// ls-files` for a repo's first force-added lookup (#618) -- happens
    /// BEFORE that lock is taken, keyed by a snapshot of the recursive
    /// registrations read under a short separate lock
    /// ([`FileSystemWatcher::poll_swap_snapshot`]). `self.backend` is the one
    /// mutex shared by every repo this watcher serves, so doing that walk
    /// while holding it would let one repo's cold start stall
    /// `watch`/`watch_shallow`/`unwatch` for every other repo.
    ///
    /// Because a `PollWatcher` takes its baseline snapshot when a path is
    /// armed, changes made between agent startup and the swap would otherwise
    /// be invisible until the ~45s reconcile. To close that, each re-armed
    /// root gets one synthetic `Flag::Rescan` event pushed through the same
    /// channel, which `process_event` already routes to `handle_rescan` and
    /// hence to one debounced git refresh per root (#431).
    fn engage_poll_fallback(
        backend: &Arc<Mutex<WatcherBackend>>,
        event_tx: &Sender<Event>,
        stop_flag: &AtomicBool,
        poll_interval: Duration,
        registry: &Arc<WatchRegistry>,
    ) {
        if stop_flag.load(Ordering::Relaxed) {
            return;
        }

        // Built OUTSIDE the lock: constructing a PollWatcher spawns a thread
        // and does an initial scan, and holding the backend lock across it
        // would block concurrent `watch_directory` calls for no reason.
        let poll_watcher = match Self::build_poll_watcher(event_tx, poll_interval) {
            Ok(watcher) => watcher,
            Err(error) => {
                // Degraded-but-safe: keep the (silent) native backend, and let
                // the periodic reconcile remain the backstop it is today. The
                // prompt still renders; it just refreshes at reconcile speed.
                crate::debug::write_debug_log(
                    "watcher",
                    &format!(
                        "poll fallback unavailable ({error}); staying on the native \
                         backend and relying on the periodic reconcile"
                    ),
                );
                return;
            }
        };

        let Some(recursive_paths) = Self::poll_swap_snapshot(backend, stop_flag) else {
            return;
        };

        // No lock held: compute the poll-target expansion (and any
        // transitive `git ls-files`, #618) for each recursive directory
        // registration in the snapshot -- exactly the work `rearm_poll_paths`
        // used to do while holding the lock.
        let mut precomputed = HashMap::with_capacity(recursive_paths.len());
        for path in recursive_paths {
            if path.is_dir() {
                let targets = Self::poll_watch_targets(&path, registry);
                precomputed.insert(path, targets);
            }
        }

        let armed = ArmedPollWatcher {
            watcher: poll_watcher,
            poll_interval,
        };
        let Some(paths) = Self::apply_poll_swap(backend, stop_flag, armed, registry, &precomputed)
        else {
            return;
        };

        for path in paths {
            let rescan = Event::new(EventKind::Any)
                .add_path(path)
                .set_flag(Flag::Rescan);
            let _ = event_tx.send(rescan);
        }
    }

    /// Rebuild `state`'s OS-level watch set against the `PollWatcher` now
    /// installed in it, from the logical registrations alone (#687): every
    /// registration is re-expanded for the poll backend (#463) and its
    /// targets re-counted in a fresh [`WatchSet`], so targets shared by
    /// overlapping roots stay armed until the last root releases them.
    /// Returns each re-armed path once, for the caller's synthetic rescans.
    /// Split out of [`FileSystemWatcher::engage_poll_fallback`] to keep it
    /// under clippy's line-count limit.
    ///
    /// `precomputed` holds the poll-target expansion
    /// [`FileSystemWatcher::engage_poll_fallback`] already computed, with
    /// `state`'s lock released, for every recursive directory registration in
    /// its pre-swap snapshot (#618). A registration absent from `precomputed`
    /// was added by a concurrent `watch()` call after that snapshot was
    /// taken. Its expansion is computed here, still under `state`'s lock, only
    /// as a defensive fallback so a rearm can never silently skip a path --
    /// the same rare case #618 otherwise leaves running transitively under
    /// the lock, but bounded to at most the paths added in one narrow race
    /// window rather than every path on every swap.
    fn rearm_poll_paths(
        state: &mut WatcherBackend,
        registry: &Arc<WatchRegistry>,
        precomputed: &HashMap<PathBuf, Vec<PathBuf>>,
    ) -> Vec<PathBuf> {
        state.os = WatchSet::new(state.kind.removal_cascades());
        let mut ops = Vec::new();
        let mut paths: Vec<PathBuf> = Vec::new();
        for ((path, mode), registration) in &mut state.registrations {
            // A shallow registration is never expanded: re-arming it
            // recursively would expand it into `objects/**` for a 2s stat
            // walk (#463) -- the exact reason it is shallow.
            let poll_targets = (*mode == WatchMode::Recursive && path.is_dir()).then(|| {
                precomputed
                    .get(path)
                    .map_or_else(|| Self::poll_watch_targets(path, registry), Clone::clone)
            });
            registration.targets =
                WatcherBackend::targets_for(state.kind, path, *mode, poll_targets);
            for (target, target_mode) in &registration.targets {
                ops.extend(state.os.acquire(target, *target_mode));
            }
            // Keys are ordered by path first, so a path registered in both
            // modes is adjacent and `dedup` below keeps it once.
            paths.push(path.clone());
        }
        paths.dedup();
        for (path, error) in state.apply(ops) {
            crate::debug::write_debug_log(
                "watcher",
                &format!("poll fallback could not re-arm {}: {error}", path.display()),
            );
        }
        paths
    }

    /// Start watching a directory recursively
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be watched.
    pub fn watch<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path_buf = path.as_ref().to_path_buf();
        let is_dir = path_buf.is_dir();

        loop {
            // Peek the armed backend kind under a short lock, then release it
            // before doing any poll-target discovery below: `poll_watch_targets`
            // walks the directory tree and can transitively run `git ls-files`
            // for a repo's first force-added lookup (#618), and `self.backend`
            // is the one mutex shared by every repo this watcher serves --
            // holding it across that walk would let one repo's cold start
            // stall another repo's `watch`/`watch_shallow`/`unwatch` call.
            let kind = {
                let Ok(peek_guard) = self.backend.lock() else {
                    return Err(Error::watcher("Watcher backend lock poisoned".to_owned()));
                };
                peek_guard.kind
            };

            let poll_targets = (kind == BackendKind::Poll && is_dir)
                .then(|| Self::poll_watch_targets(&path_buf, &self.registry));

            let Ok(mut backend_guard) = self.backend.lock() else {
                return Err(Error::watcher("Watcher backend lock poisoned".to_owned()));
            };
            // The health probe can swap native -> poll between the peek above
            // and this re-lock (#442). That swap happens at most once per
            // process, so retrying here is bounded to a single extra
            // iteration: recompute against the now-current backend rather than
            // arming a stale expansion (or an unwanted recursive watch) onto
            // the wrong one.
            if backend_guard.kind != kind {
                drop(backend_guard);
                continue;
            }
            return backend_guard.acquire(&path_buf, WatchMode::Recursive, poll_targets);
        }
    }

    /// Start watching a directory *non-recursively* -- its direct entries only.
    ///
    /// Used for a linked worktree's common git directory (#468), where the
    /// interesting leaves (`packed-refs`, `config`) sit directly inside but a
    /// recursive watch would drag in `objects/**`: under the poll backend
    /// [`FileSystemWatcher::poll_watch_targets`] never prunes inside `.git`, so
    /// that would arm a pack directory for a stat walk every poll interval
    /// (#463). Watching the containing directory rather than those two files
    /// individually is also what survives git's write-lock-then-rename, which
    /// orphans an inode-based file watch.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be watched.
    pub fn watch_shallow<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let Ok(mut backend_guard) = self.backend.lock() else {
            return Err(Error::watcher("Watcher backend lock poisoned".to_owned()));
        };
        // Never expanded, on either backend: a shallow watch is exactly one
        // `NonRecursive` OS registration.
        backend_guard.acquire(path.as_ref(), WatchMode::Shallow, None)
    }

    /// Whether an already-armed watch delivers events for `path`.
    ///
    /// A recursive watch covers itself and every descendant; a shallow watch
    /// (see [`FileSystemWatcher::watch_shallow`]) covers only itself. Lets
    /// callers skip a redundant `watch` call -- which matters beyond tidiness on
    /// macOS, where notify cannot add a path to a live `FSEventStream` and instead
    /// tears it down and rebuilds it, and the rebuilt stream can silently drop
    /// its first event (#388).
    ///
    /// Answered from the logical registrations, which the reference-counted
    /// OS watch set keeps armed for as long as any of them holds (#687).
    #[must_use]
    pub fn covers<P: AsRef<Path>>(&self, path: P) -> bool {
        self.backend
            .lock()
            .is_ok_and(|guard| guard.covers(path.as_ref()))
    }

    /// Snapshot of what this watcher holds, for tests (#718): every logical
    /// registration with its owner count, and the OS watches armed beneath
    /// them.
    #[cfg(test)]
    pub(crate) fn held_watches(&self) -> HeldWatches {
        self.backend.lock().map_or_else(
            |_| (BTreeMap::new(), BTreeMap::new()),
            |guard| {
                let owners = guard
                    .registrations
                    .iter()
                    .map(|(key, registration)| (key.clone(), registration.owners))
                    .collect();
                (owners, guard.os.armed_paths())
            },
        )
    }

    /// Release one recursive [`FileSystemWatcher::watch`] of `path`.
    ///
    /// Undoes only that one registration: OS watches another registration
    /// still needs -- the same path watched by another owner, a nested or
    /// enclosing repository, a shared poll-expansion directory -- stay armed
    /// (#687).
    ///
    /// # Errors
    ///
    /// Returns an error if `path` is not watched recursively.
    pub fn unwatch<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let Ok(mut backend_guard) = self.backend.lock() else {
            return Err(Error::watcher("Watcher backend lock poisoned".to_owned()));
        };
        backend_guard.release(path.as_ref(), WatchMode::Recursive)
    }

    /// Release one [`FileSystemWatcher::watch_shallow`] of `path`, with the
    /// same ownership rules as [`FileSystemWatcher::unwatch`].
    ///
    /// # Errors
    ///
    /// Returns an error if `path` is not watched shallowly.
    pub fn unwatch_shallow<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let Ok(mut backend_guard) = self.backend.lock() else {
            return Err(Error::watcher("Watcher backend lock poisoned".to_owned()));
        };
        backend_guard.release(path.as_ref(), WatchMode::Shallow)
    }

    /// Process a notify event and trigger callback if relevant
    fn process_event(
        event: &Event,
        callback_arc: &Arc<PendingCallback>,
        registry: &Arc<WatchRegistry>,
        followup_delay: Duration,
        schedule_delayed: &DelayedEventScheduler,
    ) {
        if event.need_rescan() {
            // Kernel event-queue overflow / MustScanSubDirs: the backend
            // dropped events, so force a full rescan of the affected watched
            // root(s) rather than waiting for the ~45s reconcile (#431).
            // Emitting one synthetic Git event per root routes through the
            // same debouncer/refresh_and_notify_coalesced path as a normal
            // change, so it coalesces and can't race an in-flight scan
            // (#418).
            Self::handle_rescan(event, callback_arc, registry);
            return;
        }

        let worktree_enabled = registry.worktree_enabled();
        let worktree_ctx = WorktreeEventContext {
            event,
            registry,
            worktree_enabled,
        };

        // An `Access` event (`Open`/`Close`/`Read`) never denotes a change to
        // file content, name, or metadata, so nothing below it should run for
        // one. Only the inotify backend emits `Access` at all — the FSEvents,
        // ReadDirectoryChangesW, and poll backends never construct the variant —
        // and it arms every watch with `WatchMask::OPEN` included
        // (`notify-8.2.0/src/inotify.rs::add_watch`), turning every *read* of a
        // watched file into an event shaped exactly like a real change, with
        // `need_rescan() == false` so the overflow branch above never catches it.
        //
        // Two distinct bugs came from classifying those (#550, #551):
        //
        // - Arming a recursive watch `WalkDir`-descends the tree, and that walk's
        //   own `readdir` on a directory it just armed reports `IN_OPEN` right
        //   back at the watcher. The fan-out arms below key on path alone, so a
        //   self-inflicted open of shared git metadata (`refs/heads` and friends)
        //   woke every dependent worktree exactly as a real write would.
        // - A reload's own read of the file that triggered it re-triggers the
        //   watch, so a single theme or config edit on Linux left the manager
        //   reloading and re-signalling every registered shell roughly every
        //   57ms, indefinitely.
        //
        // Dropping the event here loses no real change: inotify raises one
        // `Event` per mask bit (`inotify.rs` pushes each into its own `evs`
        // entry), so a genuine write arrives as its own `Modify(Data)` — and a
        // rename or delete as `Create`/`Remove` — alongside, never as an `Access`
        // alone.
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }

        for path in &event.paths {
            // Shared git metadata for linked worktrees (#468). ADDITIVE and
            // deliberately unconditional-on-classification: a linked worktree's
            // common directory is normally the main checkout's in-tree `.git`,
            // so `should_trigger_update` below classifies (and `break`s on)
            // `<main>/.git/refs/remotes/origin/main` as the main checkout's own
            // event whether or not the main checkout is registered. Making the
            // fan-out a fallback arm would therefore make it unreachable for the
            // common layout. Emitting it here leaves that delivery byte-for-byte
            // unchanged and adds one event per dependent worktree alongside it;
            // the bare-superproject layout (`/repos/foo.git/refs/...`, which
            // classifies as nothing) is reached the same way.
            //
            // No "is a repo registry backing this watcher?" gate any more
            // (#617): both fan-outs are keyed on `path` lying under a
            // registered gitdir, and a theme/config path never does — the
            // gitdir maps are populated only at repo registration, and the
            // paths those watchers see live outside every repository's git
            // directory. Each therefore returns without allocating, exactly
            // as the old `watched_roots.is_some()` gate made it.
            //
            // Both fan-outs forward straight through to
            // `attribute_common_gitdir`/`attribute_submodule_parents`, which take
            // only `path`, never `event.kind`; the pure-`Access` early return
            // above is what keeps a self-inflicted watch-bootstrap open off them
            // (#550).
            Self::emit_common_gitdir_fanout(path, registry, callback_arc);
            // Superproject gitlink fan-out (#467). Additive for the same
            // reason: in the child-registered case the external-gitdir arm
            // below classifies and `break`s, so a fallback arm would never
            // run, and in the parent-only case no arm fires at all. The two
            // fan-outs cannot double-emit for one repository — this one
            // yields superproject roots, that one linked-worktree roots, and
            // their predicates reject each other's paths.
            Self::emit_submodule_parent_fanout(path, registry, callback_arc);

            if let Some(pending) = should_trigger_update(path, registry) {
                if Self::is_unregistered_common_gitdir_wake(&pending.repo, path, registry) {
                    // Dropped, but still `break`: the control flow is exactly
                    // today's — this path classified, so no later path of the
                    // same event is examined — only the delivery is dropped.
                    break;
                }
                Self::emit_classified_event(&worktree_ctx, path, pending, callback_arc);
                // Only trigger once per event. No backend in use emits a
                // multi-path event carrying unique information: the FSEvents
                // and poll backends add exactly one path per event, and
                // inotify's only multi-path event (`RenameMode::Both`) is
                // pushed alongside the single-path `From`/`To` events for the
                // same cookie, so both paths reach the debouncer anyway.
                // Classifying every path of a large batch would instead charge
                // a `find_git_root` ancestor walk and an ignore-chain check per
                // path. Coalescing across events is the debouncer's job (#466).
                break;
            }

            // Submodule / linked-worktree git metadata lives at an *external*
            // gitdir (`<super>/.git/modules|worktrees/<name>/…`), which
            // `should_trigger_update` can neither classify (parent isn't
            // `.git`) nor attribute (the gitdir isn't a descendant of the
            // working root). Recover the correct working root from the
            // registration-time gitdir→root map instead (#428). Only the repo
            // watcher populates that map; a theme/config watcher's registry is
            // empty here, so this arm cannot fire for one. Not gated on
            // `worktree_enabled` — a branch switch must refresh the prompt
            // regardless, exactly like the normal `.git` path above. The mapped
            // `repo` is canonical (stored canonical at registration), as
            // downstream path-validation requires.
            if let Some(repo) = registry.attribute_external_gitdir(path) {
                callback_arc(PendingEvent::git(repo, path.clone()));
                break;
            }

            // `require_genuine_repo: false` — this arm's parent-guess
            // fallback is existing, pinned behavior (see
            // `event_outside_watched_roots_falls_back_to_ancestor_walk`) and
            // must not change here; only the additional Language/Config
            // emission above gets the stricter gate (#443).
            if let Some(repo) = worktree_git_repo(
                event,
                path,
                registry,
                worktree_enabled,
                RepoEvidence::AnyAttribution,
            ) {
                Self::emit_attributed_worktree_event(
                    AttributedWorktreeEvent {
                        event,
                        path,
                        repo,
                        registry,
                    },
                    callback_arc,
                    followup_delay,
                    schedule_delayed,
                );
                break;
            }
        }
    }

    /// Deliver the third arm's watched-root/ancestor-walk attributed event,
    /// split out of [`FileSystemWatcher::process_event`] to keep it under
    /// clippy's line-count limit.
    fn emit_attributed_worktree_event(
        attributed: AttributedWorktreeEvent<'_>,
        callback_arc: &Arc<PendingCallback>,
        followup_delay: Duration,
        schedule_delayed: &DelayedEventScheduler,
    ) {
        let AttributedWorktreeEvent {
            event,
            path,
            repo,
            registry,
        } = attributed;

        if Self::is_unregistered_common_gitdir_wake(&repo, path, registry) {
            return;
        }

        // A brand-new directory's recursive watch may not be armed
        // before files are written into it, so those file-create
        // events can be dropped entirely (never delivered). Schedule
        // one delayed, coalesced follow-up refresh of the owning repo
        // so those contents are picked up without waiting for the
        // periodic reconcile (#416). Only directory-creates pay this
        // cost; plain file events take exactly the path they do today.
        //
        // Watches themselves need no re-arming here: native watches are
        // recursive, and on the poll backend the event thread has already
        // added the directory to the expansion before this runs (#721), so
        // the follow-up covers only what landed before that watch's
        // baseline. Residual gap: this does not fully close the FSEvents
        // stream-rebuild first-event drop (#388) — a single follow-up scan
        // handles the common mkdir-then-write race, and the periodic
        // reconcile owns anything that still slips through. See #388, kept
        // separate.
        if is_directory_create(event.kind, path) {
            Self::schedule_directory_followup(
                schedule_delayed,
                repo.clone(),
                path.to_path_buf(),
                followup_delay,
            );
        }
        callback_arc(PendingEvent::git(repo, path.to_path_buf()));
    }

    /// Whether the classified/attributed emission of `path` for `repo` is a
    /// *cross-repository* wake introduced by the common-git-directory watch, and
    /// must be dropped (#468, epic #465's "no spurious cross-repository
    /// refresh").
    ///
    /// The shallow watch armed on a linked worktree's common git directory is
    /// armed *exactly* when the main checkout is not registered — the
    /// coverage-by-ancestor check skips it otherwise. So it makes the watcher
    /// see `<main>/.git/index` and `<main>/.git/refs/heads/<branch>`, which
    /// `should_trigger_update` faithfully classifies and attributes to
    /// `<main>` via `find_git_root`. `handle_file_event` has no watched-repo
    /// gate, so delivering that runs a full git capture round on a repository no
    /// shell is registered for: a cache write nobody reads and a doorbell with no
    /// recipient. The `refs/heads` route matters most — every commit made *in*
    /// the worktree takes it.
    ///
    /// Deliberately narrow, and NOT a general watched-root gate on
    /// classification: it fires only when the path sits under a registered
    /// common git directory AND the attributed root is unregistered. The
    /// permissive ancestor-walk and parent-guess fallbacks for every other path
    /// are untouched (`event_outside_watched_roots_falls_back_to_ancestor_walk`,
    /// `RepoEvidence::AnyAttribution`). When the main checkout IS registered the
    /// second condition is false and delivery proceeds exactly as before.
    ///
    /// Checked at the first arm, where the in-tree layout (`<main>/.git/…`)
    /// lands, and at the third, which `is_in_git_dir` makes unreachable for that
    /// layout but which a bare superproject's common directory
    /// (`/repos/foo.git/…`, no `.git` path component) can still reach.
    ///
    /// Not redundant with #480's `ClientDirectory::has_subscriber` gate in
    /// `agent/events.rs`'s `create_watcher` callback, even though both exist
    /// to stop the same "cache write nobody reads" waste described above.
    /// That gate suppresses *refresh*, one layer downstream of *emission*
    /// here — it never suppresses this function's own layer, since the
    /// #467/#468 wake-count tests in `multi_repo_integration_tests.rs`
    /// build their own watcher with a callback that only records
    /// `event.repo`, never reaching `agent/events.rs` at all. Deleting this
    /// guard would still fail those tests even with #480's gate in place.
    fn is_unregistered_common_gitdir_wake(
        repo: &Path,
        path: &Path,
        registry: &WatchRegistry,
    ) -> bool {
        // Checked first: almost every event attributes to a registered root, and
        // that short-circuits before the common-dir registry is consulted.
        // Fail open on a poisoned lock: dropping refreshes is the worse failure.
        let Some(registered) = registry.root_is_registered(repo) else {
            return false;
        };
        if registered {
            return false;
        }
        registry.is_under_registered_common_gitdir(path)
    }

    /// Emit one synthetic Git event per linked-worktree working root that reads
    /// the shared metadata `path` changed (#468).
    ///
    /// Each emitted root is canonical (stored canonical at registration), as
    /// downstream path validation requires, and each coalesces on its own
    /// debouncer key. Returns immediately -- allocating nothing -- for any path
    /// that is not shared metadata under a registered common directory, which is
    /// every path in a repository without linked worktrees.
    fn emit_common_gitdir_fanout(
        path: &Path,
        registry: &WatchRegistry,
        callback_arc: &Arc<PendingCallback>,
    ) {
        for repo in registry.attribute_common_gitdir(path) {
            callback_arc(PendingEvent::git(repo, path.to_path_buf()));
        }
    }

    /// Emit one synthetic Git event per REGISTERED parent repository whose
    /// gitlink status can change because submodule git metadata at `path`
    /// changed (#467).
    ///
    /// A submodule mutation changes two visible states — the submodule's own
    /// branch and status, and every ancestor superproject's gitlink — but only
    /// the first is delivered today. `<super>/.git/modules/<name>/HEAD` reaches
    /// no arm of [`FileSystemWatcher::process_event`]: `classify_event` does not
    /// treat `modules/<name>/HEAD` as significant, `attribute_external_gitdir`
    /// maps that gitdir only when the *submodule* is registered (and then to the
    /// submodule, ending the loop), and `worktree_git_repo` rejects anything
    /// inside a `.git` directory.
    ///
    /// No new watch is needed for any of this: an ordinary superproject's own
    /// registration already takes `start_repo_watches`' single-recursive-watch
    /// branch over `<super>`, which covers `<super>/.git/modules/**`. The events
    /// are already being delivered and are dropped at classification.
    ///
    /// Filtered against the registry's watched roots before emitting: a parent no shell is
    /// subscribed to must never buy a capture round (gpy#480). Fails *closed* on
    /// a poisoned registry lock — unlike
    /// [`FileSystemWatcher::is_unregistered_common_gitdir_wake`], which fails
    /// open — because the failure this gate exists to prevent is waking the
    /// wrong repository, not missing a refresh.
    fn emit_submodule_parent_fanout(
        path: &Path,
        registry: &WatchRegistry,
        callback_arc: &Arc<PendingCallback>,
    ) {
        let parents = registry.attribute_submodule_parents(path);
        if parents.is_empty() {
            return;
        }
        // The roots lock is released inside `filter_registered_roots`, before
        // the callbacks below: delivery reaches the whole refresh pipeline,
        // which must never run under the registry lock.
        let Some(registered) = registry.filter_registered_roots(parents) else {
            return;
        };

        for repo in registered {
            callback_arc(PendingEvent::git(repo, path.to_path_buf()));
        }
    }

    /// Deliver the event `should_trigger_update` classified for `path`, plus
    /// — for `Language`/`Config` classifications only — an additional Git
    /// refresh when the same worktree mutation independently qualifies as
    /// one too (e.g. editing `Cargo.toml` both updates the language
    /// subsystem and changes `git status`). `should_trigger_update` only
    /// ever returns one `FileEvent`, so without this the git instant cache
    /// goes stale until another Git event, a stale-cache pull, or the ~45s
    /// reconcile catches up (#443).
    ///
    /// Reuses every existing worktree eligibility filter via
    /// `worktree_git_repo` so this added emission can't drift from the plain
    /// worktree-Git arm in `process_event`, and attributes the Git event via
    /// the worktree path's registry-backed `attribute_repo_detailed` — NOT
    /// `should_trigger_update`'s own `find_git_root`/parent attribution —
    /// since that's what the downstream git-refresh machinery and its path
    /// validation expect.
    ///
    /// `Theme` also gets the added refresh (#777): a tracked theme file in a
    /// dotfiles repo (`.config/gpy/themes/...`) must refresh that repo's git
    /// segment. This is safe because of the `RepoEvidence::GenuineRepoOnly`
    /// gate below — a theme path outside any genuine repository (the usual
    /// `~/.config/gpy/themes/...`) never reaches a git scan (#443). `Git` is
    /// excluded: it is already the specialized event, so there is nothing to
    /// add.
    ///
    /// Passes `RepoEvidence::GenuineRepoOnly`, which closes the same hole
    /// for `Config`/`Language`: without it, `attribute_repo_detailed` can
    /// fall all the way through to `RepoAttribution::ParentGuess` — a bare
    /// "assume the parent directory is a repo" guess with no `.git`
    /// evidence anywhere. That's exactly what the theme/config watcher's own
    /// `~/.config/gpy/config.toml` hits, since it has no `.git` above it at
    /// all; firing a Git refresh there would be wasted scanning (and a
    /// potential spurious repaint) against a path that was never a
    /// repository.
    fn emit_classified_event(
        worktree_ctx: &WorktreeEventContext<'_>,
        path: &Path,
        pending: PendingEvent,
        callback_arc: &Arc<PendingCallback>,
    ) {
        if matches!(
            pending.event,
            super::FileEvent::Language { .. }
                | super::FileEvent::Config { .. }
                | super::FileEvent::Theme { .. }
        ) && let Some(repo) = worktree_git_repo(
            worktree_ctx.event,
            path,
            worktree_ctx.registry,
            worktree_ctx.worktree_enabled,
            RepoEvidence::GenuineRepoOnly,
        ) {
            callback_arc(PendingEvent::git(repo, path.to_path_buf()));
        }

        callback_arc(pending);
    }

    /// Handle a backend-reported overflow/rescan event (#431): emit one
    /// synthetic `FileEvent::Git` `PendingEvent` per affected watched root so
    /// each rescans through the normal debounced refresh path.
    fn handle_rescan(event: &Event, callback_arc: &Arc<PendingCallback>, registry: &WatchRegistry) {
        for root in Self::affected_rescan_roots(event, registry) {
            callback_arc(PendingEvent::git_whole_repo(root));
        }
    }

    /// Determine which watched roots a rescan event affects.
    ///
    /// - If the event names paths (`FSEvents` can identify the affected
    ///   subtree), attribute each one to its owning watched root via the same
    ///   longest-prefix match as a normal event, then de-duplicate so several
    ///   dropped paths under one repo yield a single rescan.
    /// - If the event names no paths (inotify's `IN_Q_OVERFLOW` names
    ///   nothing), the drop could have affected any watched root, so rescan
    ///   all of them.
    /// - With an empty watched-root set there is nothing to rescan.
    ///
    /// The one place #617's shared registry is not byte-identical to the old
    /// per-coordinator `Option<&WatchedRoots>`: a standalone theme/config
    /// coordinator still sees an empty set, but the agent's theme/config
    /// coordinators share its registry, so a *paths-less* overflow on one of
    /// them now rescans the process's watched repos instead of nothing. That
    /// is extra work in a rare degraded case, never a dropped refresh, and it
    /// coalesces on the debouncer and is suppressed downstream when the scan
    /// finds no change.
    fn affected_rescan_roots(event: &Event, registry: &WatchRegistry) -> Vec<PathBuf> {
        if event.paths.is_empty() {
            return registry.root_snapshot();
        }

        let mut roots: Vec<PathBuf> = event
            .paths
            .iter()
            .filter_map(|path| attribute_repo(path, registry))
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }

    /// Schedule one coalesced follow-up `FileEvent::Git` refresh for `repo`,
    /// due no earlier than `delay` from now. Handing it to `schedule_delayed`
    /// (ultimately `DebounceEngine::handle_event_due`) rather than notifying
    /// directly means:
    ///
    /// - the follow-up coalesces with any other pending Git work for the same
    ///   repo — the debouncer keys purely by `(Git, repo)`, so a burst of
    ///   directory-creates (e.g. `mkdir -p a/b/c`) collapses to one flush; and
    /// - the downstream `status_changed` no-op gate still suppresses the
    ///   signal when the follow-up scan finds no diff.
    ///
    /// Synchronous, and it spawns nothing: it records a due time on the
    /// debouncer's in-memory entry and returns. The waiting is done by the one
    /// flush thread `WatchCoordinator` already owns, so a checkout that creates
    /// hundreds of directories costs hundreds of map updates rather than
    /// hundreds of sleeping threads, and nothing survives `stop` (#569).
    fn schedule_directory_followup(
        schedule_delayed: &DelayedEventScheduler,
        repo: PathBuf,
        dir_path: PathBuf,
        delay: Duration,
    ) {
        // `Instant + Duration` panics on overflow; degrading to "no floor
        // beyond now" is the harmless answer for a case that cannot occur
        // with a debounce-window-sized delay.
        let due = Instant::now()
            .checked_add(delay)
            .unwrap_or_else(Instant::now);
        schedule_delayed(PendingEvent::git(repo, dir_path), due);
    }

    /// The backend currently armed. Test-only observability for the #442
    /// swap; production code learns the same thing from the debug log.
    #[cfg(test)]
    fn backend_kind(&self) -> Option<BackendKind> {
        self.backend.lock().ok().map(|guard| guard.kind)
    }

    /// Swap to the polling backend on demand, bypassing the health probe.
    /// Test-only entry point onto the exact production swap path (#442).
    #[cfg(test)]
    fn force_poll_fallback(&self, poll_interval: Duration) {
        if let Some(event_tx) = self.event_tx.as_ref() {
            Self::engage_poll_fallback(
                &self.backend,
                event_tx,
                &self.stop_flag,
                poll_interval,
                &self.registry,
            );
        }
    }

    /// Stop the file watcher and cleanup
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);

        // Drop the watcher first to stop sending events. Scoped so the lock is
        // released before the joins below: the probe thread may be blocked on
        // this very mutex mid-swap, and joining it while holding the guard
        // would deadlock (#442).
        if let Ok(mut backend_guard) = self.backend.lock() {
            backend_guard.watcher = None;
        }

        // Drop the sender to stop the background thread
        self.event_tx.take();

        // Wait for the threads to finish. The probe observes `stop_flag` on
        // every iteration, so this returns promptly rather than blocking for
        // the full probe timeout.
        if let Some(handle) = self.probe_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Actively determine whether the OS-native notify backend actually delivers
/// events on this machine (#442).
///
/// The check is ACTIVE rather than passive because "no events arrived" is
/// otherwise indistinguishable from "nothing changed": passive observation
/// would need to know that a watched tree definitely changed, which the agent
/// cannot know without doing the very scanning this machinery exists to
/// avoid. So the probe manufactures a change it fully controls.
///
/// Pollution is avoided structurally, not by filtering:
///
/// - The sentinel lives in a private directory under the system temp dir, so
///   it is outside every watched repository. Even if it somehow reached
///   `process_event`, there would be no repo to attribute it to.
/// - It is watched by a SEPARATE `RecommendedWatcher` on a SEPARATE channel,
///   so probe events never enter the real event stream at all.
///
/// Returns `true` (healthy — do not swap) whenever the probe itself cannot be
/// set up. A broken probe must never be allowed to demote a working native
/// backend to polling.
fn native_backend_delivers_events(timeout: Duration, stop_flag: &AtomicBool) -> bool {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_u128, |since| since.as_nanos());
    let probe_dir =
        std::env::temp_dir().join(format!("gpy-watch-probe-{}-{nanos}", std::process::id()));
    if std::fs::create_dir_all(&probe_dir).is_err() {
        return true;
    }
    // macOS resolves `/var` -> `/private/var`; watch the canonical form so the
    // probe measures the same path shape the real watches use.
    let canonical_dir = std::fs::canonicalize(&probe_dir).unwrap_or_else(|_| probe_dir.clone());

    let delivered = probe_sentinel_event(&canonical_dir, timeout, stop_flag);

    let _ = std::fs::remove_dir_all(&probe_dir);
    delivered
}

/// Watch `probe_dir` with a throwaway native watcher, repeatedly touch a
/// sentinel file inside it, and report whether ANY event came back before
/// `timeout` elapsed.
fn probe_sentinel_event(probe_dir: &Path, timeout: Duration, stop_flag: &AtomicBool) -> bool {
    let (probe_tx, probe_rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();
    let Ok(mut probe_watcher) = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let _ = probe_tx.send(event);
            }
        },
        notify::Config::default(),
    ) else {
        return true;
    };
    if probe_watcher
        .watch(probe_dir, RecursiveMode::NonRecursive)
        .is_err()
    {
        return true;
    }

    let sentinel = probe_dir.join("sentinel");
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    let mut touch = 0_u64;
    while Instant::now() < deadline {
        if stop_flag.load(Ordering::Relaxed) {
            return true;
        }
        touch = touch.saturating_add(1_u64);
        if std::fs::write(&sentinel, touch.to_string().as_bytes()).is_err() {
            return true;
        }
        // `Ok` means the backend is alive; `Disconnected` means the probe
        // watcher died and can no longer answer the question — both report
        // "healthy" so a broken probe never demotes a working backend.
        if !matches!(
            probe_rx.recv_timeout(PROBE_TOUCH_INTERVAL),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            return true;
        }
    }

    false
}

impl Drop for FileSystemWatcher {
    fn drop(&mut self) {
        // Ensure cleanup happens even if stop() wasn't called
        self.stop();
    }
}

fn is_in_git_dir(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == OsStr::new(".git"))
}

/// Whether `worktree_git_repo` may accept `RepoAttribution::ParentGuess` — a
/// bare "assume the parent directory is a repo" guess with no `.git`
/// evidence — or must require a genuine repo match (#443).
///
/// A newtype enum rather than a second `bool` parameter on
/// `worktree_git_repo`: clippy's `fn_params_excessive_bools` flags more than
/// one `bool` parameter, and the two flags there (`worktree_enabled`, this
/// one) read as unrelated toggles rather than a pair, so folding them into
/// one enum would only obscure the call sites.
#[derive(Clone, Copy)]
enum RepoEvidence {
    /// Accept any attribution, including `ParentGuess`. This is the
    /// pre-#443 behavior for the plain worktree-Git arm — preserved exactly,
    /// including the bare parent-directory guess
    /// `event_outside_watched_roots_falls_back_to_ancestor_walk` pins.
    AnyAttribution,
    /// Require `RepoAttribution::is_genuine_repo` (`WatchedRoot` or
    /// `AncestorGitDir`); reject `ParentGuess`. Used by the additional Git
    /// emission for a path already classified as Language/Config: that path
    /// only ever gets ONE chance to be silently skipped (there's no
    /// user-visible "this Language/Config path also isn't a repo" signal
    /// downstream), so it must not fire off the evidence-free `ParentGuess`
    /// case — e.g. the theme/config watcher's `~/.config/gpy/config.toml`,
    /// which has no `.git` anywhere above it and isn't a repository at all.
    GenuineRepoOnly,
}

/// Determine whether `path` qualifies for a worktree-attributed Git refresh.
///
/// Applies every existing worktree eligibility filter: `worktree_enabled`, the event-kind
/// allowlist, `!is_in_git_dir`, a genuine repo attribution (`attribute_repo_detailed`), the
/// high-churn ignore-token check scoped to that attribution, and the repo's
/// `.gitignore`/`.git/info/exclude` rules.
///
/// Returns the attributed repo root on success.
///
/// Extracted (#443) so the plain worktree-Git arm in `process_event` and the
/// additional Git emission for a path independently classified as
/// Language/Config share one implementation and cannot drift — a gitignored
/// `Cargo.toml`, for instance, must not trigger a git refresh via either
/// path. See [`RepoEvidence`] for what `repo_evidence` controls and why each
/// call site passes what it does.
fn worktree_git_repo(
    event: &Event,
    path: &Path,
    registry: &Arc<WatchRegistry>,
    worktree_enabled: bool,
    repo_evidence: RepoEvidence,
) -> Option<PathBuf> {
    if !worktree_enabled {
        return None;
    }
    // Metadata changes must be accepted here, not just Name/Data/Any (#447).
    // `PollWatcher` — the #442 fallback backend — compares mtime/size rather
    // than observing syscalls, so the ONLY thing it can report for an edit to
    // an existing file is `Modify(Metadata(WriteTime))`. With metadata
    // excluded, the fallback silently dropped every worktree edit and
    // rescued only file creations (`Create`), which defeats the purpose it
    // was added for. `.git` writes masked the gap in testing because those
    // reach `should_trigger_update` before this allowlist ever runs.
    //
    // `AccessTime` is deliberately carved out rather than accepting
    // `Metadata(_)` wholesale: reading a file cannot change `git status`, so
    // a pure atime touch would wake the refresh pipeline for nothing. Every
    // other metadata kind can legitimately change what `git status` reports —
    // notably `Permissions`, since git tracks the executable mode bit.
    if !matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(
                ModifyKind::Name(_)
                    | ModifyKind::Data(_)
                    | ModifyKind::Any
                    | ModifyKind::Metadata(
                        MetadataKind::Any
                            | MetadataKind::WriteTime
                            | MetadataKind::Permissions
                            | MetadataKind::Ownership
                            | MetadataKind::Extended
                            | MetadataKind::Other
                    )
            )
    ) {
        return None;
    }
    if is_in_git_dir(path) {
        return None;
    }
    // Attribution must run *before* the ignore check (#445): the high-churn
    // tokens (`build`, `target`, `node_modules`, ...) are only meaningful
    // relative to a repo root, and that root isn't known until attribution
    // runs. See `path_matches_ignored_patterns` for how the scoping root is
    // chosen.
    let attribution = attribute_repo_detailed(path, registry)?;
    if matches!(repo_evidence, RepoEvidence::GenuineRepoOnly) && !attribution.is_genuine_repo() {
        return None;
    }
    // A directory-name token is only a hint (#719): committed files live
    // under `vendor/`, `dist/`, `build/`, ... so inside a trusted repo root it
    // defers to the gitignore check below. Without a trusted root there is no
    // repo boundary to consult, so the token stays a verdict.
    match path_matches_ignored_patterns(path, attribution.trusted_root()) {
        Some(IgnoreTokenHit::Noise) => return None,
        Some(IgnoreTokenHit::Directory) if attribution.trusted_root().is_none() => return None,
        Some(IgnoreTokenHit::Directory) | None => {}
    }
    let repo = attribution.into_repo();
    // Honor the repo's .gitignore/.git/info/exclude: ignored files never
    // appear in `git status`, so don't wake the prompt for build-dir churn.
    // If this misfilters (e.g. a stale matcher after editing .gitignore),
    // change-detection still suppresses any no-op refresh downstream.
    if path_is_gitignored(&repo, path, registry) {
        return None;
    }
    Some(repo)
}

/// Decide whether a `Create` event denotes a *directory* being created, which
/// is the only case that schedules a follow-up rescan (#416).
///
/// - `CreateKind::Folder` is authoritative (inotify with `IN_ISDIR`), trusted
///   without a stat.
/// - `CreateKind::Any`/`CreateKind::Other` are emitted by backends that don't
///   distinguish file vs. directory (notably macOS `FSEvents`), so fall back to
///   a best-effort `path.is_dir()`. If the directory was already removed or
///   renamed by the time we check, `is_dir()` returns `false` and we simply
///   treat it as "not a directory" — a missed follow-up there is harmless
///   because the racing writes, if any, would be caught by reconcile.
/// - Plain `CreateKind::File` (and every non-`Create` kind) returns `false`
///   with no stat, so the common file-event path pays no extra cost.
fn is_directory_create(kind: EventKind, path: &Path) -> bool {
    match kind {
        EventKind::Create(CreateKind::Folder) => true,
        EventKind::Create(CreateKind::Any | CreateKind::Other) => path.is_dir(),
        _ => false,
    }
}

/// Whether `event` is the poll backend's report that a directory's own mtime
/// moved (#817).
///
/// `PollWatcher` stats directories as well as files, so every create or remove
/// inside one also surfaces as `Modify(Metadata(WriteTime))` on the directory
/// path. The native backends never report that: they emit only the child's
/// event. Delivered as is, a write to a high-churn file inside a significant
/// directory (`.git/rebase-merge/patch`) wakes the repository through the
/// directory's own path. The child event carries the real change, so this one
/// is dropped. Only a `Modify(Metadata(WriteTime))` is tested, so a file's
/// edit -- which poll reports under the same kind -- never pays the stat.
fn is_poll_directory_mtime_noise(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime))
    ) && !event.paths.is_empty()
        && event.paths.iter().all(|path| path.is_dir())
}

/// Result of attributing an event path to a repo.
///
/// Distinguishes a trustworthy registered-watch match from the defensive ancestor-walk
/// fallback, and — within that fallback — a genuine `.git` hit from a bare "assume the parent
/// directory is a repo" guess.
///
/// `path_matches_ignored_patterns` (#445) only cares about the first distinction: only a
/// genuine watched-root match carries a repo boundary worth scoping the high-churn ignore
/// check to. The second distinction (#443) matters to `worktree_git_repo`'s optional stricter
/// mode: emitting an *additional* Git refresh for a path that was already classified as
/// something else (Language/Config) must not fire off `ParentGuess`, since that variant
/// doesn't mean `path` is even inside a repository — see the `ParentGuess` doc comment below.
enum RepoAttribution {
    /// `path` fell under a root that is actually registered in the
    /// registry's watched-root set — i.e. a live notify watch produced this
    /// event from inside that repo. This root is a trustworthy repo boundary.
    WatchedRoot(PathBuf),
    /// No registered watched root was a prefix of `path` (or the registry
    /// tracks none at all, e.g. the theme/config watchers), but an ancestor
    /// `.git` was found by walking up from `path`. `path` is therefore
    /// genuinely inside a repository, even though the match didn't come from
    /// the registry.
    AncestorGitDir(PathBuf),
    /// No registered watched root was a prefix of `path`, and no ancestor
    /// `.git` was found either. The returned path is nothing more than
    /// `path`'s immediate parent directory — a bare guess with no evidence
    /// `path` is inside a repository at all. This is the case that fires for
    /// e.g. the theme/config watcher's `~/.config/gpy/config.toml`, which
    /// has no `.git` anywhere above it. Not a boundary we can trust to scope
    /// the ignore check against, and not evidence good enough to justify an
    /// *additional* Git refresh for a path classified as something else.
    ParentGuess(PathBuf),
}

impl RepoAttribution {
    /// The repo root a `PendingEvent` should be attributed to, regardless of
    /// which variant produced it.
    fn into_repo(self) -> PathBuf {
        match self {
            Self::WatchedRoot(root) | Self::AncestorGitDir(root) | Self::ParentGuess(root) => root,
        }
    }

    /// The repo root to scope `path_matches_ignored_patterns` against, or
    /// `None` when attribution isn't trustworthy enough to scope by (see the
    /// `AncestorGitDir`/`ParentGuess` doc comments above). Unchanged from
    /// pre-#443 behavior: only `WatchedRoot` was ever scoped by, and that
    /// stays true now that the old `Fallback` variant is split in two.
    fn trusted_root(&self) -> Option<&Path> {
        match self {
            Self::WatchedRoot(root) => Some(root.as_path()),
            Self::AncestorGitDir(_) | Self::ParentGuess(_) => None,
        }
    }

    /// Whether `path` is genuinely inside a repository — a `WatchedRoot` or
    /// `AncestorGitDir` hit — as opposed to the `ParentGuess` case, which is
    /// evidence-free (#443).
    const fn is_genuine_repo(&self) -> bool {
        !matches!(self, Self::ParentGuess(_))
    }
}

/// Attribute a raw filesystem event path to the watched repository whose notify watch produced
/// it.
///
/// Reports *how* it was attributed so callers can tell a trustworthy watched-root match from
/// the defensive fallback (see `RepoAttribution`).
///
/// # Longest-prefix match (the fast path, zero stats)
///
/// notify watches are registered per repository root (see
/// `MultiRepoWatcher::register_client`), so a live event's path is always a
/// descendant of *some* watched root. Among the watched roots that are a
/// prefix of `path`, the LONGEST one is picked as the owner:
///
/// - If a nested repo (e.g. a submodule) is *also* watched, its root is a
///   longer prefix and wins here, matching what an ancestor `.git` stat walk
///   would find by climbing up from the event path.
/// - If the nested repo is *not* watched, no notify watch is registered for
///   it, so its events are never delivered in the first place. The longest
///   watched prefix is then the *outer* root, which is exactly the watch
///   that produced the event. An ancestor stat walk would instead find the
///   inner (unwatched) repo — attributing the event to a repository the
///   watcher isn't even tracking.
///
/// The watched set is a handful of repos per agent, so this is a plain
/// linear scan over already-known roots rather than a trie or other
/// specialized structure. It performs zero filesystem stats: the watched-root
/// set is populated at repo-registration time, not looked up here.
///
/// # Fallback (defensive only)
///
/// When no watched root is a prefix of `path` — or the registry tracks no
/// roots at all (e.g. the theme/config watchers, whose registry nothing ever
/// registers a repo on) — this falls back to exactly the previous
/// ancestor-walk behavior. Real events from a live notify watch should always
/// land under a registered root, so this path should rarely execute for repo
/// watchers.
fn attribute_repo_detailed(path: &Path, registry: &WatchRegistry) -> Option<RepoAttribution> {
    if let Some(root) = registry.longest_root_prefix(path) {
        return Some(RepoAttribution::WatchedRoot(root));
    }

    if let Some(root) = MultiRepoWatcher::find_git_root(path) {
        return Some(RepoAttribution::AncestorGitDir(root));
    }

    path.parent()
        .map(Path::to_path_buf)
        .map(RepoAttribution::ParentGuess)
}

/// Attribute `path` to its repo root, discarding the provenance `attribute_repo_detailed`
/// tracks.
///
/// Used by callers (e.g. rescan attribution) that only need the resulting root, not whether it
/// came from a trustworthy watched-root match or the defensive fallback.
fn attribute_repo(path: &Path, registry: &WatchRegistry) -> Option<PathBuf> {
    attribute_repo_detailed(path, registry).map(RepoAttribution::into_repo)
}

fn ignored_tokens() -> &'static [String] {
    static TOKENS: OnceLock<Vec<String>> = OnceLock::new();
    TOKENS.get_or_init(|| {
        get_ignore_patterns()
            .into_iter()
            .map(|pattern| pattern.trim_matches('*').trim_matches('/').to_owned())
            .filter(|token| !token.is_empty())
            .collect()
    })
}

/// Check whether `path` matches one of the built-in high-churn ignore
/// tokens (`node_modules`, `build`, `target`, ...).
///
/// `repo_root` scopes the check (#445): the tokens are meant to describe
/// directories *inside* a repository, not the path an ancestor directory
/// happens to be checked out under (e.g. a repo cloned to
/// `/work/build/my-repo` must not have every worktree event suppressed just
/// because `build` sits above the repo root). When `repo_root` is `Some`,
/// only the portion of `path` relative to that root is tested.
///
/// `repo_root` is deliberately `None` — and the *whole* absolute path is
/// tested, matching pre-#445 behavior — whenever attribution didn't resolve
/// to a genuine registered watched root (see `RepoAttribution::AncestorGitDir`
/// / `RepoAttribution::ParentGuess`):
///
/// - The theme/config watchers never populate `watched_roots` at all, so
///   every path they see attributes via the ancestor-walk/parent fallback.
///   There is no repo concept there to scope against, so component matching
///   over the full path is the only sensible behavior, and is what they've
///   always relied on.
/// - When a repo watcher sees a path outside every registered root, the
///   fallback-derived "root" (an ancestor `.git`, or just the parent
///   directory) isn't a boundary the registry vouches for — it could easily
///   be one directory short or one too many. Trusting it as a scoping root
///   would be trading one kind of misattribution for another, so the
///   whole-path check is kept as the safe default.
///
/// Directory-name tokens are a hint, not a verdict (#719): the result says
/// *which* kind of token hit so `worktree_git_repo` can defer directory hits
/// to the repo's own ignore rules (committed `vendor/`/`dist/` files must
/// still refresh the prompt) when `repo_root` is trusted.
fn path_matches_ignored_patterns(path: &Path, repo_root: Option<&Path>) -> Option<IgnoreTokenHit> {
    let scoped_path = repo_root.map_or(path, |root| path.strip_prefix(root).unwrap_or(path));
    let path_str = scoped_path.to_string_lossy();
    let file_name = scoped_path.file_name();
    let mut directory_hit = false;

    for token in ignored_tokens() {
        if token.contains('/') {
            if path_str.contains(token) {
                return Some(IgnoreTokenHit::Noise);
            }
            continue;
        }

        let desired = OsStr::new(token);
        let hit = file_name == Some(desired)
            || scoped_path
                .components()
                .any(|component| component.as_os_str() == desired);
        if !hit {
            continue;
        }
        if is_noise_file_token(token) {
            return Some(IgnoreTokenHit::Noise);
        }
        directory_hit = true;
    }

    directory_hit.then_some(IgnoreTokenHit::Directory)
}

/// Which kind of built-in token `path_matches_ignored_patterns` hit (#719).
enum IgnoreTokenHit {
    /// A directory-name token (`vendor`, `dist`, `build`, ...). Only a hint:
    /// committed files legitimately live under these names, so a hit inside a
    /// genuine repo defers to the repo's own ignore rules.
    Directory,
    /// Never-meaningful noise (`.DS_Store`, `Thumbs.db`, `.git/objects`,
    /// `.git/logs`): suppressed unconditionally.
    Noise,
}

/// Whether a bare (slash-free) token names an OS metadata file rather than a
/// directory.
fn is_noise_file_token(token: &str) -> bool {
    matches!(token, ".DS_Store" | "Thumbs.db")
}

/// Fingerprint of an ignore-source file (`.gitignore`, `.git/info/exclude`),
/// or an index file: modified time and length, or `None` when the file is
/// absent/unreadable.
///
/// Combining mtime and length catches both edits and create/remove transitions,
/// even on filesystems with coarse mtime resolution where a same-second edit
/// would otherwise look unchanged.
type FileFingerprint = Option<(std::time::SystemTime, u64)>;

/// Fingerprint of the full ancestor `.gitignore` chain backing a cached matcher, so the cache
/// can rebuild it when any file in the chain changes.
///
/// Ordered: `[root .gitignore, .git/info/exclude, then each nested dir's .gitignore
/// root->leaf]`.
type NestedIgnoreSignature = Vec<FileFingerprint>;

/// A compiled ignore matcher together with the source fingerprint it was built
/// from, so the cache can rebuild it when the underlying files change.
struct CachedIgnore {
    matcher: Arc<Gitignore>,
    signature: NestedIgnoreSignature,
}

/// Cache of compiled `.gitignore` matchers, keyed by the *directory* whose ancestor chain the
/// matcher was compiled from.
///
/// Not the repo root, so files sharing a directory share one matcher while distinct nested
/// directories get their own nested-aware matcher.
///
type RepoIgnoreCache = Mutex<HashMap<PathBuf, CachedIgnore>>;

/// The two ignore-related caches, owned by a [`WatchRegistry`] rather than by
/// the process (#617).
///
/// Both are pure memoization of filesystem state keyed by path, so sharing
/// them process-wide was never a correctness requirement — only a convenience
/// that forced every test touching them to be `#[serial]`. A watcher's own
/// caches are warmed by its own events.
#[derive(Default)]
pub(crate) struct IgnoreCaches {
    /// Per-directory cache of compiled `.gitignore` matchers.
    repo_ignore: RepoIgnoreCache,
    /// Per-repo cache of force-added file sets.
    force_added: ForceAddedCache,
}

/// A cached force-added set (tracked files that also match ignore rules)
/// together with the `.git/index` fingerprint it was derived from.
struct CachedForceAdded {
    files: Arc<HashSet<PathBuf>>,
    index_signature: FileFingerprint,
    /// `false` until a `git ls-files` run has actually filled `files`, so a
    /// placeholder inserted before the first run is never mistaken for a
    /// genuinely empty set.
    populated: bool,
    /// `true` while a run is in flight for this repo. Without it every
    /// ignored-path event arriving after an index write would start its own
    /// `git ls-files` (#568).
    refreshing: bool,
}

impl CachedForceAdded {
    /// An empty, not-yet-computed entry. `index_signature` is deliberately
    /// `None` so it never compares equal to a real fingerprint.
    fn placeholder() -> Self {
        Self {
            files: Arc::new(HashSet::new()),
            index_signature: None,
            populated: false,
            refreshing: false,
        }
    }
}

/// What [`force_added_set`] must do about the entry it just inspected.
enum ForceAddedRefresh {
    /// Entry is current, or a run is already in flight: serve what we have.
    None,
    /// No set has ever been computed for this repo, so there is nothing stale
    /// to serve — run now, on this thread, with no lock held.
    Blocking,
    /// The index changed under a populated entry: serve the stale set and
    /// refresh off-thread (#568).
    Background,
}

/// Cache of force-added sets, keyed by repo root.
type ForceAddedCache = Mutex<HashMap<PathBuf, CachedForceAdded>>;

/// Fingerprint a single file (ignore source or index). Absent or unreadable
/// files map to `None`, so create/remove transitions are observable.
fn fingerprint_file(path: &Path) -> FileFingerprint {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some((modified, metadata.len()))
}

/// Collect the directory chain from `repo_root` down to `dir` (inclusive), in
/// root->leaf order. Each returned directory is where a nested `.gitignore`
/// could live.
///
/// If `dir` is not a descendant of `repo_root` (defensive; real events always
/// land under a watched root), the chain degrades to just `[repo_root]`, which
/// reproduces the historical root-only behavior.
fn ancestor_gitignore_dirs(repo_root: &Path, dir: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = Some(dir);
    while let Some(component_dir) = current {
        chain.push(component_dir.to_path_buf());
        if component_dir == repo_root {
            break;
        }
        current = component_dir.parent();
    }

    if chain.last().map(PathBuf::as_path) != Some(repo_root) {
        // `dir` is not under `repo_root`; fall back to the root chain only.
        return vec![repo_root.to_path_buf()];
    }

    chain.reverse();
    chain
}

/// Compute the combined fingerprint of the ancestor `.gitignore` chain.
///
/// Order mirrors [`build_nested_ignore`]: root `.gitignore`, then
/// `.git/info/exclude`, then each nested directory's `.gitignore` walking
/// root->leaf. `chain[0]` is `repo_root`, whose `.gitignore` is fingerprinted
/// once via the first entry — nested entries start at `chain[1..]`.
fn nested_ignore_signature(repo_root: &Path, chain: &[PathBuf]) -> NestedIgnoreSignature {
    let mut signature = Vec::with_capacity(chain.len().saturating_add(1));
    signature.push(fingerprint_file(&repo_root.join(".gitignore")));
    signature.push(fingerprint_file(&repo_root.join(".git/info/exclude")));
    for dir in chain.iter().skip(1) {
        signature.push(fingerprint_file(&dir.join(".gitignore")));
    }
    signature
}

/// Build a nested-aware `.gitignore` matcher from the ancestor chain.
///
/// Sources are added lowest-precedence first so deeper rules (including
/// negations that re-include a path) override shallower ones: root `.gitignore`,
/// then `.git/info/exclude`, then each nested directory's `.gitignore` from just
/// below `repo_root` down to the leaf directory. All sources are added to one
/// `GitignoreBuilder` rooted at `repo_root`. Returns an empty matcher (ignores
/// nothing) on error.
fn build_nested_ignore(repo_root: &Path, chain: &[PathBuf]) -> Gitignore {
    let mut builder = GitignoreBuilder::new(repo_root);
    let _ = builder.add(repo_root.join(".gitignore"));
    let _ = builder.add(repo_root.join(".git/info/exclude"));
    for dir in chain.iter().skip(1) {
        let _ = builder.add(dir.join(".gitignore"));
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Cap a matcher/force-added cache to [`IGNORE_CACHE_CAPACITY`], never evicting
/// the `keep` key just populated. Evicted entries are cheaply rebuilt on the
/// next cache miss.
fn evict_cache_overflow<V>(cache: &mut HashMap<PathBuf, V>, keep: &Path) {
    if cache.len() <= IGNORE_CACHE_CAPACITY {
        return;
    }
    let excess = cache.len().saturating_sub(IGNORE_CACHE_CAPACITY);
    let to_remove: Vec<PathBuf> = cache
        .keys()
        .filter(|k| k.as_path() != keep)
        .take(excess)
        .cloned()
        .collect();
    for k in to_remove {
        cache.remove(&k);
    }
}

/// Look up (refreshing on a fingerprint change) the force-added set for
/// `repo_root`: tracked files that also match ignore rules (`git add -f`).
///
/// The `git ls-files` subprocess runs ONLY when the `.git/index` fingerprint
/// (mtime+len) changes — a force-add, commit, or any index write. On the common
/// path (index unchanged) this is a single `fs::metadata` stat plus a cache
/// read, no subprocess. A lock failure fails open to an empty set (the path is
/// then suppressed exactly as before this fix).
///
/// The cache mutex is shared by every repo this watcher tracks, so it is held
/// only long enough to read and update the entry's bookkeeping — never across
/// the subprocess (#568). When the index
/// has moved under an already-populated entry, the stale set is served straight
/// back and one background thread refreshes it; every other event for that repo
/// meanwhile sees `refreshing` and neither waits nor starts a second run. Only
/// the very first lookup for a repo, which has no stale set to serve, runs the
/// subprocess on the calling thread, never under the cache mutex: a slow repo
/// still cannot stall any other repo's lookup, and the answer is correct from
/// the first event (a force-added file must not be suppressed even once). That
/// is one run per repo per agent lifetime; the repeating case this bug was
/// about — every commit or `git add` re-firing it — is the background one.
fn force_added_set(repo_root: &Path, registry: &Arc<WatchRegistry>) -> Arc<HashSet<PathBuf>> {
    let index_signature = fingerprint_file(&repo_root.join(".git/index"));

    let (files, refresh) = {
        let Ok(mut cache) = registry.ignore().force_added.lock() else {
            return Arc::new(HashSet::new());
        };
        let cached = cache
            .entry(repo_root.to_path_buf())
            .or_insert_with(CachedForceAdded::placeholder);
        let refresh = if cached.refreshing {
            ForceAddedRefresh::None
        } else if !cached.populated {
            cached.refreshing = true;
            ForceAddedRefresh::Blocking
        } else if cached.index_signature == index_signature {
            ForceAddedRefresh::None
        } else {
            cached.refreshing = true;
            ForceAddedRefresh::Background
        };
        let files = Arc::clone(&cached.files);
        evict_cache_overflow(&mut cache, repo_root);
        (files, refresh)
    };

    match refresh {
        ForceAddedRefresh::None => files,
        ForceAddedRefresh::Blocking => {
            refresh_force_added(repo_root, &registry.ignore().force_added).unwrap_or(files)
        }
        ForceAddedRefresh::Background => {
            let repo_root_owned = repo_root.to_path_buf();
            // An owned registry handle, not a borrow: this thread outlives the
            // call, and the caches now belong to the registry rather than to
            // the process (#617).
            let registry_for_refresh = Arc::clone(registry);
            thread::spawn(move || {
                drop(refresh_force_added(
                    &repo_root_owned,
                    &registry_for_refresh.ignore().force_added,
                ));
            });
            files
        }
    }
}

/// Run `git ls-files` for `repo_root` and store the result in the cache.
///
/// The caller must have claimed the entry by setting `refreshing`; this clears
/// it. **No lock is held while the subprocess runs** — the mutex is taken only
/// afterwards, to store the result (#568). Returns `None` if the cache mutex is
/// poisoned, leaving the caller to serve whatever it already had.
///
/// The fingerprint is taken *before* the run, so an index write that races the
/// run leaves the entry stale and it is recomputed on the next lookup rather
/// than being recorded as current.
fn refresh_force_added(
    repo_root: &Path,
    cache_slot: &ForceAddedCache,
) -> Option<Arc<HashSet<PathBuf>>> {
    let index_signature = fingerprint_file(&repo_root.join(".git/index"));
    let files: Arc<HashSet<PathBuf>> = Arc::new(
        compute_tracked_ignored_files(repo_root)
            .into_iter()
            .collect(),
    );

    let mut cache = cache_slot.lock().ok()?;
    let cached = cache
        .entry(repo_root.to_path_buf())
        .or_insert_with(CachedForceAdded::placeholder);
    cached.files = Arc::clone(&files);
    cached.index_signature = index_signature;
    cached.populated = true;
    cached.refreshing = false;
    evict_cache_overflow(&mut cache, repo_root);
    drop(cache);
    Some(files)
}

/// Production path: the real `git ls-files` run via [`NativeGitBackend`].
///
/// Tests can override this per-repo via `set_tracked_ignored_hook_for_test` to
/// stand in a slow or instrumented backend without spawning a real `git`
/// process (#568). The hook table is keyed by repo root rather than a single
/// global slot so tests using distinct temp-repo roots stay independent.
fn compute_tracked_ignored_files(repo_root: &Path) -> Vec<PathBuf> {
    #[cfg(test)]
    {
        // The hook is cloned out and the table's lock released before it runs,
        // so a slow stand-in cannot serialise other repos through the harness
        // either.
        let hook = tracked_ignored_test_hooks()
            .lock()
            .ok()
            .and_then(|hooks| hooks.get(repo_root).map(Arc::clone));
        if let Some(hook_fn) = hook {
            return hook_fn(repo_root);
        }
    }
    NativeGitBackend::tracked_ignored_files(repo_root)
}

/// A test stand-in for `git ls-files`, registered per repo root.
#[cfg(test)]
type TrackedIgnoredHook = Arc<dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync>;

/// Per-repo table of test stand-ins consulted by [`compute_tracked_ignored_files`].
#[cfg(test)]
fn tracked_ignored_test_hooks() -> &'static Mutex<HashMap<PathBuf, TrackedIgnoredHook>> {
    static HOOKS: OnceLock<Mutex<HashMap<PathBuf, TrackedIgnoredHook>>> = OnceLock::new();
    HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Install a stand-in backend for `repo_root`, replacing any previous one.
#[cfg(test)]
fn set_tracked_ignored_hook_for_test(
    repo_root: &Path,
    hook: impl Fn(&Path) -> Vec<PathBuf> + Send + Sync + 'static,
) {
    if let Ok(mut hooks) = tracked_ignored_test_hooks().lock() {
        hooks.insert(repo_root.to_path_buf(), Arc::new(hook));
    }
}

/// Remove the stand-in backend for `repo_root`, restoring the real one.
#[cfg(test)]
fn clear_tracked_ignored_hook_for_test(repo_root: &Path) {
    if let Ok(mut hooks) = tracked_ignored_test_hooks().lock() {
        hooks.remove(repo_root);
    }
}

/// Returns `true` when `path` is a force-added tracked file (`git add -f` on an
/// otherwise-ignored path), which must keep refreshing the prompt even though it
/// matches an ignore rule.
///
/// The set is normally empty (force-adds are rare); the empty case short-circuits
/// before the canonicalizing stat, keeping the common path to a fingerprint stat
/// plus an empty-set check.
fn is_force_added(repo_root: &Path, path: &Path, registry: &Arc<WatchRegistry>) -> bool {
    let set = force_added_set(repo_root, registry);
    if set.is_empty() {
        return false;
    }
    // Match the canonical form stored by `tracked_ignored_files`.
    let candidate = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    set.contains(&candidate)
}

/// Returns `true` when `path` is ignored by the repo's `.gitignore` rules AND is
/// not a force-added tracked file.
///
/// The matcher is nested-aware: it is compiled from the ancestor `.gitignore`
/// chain (root `.gitignore` + `.git/info/exclude` + each nested directory's
/// `.gitignore` from `repo_root` down to the path's directory), so nested rules
/// — including negations that re-include a path — are honored (issue #435).
///
/// Matching uses [`Gitignore::matched_path_or_any_parents`] so a file whose
/// *parent directory* is ignored (git's real behavior: `build/` ignores
/// everything under `subdir/build/`) is classified correctly, and a nested
/// negation re-including that directory flips the whole subtree. Plain
/// `matched` only inspects the leaf's own name and would miss both.
///
/// Caching keeps the per-event cost at ~µs on the common path:
/// - The matcher is cached keyed by the path's *directory* and rebuilt only when
///   the ancestor-chain fingerprint (mtime+len per `.gitignore`) changes, so a
///   nested `.gitignore` edit is picked up without an agent restart (issue #152,
///   #435) while an unchanged chain costs one `fs::metadata` stat per ancestor
///   plus a cheap in-memory `matched_path_or_any_parents` lookup — never a
///   rebuild.
/// - Only after the matcher reports "ignored" is the force-added set consulted;
///   its `git ls-files` subprocess runs solely on a `.git/index` fingerprint
///   change (see [`force_added_set`]), never per event.
fn path_is_gitignored(repo_root: &Path, path: &Path, registry: &Arc<WatchRegistry>) -> bool {
    let dir = path.parent().unwrap_or(repo_root);
    let chain = ancestor_gitignore_dirs(repo_root, dir);
    let signature = nested_ignore_signature(repo_root, &chain);
    let matcher = {
        let Ok(mut cache) = registry.ignore().repo_ignore.lock() else {
            return false;
        };
        let cached = cache
            .entry(dir.to_path_buf())
            .or_insert_with(|| CachedIgnore {
                matcher: Arc::new(build_nested_ignore(repo_root, &chain)),
                signature: signature.clone(),
            });
        if cached.signature != signature {
            cached.matcher = Arc::new(build_nested_ignore(repo_root, &chain));
            cached.signature.clone_from(&signature);
        }
        let matcher = Arc::clone(&cached.matcher);
        evict_cache_overflow(&mut cache, dir);
        matcher
    };
    if !matcher
        .matched_path_or_any_parents(path, path.is_dir())
        .is_ignore()
    {
        return false;
    }
    // A `git add -f`'d file matches an ignore glob but is tracked, so its edits
    // still change `git status` and must refresh the prompt (issue #435).
    !is_force_added(repo_root, path, registry)
}

/// Built-in high-churn hints for build/dependency directories and OS noise.
///
/// In a trusted repo a directory-name hit only defers to the `.gitignore`
/// matcher (#719); `.DS_Store`, `Thumbs.db` and the `.git/{objects,logs}`
/// entries are suppressed outright.
///
/// These are matched as exact path components, so only well-known build-output
/// directory names qualify. Generic words (`tmp`, `temp`, `env`) are deliberately
/// excluded: they collide with ordinary path components (e.g. repos under `/tmp`,
/// or a tracked `env/` config dir). Anything not listed here defers to the repo's
/// `.gitignore`, which is the authoritative source of what to ignore.
#[must_use]
fn get_ignore_patterns() -> Vec<&'static str> {
    vec![
        // Node.js
        "**/node_modules/**",
        "**/dist/**",
        "**/build/**",
        "**/.next/**",
        // Rust
        "**/target/**",
        // Go
        "**/vendor/**",
        // Python
        "**/__pycache__/**",
        "**/venv/**",
        "**/.venv/**",
        // General
        "**/.git/objects/**",
        "**/.git/logs/**",
        "**/.DS_Store",
        "**/Thumbs.db",
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use crate::watcher::{FileEvent, GitPaths, PendingCallback, PendingEvent};
    use notify::event::{
        AccessKind, AccessMode, CreateKind, DataChange, EventAttributes, EventKind, Flag,
        MetadataKind, ModifyKind, RenameMode,
    };
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Follow-up delay used by `process_event` in tests.
    ///
    /// Short so the directory-create follow-up thread fires promptly, but long enough that a
    /// non-directory event has clearly *not* scheduled one by the time we poll.
    const TEST_FOLLOWUP_DELAY: Duration = Duration::from_millis(50);

    /// A private [`WatchRegistry`] for one test, with `worktree_enabled`
    /// applied and `roots` as its watched-root set.
    ///
    /// The per-test replacement for the five process-global registries #617
    /// retired: the old `Option<&WatchedRoots>` handle plus the process-wide
    /// worktree flag, gitdir maps and ignore caches. An empty set is what the
    /// old `None` handle meant — no repo registry backs this watcher — and
    /// leaves every registry-consulting arm of `process_event` a no-op, so
    /// attribution falls through to the same ancestor `.git` walk.
    fn test_registry(worktree_enabled: bool, roots: HashSet<PathBuf>) -> Arc<WatchRegistry> {
        let registry = Arc::new(WatchRegistry::new());
        registry.set_worktree_enabled(worktree_enabled);
        for root in roots {
            registry.insert_root(root);
        }
        registry
    }

    /// Capture events emitted when running the provided callback.
    ///
    /// # Panics
    ///
    /// Panics if the captured event buffer cannot be extracted.
    fn run_with_capture<F: FnOnce(Arc<Mutex<Vec<PendingEvent>>>)>(call: F) -> Vec<PendingEvent> {
        let captured = Arc::new(Mutex::new(Vec::new()));
        call(Arc::clone(&captured));
        Arc::try_unwrap(captured).unwrap().into_inner().unwrap()
    }

    /// Reading a watched file must not look like writing it (#551).
    ///
    /// inotify arms every watch with `WatchMask::OPEN`, so a reload's own read of
    /// the file that triggered it comes straight back as `Access(Open)`. Before
    /// the gate in `process_event`, that classified as a change and reloaded
    /// again, so one theme or config edit on Linux left the manager reloading —
    /// and signalling every registered shell — about every 57ms, forever.
    ///
    /// # Panics
    ///
    /// Panics if the mocked callback cannot record the triggered event.
    #[test]
    fn pure_access_events_are_not_changes() {
        let registry = test_registry(true, HashSet::new());
        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            EventKind::Access(AccessKind::Read),
        ] {
            let events = run_with_capture(|captured| {
                let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                    captured.lock().unwrap().push(event);
                }));

                let evt = Event {
                    kind,
                    paths: vec![PathBuf::from("/repo/.git/HEAD")],
                    attrs: EventAttributes::default(),
                };

                FileSystemWatcher::process_event(
                    &evt,
                    &callback_arc,
                    &registry,
                    TEST_FOLLOWUP_DELAY,
                    &no_op_scheduler(),
                );
            });

            assert!(
                events.is_empty(),
                "{kind:?} is a read, not a change, and must not trigger an update"
            );
        }
    }

    /// The gate above drops reads only: inotify raises one event per mask bit, so
    /// the write that accompanies a `Close(Write)` still arrives as its own
    /// `Modify(Data)` and must still classify (#551).
    ///
    /// # Panics
    ///
    /// Panics if the mocked callback cannot record the triggered event.
    #[test]
    fn write_alongside_a_close_still_triggers() {
        let registry = test_registry(true, HashSet::new());
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                paths: vec![PathBuf::from("/repo/.git/HEAD")],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(events.len(), 1, "the write half must still be delivered");
    }

    /// # Panics
    ///
    /// Panics if the mocked callback cannot record the triggered event.
    #[test]
    fn gitdir_changes_trigger_without_aggressive_mode() {
        let registry = test_registry(true, HashSet::new());
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
                paths: vec![PathBuf::from("/repo/.git/HEAD")],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(events.len(), 1, "Expected gitdir event to trigger");
        assert!(matches!(
            events.first().map(|e| &e.event),
            Some(FileEvent::Git { .. })
        ));
    }

    /// # Panics
    ///
    /// Panics if the mocked callback cannot record the triggered event.
    #[test]
    fn worktree_events_skipped_when_disabled() {
        let registry = test_registry(false, HashSet::new());
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Create(CreateKind::File),
                paths: vec![PathBuf::from("/repo/new_file.txt")],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert!(
            events.is_empty(),
            "Worktree events should be skipped when watching is disabled"
        );
    }

    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the event fails.
    #[test]
    fn worktree_content_edit_triggers_when_enabled() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let file = repo.join("tracked.txt");
        std::fs::write(&file, "data").expect("write file");

        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(
            events.len(),
            1,
            "a tracked-file content edit should emit one git event"
        );
        assert!(matches!(
            events.first().map(|e| &e.event),
            Some(FileEvent::Git { .. })
        ));
    }

    /// Regression test for #443: a worktree file with BOTH language/config significance AND
    /// git-status significance must emit a Git refresh in ADDITION to the specialized event,
    /// not instead of it.
    ///
    /// Before the fix, `should_trigger_update` classified the path, `process_event` emitted
    /// only that specialized event, and immediately `break`d — so the same filesystem mutation
    /// never reached the git-status refresh, leaving the git instant cache stale until another
    /// Git event, a stale-cache pull, or the ~45s reconcile.
    ///
    /// Shared by the language-manifest, lockfile, and config-like-filename
    /// cases below so they can't drift from one another.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the events fails.
    fn assert_worktree_edit_emits_both_specialized_and_git(
        filename: &str,
        is_expected_specialized: impl Fn(&FileEvent) -> bool,
    ) {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let file = repo.join(filename);
        std::fs::create_dir_all(file.parent().expect("file parent")).expect("parent dirs");
        std::fs::write(&file, "data").expect("write file");

        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(
            events.len(),
            2,
            "editing {filename} should emit both the specialized event and a \
             Git refresh, got: {events:?}"
        );
        assert!(
            events.iter().any(|e| is_expected_specialized(&e.event)),
            "expected the specialized event for {filename}, got: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e.event, FileEvent::Git { .. })),
            "expected an additional Git refresh event for {filename} so the \
             git instant cache doesn't go stale, got: {events:?}"
        );
    }

    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the events fails.
    #[test]
    fn worktree_language_manifest_edit_also_emits_git_refresh() {
        assert_worktree_edit_emits_both_specialized_and_git("Cargo.toml", |event| {
            matches!(event, FileEvent::Language { .. })
        });
    }

    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the events fails.
    #[test]
    fn worktree_lockfile_edit_also_emits_git_refresh() {
        assert_worktree_edit_emits_both_specialized_and_git("Cargo.lock", |event| {
            matches!(event, FileEvent::Language { .. })
        });
    }

    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the events fails.
    #[test]
    fn worktree_config_like_filename_edit_also_emits_git_refresh() {
        assert_worktree_edit_emits_both_specialized_and_git("config.toml", |event| {
            matches!(event, FileEvent::Config { .. })
        });
    }

    /// A tracked gpy theme inside a dotfiles repository (`.config/gpy/themes/a.toml`)
    /// emits a Git refresh as well as the `Theme` signal (#777).
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the events fails.
    #[test]
    fn tracked_theme_file_in_repo_also_emits_git() {
        assert_worktree_edit_emits_both_specialized_and_git(".config/gpy/themes/a.toml", |event| {
            matches!(event, FileEvent::Theme { .. })
        });
    }

    /// A `.gitignore`d language manifest must not gain the additional Git refresh.
    ///
    /// Regression guard for #443: a `.gitignore`d language manifest must NOT
    /// gain the additional Git refresh — the added emission reuses the exact
    /// same `path_is_gitignored` filter as the plain worktree-Git arm, so an
    /// ignored file must stay silent on the git side exactly as it always has.
    ///
    /// The specialized (Language) event still fires: gitignore status is a
    /// git-status concept, not a language-detection one.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the events fails.
    #[test]
    fn gitignored_language_manifest_does_not_emit_git_refresh() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        // Canonicalize up front (as `attribute_repo_detailed`'s `find_git_root`
        // does internally): on macOS `tmp.path()` is under a `/var` symlink to
        // `/private/var`, and a root/path canonicalization mismatch makes the
        // `ignore` crate's matcher panic ("path is expected to be under the
        // root") rather than simply not matching.
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        std::fs::write(repo.join(".gitignore"), "Cargo.toml\n").expect("write gitignore");
        let file = repo.join("Cargo.toml");
        std::fs::write(&file, "data").expect("write file");

        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(
            events.len(),
            1,
            "a gitignored manifest should still emit the specialized event \
             but no Git refresh, got: {events:?}"
        );
        assert!(matches!(
            events.first().map(|e| &e.event),
            Some(FileEvent::Language { .. })
        ));
    }

    /// A nested event must attribute to its watched root by longest prefix.
    ///
    /// Regression test for #344: a raw event in a nested subdirectory of a
    /// watched repo root must attribute to that root via the longest-prefix
    /// match, not an ancestor `.git` stat walk. No `.git` directory exists
    /// anywhere in this tree, so a correct result here is only reachable via
    /// the prefix match against `watched_roots`: the old ancestor-walk
    /// fallback would find no `.git`, and would misattribute to the file's
    /// immediate parent directory instead of the watched root.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the event fails.
    #[test]
    fn nested_event_attributes_to_watched_root_via_prefix_match() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let watched_root = tmp.path().to_path_buf();
        let nested_dir = watched_root.join("nested").join("deep");
        std::fs::create_dir_all(&nested_dir).expect("nested dirs");
        let file = nested_dir.join("tracked.txt");
        std::fs::write(&file, "data").expect("write file");

        let registry = test_registry(true, HashSet::from([watched_root.clone()]));
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(
            events.len(),
            1,
            "a nested tracked-file edit should emit one git event"
        );
        assert_eq!(
            events.first().map(|e| &e.repo),
            Some(&watched_root),
            "event should attribute to the watched root via longest-prefix match"
        );
        assert!(matches!(
            events.first().map(|e| &e.event),
            Some(FileEvent::Git { .. })
        ));
    }

    /// An event outside every watched root keeps the ancestor-walk fallback.
    ///
    /// Regression test for #344: an event outside any watched root must fall
    /// back to exactly today's ancestor-walk behavior (`find_git_root`, then
    /// its immediate parent), pinning current behavior for the defensive
    /// no-match path.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp dirs or capturing the event fails.
    #[test]
    fn event_outside_watched_roots_falls_back_to_ancestor_walk() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let outside_dir = tmp.path().to_path_buf();
        let file = outside_dir.join("tracked.txt");
        std::fs::write(&file, "data").expect("write file");

        // An unrelated watched root that is NOT a prefix of `file`.
        let other_tmp = tempfile::TempDir::new().expect("other temp dir");
        let registry = test_registry(true, HashSet::from([other_tmp.path().to_path_buf()]));
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        let expected_repo = MultiRepoWatcher::find_git_root(&file)
            .or_else(|| file.parent().map(Path::to_path_buf))
            .expect("fallback always yields a parent for an absolute path");

        assert_eq!(
            events.len(),
            1,
            "a tracked-file edit outside any watched root should still emit one git event"
        );
        assert_eq!(
            events.first().map(|e| &e.repo),
            Some(&expected_repo),
            "no watched root matches, so attribution must match today's ancestor-walk fallback"
        );
    }

    /// A high-churn token in an ANCESTOR of the repo root must not suppress events.
    ///
    /// Regression test for #445: a repo checked out under an ancestor
    /// directory that happens to share a name with a high-churn ignore token
    /// (e.g. a repo at `/work/build/my-repo`) must not have its worktree
    /// events suppressed just because `build` appears above the repo root.
    ///
    /// The event path itself lives entirely inside the repo and matches no
    /// ignore token once scoped to the repo-relative path, so it must be
    /// delivered.
    ///
    /// This uses a genuine watched-root prefix match (not the `None`
    /// fallback) since only that path is meant to gain repo-relative
    /// scoping — see the fallback-attribution reasoning on
    /// `path_matches_ignored_patterns`.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the event fails.
    #[test]
    fn ignored_ancestor_directory_does_not_suppress_worktree_events() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        // The repo root itself sits under a `build` ancestor component,
        // mirroring the reported `/work/build/my-repo` layout.
        let watched_root = tmp.path().join("build").join("my-repo");
        let src_dir = watched_root.join("src");
        std::fs::create_dir_all(&src_dir).expect("nested dirs");
        let file = src_dir.join("main.rs");
        std::fs::write(&file, "fn main() {}").expect("write file");

        let registry = test_registry(true, HashSet::from([watched_root.clone()]));
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(
            events.len(),
            1,
            "an in-repo edit must not be suppressed just because a `build` \
             ancestor sits above the watched repo root"
        );
        assert_eq!(
            events.first().map(|e| &e.repo),
            Some(&watched_root),
            "event should still attribute to the watched root"
        );
    }

    /// A high-churn token INSIDE the repo must still suppress events.
    ///
    /// Regression test for #445: the built-in high-churn ignore tokens must
    /// still suppress a matching directory that lives *inside* a genuinely
    /// attributed repo (as opposed to an ancestor of it). Companion to
    /// `ignored_ancestor_directory_does_not_suppress_worktree_events` above,
    /// which proves the opposite case.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or capturing the event fails.
    #[test]
    fn ignored_directory_within_repo_still_suppressed() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let watched_root = tmp.path().to_path_buf();
        let node_modules_dir = watched_root.join("node_modules").join("pkg");
        std::fs::create_dir_all(&node_modules_dir).expect("nested dirs");
        let file = node_modules_dir.join("index.js");
        std::fs::write(&file, "module.exports = {};").expect("write file");
        // Real repositories ignore `node_modules/`; the token is only a hint
        // (#719), so the repo's own ignore rule is what suppresses the event.
        std::fs::write(watched_root.join(".gitignore"), "node_modules/\n").expect("gitignore");

        let registry = test_registry(true, HashSet::from([watched_root]));
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert!(
            events.is_empty(),
            "High-churn build directories inside the repo should stay filtered"
        );
    }

    /// # Panics
    ///
    /// Panics if the mocked callback cannot record the triggered event.
    #[test]
    fn high_churn_dirs_filtered_when_enabled() {
        let registry = test_registry(true, HashSet::new());
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
                paths: vec![PathBuf::from("/repo/node_modules/pkg/index.js")],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert!(
            events.is_empty(),
            "High-churn build directories should stay filtered"
        );
    }

    /// # Panics
    ///
    /// Panics if creating the temp repo or gitignore file fails.
    #[test]
    fn gitignored_glob_paths_are_detected() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        std::fs::write(repo.join(".gitignore"), "*.log\nbuild_output\n").expect("gitignore");

        assert!(
            path_is_gitignored(repo, &repo.join("debug.log"), &registry),
            "files matching an ignore glob should be detected"
        );
        assert!(
            path_is_gitignored(repo, &repo.join("build_output"), &registry),
            "an ignored name should be detected"
        );
        assert!(
            !path_is_gitignored(repo, &repo.join("src.rs"), &registry),
            "tracked files should not be reported as ignored"
        );
    }

    /// Regression for #152: editing `.gitignore` so a path stops being ignored
    /// must invalidate the cached matcher, so live updates resume without an
    /// agent restart.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or rewriting the gitignore file fails.
    #[test]
    fn unignored_path_is_no_longer_suppressed_after_gitignore_edit() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let gitignore = repo.join(".gitignore");
        let target = repo.join("artifact.txt");

        // Initially ignored: the matcher is built and cached.
        std::fs::write(&gitignore, "artifact.txt\n").expect("write gitignore");
        assert!(
            path_is_gitignored(repo, &target, &registry),
            "path should be ignored while the rule is present"
        );

        // Drop the rule. The mtime+length fingerprint changes, so the stale
        // matcher must be rebuilt rather than continuing to suppress the path.
        std::fs::write(&gitignore, "# nothing ignored\n").expect("rewrite gitignore");
        assert!(
            !path_is_gitignored(repo, &target, &registry),
            "path should stop being suppressed once the ignore rule is removed"
        );
    }

    /// A new `.git/info/exclude` rule must be picked up without a restart.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or exclude file fails.
    #[test]
    fn new_info_exclude_rule_invalidates_cached_matcher() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path();
        let info_dir = repo.join(".git/info");
        std::fs::create_dir_all(&info_dir).expect("git info dir");
        let target = repo.join("local-only.txt");

        // No ignore rules yet: matcher built and cached as "ignores nothing".
        assert!(
            !path_is_gitignored(repo, &target, &registry),
            "path should not be ignored before any exclude rule exists"
        );

        // Add a personal exclude. The fingerprint flips from absent to present.
        std::fs::write(info_dir.join("exclude"), "local-only.txt\n").expect("write exclude");
        assert!(
            path_is_gitignored(repo, &target, &registry),
            "a newly added .git/info/exclude rule should take effect"
        );
    }

    /// Run a git command in `dir`, panicking on failure. Test-only helper.
    fn git_in(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Regression for #435 (FIX #1): a `git add -f`'d file matches an ignore glob but is
    /// TRACKED, so its edits still change `git status` and must not be suppressed.
    ///
    /// A non-force-added file matching the same glob must still be reported as ignored.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or running git fails.
    #[test]
    fn force_added_tracked_file_is_not_suppressed() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

        git_in(&repo, &["init"]);
        git_in(&repo, &["config", "user.email", "test@example.com"]);
        git_in(&repo, &["config", "user.name", "Test User"]);
        git_in(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join(".gitignore"), "*.log\n").expect("write gitignore");
        std::fs::write(repo.join("ignored.log"), "tracked\n").expect("write ignored.log");
        std::fs::write(repo.join("other.log"), "untracked\n").expect("write other.log");
        // Force-add the ignored file so it becomes tracked-but-ignored.
        git_in(&repo, &["add", "-f", "ignored.log"]);
        git_in(&repo, &["add", ".gitignore"]);
        git_in(&repo, &["commit", "-m", "init"]);

        assert!(
            !path_is_gitignored(&repo, &repo.join("ignored.log"), &registry),
            "a force-added (tracked) file must not be suppressed"
        );
        assert!(
            path_is_gitignored(&repo, &repo.join("other.log"), &registry),
            "an untracked file matching the ignore glob must still be suppressed"
        );
    }

    /// Feed a content edit of `file` through `process_event` with `root` as
    /// the only watched root and return the pending events it produced.
    fn events_for_edit(root: &Path, file: &Path) -> Vec<PendingEvent> {
        let registry = test_registry(true, HashSet::from([root.to_path_buf()]));
        run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));
            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![file.to_path_buf()],
                attrs: EventAttributes::default(),
            };
            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        })
    }

    /// Regression for #719: directory-name tokens are a hint, not a verdict.
    ///
    /// A committed, non-ignored file under `vendor/`, `dist/`, `build/` or
    /// `target/` changes `git status` and must refresh the prompt; churn in an
    /// ignored `node_modules/` must not, and a tracked file inside an ignored
    /// `dist/` must (#435).
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or running git fails.
    #[test]
    fn tracked_file_under_high_churn_token_dir_triggers_git_refresh() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

        git_in(&repo, &["init"]);
        git_in(&repo, &["config", "user.email", "test@example.com"]);
        git_in(&repo, &["config", "user.name", "Test User"]);
        git_in(&repo, &["config", "commit.gpgsign", "false"]);
        let tracked = [
            "vendor/lib.go",
            "dist/index.js",
            "build/script.sh",
            "target/keep.txt",
        ];
        for rel in tracked {
            let file = repo.join(rel);
            std::fs::create_dir_all(file.parent().expect("parent")).expect("dirs");
            std::fs::write(&file, "committed\n").expect("write tracked file");
        }
        git_in(&repo, &["add", "-A"]);
        git_in(&repo, &["commit", "-m", "init"]);

        for rel in tracked {
            let events = events_for_edit(&repo, &repo.join(rel));
            assert_eq!(events.len(), 1, "tracked `{rel}` edit must emit one event");
            assert_eq!(events.first().map(|e| &e.repo), Some(&repo), "`{rel}`");
            assert!(
                matches!(
                    events.first().map(|e| &e.event),
                    Some(FileEvent::Git { .. })
                ),
                "`{rel}` edit must be a Git event"
            );
        }

        // Ignored churn stays suppressed.
        std::fs::write(repo.join(".gitignore"), "node_modules/\ndist/\n").expect("gitignore");
        let ignored = repo.join("node_modules").join("pkg").join("index.js");
        std::fs::create_dir_all(ignored.parent().expect("parent")).expect("dirs");
        std::fs::write(&ignored, "x\n").expect("write ignored");
        assert!(
            events_for_edit(&repo, &ignored).is_empty(),
            "an ignored node_modules edit must stay suppressed"
        );

        // `dist/` is now ignored, but `dist/index.js` is tracked (force-added).
        assert_eq!(
            events_for_edit(&repo, &repo.join("dist/index.js")).len(),
            1,
            "a tracked file inside an ignored dist/ must still emit an event"
        );
        let untracked = repo.join("dist").join("new.js");
        std::fs::write(&untracked, "x\n").expect("write untracked");
        assert!(
            events_for_edit(&repo, &untracked).is_empty(),
            "an untracked file in an ignored dist/ must stay suppressed"
        );
    }

    /// How long the stand-in backend in
    /// [`a_slow_repo_does_not_delay_another_repos_lookup`] pretends `git
    /// ls-files` takes.
    const SLOW_BACKEND_RUN: Duration = Duration::from_millis(300);

    /// Budget for a second repo's lookup while the slow one is mid-run. Well
    /// under [`SLOW_BACKEND_RUN`], so serialising the two blows it.
    const FAST_LOOKUP_BUDGET: Duration = Duration::from_millis(100);

    /// Generous ceiling for gates and async refreshes; only a hang reaches it.
    const GATE_BUDGET: Duration = Duration::from_secs(10);

    /// A one-shot gate: the stand-in backend opens it, the test waits on it.
    type TestGate = Arc<(Mutex<bool>, std::sync::Condvar)>;

    /// Create a closed [`TestGate`].
    fn new_gate() -> TestGate {
        Arc::new((Mutex::new(false), std::sync::Condvar::new()))
    }

    /// Open `gate`, waking every waiter.
    fn open_gate(gate: &TestGate) {
        let (flag, condvar) = &**gate;
        *flag.lock().unwrap() = true;
        condvar.notify_all();
    }

    /// Block until `gate` opens; `false` means `budget` elapsed first.
    fn wait_for_gate(gate: &TestGate, budget: Duration) -> bool {
        let (flag, condvar) = &**gate;
        let deadline = Instant::now()
            .checked_add(budget)
            .unwrap_or_else(Instant::now);
        let mut open = flag.lock().unwrap();
        while !*open {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, _) = condvar.wait_timeout(open, remaining).unwrap();
            open = next;
        }
        drop(open);
        true
    }

    /// Give `repo` the `.git/index` file [`force_added_set`] fingerprints,
    /// without the cost of a real `git init` (the backend is stubbed anyway).
    fn fake_repo_with_index(tmp: &tempfile::TempDir) -> PathBuf {
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        std::fs::write(repo.join(".git").join("index"), b"index").expect("write index");
        repo
    }

    /// As [`fake_repo_with_index`], but consumes and leaks `tmp` (via
    /// `TempDir::keep`) instead of borrowing it, for a caller that must
    /// return the repo path out of the scope the `TempDir` was created in.
    fn fake_repo_with_index_leaked(tmp: tempfile::TempDir) -> PathBuf {
        let repo = std::fs::canonicalize(tmp.keep()).expect("canonicalize repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        std::fs::write(repo.join(".git").join("index"), b"index").expect("write index");
        repo
    }

    /// #568: `force_added_set` must never hold the force-added cache mutex
    /// while `git ls-files` runs.
    ///
    /// The stand-in backend parks inside the "subprocess" and the test then
    /// tries to take the cache mutex: before the fix the mutex was held across
    /// the spawn, so this `try_lock` failed.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo, the gate, or the worker thread fails.
    #[test]
    fn force_added_refresh_never_holds_the_cache_mutex() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = fake_repo_with_index(&tmp);

        let entered = new_gate();
        let release = new_gate();
        let entered_for_hook = Arc::clone(&entered);
        let release_for_hook = Arc::clone(&release);
        set_tracked_ignored_hook_for_test(&repo, move |_| {
            open_gate(&entered_for_hook);
            wait_for_gate(&release_for_hook, GATE_BUDGET);
            Vec::new()
        });

        let repo_for_worker = repo.clone();
        let registry_for_worker = Arc::clone(&registry);
        let worker = std::thread::spawn(move || {
            drop(force_added_set(&repo_for_worker, &registry_for_worker));
        });

        assert!(
            wait_for_gate(&entered, GATE_BUDGET),
            "the stand-in backend should have been entered"
        );
        let cache_free_during_git = registry.ignore().force_added.try_lock().is_ok();
        open_gate(&release);
        worker.join().expect("worker thread");
        clear_tracked_ignored_hook_for_test(&repo);

        assert!(
            cache_free_during_git,
            "the force-added cache mutex must be free while `git ls-files` runs (#568)"
        );
    }

    /// #568: one repo's slow `git ls-files` must not stall another repo's
    /// lookup through the shared force-added cache mutex.
    ///
    /// # Panics
    ///
    /// Panics if the temp repos, the gate, or the worker thread fails.
    #[test]
    fn a_slow_repo_does_not_delay_another_repos_lookup() {
        let registry = test_registry(true, HashSet::new());
        let slow_tmp = tempfile::TempDir::new().expect("temp dir");
        let fast_tmp = tempfile::TempDir::new().expect("temp dir");
        let slow_repo = fake_repo_with_index(&slow_tmp);
        let fast_repo = fake_repo_with_index(&fast_tmp);

        let entered = new_gate();
        let entered_for_hook = Arc::clone(&entered);
        set_tracked_ignored_hook_for_test(&slow_repo, move |_| {
            open_gate(&entered_for_hook);
            std::thread::sleep(SLOW_BACKEND_RUN);
            Vec::new()
        });
        set_tracked_ignored_hook_for_test(&fast_repo, |_| Vec::new());

        let slow_for_worker = slow_repo.clone();
        let registry_for_worker = Arc::clone(&registry);
        let worker = std::thread::spawn(move || {
            drop(force_added_set(&slow_for_worker, &registry_for_worker));
        });

        assert!(
            wait_for_gate(&entered, GATE_BUDGET),
            "the slow stand-in backend should have been entered"
        );
        let started = Instant::now();
        drop(force_added_set(&fast_repo, &registry));
        let elapsed = started.elapsed();

        worker.join().expect("worker thread");
        clear_tracked_ignored_hook_for_test(&slow_repo);
        clear_tracked_ignored_hook_for_test(&fast_repo);

        assert!(
            elapsed < FAST_LOOKUP_BUDGET,
            "a lookup for an unrelated repo took {elapsed:?}, so it queued behind \
             the slow repo's `git ls-files` (#568)"
        );
    }

    /// #618: one repo's cold-start walk must never block another repo's `watch()`.
    ///
    /// `FileSystemWatcher::watch()`'s `BackendKind::Poll` branch calls
    /// `poll_watch_targets`, which walks the directory tree and can
    /// transitively run `git ls-files` (via `path_is_gitignored` ->
    /// `is_force_added` -> `force_added_set`'s `ForceAddedRefresh::Blocking`
    /// first-lookup path) for a repo's first walk. `self.backend` is the one
    /// mutex shared by every repo a `FileSystemWatcher` serves, so before the
    /// fix a slow first walk for one repo (repo A) stalled `watch()` for an
    /// unrelated repo (repo B) onboarding at the same moment.
    ///
    /// Repo A is given a real `.gitignore`-matched subdirectory so the
    /// `poll_watch_targets` descent reaches `path_is_gitignored` for it, and a
    /// stand-in `git ls-files` backend that parks on a gate -- exactly the
    /// #568 gate pattern, applied one level up (through `watch()` rather than
    /// `force_added_set` directly).
    ///
    /// # Panics
    ///
    /// Panics if the temp repos, the gate, or the worker thread fails.
    #[test]
    fn cold_start_for_one_repo_does_not_block_another_repos_watch() {
        let primary_tmp = tempfile::TempDir::new().expect("temp dir");
        let secondary_tmp = tempfile::TempDir::new().expect("temp dir");
        let repo_a = fake_repo_with_index(&primary_tmp);
        let repo_b = fake_repo_with_index(&secondary_tmp);

        // A real ignored subdirectory so `poll_watch_targets`'s descent calls
        // `path_is_gitignored` on it, which (since it matches) goes on to
        // consult `is_force_added` -> `force_added_set`.
        std::fs::write(repo_a.join(".gitignore"), b"ignored/\n").expect("write .gitignore");
        std::fs::create_dir_all(repo_a.join("ignored")).expect("ignored dir");

        let registry = test_registry(true, HashSet::from([repo_a.clone(), repo_b.clone()]));
        let (callback, _events) = recording_callback_boxed();
        let watcher = FileSystemWatcher::with_fallback_policy(
            callback,
            Some(Arc::clone(&registry)),
            Duration::from_millis(50),
            no_op_scheduler(),
            FallbackPolicy::ForcePoll {
                poll_interval: Duration::from_secs(3_600),
            },
        )
        .expect("create watcher");

        let entered = new_gate();
        let release = new_gate();
        let entered_for_hook = Arc::clone(&entered);
        let release_for_hook = Arc::clone(&release);
        set_tracked_ignored_hook_for_test(&repo_a, move |_| {
            open_gate(&entered_for_hook);
            wait_for_gate(&release_for_hook, GATE_BUDGET);
            Vec::new()
        });

        std::thread::scope(|scope| {
            let watcher_ref = &watcher;
            let repo_a_for_worker = repo_a.clone();
            let worker = scope.spawn(move || {
                watcher_ref.watch(&repo_a_for_worker).expect("watch repo a");
            });

            assert!(
                wait_for_gate(&entered, GATE_BUDGET),
                "the stand-in backend for repo A's cold start should have been entered"
            );

            let started = Instant::now();
            watcher.watch(&repo_b).expect("watch repo b");
            let elapsed = started.elapsed();

            open_gate(&release);
            worker.join().expect("worker thread");

            assert!(
                elapsed < FAST_LOOKUP_BUDGET,
                "watch() for an unrelated repo took {elapsed:?} while repo A's cold \
                 start was still running, so it queued behind the shared backend \
                 mutex (#618)"
            );
        });

        clear_tracked_ignored_hook_for_test(&repo_a);
    }

    /// #618 / #442: `watch()` racing the health probe's native -> poll swap
    /// must still end up armed correctly, whichever side of the swap it
    /// lands on.
    ///
    /// Deterministic via the same gate pattern as
    /// [`cold_start_for_one_repo_does_not_block_another_repos_watch`]: repo A
    /// is already watched (natively) and has a gitignored subdirectory plus
    /// a stand-in `git ls-files` backend that parks
    /// [`FileSystemWatcher::force_poll_fallback`] (the test-only entry point
    /// onto the production swap path) mid-flight -- after its pre-swap
    /// snapshot (phase 1) but before the swap itself (phase 3). While parked,
    /// `watch()` for repo B is guaranteed to observe `kind == Native` and arm
    /// natively; repo B is then a registration that was never part of the
    /// swap's pre-swap snapshot, so ending up correctly re-armed on the poll
    /// backend exercises `rearm_poll_paths`'s defensive fallback (#618) for a
    /// path added between the snapshot and the final lock.
    ///
    /// # Panics
    ///
    /// Panics if the temp repos or the watcher cannot be created.
    fn swap_race_fixture() -> (FileSystemWatcher, PathBuf, PathBuf, TestGate, TestGate) {
        let repo_a = fake_repo_with_index_leaked(tempfile::TempDir::new().expect("temp dir"));
        let repo_b = fake_repo_with_index_leaked(tempfile::TempDir::new().expect("temp dir"));
        std::fs::write(repo_a.join(".gitignore"), b"ignored/\n").expect("write .gitignore");
        std::fs::create_dir_all(repo_a.join("ignored")).expect("ignored dir");

        let registry = test_registry(true, HashSet::from([repo_a.clone(), repo_b.clone()]));
        let (callback, _events) = recording_callback_boxed();
        let watcher = FileSystemWatcher::with_fallback_policy(
            callback,
            Some(Arc::clone(&registry)),
            Duration::from_millis(50),
            no_op_scheduler(),
            FallbackPolicy::NativeOnly,
        )
        .expect("create watcher");
        watcher.watch(&repo_a).expect("watch repo a natively");

        let entered = new_gate();
        let release = new_gate();
        let entered_for_hook = Arc::clone(&entered);
        let release_for_hook = Arc::clone(&release);
        set_tracked_ignored_hook_for_test(&repo_a, move |_| {
            open_gate(&entered_for_hook);
            wait_for_gate(&release_for_hook, GATE_BUDGET);
            Vec::new()
        });

        (watcher, repo_a, repo_b, entered, release)
    }

    /// # Panics
    ///
    /// Panics if the fixture, the gate, or the swap thread fails.
    #[test]
    fn watch_racing_the_native_to_poll_swap_still_ends_up_armed_correctly() {
        let (watcher, repo_a, repo_b, entered, release) = swap_race_fixture();

        std::thread::scope(|scope| {
            let watcher_ref = &watcher;
            let swap = scope.spawn(move || {
                watcher_ref.force_poll_fallback(Duration::from_secs(3_600));
            });

            assert!(
                wait_for_gate(&entered, GATE_BUDGET),
                "the swap should have parked on repo A's cold start"
            );
            assert_eq!(
                watcher.backend_kind(),
                Some(BackendKind::Native),
                "the swap must not have applied yet -- it is parked before its own \
                 final lock (#618)"
            );

            watcher
                .watch(&repo_b)
                .expect("watch repo b while the swap is parked");

            open_gate(&release);
            swap.join().expect("swap thread");
        });

        clear_tracked_ignored_hook_for_test(&repo_a);

        assert_eq!(
            watcher.backend_kind(),
            Some(BackendKind::Poll),
            "the swap must have completed"
        );
        let guard = watcher.backend.lock().expect("backend lock");
        let registration = guard
            .registrations
            .get(&(repo_b.clone(), WatchMode::Recursive))
            .expect("repo B must still be a watched path after the swap");
        assert!(
            registration.targets.contains(&(repo_b, WatchMode::Shallow)),
            "repo B, added while the swap was mid-flight and so absent from its \
             pre-swap snapshot, must still end up re-armed on the poll backend \
             with its expansion (#618), got: {:?}",
            registration.targets
        );
        drop(guard);
    }

    /// #568: once the off-thread refresh lands, force-added behaviour is
    /// unchanged — a file force-added after the set was cached is picked up,
    /// and the previously force-added one is still exempt.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or running git fails.
    #[test]
    fn force_added_set_picks_up_an_index_change_after_the_refresh() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

        git_in(&repo, &["init"]);
        git_in(&repo, &["config", "user.email", "test@example.com"]);
        git_in(&repo, &["config", "user.name", "Test User"]);
        git_in(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join(".gitignore"), "*.log\n").expect("write gitignore");
        std::fs::write(repo.join("first.log"), "tracked\n").expect("write first.log");
        std::fs::write(repo.join("other.log"), "untracked\n").expect("write other.log");
        git_in(&repo, &["add", "-f", "first.log"]);
        git_in(&repo, &["add", ".gitignore"]);
        git_in(&repo, &["commit", "-m", "init"]);

        // Prime the cache for this repo.
        assert!(
            !path_is_gitignored(&repo, &repo.join("first.log"), &registry),
            "the force-added file must not be suppressed"
        );

        // A second force-add rewrites `.git/index`, so the cached set is stale.
        let second = repo.join("second.log");
        std::fs::write(&second, "also tracked\n").expect("write second.log");
        git_in(&repo, &["add", "-f", "second.log"]);

        let deadline = Instant::now()
            .checked_add(GATE_BUDGET)
            .unwrap_or_else(Instant::now);
        let mut picked_up = false;
        while Instant::now() < deadline {
            if is_force_added(&repo, &second, &registry) {
                picked_up = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            picked_up,
            "the newly force-added file must appear once the refresh completes"
        );
        assert!(
            !path_is_gitignored(&repo, &repo.join("first.log"), &registry),
            "the original force-add must survive the refresh"
        );
        assert!(
            path_is_gitignored(&repo, &repo.join("other.log"), &registry),
            "an untracked file matching the ignore glob must still be suppressed"
        );
    }

    /// Regression for #435 (FIX #2): a nested `.gitignore` un-ignoring a root-ignored path
    /// must be honored and stay live.
    ///
    /// Editing that nested `.gitignore` must invalidate the cached matcher so the result flips
    /// without an agent restart.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or rewriting the gitignore files fails.
    #[test]
    fn nested_unignore_is_honored_and_invalidates_on_edit() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        std::fs::write(repo.join(".gitignore"), "build/\n").expect("root gitignore");

        let subdir = repo.join("subdir");
        std::fs::create_dir_all(subdir.join("build")).expect("nested build dir");
        let target = subdir.join("build").join("foo");
        std::fs::write(&target, "x").expect("write target");

        // Without the nested rule the root `build/` ignores `subdir/build/foo`.
        assert!(
            path_is_gitignored(repo, &target, &registry),
            "root build/ rule should ignore the nested path with no override"
        );

        // A nested un-ignore must flip the result. The fingerprint goes from
        // absent to present, so the cached matcher is rebuilt.
        std::fs::write(subdir.join(".gitignore"), "!build/\n").expect("nested un-ignore");
        assert!(
            !path_is_gitignored(repo, &target, &registry),
            "a nested !build/ rule must re-include the path"
        );

        // Editing the nested `.gitignore` to drop the negation (different length,
        // so the mtime+len fingerprint flips deterministically) must invalidate
        // the cached matcher and re-ignore the path.
        std::fs::write(subdir.join(".gitignore"), "# no override here now\n")
            .expect("rewrite nested gitignore");
        assert!(
            path_is_gitignored(repo, &target, &registry),
            "dropping the nested negation must re-ignore the path without a restart"
        );
    }

    /// Collect events into a shared buffer that outlives `process_event`, for
    /// the tests that keep reading it after the call returns.
    ///
    /// Unlike `run_with_capture`, this never `try_unwrap`s the buffer, so a
    /// surviving `Arc` clone (the watcher's own, in the real-backend tests)
    /// cannot make the read fail.
    fn recording_callback() -> (Arc<PendingCallback>, CapturedEvents) {
        let captured: Arc<Mutex<Vec<PendingEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_cb = Arc::clone(&captured);
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured_for_cb.lock().unwrap().push(event);
        }));
        (callback_arc, captured)
    }

    /// Shared buffer of events captured by a recording callback.
    type CapturedEvents = Arc<Mutex<Vec<PendingEvent>>>;

    /// A delayed-event scheduler that does nothing, for the tests that drive
    /// `process_event` over paths which never reach the directory-create
    /// follow-up, and so never invoke it.
    fn no_op_scheduler() -> DelayedEventScheduler {
        Arc::new(|_, _| {})
    }

    /// Every `(PendingEvent, Instant)` a scheduler was invoked with.
    type CapturedSchedules = Arc<Mutex<Vec<(PendingEvent, Instant)>>>;

    /// A scheduler that records what it was asked to schedule, for the tests
    /// that assert on `schedule_directory_followup`'s decision directly rather
    /// than waiting on a delivery.
    ///
    /// Since #569 the waiting belongs to `DebounceEngine` (and is covered by
    /// `debouncer.rs`'s own tests), so what is left to pin here is which events
    /// get a follow-up scheduled at all, and with what repo, path and due time.
    fn recording_scheduler() -> (DelayedEventScheduler, CapturedSchedules) {
        let captured: CapturedSchedules = Arc::new(Mutex::new(Vec::new()));
        let captured_for_hook = Arc::clone(&captured);
        let scheduler: DelayedEventScheduler = Arc::new(move |pending, due| {
            captured_for_hook.lock().unwrap().push((pending, due));
        });
        (scheduler, captured)
    }

    /// Like [`recording_callback`], but yields the bare [`PendingCallback`]
    /// that [`FileSystemWatcher::new`] consumes, for the real-filesystem
    /// backend tests (#442).
    fn recording_callback_boxed() -> (PendingCallback, CapturedEvents) {
        let captured: Arc<Mutex<Vec<PendingEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_for_cb = Arc::clone(&captured);
        let callback: PendingCallback = Box::new(move |event| {
            captured_for_cb.lock().unwrap().push(event);
        });
        (callback, captured)
    }

    /// A directory create emits its own git event and schedules exactly one follow-up.
    ///
    /// Core of #416: a directory-`Create` event must (a) emit the immediate git
    /// event for the mkdir itself, then (b) schedule exactly one delayed
    /// follow-up git event for the same repo — the mechanism that rescans a
    /// brand-new directory whose file-create events raced (and were dropped by)
    /// the not-yet-armed recursive watch.
    ///
    /// Since #569 the follow-up is scheduled on the debouncer rather than on a
    /// thread of its own, so this asserts on the scheduling call — the decision
    /// `process_event` actually makes. That the scheduled event is then held
    /// until its due time and delivered once is `DebounceEngine`'s contract,
    /// pinned by `debouncer.rs`'s `a_due_floor_holds_an_entry_past_its_quiet_gap`.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or locking the capture buffer fails.
    #[test]
    fn directory_create_schedules_one_followup_git_event() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let new_dir = tmp.path().join("newmod");
        std::fs::create_dir_all(&new_dir).expect("create new dir");

        let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
        let (callback_arc, events) = recording_callback();
        let (scheduler, captured_schedules) = recording_scheduler();

        let evt = Event {
            kind: EventKind::Create(CreateKind::Folder),
            paths: vec![new_dir.clone()],
            attrs: EventAttributes::default(),
        };
        let called_at = Instant::now();
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &scheduler,
        );

        // The immediate mkdir event is delivered synchronously.
        let recorded = events.lock().unwrap();
        assert_eq!(
            recorded.len(),
            1,
            "directory-create should emit its own git event immediately"
        );
        let immediate = recorded.first().expect("the immediate event");
        assert!(
            matches!(immediate.event, FileEvent::Git { .. }),
            "the immediate event must be a git event"
        );
        assert_eq!(immediate.repo, tmp.path().to_path_buf());
        drop(recorded);

        // ...and exactly one follow-up is scheduled, for the same repo, no
        // earlier than the follow-up delay from now.
        let scheduled_guard = captured_schedules.lock().unwrap();
        assert_eq!(
            scheduled_guard.len(),
            1,
            "exactly one follow-up git event should be scheduled for a directory create"
        );
        let (pending, due) = scheduled_guard
            .first()
            .expect("the scheduled follow-up")
            .clone();
        drop(scheduled_guard);
        assert_eq!(
            pending.repo,
            tmp.path().to_path_buf(),
            "follow-up must target the same repo the mkdir attributed to"
        );
        let FileEvent::Git { paths } = &pending.event else {
            panic!("the follow-up must be a git event");
        };
        assert_eq!(
            paths.as_slice(),
            Some([new_dir].as_slice()),
            "the follow-up must name the created directory"
        );
        assert!(
            due >= called_at + TEST_FOLLOWUP_DELAY,
            "the follow-up must not come due before the follow-up delay has passed"
        );
    }

    /// A plain file-`Create` event must NOT schedule any follow-up: the single
    /// immediate git event is all it produces. Guards the acceptance criterion
    /// that ordinary file events pay no extra-scan cost.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or locking the capture buffer fails.
    #[test]
    fn plain_file_create_schedules_no_followup() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let file = tmp.path().join("new_file.txt");
        std::fs::write(&file, "data").expect("write file");

        let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
        let (callback_arc, events) = recording_callback();
        let (scheduler, captured_schedules) = recording_scheduler();

        let evt = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![file],
            attrs: EventAttributes::default(),
        };
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &scheduler,
        );

        assert_eq!(
            events.lock().unwrap().len(),
            1,
            "a file create should emit exactly one git event"
        );
        // Nothing is deferred to a later thread any more (#569), so the
        // absence of a follow-up is decided by the time `process_event`
        // returns — no waiting needed to observe it.
        assert!(
            captured_schedules.lock().unwrap().is_empty(),
            "a plain file create must not schedule a follow-up rescan"
        );
    }

    /// An ambiguous `Create` whose path is a directory still schedules a follow-up.
    ///
    /// A `CreateKind::Any` event (as emitted by backends that don't distinguish
    /// file vs. directory, e.g. macOS `FSEvents`) whose path is a real directory
    /// must still schedule a follow-up via the `path.is_dir()` fallback.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or locking the capture buffer fails.
    #[test]
    fn ambiguous_create_of_directory_schedules_followup_via_is_dir() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let new_dir = tmp.path().join("ambiguous_dir");
        std::fs::create_dir_all(&new_dir).expect("create new dir");

        let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
        let (callback_arc, events) = recording_callback();
        let (scheduler, captured_schedules) = recording_scheduler();

        let evt = Event {
            kind: EventKind::Create(CreateKind::Any),
            paths: vec![new_dir],
            attrs: EventAttributes::default(),
        };
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &scheduler,
        );

        assert_eq!(
            events.lock().unwrap().len(),
            1,
            "the ambiguous create should still emit its own git event immediately"
        );
        assert_eq!(
            captured_schedules.lock().unwrap().len(),
            1,
            "an ambiguous create resolving to a real directory should schedule a follow-up"
        );
    }

    /// An ambiguous `CreateKind::Any` event whose path is a plain file must NOT
    /// schedule a follow-up: the `is_dir()` fallback returns false, so the file
    /// path stays on the cheap single-event path.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp repo or locking the capture buffer fails.
    #[test]
    fn ambiguous_create_of_file_schedules_no_followup() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let file = tmp.path().join("ambiguous.txt");
        std::fs::write(&file, "data").expect("write file");

        let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
        let (callback_arc, events) = recording_callback();
        let (scheduler, captured_schedules) = recording_scheduler();

        let evt = Event {
            kind: EventKind::Create(CreateKind::Any),
            paths: vec![file],
            attrs: EventAttributes::default(),
        };
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &scheduler,
        );

        assert_eq!(
            events.lock().unwrap().len(),
            1,
            "the ambiguous create should still emit its own git event immediately"
        );
        assert!(
            captured_schedules.lock().unwrap().is_empty(),
            "an ambiguous create of a plain file must not schedule a follow-up"
        );
    }

    /// Regression for #431: a no-paths backend overflow must rescan every watched root.
    ///
    /// An inotify `IN_Q_OVERFLOW` (or any backend overflow that names no paths) must force a
    /// rescan of every currently watched root rather than being silently dropped until the
    /// periodic reconcile.
    ///
    /// `notify` surfaces this as `Ok(Event)` with `need_rescan()` true and an empty `paths`,
    /// not an `Err`.
    ///
    /// # Panics
    ///
    /// Panics if locking the capture buffer fails.
    #[test]
    fn rescan_overflow_with_no_paths_triggers_all_watched_roots() {
        let tmp1 = tempfile::TempDir::new().expect("temp dir 1");
        let tmp2 = tempfile::TempDir::new().expect("temp dir 2");
        let root1 = tmp1.path().to_path_buf();
        let root2 = tmp2.path().to_path_buf();

        let registry = test_registry(true, HashSet::from([root1.clone(), root2.clone()]));
        let (callback_arc, events) = recording_callback();

        let evt = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );

        let recorded = events.lock().unwrap();
        assert_eq!(
            recorded.len(),
            2,
            "an overflow with no paths should rescan every watched root exactly once"
        );
        let mut rescanned_repos: Vec<&PathBuf> = recorded.iter().map(|p| &p.repo).collect();
        rescanned_repos.sort();
        let mut expected_repos = vec![&root1, &root2];
        expected_repos.sort();
        assert_eq!(rescanned_repos, expected_repos);
        for pending in recorded.iter() {
            assert!(
                matches!(pending.event, FileEvent::Git { .. }),
                "a rescan should be delivered as a synthetic git event"
            );
        }
    }

    /// Regression for #431: a path-naming overflow must attribute to only its owning root.
    ///
    /// A `MustScanSubDirs`-style overflow that DOES name an affected path must attribute it to
    /// the owning watched root via the same longest-prefix match as a normal event, and must
    /// NOT rescan unrelated watched roots.
    ///
    /// # Panics
    ///
    /// Panics if locking the capture buffer fails.
    #[test]
    fn rescan_overflow_with_path_attributes_to_owning_root_only() {
        let tmp1 = tempfile::TempDir::new().expect("temp dir 1");
        let tmp2 = tempfile::TempDir::new().expect("temp dir 2");
        let root1 = tmp1.path().to_path_buf();
        let root2 = tmp2.path().to_path_buf();
        let subdir_path = root1.join("some").join("subdir");

        let registry = test_registry(true, HashSet::from([root1.clone(), root2]));
        let (callback_arc, events) = recording_callback();

        let evt = Event::new(EventKind::Other)
            .set_flag(Flag::Rescan)
            .add_path(subdir_path);
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );

        let recorded = events.lock().unwrap();
        let recorded_len = recorded.len();
        let first_repo = recorded.first().map(|p| p.repo.clone());
        let first_is_git_event = recorded
            .first()
            .is_some_and(|p| matches!(p.event, FileEvent::Git { .. }));
        drop(recorded);
        assert_eq!(
            recorded_len, 1,
            "an overflow naming a path under root1 should rescan only root1"
        );
        assert_eq!(
            first_repo,
            Some(root1),
            "the rescan should attribute to root1 via longest-prefix match"
        );
        assert!(first_is_git_event);
    }

    /// Regression for #431: a normal (non-rescan) event must be entirely
    /// unaffected by the new overflow branch — the early `need_rescan()`
    /// check must not change behavior for the overwhelming common case.
    ///
    /// # Panics
    ///
    /// Panics if locking the capture buffer fails.
    #[test]
    fn non_rescan_event_is_unaffected_by_overflow_handling() {
        let registry = test_registry(true, HashSet::new());
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
                paths: vec![PathBuf::from("/repo/.git/HEAD")],
                attrs: EventAttributes::default(),
            };
            assert!(
                !evt.need_rescan(),
                "test fixture must not be a rescan event"
            );

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(
            events.len(),
            1,
            "a normal gitdir event should still trigger exactly one git event, unchanged"
        );
        assert!(matches!(
            events.first().map(|e| &e.event),
            Some(FileEvent::Git { .. })
        ));
    }

    /// Regression for #431: several overflow-affected paths under the SAME
    /// watched root must collapse to exactly one rescan event for that root,
    /// not one per path.
    ///
    /// # Panics
    ///
    /// Panics if locking the capture buffer fails.
    #[test]
    fn rescan_overflow_dedupes_multiple_paths_under_same_root() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let root = tmp.path().to_path_buf();

        let registry = test_registry(true, HashSet::from([root.clone()]));
        let (callback_arc, events) = recording_callback();

        let evt = Event::new(EventKind::Other)
            .set_flag(Flag::Rescan)
            .add_path(root.join("a"))
            .add_path(root.join("b").join("c"))
            .add_path(root.join("d"));
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );

        let recorded = events.lock().unwrap();
        assert_eq!(
            recorded.len(),
            1,
            "multiple overflow paths under one root should dedupe to a single rescan event"
        );
        assert_eq!(recorded.first().map(|p| &p.repo), Some(&root));
    }

    /// #616: a rescan must emit `GitPaths::WholeRepo`, not `GitPaths::single(root)`.
    ///
    /// A rescan can't identify which paths changed (that's exactly why it
    /// forces a full scan), so naming the watched root as if it were a
    /// specific changed path was a fiction the debouncer and
    /// `git_paths_hint`'s repo-root escalation had to reconstruct the meaning
    /// of ("path == repo root" means "whole repo"). Emitting `WholeRepo`
    /// directly says what actually happened and lets it merge (via
    /// `GitPaths::merge`) with any other pending event for the same repo the
    /// way a real whole-repo signal should.
    ///
    /// # Panics
    ///
    /// Panics if locking the capture buffer fails.
    #[test]
    fn rescan_emits_whole_repo_for_each_affected_root() {
        let tmp1 = tempfile::TempDir::new().expect("temp dir 1");
        let tmp2 = tempfile::TempDir::new().expect("temp dir 2");
        let root1 = tmp1.path().to_path_buf();
        let root2 = tmp2.path().to_path_buf();

        let registry = test_registry(true, HashSet::from([root1, root2]));
        let (callback_arc, events) = recording_callback();

        let evt = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );

        let recorded = events.lock().unwrap();
        assert_eq!(recorded.len(), 2, "one rescan event per affected root");
        for pending in recorded.iter() {
            assert!(
                matches!(
                    &pending.event,
                    FileEvent::Git {
                        paths: GitPaths::WholeRepo
                    }
                ),
                "a rescan must carry GitPaths::WholeRepo, got: {:?}",
                pending.event
            );
        }
    }

    /// Shared by the Config and Language "outside any repo" regression tests
    /// below so they can't drift from one another.
    ///
    /// Regression test for #443: a path classified Language/Config that has
    /// no `.git` anywhere above it (and no watched-root match either) must
    /// NOT gain the additional Git refresh. `attribute_repo_detailed` falls
    /// all the way through to `RepoAttribution::ParentGuess` in this case — a
    /// bare "assume the parent directory is a repo" guess with zero evidence
    /// `path` is inside a repository at all. This is exactly what the real
    /// theme/config watcher hits for the user's actual
    /// `~/.config/gpy/config.toml`: it passes `watched_roots: None` and that
    /// path has no `.git` above it, so before this gate, `worktree_git_repo`
    /// would attribute it to `~/.config/gpy` and fire a Git refresh against a
    /// directory that is not a repository — wasted git scanning and a
    /// potential spurious repaint on every config edit.
    ///
    /// The specialized event still fires: only the added Git refresh is
    /// gated, not the pre-existing classification.
    ///
    /// # Panics
    ///
    /// Panics if creating the temp dir or capturing the events fails.
    fn assert_path_outside_any_repo_emits_no_git_refresh(
        filename: &str,
        is_expected_specialized: impl Fn(&FileEvent) -> bool,
    ) {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        // Canonicalize so this can't spuriously pass/fail on the macOS
        // `/var` -> `/private/var` symlink mismatch (see the gitignored-manifest
        // test above for the same reasoning).
        let dir = std::fs::canonicalize(tmp.path()).expect("canonicalize dir");
        // NOTE: deliberately NO `.git` directory anywhere in this tree.
        let file = dir.join(filename);
        std::fs::create_dir_all(file.parent().expect("file parent")).expect("parent dirs");
        std::fs::write(&file, "data").expect("write file");

        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));
            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };
            // `watched_roots: None` matches the real theme/config watcher,
            // which is the production path this gate protects.
            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(
            events.len(),
            1,
            "{filename} outside any repo should still emit the specialized \
             event but no Git refresh, got: {events:?}"
        );
        assert!(
            events.iter().any(|e| is_expected_specialized(&e.event)),
            "expected the specialized event for {filename}, got: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e.event, FileEvent::Git { .. })),
            "{filename} outside any repo must not emit a Git event, got: {events:?}"
        );
    }

    /// # Panics
    ///
    /// Panics if creating the temp dir or capturing the events fails.
    #[test]
    fn config_file_outside_any_repo_does_not_emit_git_refresh() {
        assert_path_outside_any_repo_emits_no_git_refresh("config.toml", |event| {
            matches!(event, FileEvent::Config { .. })
        });
    }

    /// A `~/.config/gpy/themes/x.toml`-shaped path with no enclosing repository
    /// still emits no Git refresh (#777 keeps the #443 behaviour).
    ///
    /// # Panics
    ///
    /// Panics if creating the temp dir or capturing the events fails.
    #[test]
    fn theme_file_outside_any_repo_does_not_emit_git_refresh() {
        assert_path_outside_any_repo_emits_no_git_refresh("gpy/themes/x.toml", |event| {
            matches!(event, FileEvent::Theme { .. })
        });
    }

    /// # Panics
    ///
    /// Panics if creating the temp dir or capturing the events fails.
    #[test]
    fn language_manifest_outside_any_repo_does_not_emit_git_refresh() {
        assert_path_outside_any_repo_emits_no_git_refresh("package.json", |event| {
            matches!(event, FileEvent::Language { .. })
        });
    }

    /// A real worktree write must be delivered whatever the native backend's health.
    ///
    /// Acceptance test for #442, over a REAL filesystem and a REAL notify
    /// backend (no synthetic `Event` values): a worktree write inside a
    /// watched repo must reach the callback well before the ~45s reconcile,
    /// *whatever* the health of the platform's native backend.
    ///
    /// On a healthy machine this passes through the native backend within
    /// milliseconds. On a machine where macOS `FSEvents` accepts the watch
    /// and then silently delivers zero events, it passes only because the
    /// health probe notices the silence and swaps in `notify::PollWatcher`.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the callback lock is
    /// poisoned.
    #[test]
    fn real_worktree_write_is_delivered_even_when_native_backend_is_silent() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");

        let registry = test_registry(true, HashSet::from([repo.clone()]));
        let (callback, events) = recording_callback_boxed();

        let mut watcher = FileSystemWatcher::new(
            callback,
            Some(Arc::clone(&registry)),
            Duration::from_millis(50),
            no_op_scheduler(),
        )
        .expect("create watcher");
        watcher.watch(&repo).expect("watch repo");

        // Let the recursive watch arm before racing it.
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(repo.join("untracked.txt"), b"content").expect("write worktree file");

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut delivered = false;
        while Instant::now() < deadline {
            if !events.lock().unwrap().is_empty() {
                delivered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        watcher.stop();

        assert!(
            delivered,
            "a real worktree write must be delivered without waiting for the \
             periodic reconcile, even when the native backend is silent"
        );
    }

    /// Build a real one-directory git-ish repo and a `FileSystemWatcher` over it under an
    /// explicit [`FallbackPolicy`], returning the watcher, the canonical repo root, and the
    /// shared capture buffer.
    ///
    /// Shared by the real-filesystem backend tests (#442).
    ///
    /// # Panics
    ///
    /// Panics if the temp repo or the watcher cannot be created.
    fn real_repo_watcher(policy: FallbackPolicy) -> (FileSystemWatcher, PathBuf, CapturedEvents) {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        // Deliberately leaked: the watcher must outlive the `TempDir` guard,
        // and these tests tear the watcher down explicitly instead.
        let repo = std::fs::canonicalize(tmp.keep()).expect("canonicalize repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");

        let registry = test_registry(true, HashSet::from([repo.clone()]));
        // Every caller exercises worktree-level delivery, so the flag is on
        // for this watcher's own registry (#617).
        let (callback, events) = recording_callback_boxed();
        let watcher = FileSystemWatcher::with_fallback_policy(
            callback,
            Some(Arc::clone(&registry)),
            Duration::from_millis(50),
            no_op_scheduler(),
            policy,
        )
        .expect("create watcher");
        (watcher, repo, events)
    }

    /// Poll `events` until non-empty or `budget` elapses.
    fn wait_for_any_event(events: &CapturedEvents, budget: Duration) -> bool {
        let deadline = Instant::now()
            .checked_add(budget)
            .unwrap_or_else(Instant::now);
        while Instant::now() < deadline {
            if !events.lock().unwrap().is_empty() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    /// The `PollWatcher` fallback must observe real worktree changes.
    ///
    /// #442: the `PollWatcher` fallback must observe real worktree changes on
    /// a real filesystem — this is the backend a machine with a dead
    /// `FSEvents` daemon ends up on, so if it cannot see a plain file write
    /// the whole fallback is worthless. No synthetic `Event` values here.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the capture lock is
    /// poisoned.
    #[test]
    fn poll_fallback_backend_delivers_real_worktree_writes() {
        let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });
        assert_eq!(
            watcher.backend_kind(),
            Some(BackendKind::Poll),
            "ForcePoll must arm the polling backend without any probe"
        );

        watcher.watch(&repo).expect("watch repo");
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(repo.join("untracked.txt"), b"content").expect("write worktree file");

        let delivered = wait_for_any_event(&events, Duration::from_secs(5));
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            delivered,
            "the poll fallback backend must deliver real worktree writes"
        );
    }

    /// Regression test for #447: the poll fallback must deliver edits to an
    /// EXISTING tracked file, not just newly-created ones.
    ///
    /// `PollWatcher` reports a plain append as
    /// `Modify(Metadata(MetadataKind::WriteTime))` — it compares mtime/size
    /// rather than watching syscalls, so it cannot report `Data`. The
    /// worktree event-kind allowlist originally accepted only
    /// `Create`/`Remove`/`Modify(Name|Data|Any)`, so every edit to an
    /// existing file was silently dropped whenever the fallback was active —
    /// precisely the case #442 exists to rescue. `.git` writes masked it,
    /// because those reach `should_trigger_update` before the allowlist ever
    /// runs.
    ///
    /// This creates the file BEFORE arming the watch, so the only event that
    /// can satisfy it is the modification.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the watcher cannot arm.
    #[test]
    fn poll_fallback_backend_delivers_edits_to_existing_files() {
        let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });

        // Seed the file and arm the watch, so the baseline snapshot already
        // contains it: only a later append can produce an event.
        let tracked = repo.join("tracked.txt");
        std::fs::write(&tracked, b"hello\n").expect("seed tracked file");
        watcher.watch(&repo).expect("watch repo");

        // Re-append on an interval rather than sleeping a fixed settle time
        // and appending once. `PollWatcher`'s initial snapshot is taken on a
        // background thread, so under parallel test load a single early
        // append can land BEFORE that baseline is captured — the file then
        // looks unchanged forever and the test flakes. Repeating the edit
        // guarantees at least one append happens after the baseline, however
        // long it took. Mirrors how `probe_sentinel_event` re-touches its own
        // sentinel for exactly this reason.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut delivered = false;
        while std::time::Instant::now() < deadline {
            let mut handle = std::fs::OpenOptions::new()
                .append(true)
                .open(&tracked)
                .expect("open tracked file for append");
            std::io::Write::write_all(&mut handle, b"modified\n").expect("append");
            drop(handle);

            if wait_for_any_event(&events, Duration::from_millis(500)) {
                delivered = true;
                break;
            }
        }
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            delivered,
            "the poll fallback must deliver edits to an existing worktree file, \
             not only file creations"
        );
    }

    /// Regression test for #817: the poll fallback must report a same-size
    /// rewrite that lands in the same whole second as the previous write.
    ///
    /// notify's poll compares mtimes truncated to seconds and ignores size, so
    /// a branch ref moved to another commit looked unchanged. The mtime is
    /// pinned to one instant before and after each rewrite, making "same
    /// second" deterministic instead of timing-dependent. The rewrite alternates
    /// content so one lands after the baseline snapshot whatever the load.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the watcher cannot arm.
    #[test]
    fn poll_fallback_detects_same_second_same_size_rewrite() {
        let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });
        let pinned = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let pin = |path: &Path, contents: &[u8]| {
            std::fs::write(path, contents).expect("write ref file");
            std::fs::File::options()
                .write(true)
                .open(path)
                .expect("open ref file")
                .set_modified(pinned)
                .expect("pin mtime");
        };

        let ref_file = repo.join("branch-ref");
        pin(&ref_file, b"1111111111111111111111111111111111111111\n");
        watcher.watch(&repo).expect("watch repo");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut delivered = false;
        let mut flip = false;
        while Instant::now() < deadline {
            flip = !flip;
            let digit: &[u8] = if flip {
                b"2222222222222222222222222222222222222222\n"
            } else {
                b"1111111111111111111111111111111111111111\n"
            };
            pin(&ref_file, digit);
            if wait_for_any_event(&events, Duration::from_millis(500)) {
                delivered = true;
                break;
            }
        }
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            delivered,
            "a same-size rewrite within one mtime second must be delivered"
        );
    }

    /// Regression test for #817: the poll backend's directory-mtime report is
    /// noise, while a file's mtime report and every other kind are not.
    ///
    /// The end-to-end case is `rebase_high_churn_file_wakes_zero_times` in
    /// `multi_repo_integration_tests`, run with `GPY_WATCH_FORCE_POLL=1`.
    ///
    /// # Panics
    ///
    /// Panics if the temp dir cannot be created.
    #[test]
    fn poll_directory_mtime_event_is_noise_but_file_edit_is_not() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let file = tmp.path().join("f");
        std::fs::write(&file, b"x").expect("write file");
        let mtime = EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime));

        assert!(is_poll_directory_mtime_noise(
            &Event::new(mtime).add_path(tmp.path().to_path_buf())
        ));
        assert!(!is_poll_directory_mtime_noise(
            &Event::new(mtime).add_path(file)
        ));
        assert!(!is_poll_directory_mtime_noise(&Event::new(mtime)));
        assert!(!is_poll_directory_mtime_noise(
            &Event::new(EventKind::Create(CreateKind::Folder)).add_path(tmp.path().to_path_buf())
        ));
    }

    /// Regression test for #687: unwatching an outer poll root keeps a nested root armed.
    ///
    /// Under the poll backend the outer root's expansion includes every
    /// directory of the nested root. Unwatching the outer root used to unwatch
    /// all of them, leaving the still-watched inner root blind.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the watcher cannot arm.
    #[test]
    fn poll_unwatch_of_outer_keeps_inner_expansion() {
        let (mut watcher, outer, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });
        let inner = outer.join("inner");
        let sub = inner.join("sub");
        std::fs::create_dir_all(&sub).expect("create inner/sub");
        let tracked = sub.join("f.txt");
        std::fs::write(&tracked, b"seed\n").expect("seed inner file");

        watcher.watch(&outer).expect("watch outer");
        watcher.watch(&inner).expect("watch inner");
        watcher.unwatch(&outer).expect("unwatch outer");
        let still_covered = watcher.covers(&sub);
        events.lock().unwrap().clear();

        // Re-append until delivered, for the same baseline race as
        // `poll_fallback_backend_delivers_edits_to_existing_files`.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut delivered = false;
        while Instant::now() < deadline {
            let mut handle = std::fs::OpenOptions::new()
                .append(true)
                .open(&tracked)
                .expect("open inner file for append");
            std::io::Write::write_all(&mut handle, b"modified\n").expect("append");
            drop(handle);

            if wait_for_any_event(&events, Duration::from_millis(500)) {
                delivered = true;
                break;
            }
        }
        watcher.stop();
        let _ = std::fs::remove_dir_all(&outer);

        assert!(still_covered, "the inner root still covers its subtree");
        assert!(
            delivered,
            "unwatching the outer root must not remove the inner root's poll targets"
        );
    }

    /// Whether `events` holds a Git event naming `path`.
    fn has_git_event_for(events: &CapturedEvents, path: &Path) -> bool {
        events
            .lock()
            .unwrap()
            .iter()
            .any(|pending| match &pending.event {
                FileEvent::Git { paths } => paths
                    .as_slice()
                    .is_some_and(|named| named.iter().any(|named_path| named_path == path)),
                _ => false,
            })
    }

    /// Poll `events` until it holds a Git event naming `path`, or `budget`
    /// elapses.
    fn wait_for_git_event_for(events: &CapturedEvents, path: &Path, budget: Duration) -> bool {
        let deadline = Instant::now()
            .checked_add(budget)
            .unwrap_or_else(Instant::now);
        while Instant::now() < deadline {
            if has_git_event_for(events, path) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    /// The OS watches `watcher` holds armed, by path.
    fn armed_dirs(watcher: &FileSystemWatcher) -> BTreeSet<PathBuf> {
        watcher.held_watches().1.into_keys().collect()
    }

    /// The poll targets the recursive registration of `root` holds.
    fn poll_targets_of(watcher: &FileSystemWatcher, root: &Path) -> BTreeSet<PathBuf> {
        let guard = watcher.backend.lock().expect("backend lock");
        guard
            .registrations
            .get(&(root.to_path_buf(), WatchMode::Recursive))
            .map(|registration| {
                registration
                    .targets
                    .iter()
                    .map(|(target, _)| target.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The union of every registration's targets: exactly what must be armed
    /// while the reference counts are right (#687, #721).
    fn all_targets(watcher: &FileSystemWatcher) -> BTreeSet<PathBuf> {
        let guard = watcher.backend.lock().expect("backend lock");
        guard
            .registrations
            .values()
            .flat_map(|registration| registration.targets.iter())
            .map(|(target, _)| target.clone())
            .collect()
    }

    /// Poll until `done` holds, or `budget` elapses.
    fn wait_until(budget: Duration, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now()
            .checked_add(budget)
            .unwrap_or_else(Instant::now);
        while Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    /// Regression test for #721: under the poll backend, an edit inside a
    /// directory created after the watch was armed is delivered.
    ///
    /// The poll expansion (#463) arms one shallow `PollWatcher` entry per
    /// directory that existed at arm time, so a later directory showed up only
    /// as an entry in its parent's one-level scan and nothing inside it was
    /// ever scanned.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the watcher cannot arm.
    #[test]
    fn poll_backend_watches_directories_created_after_arming() {
        let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });
        watcher.watch(&repo).expect("watch repo");

        let new_dir = repo.join("newdir");
        std::fs::create_dir_all(&new_dir).expect("create newdir");
        let tracked = new_dir.join("a.txt");
        std::fs::write(&tracked, b"seed\n").expect("seed newdir/a.txt");

        // Let the creation be reported, plus two poll intervals for its
        // stragglers, so only the edits below can satisfy the assertion.
        let created = wait_for_any_event(&events, Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(400));
        events.lock().unwrap().clear();

        // Re-append until delivered: `PollWatcher` takes the new directory's
        // baseline on its own schedule and compares whole-second mtimes, so
        // only an append made after the baseline, in a later second, shows.
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);
        let mut delivered = false;
        while Instant::now() < deadline {
            let mut handle = std::fs::OpenOptions::new()
                .append(true)
                .open(&tracked)
                .expect("open newdir/a.txt for append");
            std::io::Write::write_all(&mut handle, b"modified\n").expect("append");
            drop(handle);

            if wait_for_git_event_for(&events, &tracked, Duration::from_millis(500)) {
                delivered = true;
                break;
            }
        }
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(created, "creating newdir must itself be reported");
        assert!(
            delivered,
            "the poll backend must deliver edits inside a directory created after arming"
        );
    }

    /// #721: a commit on a branch whose `refs/heads/<prefix>/` directory was
    /// created after arming is detected under the poll backend.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo, git, or the watcher fails.
    #[test]
    fn poll_backend_detects_commit_on_branch_dir_created_after_arming() {
        let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });
        std::fs::remove_dir_all(repo.join(".git")).expect("drop the placeholder .git");
        git_in(&repo, &["init", "-q"]);
        git_in(&repo, &["config", "user.email", "test@example.com"]);
        git_in(&repo, &["config", "user.name", "Test User"]);
        git_in(&repo, &["config", "commit.gpgsign", "false"]);
        git_in(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        watcher.watch(&repo).expect("watch repo");

        // The first slash-named branch: creates `.git/refs/heads/feature/`
        // after the watch was armed.
        git_in(&repo, &["checkout", "-q", "-b", "feature/x"]);
        let feature_ref = repo.join(".git/refs/heads/feature/x");

        // Commit until one is reported, for the same baseline and
        // whole-second-mtime reasons as the test above.
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);
        let mut delivered = false;
        while Instant::now() < deadline {
            git_in(
                &repo,
                &["commit", "-q", "--allow-empty", "-m", "on feature"],
            );
            if wait_for_git_event_for(&events, &feature_ref, Duration::from_millis(500)) {
                delivered = true;
                break;
            }
        }
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            delivered,
            "a commit moving refs/heads/feature/x, whose directory was created \
             after arming, must be delivered under the poll backend"
        );
    }

    /// #721: removing a directory drops it, and everything under it, from the
    /// poll expansion, so repeated mkdir/rmdir cycles return the expansion to
    /// its baseline instead of growing it.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the watcher cannot arm.
    #[test]
    fn poll_backend_drops_expansion_for_removed_directory() {
        let (mut watcher, repo, _events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });
        watcher.watch(&repo).expect("watch repo");
        let baseline = poll_targets_of(&watcher, &repo);
        let new_dir = repo.join("newdir");
        let nested = new_dir.join("nested");

        let mut cycles = Vec::new();
        for _ in 0_u8..3_u8 {
            std::fs::create_dir_all(&nested).expect("create newdir/nested");
            let expanded = wait_until(Duration::from_secs(5), || {
                let targets = poll_targets_of(&watcher, &repo);
                targets.contains(&new_dir) && targets.contains(&nested)
            });
            let consistent_expanded = armed_dirs(&watcher) == all_targets(&watcher);

            std::fs::remove_dir_all(&new_dir).expect("remove newdir");
            let dropped = wait_until(Duration::from_secs(5), || {
                poll_targets_of(&watcher, &repo) == baseline
            });
            let consistent_dropped = armed_dirs(&watcher) == all_targets(&watcher);
            cycles.push((expanded, consistent_expanded, dropped, consistent_dropped));
        }
        let armed_after = armed_dirs(&watcher);
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        for (cycle, (expanded, consistent_expanded, dropped, consistent_dropped)) in
            cycles.into_iter().enumerate()
        {
            assert!(
                expanded,
                "cycle {cycle}: newdir and newdir/nested must join the expansion"
            );
            assert!(
                consistent_expanded,
                "cycle {cycle}: the armed set must equal the registrations' targets"
            );
            assert!(
                dropped,
                "cycle {cycle}: removing newdir must return the expansion to its baseline"
            );
            assert!(
                consistent_dropped,
                "cycle {cycle}: removing newdir must unwatch what it dropped"
            );
        }
        assert_eq!(
            armed_after, baseline,
            "repeated mkdir/rmdir cycles must leave the armed set at its baseline"
        );
    }

    /// #721: a gitignored directory created after arming is not armed, by the
    /// same pruning `poll_watch_targets` applies at arm time (#463).
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the watcher cannot arm.
    #[test]
    fn poll_backend_does_not_arm_gitignored_directory_created_after_arming() {
        let (mut watcher, repo, _events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });
        std::fs::write(repo.join(".gitignore"), b"node_modules/\n").expect("write .gitignore");
        watcher.watch(&repo).expect("watch repo");

        let ignored = repo.join("node_modules");
        let control = repo.join("src");
        std::fs::create_dir_all(ignored.join("pkg")).expect("create node_modules/pkg");
        std::fs::create_dir_all(&control).expect("create src");

        // `src` joining proves the directory-create path ran; two more poll
        // intervals let a `node_modules` create from the same scan land too.
        let control_armed = wait_until(Duration::from_secs(5), || {
            armed_dirs(&watcher).contains(&control)
        });
        std::thread::sleep(Duration::from_millis(400));
        let armed = armed_dirs(&watcher);
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(control_armed, "a new non-ignored directory must be armed");
        assert!(
            !armed.iter().any(|path| path.starts_with(&ignored)),
            "a new gitignored directory must not be armed, got: {armed:?}"
        );
    }

    /// #721 on #687's reference counts: a directory created under two
    /// overlapping roots joins both expansions, so unwatching either root
    /// keeps it armed for the other, and releasing both unarms it.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the watcher cannot arm.
    #[test]
    fn poll_expansion_of_new_directory_is_held_by_every_covering_root() {
        let (mut watcher, outer, _events) = real_repo_watcher(FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(200),
        });
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).expect("create inner");
        watcher.watch(&outer).expect("watch outer");
        watcher.watch(&inner).expect("watch inner");

        let new_dir = inner.join("newdir");
        std::fs::create_dir_all(&new_dir).expect("create inner/newdir");
        let held_by_both = wait_until(Duration::from_secs(5), || {
            poll_targets_of(&watcher, &outer).contains(&new_dir)
                && poll_targets_of(&watcher, &inner).contains(&new_dir)
        });
        let consistent = armed_dirs(&watcher) == all_targets(&watcher);

        watcher.unwatch(&outer).expect("unwatch outer");
        let kept = armed_dirs(&watcher).contains(&new_dir);
        watcher.unwatch(&inner).expect("unwatch inner");
        let left = armed_dirs(&watcher);
        watcher.stop();
        let _ = std::fs::remove_dir_all(&outer);

        assert!(
            held_by_both,
            "the new directory must join both the outer and the inner expansion"
        );
        assert!(
            consistent,
            "the armed set must equal the registrations' targets"
        );
        assert!(
            kept,
            "unwatching the outer root must keep the inner root's new directory armed"
        );
        assert!(
            left.is_empty(),
            "releasing both roots must unarm the new directory too, got: {left:?}"
        );
    }

    /// Companion to the test above at the classification level (#447): a
    /// metadata-only event for a worktree file must reach the Git path.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created.
    #[test]
    fn worktree_metadata_write_time_event_triggers_git_refresh() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let file = repo.join("tracked.txt");
        std::fs::write(&file, b"hello").expect("write file");

        let registry = test_registry(true, HashSet::from([repo]));
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert_eq!(
            events.len(),
            1,
            "an mtime-only change to a worktree file must trigger a git refresh \
             (this is all PollWatcher can report), got: {events:?}"
        );
    }

    /// A pure access-time event must not trigger a scan.
    ///
    /// The counterpart exclusion (#447): a pure *access-time* event must NOT
    /// trigger a scan. Reading a file cannot change `git status`, so waking
    /// the whole refresh pipeline for it would be pure cost — this is why the
    /// allowlist admits metadata changes generally but carves `AccessTime`
    /// out rather than accepting `Metadata(_)` wholesale.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created.
    #[test]
    fn worktree_access_time_event_does_not_trigger_git_refresh() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
        std::fs::create_dir_all(repo.join(".git")).expect("git dir");
        let file = repo.join("tracked.txt");
        std::fs::write(&file, b"hello").expect("write file");

        let registry = test_registry(true, HashSet::from([repo]));
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
                paths: vec![file.clone()],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

        assert!(
            events.is_empty(),
            "a read-only access-time touch must not wake the git refresh \
             pipeline, got: {events:?}"
        );
    }

    /// A backend swap must re-arm every already-registered directory.
    ///
    /// #442's biggest correctness trap: swapping the backend must re-arm every
    /// directory that was ALREADY registered against the old one. A swap that
    /// forgot the existing registrations would leave live repos silently unwatched — the
    /// exact failure the fallback exists to fix, just moved.
    ///
    /// The file is written only AFTER the swap, so delivery here can only come
    /// from a watch the swap itself re-armed.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the capture lock is
    /// poisoned.
    #[test]
    fn backend_swap_rearms_already_watched_paths() {
        // Probe disabled: this test drives the swap itself, so the outcome
        // does not depend on how healthy this machine's native backend is.
        let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::NativeOnly);

        // Registered BEFORE the swap — this is the path that must survive it.
        watcher.watch(&repo).expect("watch repo");
        assert_eq!(watcher.backend_kind(), Some(BackendKind::Native));

        watcher.force_poll_fallback(Duration::from_millis(200));
        assert_eq!(
            watcher.backend_kind(),
            Some(BackendKind::Poll),
            "the swap must leave the polling backend armed"
        );

        events.lock().unwrap().clear();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(repo.join("after_swap.txt"), b"content").expect("write worktree file");

        let delivered = wait_for_any_event(&events, Duration::from_secs(5));
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            delivered,
            "a repo watched before the backend swap must still be watched after it"
        );
    }

    /// A backend swap must emit one rescan per re-armed root.
    ///
    /// #442: because `PollWatcher` snapshots each path when it is armed, any
    /// change made between agent startup and the swap is invisible to it. The
    /// swap therefore emits one `Flag::Rescan` event per re-armed root, which
    /// `handle_rescan` turns into a debounced git refresh — otherwise those
    /// changes would still wait out the ~45s reconcile.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created or the capture lock is
    /// poisoned.
    #[test]
    fn backend_swap_emits_rescan_for_each_rearmed_root() {
        let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::NativeOnly);
        watcher.watch(&repo).expect("watch repo");
        events.lock().unwrap().clear();

        watcher.force_poll_fallback(Duration::from_secs(1));

        let delivered = wait_for_any_event(&events, Duration::from_secs(2));
        let repos: Vec<PathBuf> = events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.repo.clone())
            .collect();
        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            delivered,
            "the swap must emit a rescan for the watched root"
        );
        assert!(
            repos.contains(&repo),
            "the rescan must be attributed to the re-armed root, got: {repos:?}"
        );
    }

    /// The swap is idempotent: a second attempt must not tear down and rebuild
    /// a working poll backend (and so must not drop events while re-arming).
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created.
    #[test]
    fn repeated_swap_is_a_no_op() {
        let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::NativeOnly);
        watcher.watch(&repo).expect("watch repo");

        watcher.force_poll_fallback(Duration::from_secs(1));
        let after_first = wait_for_any_event(&events, Duration::from_secs(2));
        events.lock().unwrap().clear();
        watcher.force_poll_fallback(Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(200));
        let rescans_after_second = events.lock().unwrap().len();

        watcher.stop();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(after_first, "the first swap should emit a rescan");
        assert_eq!(
            rescans_after_second, 0,
            "a redundant swap must not re-arm or re-rescan"
        );
    }

    /// #463: the `PollWatcher` fallback must not watch a `.gitignore`-excluded directory.
    ///
    /// Must not register a watch on (and so must not pay the recursive stat-walk cost of) a
    /// directory the repo's own `.gitignore` excludes — a large vendored/bundled subtree is
    /// the case that motivated this, but any ignored directory qualifies.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created.
    #[test]
    fn poll_watch_targets_excludes_gitignored_directories() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
        std::fs::write(repo.join(".gitignore"), b"vendor/\n").expect("write .gitignore");
        std::fs::create_dir_all(repo.join("vendor").join("deep")).expect("vendor dir");
        std::fs::write(repo.join("vendor").join("deep").join("bundled.txt"), b"x")
            .expect("write vendored file");
        std::fs::create_dir_all(repo.join("src")).expect("src dir");

        let targets = FileSystemWatcher::poll_watch_targets(&repo, &registry);

        assert!(
            targets.contains(&repo.join("src")),
            "a non-ignored directory must still be a watch target, got: {targets:?}"
        );
        assert!(
            !targets.iter().any(|p| p.starts_with(repo.join("vendor"))),
            "a gitignored directory (and everything under it) must not be a \
             watch target, got: {targets:?}"
        );
    }

    /// #463: a `.gitignore` rule must never prune anything under `.git`, mirroring the
    /// existing `is_in_git_dir` short-circuit on the event path (#443/#447).
    ///
    /// A generically-named rule like `pack/` is a realistic pattern that would otherwise
    /// incidentally blind the poll fallback to real changes under `.git/objects/pack`.
    ///
    /// # Panics
    ///
    /// Panics if the temp repo cannot be created.
    #[test]
    fn poll_watch_targets_never_prunes_inside_git_dir() {
        let registry = test_registry(true, HashSet::new());
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
        std::fs::write(repo.join(".gitignore"), b"pack/\n").expect("write .gitignore");
        std::fs::create_dir_all(repo.join(".git").join("objects").join("pack"))
            .expect(".git/objects/pack dir");

        let targets = FileSystemWatcher::poll_watch_targets(&repo, &registry);

        assert!(
            targets.contains(&repo.join(".git").join("objects").join("pack")),
            "a .gitignore rule must never prune a directory under .git, got: {targets:?}"
        );
    }

    /// #442: the health probe must be a real, self-contained round trip through a native
    /// watcher — it manufactures its own change, so its verdict never depends on unrelated
    /// filesystem activity.
    ///
    /// It must also terminate promptly when the owning watcher is stopping.
    ///
    /// This asserts the probe TERMINATES within its bound and returns a
    /// verdict, not which verdict: on a machine with a healthy `FSEvents` it
    /// answers "healthy", and on one with a dead `FSEvents` it answers
    /// "silent". Both are correct answers about that machine.
    #[test]
    fn health_probe_terminates_within_its_timeout() {
        let stop_flag = AtomicBool::new(false);
        let started = Instant::now();
        let _verdict = native_backend_delivers_events(Duration::from_millis(600), &stop_flag);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the probe must respect its timeout, took {:?}",
            started.elapsed()
        );
    }

    /// A stopping watcher must abort the probe immediately and report
    /// "healthy", so shutdown never swaps in a backend it is about to drop.
    #[test]
    fn health_probe_aborts_and_reports_healthy_when_stopping() {
        let stop_flag = AtomicBool::new(true);
        assert!(
            native_backend_delivers_events(Duration::from_secs(30), &stop_flag),
            "an aborted probe must not report the native backend as silent"
        );
    }

    #[test]
    fn fallback_policy_defaults_to_probing_for_repo_watchers() {
        assert_eq!(
            FallbackPolicy::resolve(None, None, None),
            FallbackPolicy::ProbeThenFallback {
                probe_timeout: Duration::from_millis(DEFAULT_WATCH_PROBE_MS),
                poll_interval: Duration::from_millis(DEFAULT_WATCH_POLL_MS),
            }
        );
    }

    /// Regression test for #447: the theme/config watchers must get the same
    /// probe-and-swap as the repo watcher.
    ///
    /// #442 deliberately exempted them (they pass `watched_roots: None`) on
    /// the grounds that they already ship opt-in pollers from #354. #447
    /// disproved that: during an `FSEvents` outage the config watcher silently
    /// stopped applying config edits until restart, and an opt-in remedy is
    /// no help against a failure the user cannot observe.
    #[test]
    fn fallback_policy_probes_for_watchers_without_a_repo_registry() {
        assert_eq!(
            FallbackPolicy::resolve(None, None, None),
            FallbackPolicy::ProbeThenFallback {
                probe_timeout: Duration::from_millis(DEFAULT_WATCH_PROBE_MS),
                poll_interval: Duration::from_millis(DEFAULT_WATCH_POLL_MS),
            },
            "config/theme watchers must auto-detect a silent backend too (#447)"
        );
    }

    #[test]
    fn fallback_policy_probe_zero_disables_auto_detection() {
        assert_eq!(
            FallbackPolicy::resolve(None, Some("0"), None),
            FallbackPolicy::NativeOnly
        );
    }

    #[test]
    fn fallback_policy_honors_forced_poll() {
        assert_eq!(
            FallbackPolicy::resolve(Some("1"), None, Some("750")),
            FallbackPolicy::ForcePoll {
                poll_interval: Duration::from_millis(750),
            }
        );
    }

    #[test]
    fn fallback_policy_ignores_unparsable_and_zero_overrides() {
        assert_eq!(
            FallbackPolicy::resolve(Some("no"), Some("abc"), Some("0")),
            FallbackPolicy::ProbeThenFallback {
                probe_timeout: Duration::from_millis(DEFAULT_WATCH_PROBE_MS),
                poll_interval: Duration::from_millis(DEFAULT_WATCH_POLL_MS),
            }
        );
    }

    #[test]
    fn fallback_policy_honors_custom_probe_and_poll_intervals() {
        assert_eq!(
            FallbackPolicy::resolve(None, Some(" 500 "), Some(" 1500 ")),
            FallbackPolicy::ProbeThenFallback {
                probe_timeout: Duration::from_millis(500),
                poll_interval: Duration::from_millis(1_500),
            }
        );
    }
}

// Design reference (see docs/ARCHITECTURE.md § File Watching):
// - notify crate with 100ms debouncer drives repaint-doorbell updates for prompt refreshes
// - Filters focus on git metadata, config, and language signals while skipping heavy dirs
// - Expected characteristics: ~0.1% idle CPU, sub-50ms latency, +2–5MB RSS for watchers
// - Events flow into FileEvent and ultimately SignalHandler::trigger_update for subscribed shells
