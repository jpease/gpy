//! Shared hot-reload machinery for managers that watch a single TOML file:
//! OS notifications plus an optional polling fallback.

use crate::Result;
use crate::env_config::env_u64_positive;
use crate::sync_ext::MutexExt;
use crate::watcher::WatchCoordinator;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Debounce for the config and theme watchers. The debouncer's 1 s max-delay
/// cap flushes an editor's multi-write save at about this deadline anyway, and
/// a reload of unchanged content is a no-op, so a longer window would only
/// delay delivery of edits.
pub const HOT_RELOAD_DEBOUNCE: Duration = Duration::from_secs(1);

/// The longest the poll thread sleeps before re-checking its stop flag.
const FLUSH_STOP_POLL: Duration = Duration::from_millis(25);

/// Sleep for `total`, waking early once `stop` is set.
fn sleep_unless_stopped(total: Duration, stop: &AtomicBool) {
    let deadline = Instant::now().checked_add(total);
    while !stop.load(Ordering::Relaxed) {
        let remaining = deadline.map_or(total, |at| at.saturating_duration_since(Instant::now()));
        if remaining.is_zero() {
            return;
        }
        std::thread::sleep(remaining.min(FLUSH_STOP_POLL));
    }
}

/// A fingerprint of a watched file's contents; `None` when unreadable.
///
/// Content, not mtime: Windows `LastWriteTime` is too coarse (~2–15.6ms), so
/// two quick writes would collide and the second be missed permanently.
pub fn content_fingerprint(path: &Path) -> Option<u64> {
    Some(bytes_fingerprint(&std::fs::read(path).ok()?))
}

