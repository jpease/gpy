//! IPC request latency tracking
//!
//! Tracks response time statistics for IPC operations to help monitor agent performance.

use std::sync::Mutex;
use std::time::Duration;

/// Latency tracker for IPC operations
///
/// Maintains a circular buffer of the last N request latencies and provides
/// statistical aggregations (min, max, average).
pub struct LatencyTracker {
    /// Circular buffer of latency samples (in milliseconds)
    samples: Mutex<LatencyBuffer>,
}

struct LatencyBuffer {
    /// Fixed-size buffer of latency samples
    buffer: Vec<u64>,
    /// Maximum number of samples to retain
    capacity: usize,
    /// Write position in circular buffer
    next_index: usize,
    /// Total number of samples recorded (can exceed capacity)
    total_count: u64,
}

impl LatencyTracker {
    /// Create a new latency tracker with specified capacity
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of samples to retain (default: 100)
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Mutex::new(LatencyBuffer {
                buffer: Vec::with_capacity(capacity),
                capacity,
                next_index: 0_usize,
                total_count: 0_u64,
            }),
        }
    }

    /// Record a request latency
    ///
    /// If the lock is poisoned (previous thread panicked while holding the lock),
    /// this silently skips recording as latency tracking is non-critical.
    ///
    /// # Arguments
    ///
    /// * `duration` - Time taken to process the request
    pub fn record(&self, duration: Duration) {
        // Convert duration to milliseconds, clamping at u64::MAX
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);

        // Acquire lock - if poisoned, skip recording (non-critical operation)
        let Ok(mut buffer) = self.samples.lock() else {
            return;
        };

        // If buffer isn't full yet, just append
        if buffer.buffer.len() < buffer.capacity {
            buffer.buffer.push(millis);
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "buffer.len() < capacity, so this modulo is safe and intentional"
            )]
            {
                buffer.next_index = buffer.buffer.len() % buffer.capacity;
            }
        } else {
            // Overwrite oldest sample in circular fashion
            let idx = buffer.next_index;

            // Use get_mut instead of direct indexing for safety
            if let Some(slot) = buffer.buffer.get_mut(idx) {
                *slot = millis;
            }

            #[expect(
                clippy::arithmetic_side_effects,
                reason = "circular buffer arithmetic - next_index + 1 wraps to 0 at capacity; this is intentional modulo arithmetic for the circular buffer pattern"
            )]
            {
                buffer.next_index = (buffer.next_index.wrapping_add(1_usize)) % buffer.capacity;
            }
        }

        buffer.total_count = buffer.total_count.wrapping_add(1_u64);
    }

    /// Get current latency statistics
    ///
    /// Returns `(min, max, avg, sample_count)` in milliseconds.
    /// If the lock is poisoned, returns zeros.
    #[must_use]
    pub fn stats(&self) -> (u64, u64, u64, usize) {
        // Acquire lock - if poisoned, return zeros
        let Ok(buffer) = self.samples.lock() else {
            return (0_u64, 0_u64, 0_u64, 0_usize);
        };

        if buffer.buffer.is_empty() {
            return (0_u64, 0_u64, 0_u64, 0_usize);
        }

        let min = *buffer.buffer.iter().min().unwrap_or(&0_u64);
        let max = *buffer.buffer.iter().max().unwrap_or(&0_u64);
        let sum: u64 = buffer.buffer.iter().sum();

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "division by buffer.len() which is guaranteed non-zero (checked above)"
        )]
        let avg = sum / u64::try_from(buffer.buffer.len()).unwrap_or(1_u64);

        let count = buffer.buffer.len();

        (min, max, avg, count)
    }

    /// Reset all tracking data
    ///
    /// If the lock is poisoned, this is a no-op as latency tracking is non-critical.
    pub fn reset(&self) {
        // Acquire lock - if poisoned, skip reset
        let Ok(mut buffer) = self.samples.lock() else {
            return;
        };

        buffer.buffer.clear();
        buffer.next_index = 0_usize;
        buffer.total_count = 0_u64;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;

    #[test]
    fn test_latency_tracker_empty() {
        let tracker = LatencyTracker::new(100_usize);
        let (min, max, avg, count) = tracker.stats();
        assert_eq!(min, 0_u64);
        assert_eq!(max, 0_u64);
        assert_eq!(avg, 0_u64);
        assert_eq!(count, 0_usize);
    }

    #[test]
    fn test_latency_tracker_single_sample() {
        let tracker = LatencyTracker::new(100_usize);
        tracker.record(Duration::from_millis(42_u64));

        let (min, max, avg, count) = tracker.stats();
        assert_eq!(min, 42_u64);
        assert_eq!(max, 42_u64);
        assert_eq!(avg, 42_u64);
        assert_eq!(count, 1_usize);
    }

    #[test]
    fn test_latency_tracker_multiple_samples() {
        let tracker = LatencyTracker::new(100_usize);
        tracker.record(Duration::from_millis(10_u64));
        tracker.record(Duration::from_millis(20_u64));
        tracker.record(Duration::from_millis(30_u64));

        let (min, max, avg, count) = tracker.stats();
        assert_eq!(min, 10_u64);
        assert_eq!(max, 30_u64);
        assert_eq!(avg, 20_u64); // (10 + 20 + 30) / 3 = 20
        assert_eq!(count, 3_usize);
    }

    #[test]
    fn test_latency_tracker_circular_buffer() {
        let tracker = LatencyTracker::new(3_usize);

        // Fill buffer
        tracker.record(Duration::from_millis(10_u64));
        tracker.record(Duration::from_millis(20_u64));
        tracker.record(Duration::from_millis(30_u64));

        let (_min, _max, avg1, count1) = tracker.stats();
        assert_eq!(count1, 3_usize);
        assert_eq!(avg1, 20_u64);

        // Add one more - should wrap and replace oldest (10)
        tracker.record(Duration::from_millis(40_u64));

        let (min, max, avg2, count2) = tracker.stats();
        assert_eq!(count2, 3_usize);
        assert_eq!(min, 20_u64); // 10 was replaced
        assert_eq!(max, 40_u64);
        assert_eq!(avg2, 30_u64); // (20 + 30 + 40) / 3 = 30
    }

    #[test]
    fn test_latency_tracker_reset() {
        let tracker = LatencyTracker::new(100_usize);
        tracker.record(Duration::from_millis(42_u64));
        tracker.reset();

        let (_min, _max, _avg, count) = tracker.stats();
        assert_eq!(count, 0_usize);
    }
}
