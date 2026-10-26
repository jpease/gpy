//! Single-repository watcher with debouncing and a flush thread.

use super::{DebouncedEvent, PendingEvent, WatchRegistry, debouncer, filesystem};
use crate::Result;
use crate::sync_ext::MutexExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Callback invoked once an event has passed through debouncing.
pub type EventCallback = Box<dyn Fn(DebouncedEvent) + Send + Sync>;

/// Callback invoked before debouncing (raw pending events).
pub type PendingCallback = Box<dyn Fn(PendingEvent) + Send + Sync>;

/// Schedules a [`PendingEvent`] for delivery no earlier than `due`, coalescing
/// like an immediate event. Synchronous and cheap; the flush thread waits.
pub type DelayedEventScheduler = Arc<dyn Fn(PendingEvent, Instant) + Send + Sync>;

/// Insert under the debouncer lock, release it, then wake the flush thread.
fn ingest(
    debouncer: &Mutex<debouncer::DebounceEngine>,
    kick: &mpsc::Sender<()>,
    pending: PendingEvent,
) {
    debouncer.lock_or_recover().handle_event(pending);
    let _ = kick.send(());
}

/// Single-repository file system watcher with debouncing.
pub struct WatchCoordinator {
    watcher: Option<filesystem::FileSystemWatcher>,
    debouncer: Arc<Mutex<debouncer::DebounceEngine>>,
    callback: Arc<EventCallback>,
    stop_flag: Arc<AtomicBool>,
    flush_handle: Option<JoinHandle<()>>,
    /// Wakes the flush thread: new pending events and `stop` send on it.
    flush_kick: mpsc::Sender<()>,
    /// Registry shared with the filesystem layer.
    registry: Arc<WatchRegistry>,
}

impl WatchCoordinator {
    /// Create a new file watcher with callback. `registry: None` gives a
    /// private, empty registry.
    ///
    /// # Errors
    /// Returns an error if the file system watcher cannot be created.
    pub fn new(
        debounce: Duration,
        callback: EventCallback,
        registry: Option<Arc<WatchRegistry>>,
    ) -> Result<Self> {
        let shared_registry = registry.unwrap_or_else(|| Arc::new(WatchRegistry::new()));
        let event_debouncer = debouncer::DebounceEngine::new(debounce);
        let callback_arc = Arc::new(callback);

        // Feed watcher events into the debouncer.
        let debouncer_arc = Arc::new(Mutex::new(event_debouncer));
        let debouncer_for_watcher = Arc::clone(&debouncer_arc);
        let (flush_kick, kick_rx) = mpsc::channel::<()>();

        let kick_for_ingress = flush_kick.clone();
        let ingress_callback: PendingCallback =
            Box::new(move |pending| ingest(&debouncer_for_watcher, &kick_for_ingress, pending));

        // Directory-create follow-up uses a due-time floor in the same
        // debouncer instead of a timer thread per directory.
        let debouncer_for_schedule = Arc::clone(&debouncer_arc);
        let kick_for_schedule = flush_kick.clone();
        let schedule_delayed: DelayedEventScheduler = Arc::new(move |pending, due| {
            debouncer_for_schedule
                .lock_or_recover()
                .handle_event_due(pending, due);
            let _ = kick_for_schedule.send(());
        });

        // Follow-up delay = debounce window, so contents created before the
        // new directory's watch was armed still get rescanned.
        let watcher = filesystem::FileSystemWatcher::new(
            ingress_callback,
            Some(Arc::clone(&shared_registry)),
            debounce,
            schedule_delayed,
        )?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_thread = Arc::clone(&stop_flag);
        let debouncer_for_thread = Arc::clone(&debouncer_arc);
        let callback_for_thread = Arc::clone(&callback_arc);
        let flush_handle = std::thread::spawn(move || {
            loop {
                while kick_rx.try_recv().is_ok() {}
                if stop_flag_thread.load(Ordering::Relaxed) {
                    break;
                }
                let (expired, deadline) = {
                    let mut engine = debouncer_for_thread.lock_or_recover();
                    (engine.drain_expired_events(), engine.next_deadline())
                };
                // Dispatch each event on its own thread, off the flush loop
                // and outside the debouncer lock, so a slow callback for one
                // repo cannot delay flushing or delivery for another.
                for event in expired {
                    let callback_for_event = Arc::clone(&callback_for_thread);
                    std::thread::spawn(move || {
                        (callback_for_event)(event);
                    });
                }
                // A kick sent after the lock was released stays queued, so
                // no wakeup is lost between the drain and this wait.
                let disconnected = deadline.map_or_else(
                    || kick_rx.recv().is_err(),
                    |at| {
                        matches!(
                            kick_rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                            Err(mpsc::RecvTimeoutError::Disconnected)
                        )
                    },
                );
                if disconnected {
                    break;
                }
            }
        });

        Ok(Self {
            watcher: Some(watcher),
            debouncer: debouncer_arc,
            callback: callback_arc,
            stop_flag,
            flush_handle: Some(flush_handle),
            flush_kick,
            registry: shared_registry,
        })
    }

    /// The [`WatchRegistry`] this coordinator and its filesystem layer share.
    #[must_use]
    pub const fn registry(&self) -> &Arc<WatchRegistry> {
        &self.registry
    }

    /// Start watching a directory
    ///
    /// # Errors
    /// Returns an error if the directory cannot be watched.
    pub fn watch_directory<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if let Some(watcher) = &mut self.watcher {
            watcher.watch(path)?;
        }
        Ok(())
    }

    /// Start watching a directory non-recursively.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be watched.
    pub fn watch_directory_shallow<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if let Some(watcher) = &mut self.watcher {
            watcher.watch_shallow(path)?;
        }
        Ok(())
    }

    /// Whether an already-armed watch delivers events for `path`
    /// (see [`filesystem::FileSystemWatcher::covers`]).
    #[must_use]
    pub fn covers_directory<P: AsRef<Path>>(&self, path: P) -> bool {
        self.watcher
            .as_ref()
            .is_some_and(|watcher| watcher.covers(path))
    }

    /// Release one recursive [`WatchCoordinator::watch_directory`] of `path`
    /// (see [`filesystem::FileSystemWatcher::unwatch`]).
    ///
    /// # Errors
    /// Returns an error if the directory cannot be unwatched.
    pub fn unwatch_directory<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if let Some(watcher) = &mut self.watcher {
            watcher.unwatch(path)?;
        }
        Ok(())
    }

    /// Release one [`WatchCoordinator::watch_directory_shallow`] of `path`
    /// (see [`filesystem::FileSystemWatcher::unwatch_shallow`]).
    ///
    /// # Errors
    /// Returns an error if the directory cannot be unwatched.
    pub fn unwatch_directory_shallow<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if let Some(watcher) = &mut self.watcher {
            watcher.unwatch_shallow(path)?;
        }
        Ok(())
    }

    /// Stop all watching
    pub fn stop(&mut self) {
        if let Some(watcher) = &mut self.watcher {
            watcher.stop();
        }
        self.watcher = None;

        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.flush_kick.send(());
        if let Some(handle) = self.flush_handle.take() {
            let _ = handle.join();
        }

        self.debouncer.lock_or_recover().clear();
    }

    /// Deliver pending debounced events synchronously; the callback runs
    /// after the debouncer lock is released.
    pub fn flush_pending_events(&self) {
        let expired = self.debouncer.lock_or_recover().drain_expired_events();
        for event in expired {
            (self.callback)(event);
        }
    }
}

impl Drop for WatchCoordinator {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
