//! Test harness utilities for deterministic testing
//!
//! Provides synchronization primitives to replace brittle timing-based waits
//! with deterministic signaling. Tests using this harness work reliably on
//! both macOS and Linux without race conditions.

#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

/// A signal counter with deterministic wait support
///
/// Instead of using `sleep()` and hoping the signal arrives in time,
/// use `wait_for_count()` to block until the expected number of signals
/// have been received.
///
/// # Example
///
/// ```no_run
/// use test_harness::SignalCounter;
/// use std::time::Duration;
///
/// let counter = SignalCounter::new();
///
/// // Install signal handler that calls counter.increment()
/// // ... trigger some events ...
///
/// // Wait up to 1 second for 2 signals
/// assert!(counter.wait_for_count(2, Duration::from_secs(1)).await);
/// ```
#[derive(Clone)]
pub struct SignalCounter {
    count: Arc<AtomicU32>,
    notifier: Arc<Notify>,
}

impl SignalCounter {
    /// Create a new signal counter
    #[must_use]
    pub fn new() -> Self {
        Self {
            count: Arc::new(AtomicU32::new(0)),
            notifier: Arc::new(Notify::new()),
        }
    }

    /// Increment the counter (call from signal handler)
    pub fn increment(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.notifier.notify_waiters();
    }

    /// Get the current count
    #[must_use]
    pub fn get(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }

    /// Reset the counter to zero
    pub fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
    }

    /// Wait for the counter to reach the expected count
    ///
    /// Returns `true` if the expected count was reached within the timeout,
    /// `false` if the timeout expired.
    ///
    /// # Arguments
    ///
    /// * `expected` - The count to wait for
    /// * `timeout` - Maximum time to wait
    pub async fn wait_for_count(&self, expected: u32, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .expect("timeout too large");

        loop {
            let current = self.get();
            if current >= expected {
                return true;
            }

            // Wait for notification or timeout
            if tokio::time::timeout_at(deadline, self.notifier.notified())
                .await
                .is_err()
            {
                return false; // Timeout
            }
            // Got notification, loop to check count again
        }
    }

    /// Wait for the counter to increment by at least N
    ///
    /// Similar to `wait_for_count` but waits for a delta from the current value.
    #[allow(dead_code)]
    pub async fn wait_for_increment(&self, delta: u32, timeout: Duration) -> bool {
        let initial = self.get();
        self.wait_for_count(initial.saturating_add(delta), timeout)
            .await
    }
}

impl Default for SignalCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// A boolean flag with wait support
///
/// Useful for signaling when a specific event has occurred.
///
/// # Example
///
/// ```no_run
/// use test_harness::EventFlag;
/// use std::time::Duration;
///
/// let flag = EventFlag::new();
///
/// // In some task:
/// // ... do work ...
/// // flag.set();
///
/// // In test:
/// assert!(flag.wait_until_set(Duration::from_secs(1)).await);
/// ```
#[derive(Clone)]
pub struct EventFlag {
    flag: Arc<AtomicBool>,
    notifier: Arc<Notify>,
}

impl EventFlag {
    /// Create a new event flag (initially false)
    #[must_use]
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notifier: Arc::new(Notify::new()),
        }
    }

    /// Set the flag to true
    pub fn set(&self) {
        self.flag.store(true, Ordering::Relaxed);
        self.notifier.notify_waiters();
    }

    /// Clear the flag (set to false)
    pub fn clear(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    /// Check if the flag is set
    #[must_use]
    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Wait for the flag to be set
    ///
    /// Returns `true` if the flag was set within the timeout,
    /// `false` if the timeout expired.
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
            // Got notification, loop to check is_set again
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

        // Should return immediately since count is already 2
        let result = counter.wait_for_count(2, Duration::from_millis(100)).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_signal_counter_wait_delayed() {
        let counter = SignalCounter::new();
        let counter_clone = counter.clone();

        // Spawn a task that increments after 50ms
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            counter_clone.increment();
            sleep(Duration::from_millis(50)).await;
            counter_clone.increment();
        });

        // Wait for 2 increments (should succeed within 200ms)
        let result = counter.wait_for_count(2, Duration::from_millis(200)).await;
        assert!(result);
        assert_eq!(counter.get(), 2);
    }

    #[tokio::test]
    async fn test_signal_counter_wait_timeout() {
        let counter = SignalCounter::new();

        // Wait for a count that never arrives
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

        // Should return immediately
        let result = flag.wait_until_set(Duration::from_millis(100)).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_event_flag_wait_delayed() {
        let flag = EventFlag::new();
        let flag_clone = flag.clone();

        // Spawn a task that sets the flag after 50ms
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            flag_clone.set();
        });

        // Wait for the flag (should succeed within 200ms)
        let result = flag.wait_until_set(Duration::from_millis(200)).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_event_flag_wait_timeout() {
        let flag = EventFlag::new();

        // Wait for a flag that's never set
        let result = flag.wait_until_set(Duration::from_millis(100)).await;
        assert!(!result);
    }
}
