//! Bounded queue between the notify backends and the event thread.
//!
//! Producers are notify callback threads and the poll scanners; blocking them
//! on a full queue could deadlock against the backend mutex, so a full queue
//! drops the event and raises an overflow flag instead. The event thread turns
//! the flag into a rescan of every watched root, so no refresh is lost.

use notify::Event;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};

/// Events queued before an overflow: far above any steady state, small enough
/// that a storm (a checkout, `npm install`) cannot grow memory without bound.
pub(super) const EVENT_QUEUE_CAPACITY: usize = 4096;

/// Producer side of the event queue; never blocks.
#[derive(Clone)]
pub(super) struct EventSender {
    tx: SyncSender<Event>,
    overflowed: Arc<AtomicBool>,
}

impl EventSender {
    /// Queue `event`; a full queue drops it and flags an overflow.
    pub(super) fn send(&self, event: Event) {
        let Err(TrySendError::Full(dropped)) = self.tx.try_send(event) else {
            return;
        };
        self.overflowed.store(true, Ordering::SeqCst);
        // Retry once: should the event thread have drained the queue between
        // the failed send and the flag store, this wakes it to see the flag.
        let _ = self.tx.try_send(dropped);
    }
}

/// Consumer side of the event queue, owned by the event thread.
pub(super) struct EventReceiver {
    rx: Receiver<Event>,
    overflowed: Arc<AtomicBool>,
}

impl EventReceiver {
    /// The next event; `None` once every sender is gone.
    pub(super) fn recv(&self) -> Option<Event> {
        self.rx.recv().ok()
    }

    /// The next event within `timeout`.
    #[cfg(test)]
    pub(super) fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Event, mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    /// Whether an event was dropped since the last call; clears the flag.
    pub(super) fn take_overflow(&self) -> bool {
        self.overflowed.swap(false, Ordering::SeqCst)
    }
}

/// A queue holding at most `capacity` events.
pub(super) fn event_queue(capacity: usize) -> (EventSender, EventReceiver) {
    let (tx, rx) = mpsc::sync_channel(capacity);
    let overflowed = Arc::new(AtomicBool::new(false));
    (
        EventSender {
            tx,
            overflowed: Arc::clone(&overflowed),
        },
        EventReceiver { rx, overflowed },
    )
}
