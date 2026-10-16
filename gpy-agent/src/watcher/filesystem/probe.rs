//! Native-backend health probe and the swap to the polling fallback.

use super::super::WatchRegistry;
use super::super::watch_set::{WatchMode, WatchSet};
use super::backend::{BackendKind, WatcherBackend, build_poll_watcher, poll_watch_targets};
use notify::event::{EventKind, Flag};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Health-probe timeout: well above healthy latency (<250ms), far below reconcile.
pub(super) const DEFAULT_WATCH_PROBE_MS: u64 = 2_000;

/// `PollWatcher` fallback interval; override with `GPY_WATCH_POLL_MS`.
pub(super) const DEFAULT_WATCH_POLL_MS: u64 = 2_000;

/// Wait before re-touching the sentinel, so a pre-arm write isn't read as a dead backend.
pub(super) const PROBE_TOUCH_INTERVAL: Duration = Duration::from_millis(250);

/// The two durations [`FallbackPolicy::ProbeThenFallback`] carries.
#[derive(Clone, Copy, Debug)]
pub(super) struct ProbeTiming {
    pub(super) probe_timeout: Duration,
    pub(super) poll_interval: Duration,
}

/// A poll watcher ready to install, plus its interval.
pub(super) struct ArmedPollWatcher {
    pub(super) watcher: Box<dyn Watcher + Send>,
    pub(super) poll_interval: Duration,
}

/// Reaction to a native backend that accepts watches but delivers nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FallbackPolicy {
    /// Never probe or swap (`GPY_WATCH_PROBE_MS=0`).
    NativeOnly,
    /// Probe once in the background and swap to `PollWatcher` if silent. Default
    /// for every watcher, including theme/config.
    ProbeThenFallback {
        probe_timeout: Duration,
        poll_interval: Duration,
    },
    /// Start on `PollWatcher` without probing (`GPY_WATCH_FORCE_POLL`).
    ForcePoll { poll_interval: Duration },
}

impl FallbackPolicy {
    /// Resolve from `GPY_WATCH_FORCE_POLL`, `GPY_WATCH_PROBE_MS` (`0` = native only),
    /// and `GPY_WATCH_POLL_MS`.
    pub(super) fn from_env() -> Self {
        let force_raw = std::env::var("GPY_WATCH_FORCE_POLL").ok();
        let probe_raw = std::env::var("GPY_WATCH_PROBE_MS").ok();
        let poll_raw = std::env::var("GPY_WATCH_POLL_MS").ok();
        Self::resolve(
            force_raw.as_deref(),
            probe_raw.as_deref(),
            poll_raw.as_deref(),
        )
    }

    /// Pure form of [`FallbackPolicy::from_env`], testable without `set_var`.
    pub(super) fn resolve(
        force_raw: Option<&str>,
        probe_raw: Option<&str>,
        poll_raw: Option<&str>,
    ) -> Self {
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

/// Parse a raw millisecond env value; `None` when unset or non-numeric.
pub(super) fn parse_millis(raw: Option<&str>) -> Option<u64> {
    raw?.trim().parse::<u64>().ok()
}

/// Spawn the one-shot background health probe.
///
/// Async so startup never waits on it. One-shot: a silent `FSEvents` daemon
/// doesn't heal mid-session, and re-probing would flap and drop events.
pub(super) fn spawn_health_probe(
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
             within the probe window; falling back to polling",
        );
        engage_poll_fallback(
            &backend_for_probe,
            &tx_for_probe,
            &stop_for_probe,
            timing.poll_interval,
            &registry_for_probe,
        );
    })
}

