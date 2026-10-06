//! One hot-reload mechanism, shared by every manager that watches a single
//! TOML file for changes (#588).
//!
//! `ConfigManager` and `ThemeManager` each grew their own copy of the same
//! three-part machine: a [`WatchCoordinator`] for OS-level notifications, a
//! polling fallback thread for environments where those notifications never
//! arrive (#354), and a stop/join pair to take both down. Copying it meant
//! fixing it twice: the TOCTOU baseline bug (#385) and the
//! timestamp-granularity bug (#529) were each found once and then ported by
//! hand, months apart (#567 was the theme-side port). [`HotReloadSlot`] is that
//! machine, written once.
//!
//! What stays with each manager is the part that genuinely differs: which file
//! to watch, and what to do when it changes. Those arrive as a [`PollTarget`],
//! so the slot never needs to know what a theme or a config is — which is also
//! what makes it reusable by a third caller (palette hot-reload, if it lands)
//! without another copy.

use crate::watcher::WatchCoordinator;
use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// A fingerprint of a watched file's *contents*.
///
/// `None` when the file cannot be read.
///
/// The poll loop used to compare `metadata().modified()` for equality.
/// Windows file timestamps do not advance fast enough for that: measured on
/// Windows 11, 17 of 20 back-to-back rewrites shared one `LastWriteTime`, the
/// clock moving in ~2ms steps (~15.6ms at the default timer resolution). Two
/// writes inside one tick were therefore indistinguishable — and because the
/// loop stored whatever it observed as the next baseline, the second write was
/// missed *permanently*, not merely late (#529 for the config manager, #567
/// for the theme manager's copy of the same bug).
///
/// Hashing the bytes takes the clock out of the decision entirely. It also
/// stops a touch that leaves the content unchanged from triggering a pointless
/// reload, which the mtime comparison could not distinguish.
///
/// `DefaultHasher` is not stable across Rust releases. That is fine here: the
/// value is compared only against another fingerprint taken by the same
/// process, and is never persisted or sent over the wire.
pub fn content_fingerprint(path: &Path) -> Option<u64> {
    use std::hash::Hasher as _;

    let bytes = std::fs::read(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(&bytes);
    Some(hasher.finish())
}

/// Parse a raw poll-interval env value into milliseconds.
///
/// `None` when unset, non-numeric, or `0` (disabled) — the poll fallback stays
/// off by default. Pure function, no I/O, so it's directly testable without
/// touching the real process environment.
fn parse_poll_interval_ms(raw: Option<&str>) -> Option<u64> {
    raw?.parse::<u64>().ok().filter(|value| *value > 0)
}

/// Resolves the file the poll loop should check on this tick.
pub type PathSource = Box<dyn Fn() -> Option<PathBuf> + Send>;

/// Applies a change the poll loop detected in that file.
pub type ReloadAction = Box<dyn Fn(&Path) + Send>;

/// The two manager-specific halves of a poll loop.
///
/// Keeping them together names what the generic machinery is missing: which
/// file to look at, and what to do when it changes.
pub struct PollTarget {
    /// Which file to check on each tick. Consulted every tick rather than once,
    /// because the watched path can be repointed at runtime
    /// (`ThemeManager::switch_theme`); returning `None` ends the loop, which is
    /// how a poisoned manager lock retires the thread instead of spinning on it.
    pub path_of: PathSource,
    /// Applied when that file's contents change.
    pub reload: ReloadAction,
}

/// The file watcher and poll-fallback thread backing one hot-reloaded file.
///
/// Owns no domain state: the manager keeps its own `Arc<RwLock<Arc<_>>>` and
/// hands this type closures that read the current path and apply a reload.
pub struct HotReloadSlot {
    /// OS-level file watcher, when watching.
    watcher: Arc<Mutex<Option<WatchCoordinator>>>,
    /// Stop flag for the poll-fallback thread.
    poll_stop: Arc<AtomicBool>,
    /// Handle for the poll-fallback thread, if running.
    poll_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Default for HotReloadSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl HotReloadSlot {
    /// An idle slot: no watcher, no poll thread.
    pub fn new() -> Self {
        Self {
            watcher: Arc::new(Mutex::new(None)),
            poll_stop: Arc::new(AtomicBool::new(false)),
            poll_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// A handle to the watcher slot, for a reload callback that must re-arm
    /// watches. Callers must only `try_lock` it: [`stop`](Self::stop) holds the
    /// lock while it joins the thread running that callback.
    pub fn watcher_handle(&self) -> Arc<Mutex<Option<WatchCoordinator>>> {
        Arc::clone(&self.watcher)
    }

    /// Install the watcher `build` returns, unless one is already installed.
    ///
    /// Returns `Ok(false)` when a watcher was already running, in which case
    /// `build` is never called and nothing changes — the manager's
    /// "already watching, ignore" no-op. The check and the install happen under
    /// one lock so two concurrent callers cannot both install a watcher.
    ///
    /// # Errors
    ///
    /// Returns an error if the watcher lock is poisoned, or if `build` fails.
    pub fn start_watcher<F>(&self, build: F) -> Result<bool>
    where
        F: FnOnce() -> Result<WatchCoordinator>,
    {
        let mut watcher_guard = self
            .watcher
            .lock()
            .map_err(|e| Error::config(format!("Failed to acquire watcher lock: {e}")))?;
        if watcher_guard.is_some() {
            return Ok(false);
        }

        *watcher_guard = Some(build()?);
        drop(watcher_guard);
        Ok(true)
    }

    /// Start the poll fallback when `env_var` holds a positive integer number
    /// of milliseconds. No-op otherwise.
    ///
    /// This is a defense-in-depth path for environments where the file
    /// watcher's OS-level notifications (`FSEvents` on macOS) never arrive —
    /// see #354.
    pub fn start_poll_fallback(
        &self,
        env_var: &str,
        debounce_duration: Duration,
        target: PollTarget,
    ) {
        let raw = std::env::var(env_var).ok();
        let Some(poll_interval_ms) = parse_poll_interval_ms(raw.as_deref()) else {
            return;
        };
        self.spawn_poll_thread(
            Duration::from_millis(poll_interval_ms),
            debounce_duration,
            target,
        );
    }

    /// Spawn the poll-fallback thread with an already-resolved poll interval.
    /// No-op if a poll thread is already running. Split out from
    /// [`start_poll_fallback`](Self::start_poll_fallback) so tests can exercise
    /// polling directly without mutating the process environment (this crate
    /// forbids `unsafe_code`, and `std::env::set_var` requires it).
    ///
    /// `poll_interval` is floored at `debounce_duration`: never poll faster
    /// than the watcher's own debounce would allow it to react.
    pub fn spawn_poll_thread(
        &self,
        poll_interval: Duration,
        debounce_duration: Duration,
        target: PollTarget,
    ) {
        let PollTarget { path_of, reload } = target;
        let Ok(mut handle_guard) = self.poll_handle.lock() else {
            return;
        };
        if handle_guard.is_some() {
            return;
        }

        self.poll_stop.store(false, Ordering::Relaxed);

        let stop_flag = Arc::clone(&self.poll_stop);
        let debounce_ms = u64::try_from(debounce_duration.as_millis()).unwrap_or(u64::MAX);
        let poll_ms = u64::try_from(poll_interval.as_millis()).unwrap_or(u64::MAX);
        let interval = Duration::from_millis(poll_ms.max(debounce_ms));

        // Capture the baseline fingerprint synchronously, on the calling
        // thread, before spawning the poll thread (#385). Reading it lazily as
        // the spawned thread's first statement created a TOCTOU race: if the
        // file was rewritten after this function returned but before the OS
        // actually scheduled the new thread's first tick, that first read would
        // already observe the post-write state as its "baseline" — so unless
        // the file was written *again*, the poll loop would never see a
        // difference and would silently never reload. Capturing it here
        // guarantees the baseline reflects the file state at spawn time,
        // strictly before any write the caller makes afterward.
        let initial_fingerprint = path_of().as_deref().and_then(content_fingerprint);

        let handle = std::thread::spawn(move || {
            let mut last_fingerprint = initial_fingerprint;
            while !stop_flag.load(Ordering::Relaxed) {
                let Some(path) = path_of() else {
                    break;
                };

                if let Some(fingerprint) = content_fingerprint(&path) {
                    let changed = last_fingerprint != Some(fingerprint);
                    if changed {
                        last_fingerprint = Some(fingerprint);
                        reload(&path);
                    }
                }

                std::thread::sleep(interval);
            }
        });

        *handle_guard = Some(handle);
    }

    /// Stop the poll thread and the file watcher.
    ///
    /// Idempotent — calling it repeatedly is safe. Returns `true` if a watcher
    /// was actually taken down, so callers that log or unregister on teardown
    /// can tell a real stop from a no-op.
    pub fn stop(&self) -> bool {
        self.poll_stop.store(true, Ordering::Relaxed);
        if let Ok(mut handle_guard) = self.poll_handle.lock()
            && let Some(handle) = handle_guard.take()
        {
            let _ = handle.join();
        }

        if let Ok(mut watcher_guard) = self.watcher.lock()
            && let Some(mut watcher) = watcher_guard.take()
        {
            watcher.stop();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{content_fingerprint, parse_poll_interval_ms};

    /// Pins the #529/#567 regression once, for every manager sharing this
    /// function.
    ///
    /// Change detection must not be decidable by file size, and must not depend
    /// on the clock. Both payloads are the same length, and on Windows two
    /// writes this close together routinely share one `LastWriteTime`.
    #[test]
    fn content_fingerprint_tracks_content_not_length_or_mtime() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let path = temp.path().join("watched.toml");

        let enabled = "[git]\nenabled = true \n";
        let disabled = "[git]\nenabled = false\n";
        assert_eq!(
            enabled.len(),
            disabled.len(),
            "the two payloads must be the same length for this test to mean anything"
        );

        std::fs::write(&path, enabled).expect("write file");
        let before = content_fingerprint(&path);
        std::fs::write(&path, disabled).expect("rewrite file");
        let after = content_fingerprint(&path);

        assert!(before.is_some(), "a readable file must fingerprint");
        assert_ne!(
            before, after,
            "a same-length rewrite must change the fingerprint"
        );

        // Rewriting identical bytes must NOT read as a change, so a touch
        // cannot trigger a pointless reload.
        std::fs::write(&path, disabled).expect("rewrite identical content");
        assert_eq!(
            after,
            content_fingerprint(&path),
            "identical content must fingerprint identically"
        );

        // An unreadable path yields None rather than a value that could
        // collide with a real fingerprint.
        assert_eq!(
            content_fingerprint(&temp.path().join("absent.toml")),
            None,
            "a missing file must not fingerprint"
        );
    }

    #[test]
    fn parse_poll_interval_ms_rejects_unset_zero_and_invalid() {
        assert_eq!(parse_poll_interval_ms(None), None);
        assert_eq!(parse_poll_interval_ms(Some("0")), None);
        assert_eq!(parse_poll_interval_ms(Some("abc")), None);
        assert_eq!(parse_poll_interval_ms(Some("25")), Some(25_u64));
    }
}
