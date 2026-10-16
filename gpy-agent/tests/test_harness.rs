//! Signal-based waits that replace timing-based sleeps in tests.

#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

/// Counter that tests can await instead of sleeping.
#[derive(Clone)]
pub struct SignalCounter {
    count: Arc<AtomicU32>,
    notifier: Arc<Notify>,
}

impl SignalCounter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            count: Arc::new(AtomicU32::new(0)),
            notifier: Arc::new(Notify::new()),
        }
    }

    pub fn increment(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.notifier.notify_waiters();
    }

    #[must_use]
    pub fn get(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }

    pub fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
    }

    /// Waits until the count reaches `expected`; `false` on timeout.
    pub async fn wait_for_count(&self, expected: u32, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .expect("timeout too large");

        loop {
            let current = self.get();
            if current >= expected {
                return true;
            }

            if tokio::time::timeout_at(deadline, self.notifier.notified())
                .await
                .is_err()
            {
                return false; // Timeout
            }
        }
    }
}

impl Default for SignalCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// Boolean flag that tests can await.
#[derive(Clone)]
pub struct EventFlag {
    flag: Arc<AtomicBool>,
    notifier: Arc<Notify>,
}

impl EventFlag {
    #[must_use]
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notifier: Arc::new(Notify::new()),
        }
    }

    pub fn set(&self) {
        self.flag.store(true, Ordering::Relaxed);
        self.notifier.notify_waiters();
    }

    pub fn clear(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Waits until the flag is set; `false` on timeout.
    pub async fn wait_until_set(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .expect("timeout too large");

        loop {
            if self.is_set() {
                return true;
            }

            if tokio::time::timeout_at(deadline, self.notifier.notified())
                .await
                .is_err()
            {
                return false; // Timeout
            }
        }
    }
}

impl Default for EventFlag {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_signal_counter_basic() {
        let counter = SignalCounter::new();
        assert_eq!(counter.get(), 0);

        counter.increment();
        assert_eq!(counter.get(), 1);

        counter.increment();
        assert_eq!(counter.get(), 2);

        counter.reset();
        assert_eq!(counter.get(), 0);
    }

    #[tokio::test]
    async fn test_signal_counter_wait_immediate() {
        let counter = SignalCounter::new();
        counter.increment();
        counter.increment();

        let result = counter.wait_for_count(2, Duration::from_millis(100)).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_signal_counter_wait_delayed() {
        let counter = SignalCounter::new();
        let counter_clone = counter.clone();

        // Delayed producer: the wait below is exercised against this delay.
        // delay is what the wait below is exercised against).
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            counter_clone.increment();
            sleep(Duration::from_millis(50)).await;
            counter_clone.increment();
        });

        let result = counter.wait_for_count(2, Duration::from_millis(200)).await;
        assert!(result);
        assert_eq!(counter.get(), 2);
    }

    #[tokio::test]
    async fn test_signal_counter_wait_timeout() {
        let counter = SignalCounter::new();

        let result = counter.wait_for_count(1, Duration::from_millis(100)).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_event_flag_basic() {
        let flag = EventFlag::new();
        assert!(!flag.is_set());

        flag.set();
        assert!(flag.is_set());

        flag.clear();
        assert!(!flag.is_set());
    }

    #[tokio::test]
    async fn test_event_flag_wait_immediate() {
        let flag = EventFlag::new();
        flag.set();

        let result = flag.wait_until_set(Duration::from_millis(100)).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_event_flag_wait_delayed() {
        let flag = EventFlag::new();
        let flag_clone = flag.clone();

        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            flag_clone.set();
        });

        let result = flag.wait_until_set(Duration::from_millis(200)).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_event_flag_wait_timeout() {
        let flag = EventFlag::new();

        let result = flag.wait_until_set(Duration::from_millis(100)).await;
        assert!(!result);
    }
}