/// First step of [`engage_poll_fallback`]: under a short lock, return the
/// recursive registrations to expand, or `None` if the swap is moot.
pub(super) fn poll_swap_snapshot(
    backend: &Arc<Mutex<WatcherBackend>>,
    stop_flag: &AtomicBool,
) -> Option<Vec<PathBuf>> {
    let snapshot_guard = backend.lock().unwrap_or_else(PoisonError::into_inner);
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

/// Final step of [`engage_poll_fallback`]: re-check stop, then swap and re-arm
/// atomically. Returns re-armed paths, or `None` if aborted.
pub(super) fn apply_poll_swap(
    backend: &Arc<Mutex<WatcherBackend>>,
    stop_flag: &AtomicBool,
    armed: ArmedPollWatcher,
    registry: &Arc<WatchRegistry>,
    precomputed: &HashMap<PathBuf, Vec<PathBuf>>,
) -> Option<Vec<PathBuf>> {
    let mut backend_guard = backend.lock().unwrap_or_else(PoisonError::into_inner);
    if stop_flag.load(Ordering::Relaxed) || backend_guard.watcher.is_none() {
        return None;
    }
    if backend_guard.kind == BackendKind::Poll {
        return None;
    }

    backend_guard.watcher = None;
    backend_guard.watcher = Some(armed.watcher);
    backend_guard.kind = BackendKind::Poll;

    let paths = rearm_poll_paths(&mut backend_guard, registry, precomputed);
    drop(backend_guard);
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

/// Replace the silent native backend with a `PollWatcher` and re-arm every path.
///
/// Expansion runs before the swap lock, since the lock is shared by every repo.
/// Swap and re-arm are atomic. Each re-armed root gets a synthetic `Rescan`,
/// since `PollWatcher` baselines at arm time and would miss earlier changes.
pub(super) fn engage_poll_fallback(
    backend: &Arc<Mutex<WatcherBackend>>,
    event_tx: &Sender<Event>,
    stop_flag: &AtomicBool,
    poll_interval: Duration,
    registry: &Arc<WatchRegistry>,
) {
    if stop_flag.load(Ordering::Relaxed) {
        return;
    }

    // Built outside the lock: construction spawns a thread and scans.
    let poll_watcher = match build_poll_watcher(event_tx, poll_interval) {
        Ok(watcher) => watcher,
        Err(error) => {
            // Keep the silent native backend; the reconcile remains the backstop.
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

    let Some(recursive_paths) = poll_swap_snapshot(backend, stop_flag) else {
        return;
    };

    // Expand with no lock held; it may run `git ls-files`.
    let mut precomputed = HashMap::with_capacity(recursive_paths.len());
    for path in recursive_paths {
        if path.is_dir() {
            let targets = poll_watch_targets(&path, registry);
            precomputed.insert(path, targets);
        }
    }

    let armed = ArmedPollWatcher {
        watcher: poll_watcher,
        poll_interval,
    };
    let Some(paths) = apply_poll_swap(backend, stop_flag, armed, registry, &precomputed) else {
        return;
    };

    for path in paths {
        let rescan = Event::new(EventKind::Any)
            .add_path(path)
            .set_flag(Flag::Rescan);
        let _ = event_tx.send(rescan);
    }
}

/// Rebuild `state`'s OS watch set for the newly installed `PollWatcher`,
/// returning each re-armed path once.
///
/// Registrations missing from `precomputed` raced in after the snapshot and
/// are expanded here under the lock.
pub(super) fn rearm_poll_paths(
    state: &mut WatcherBackend,
    registry: &Arc<WatchRegistry>,
    precomputed: &HashMap<PathBuf, Vec<PathBuf>>,
) -> Vec<PathBuf> {
    state.os = WatchSet::new(state.kind.removal_cascades());
    let mut ops = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    for ((path, mode), registration) in &mut state.registrations {
        // Never expand shallow registrations (e.g. `objects/`) recursively.
        let poll_targets = (*mode == WatchMode::Recursive && path.is_dir()).then(|| {
            precomputed
                .get(path)
                .map_or_else(|| poll_watch_targets(path, registry), Clone::clone)
        });
        registration.targets = WatcherBackend::targets_for(state.kind, path, *mode, poll_targets);
        for (target, target_mode) in &registration.targets {
            ops.extend(state.os.acquire(target, *target_mode));
        }
        // Path-first key order puts dual-mode paths adjacent for `dedup`.
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

/// Whether the native backend actually delivers events, tested by writing a
/// sentinel in a private temp dir watched on a separate channel.
///
/// Returns `true` if the probe can't be set up, so it never demotes a working backend.
pub(super) fn native_backend_delivers_events(timeout: Duration, stop_flag: &AtomicBool) -> bool {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_u128, |since| since.as_nanos());
    let probe_dir =
        std::env::temp_dir().join(format!("gpy-watch-probe-{}-{nanos}", std::process::id()));
    if std::fs::create_dir_all(&probe_dir).is_err() {
        return true;
    }
    // macOS resolves `/var` -> `/private/var`; probe the canonical form real watches use.
    let canonical_dir = std::fs::canonicalize(&probe_dir).unwrap_or_else(|_| probe_dir.clone());

    let delivered = probe_sentinel_event(&canonical_dir, timeout, stop_flag);

    let _ = std::fs::remove_dir_all(&probe_dir);
    delivered
}

/// Watch `probe_dir`, repeatedly touch a sentinel, and report whether any event arrived.
pub(super) fn probe_sentinel_event(
    probe_dir: &Path,
    timeout: Duration,
    stop_flag: &AtomicBool,
) -> bool {
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
        // `Disconnected` also reports healthy: a broken probe never demotes.
        if !matches!(
            probe_rx.recv_timeout(PROBE_TOUCH_INTERVAL),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            return true;
        }
    }

    false
}