/// A fingerprint of `bytes`, comparable with [`content_fingerprint`].
/// `DefaultHasher` is unstable across releases, but values are never persisted.
pub fn bytes_fingerprint(bytes: &[u8]) -> u64 {
    use std::hash::Hasher as _;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// Resolves the file the poll loop should check on this tick.
pub type PathSource = Box<dyn Fn() -> Option<PathBuf> + Send>;

/// Applies a change the poll loop detected in that file.
pub type ReloadAction = Box<dyn Fn(&Path) + Send>;

/// The two manager-specific halves of a poll loop.
pub struct PollTarget {
    /// Which file to check; consulted every tick because the path can be
    /// repointed at runtime. `None` ends the loop.
    pub path_of: PathSource,
    /// Applied when that file's contents change.
    pub reload: ReloadAction,
}

/// The file watcher and poll-fallback thread backing one hot-reloaded file.
pub struct HotReloadSlot {
    /// OS-level file watcher, when watching.
    watcher: Arc<Mutex<Option<WatchCoordinator>>>,
    /// Stop flag for the poll-fallback thread.
    poll_stop: Arc<AtomicBool>,
    /// Handle for the poll-fallback thread, if running.
    poll_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Whether a vanished polled file triggers the reload.
    reload_on_missing: bool,
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
            reload_on_missing: false,
        }
    }

    /// Also reload when the polled file is gone for two consecutive ticks
    /// (for managers that map "no file" to defaults).
    #[must_use]
    pub const fn reloading_on_missing(mut self) -> Self {
        self.reload_on_missing = true;
        self
    }

    /// A handle to the watcher slot, for a reload callback that must re-arm
    /// watches. Callers must only `try_lock` it: [`stop`](Self::stop) holds the
    /// lock while it joins the thread running that callback.
    pub fn watcher_handle(&self) -> Arc<Mutex<Option<WatchCoordinator>>> {
        Arc::clone(&self.watcher)
    }

    /// Install the watcher `build` returns; `Ok(false)` if one already runs.
    /// Check and install share one lock so concurrent callers cannot both install.
    ///
    /// # Errors
    ///
    /// Returns an error if `build` fails.
    pub fn start_watcher<F>(&self, build: F) -> Result<bool>
    where
        F: FnOnce() -> Result<WatchCoordinator>,
    {
        let mut watcher_guard = self.watcher.lock_or_recover();
        if watcher_guard.is_some() {
            return Ok(false);
        }

        *watcher_guard = Some(build()?);
        drop(watcher_guard);
        Ok(true)
    }

    /// Start the poll fallback when `env_var` holds positive milliseconds, for
    /// environments where OS notifications (`FSEvents`) never arrive.
    pub fn start_poll_fallback(
        &self,
        env_var: &str,
        debounce_duration: Duration,
        target: PollTarget,
    ) {
        let Some(poll_interval_ms) = env_u64_positive(env_var) else {
            return;
        };
        self.spawn_poll_thread(
            Duration::from_millis(poll_interval_ms),
            debounce_duration,
            target,
        );
    }

    /// Spawn the poll thread with a resolved interval (no-op if running).
    /// Split out so tests avoid `set_var`; interval is floored at `debounce_duration`.
    pub fn spawn_poll_thread(
        &self,
        poll_interval: Duration,
        debounce_duration: Duration,
        target: PollTarget,
    ) {
        let PollTarget { path_of, reload } = target;
        let mut handle_guard = self.poll_handle.lock_or_recover();
        if handle_guard.is_some() {
            return;
        }

        self.poll_stop.store(false, Ordering::Relaxed);

        let stop_flag = Arc::clone(&self.poll_stop);
        let debounce_ms = u64::try_from(debounce_duration.as_millis()).unwrap_or(u64::MAX);
        let poll_ms = u64::try_from(poll_interval.as_millis()).unwrap_or(u64::MAX);
        let interval = Duration::from_millis(poll_ms.max(debounce_ms));

        // Capture the baseline before spawning: a lazy read on the thread would
        // race a write landing before its first tick and miss it forever.
        let initial_fingerprint = path_of().as_deref().and_then(content_fingerprint);

        let reload_on_missing = self.reload_on_missing;
        let handle = std::thread::spawn(move || {
            let mut last_fingerprint = initial_fingerprint;
            // A deletion is only acted on once seen on two consecutive ticks:
            // an atomic-rename save briefly leaves no file.
            let mut missing_once = false;
            while !stop_flag.load(Ordering::Relaxed) {
                let Some(path) = path_of() else {
                    break;
                };

                match content_fingerprint(&path) {
                    Some(fingerprint) => {
                        missing_once = false;
                        let changed = last_fingerprint != Some(fingerprint);
                        if changed {
                            last_fingerprint = Some(fingerprint);
                            reload(&path);
                        }
                    }
                    None if reload_on_missing && last_fingerprint.is_some() => {
                        if missing_once {
                            missing_once = false;
                            last_fingerprint = None;
                            reload(&path);
                        } else {
                            missing_once = true;
                        }
                    }
                    None => {}
                }

                sleep_unless_stopped(interval, &stop_flag);
            }
        });

        *handle_guard = Some(handle);
    }

    /// Stop the poll thread and watcher (idempotent); `true` if a watcher was taken down.
    pub fn stop(&self) -> bool {
        self.poll_stop.store(true, Ordering::Relaxed);
        {
            let mut handle_guard = self.poll_handle.lock_or_recover();
            if let Some(handle) = handle_guard.take()
                && let Err(e) = handle.join()
            {
                crate::debug::write_debug_log(
                    "watcher",
                    &format!("hot-reload poll thread panicked: {e:?}"),
                );
            }
        }

        let mut watcher_guard = self.watcher.lock_or_recover();
        watcher_guard
            .take()
            .map(|mut watcher| watcher.stop())
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{HotReloadSlot, PollTarget, content_fingerprint};
    use crate::sync_ext::MutexExt;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A same-length rewrite landing right after spawn (before the first tick)
    /// must still reload: the baseline is captured before the thread starts.
    #[test]
    fn poll_detects_a_same_length_rewrite_written_right_after_spawn() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let file = temp.path().join("watched.toml");
        std::fs::write(&file, "color = \"white\"\n").expect("write file");

        let reloads = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&reloads);
        let watched = file.clone();
        let slot = HotReloadSlot::new();
        slot.spawn_poll_thread(
            Duration::from_millis(10),
            Duration::from_millis(10),
            PollTarget {
                path_of: Box::new(move || Some(watched.clone())),
                reload: Box::new(move |path| {
                    let content = std::fs::read_to_string(path).unwrap_or_default();
                    recorded.lock_or_recover().push(content);
                }),
            },
        );
        std::fs::write(&file, "color = \"black\"\n").expect("rewrite file");

        let deadline = Instant::now() + Duration::from_secs(6);
        let seen = loop {
            let seen = reloads
                .lock_or_recover()
                .iter()
                .any(|content| content.contains("black"));
            if seen || Instant::now() > deadline {
                break seen;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        slot.stop();
        assert!(
            seen,
            "the poll fallback missed a same-length rewrite written right after spawn"
        );
    }

    /// Same-length payloads written close together must still read as a change.
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

        // Identical bytes must not read as a change.
        std::fs::write(&path, disabled).expect("rewrite identical content");
        assert_eq!(
            after,
            content_fingerprint(&path),
            "identical content must fingerprint identically"
        );

        assert_eq!(
            content_fingerprint(&temp.path().join("absent.toml")),
            None,
            "a missing file must not fingerprint"
        );
    }

    /// `stop()` must wake the poll thread promptly, not sleep out its interval.
    #[test]
    fn stop_does_not_wait_for_full_poll_interval() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let file = temp.path().join("watched.toml");
        std::fs::write(&file, "a = 1\n").expect("write file");

        let slot = HotReloadSlot::new();
        slot.spawn_poll_thread(
            Duration::from_secs(5),
            Duration::from_secs(5),
            PollTarget {
                path_of: Box::new(move || Some(file.clone())),
                reload: Box::new(|_| {}),
            },
        );
        // Let the thread finish its first tick and enter its sleep.
        std::thread::sleep(Duration::from_millis(300));

        let started = Instant::now();
        slot.stop();
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "stop() blocked for {elapsed:?}; it must not wait out the poll interval"
        );
    }
}
