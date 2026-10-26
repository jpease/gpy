#![allow(
    clippy::missing_panics_doc,
    reason = "test helpers panic on broken fixtures"
)]

use super::{PendingEvent, WatchCoordinator, filesystem, ingest};
use crate::watcher::watch_set::WatchMode;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

impl WatchCoordinator {
    /// Logical registrations (with owner counts) and the OS watches beneath them.
    pub(crate) fn held_watches(
        &self,
    ) -> (
        BTreeMap<(PathBuf, WatchMode), u32>,
        BTreeMap<PathBuf, WatchMode>,
    ) {
        self.watcher.as_ref().map_or_else(
            Default::default,
            filesystem::FileSystemWatcher::held_watches,
        )
    }

    /// Feed `pending` through the production ingress path, kick included.
    fn ingest(&self, pending: PendingEvent) {
        ingest(&self.debouncer, &self.flush_kick, pending);
    }
}

/// A debounce longer than the CLI's 2s reload budget must not delay `stop`.
#[test]
fn stop_returns_promptly_regardless_of_debounce_window() {
    let mut coordinator =
        WatchCoordinator::new(Duration::from_secs(30), Box::new(|_event| {}), None)
            .expect("coordinator");

    let started = Instant::now();
    coordinator.stop();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "stop() must not wait out the debounce window, took {elapsed:?}"
    );
}

/// A coordinator whose callback reports each delivery instant on the channel.
fn timed_coordinator(debounce: Duration) -> (WatchCoordinator, mpsc::Receiver<Instant>) {
    let (tx, rx) = mpsc::channel();
    let coordinator = WatchCoordinator::new(
        debounce,
        Box::new(move |_| {
            let _ = tx.send(Instant::now());
        }),
        None,
    )
    .expect("coordinator");
    (coordinator, rx)
}

fn head_event() -> PendingEvent {
    PendingEvent::git(PathBuf::from("/repo"), PathBuf::from("/repo/.git/HEAD"))
}

/// A pending event's far-off deadline must not delay `stop`.
#[test]
fn stop_returns_promptly_with_pending_events() {
    let (mut coordinator, _rx) = timed_coordinator(Duration::from_secs(30));
    coordinator.ingest(head_event());

    let started = Instant::now();
    coordinator.stop();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "stop() must not wait out a pending deadline, took {elapsed:?}"
    );
}

/// The flush thread waits for the debounce deadline before delivering.
#[test]
fn event_is_not_flushed_before_its_deadline() {
    let debounce = Duration::from_millis(100);
    let (mut coordinator, rx) = timed_coordinator(debounce);

    let t0 = Instant::now();
    coordinator.ingest(head_event());
    let delivered = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("event delivered");
    coordinator.stop();

    assert!(
        delivered.duration_since(t0) >= debounce,
        "delivered {:?} after ingest, before the {debounce:?} debounce",
        delivered.duration_since(t0)
    );
}

/// One event with nothing after it is still delivered near its deadline: the
/// ingress kick arms the wait, so delivery needs neither a later event nor `stop`.
#[cfg(unix)]
#[test]
fn event_is_flushed_without_a_further_event() {
    let (mut coordinator, rx) = timed_coordinator(Duration::from_millis(100));

    coordinator.ingest(head_event());
    let delivered = rx.recv_timeout(Duration::from_millis(350));
    coordinator.stop();

    assert!(
        delivered.is_ok(),
        "the event must be delivered within debounce + 250ms"
    );
}
