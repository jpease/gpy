#![allow(
    clippy::missing_panics_doc,
    reason = "test helpers panic on broken fixtures"
)]

use super::{PendingEvent, WatchCoordinator, filesystem, ingest};
use crate::watcher::watch_set::WatchMode;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, mpsc};
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

/// A Git event for `repo`; distinct repos never coalesce.
fn event_for(name: &str) -> PendingEvent {
    let repo = PathBuf::from(name);
    let head = repo.join(".git/HEAD");
    PendingEvent::git(repo, head)
}

/// Thirty repositories' events run on at most the fixed pool of four threads.
#[test]
fn flushed_events_run_on_at_most_four_threads() {
    let (event_tx, rx) = mpsc::channel();
    let tx = Mutex::new(event_tx);
    let mut coordinator = WatchCoordinator::new(
        Duration::from_millis(20),
        Box::new(move |_event| {
            let _ = tx
                .lock()
                .expect("sender lock")
                .send(std::thread::current().id());
        }),
        None,
    )
    .expect("coordinator");

    for index in 0_i32..30_i32 {
        coordinator.ingest(event_for(&format!("/repo{index}")));
    }
    let mut threads = HashSet::new();
    for _ in 0_i32..30_i32 {
        threads.insert(
            rx.recv_timeout(Duration::from_secs(10))
                .expect("event delivered"),
        );
    }
    coordinator.stop();

    assert!(
        threads.len() <= 4,
        "30 events ran on {} threads, want at most 4",
        threads.len()
    );
}

/// A coordinator plus the channels observing its callbacks.
struct SlowAndFast {
    started: mpsc::Receiver<()>,
    delivered: mpsc::Receiver<PathBuf>,
    release: mpsc::Sender<()>,
    coordinator: WatchCoordinator,
}

/// A coordinator whose callbacks for repositories under `/slow` block until released
/// and whose other callbacks report their repository.
fn slow_and_fast(debounce: Duration) -> SlowAndFast {
    let (started_sender, started) = mpsc::channel();
    let (delivered_sender, delivered) = mpsc::channel();
    let (release, release_receiver) = mpsc::channel::<()>();
    let started_tx = Mutex::new(started_sender);
    let delivered_tx = Mutex::new(delivered_sender);
    let release_rx = Mutex::new(release_receiver);
    let coordinator = WatchCoordinator::new(
        debounce,
        Box::new(move |event| {
            if event.repo.starts_with("/slow") {
                let _ = started_tx.lock().expect("started lock").send(());
                let _ = release_rx
                    .lock()
                    .expect("release lock")
                    .recv_timeout(Duration::from_secs(10));
            } else {
                let _ = delivered_tx
                    .lock()
                    .expect("delivered lock")
                    .send(event.repo);
            }
        }),
        None,
    )
    .expect("coordinator");
    SlowAndFast {
        started,
        delivered,
        release,
        coordinator,
    }
}

/// A callback stuck in slow work must not hold the receiver lock, or no other
/// event could be delivered.
#[test]
fn slow_callback_does_not_delay_another_event() {
    let mut fixture = slow_and_fast(Duration::from_millis(20));
    fixture.coordinator.ingest(event_for("/slow/0"));
    fixture
        .started
        .recv_timeout(Duration::from_secs(5))
        .expect("slow callback started");

    fixture.coordinator.ingest(event_for("/fast"));
    let delivered = fixture.delivered.recv_timeout(Duration::from_secs(5));

    let _ = fixture.release.send(());
    fixture.coordinator.stop();
    assert_eq!(delivered.ok(), Some(PathBuf::from("/fast")));
}

/// An event still queued behind busy workers when `stop` returns is dropped.
#[test]
fn events_drained_after_stop_are_not_delivered() {
    let mut fixture = slow_and_fast(Duration::from_millis(20));
    for index in 0_i32..4_i32 {
        fixture
            .coordinator
            .ingest(event_for(&format!("/slow/{index}")));
    }
    for _ in 0_i32..4_i32 {
        fixture
            .started
            .recv_timeout(Duration::from_secs(5))
            .expect("every worker is busy");
    }

    // Drained by the flush thread, but no worker is free to take it.
    fixture.coordinator.ingest(event_for("/queued"));
    std::thread::sleep(Duration::from_millis(300));
    fixture.coordinator.stop();
    for _ in 0_i32..4_i32 {
        let _ = fixture.release.send(());
    }

    assert!(
        fixture
            .delivered
            .recv_timeout(Duration::from_millis(500))
            .is_err(),
        "an event drained before stop must not be delivered after it"
    );
}

/// Panicking callbacks must not shrink the pool.
#[test]
fn panicking_callback_does_not_stop_dispatch() {
    let (event_tx, rx) = mpsc::channel();
    let tx = Mutex::new(event_tx);
    let mut coordinator = WatchCoordinator::new(
        Duration::from_millis(20),
        Box::new(move |event| {
            assert!(
                !event.repo.starts_with("/panic"),
                "callback panics on purpose"
            );
            let _ = tx.lock().expect("sender lock").send(event.repo);
        }),
        None,
    )
    .expect("coordinator");

    // One panic per worker: a pool that loses a thread per panic would be empty.
    for index in 0_i32..4_i32 {
        coordinator.ingest(event_for(&format!("/panic/{index}")));
    }
    std::thread::sleep(Duration::from_millis(300));
    coordinator.ingest(event_for("/after"));
    let delivered = rx.recv_timeout(Duration::from_secs(5));
    coordinator.stop();

    assert_eq!(delivered.ok(), Some(PathBuf::from("/after")));
}
