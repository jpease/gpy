//! IPC request latency tracking.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// Ring buffer of recent request latencies with min/max/average.
pub struct LatencyTracker {
    /// Samples in milliseconds.
    samples: Mutex<LatencyBuffer>,
}

struct LatencyBuffer {
    buffer: Vec<u64>,
    capacity: usize,
    next_index: usize,
    /// Total recorded, may exceed capacity.
    total_count: u64,
}

impl LatencyTracker {
    /// Create a tracker retaining at most `capacity` samples.
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

    /// Record a request latency.
    pub fn record(&self, duration: Duration) {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);

        let mut buffer = self.samples.lock().unwrap_or_else(PoisonError::into_inner);

        if buffer.buffer.len() < buffer.capacity {
            buffer.buffer.push(millis);
            buffer.next_index = buffer.buffer.len() % buffer.capacity;
        } else {
            let idx = buffer.next_index;

            if let Some(slot) = buffer.buffer.get_mut(idx) {
                *slot = millis;
            }

            buffer.next_index = (buffer.next_index.wrapping_add(1_usize)) % buffer.capacity;
        }

        buffer.total_count = buffer.total_count.wrapping_add(1_u64);
    }

    /// Get current latency statistics
    ///
    /// Returns `(min, max, avg, sample_count)` in milliseconds.
    #[must_use]
    pub fn stats(&self) -> (u64, u64, u64, usize) {
        let buffer = self.samples.lock().unwrap_or_else(PoisonError::into_inner);

        if buffer.buffer.is_empty() {
            return (0_u64, 0_u64, 0_u64, 0_usize);
        }

        let min = *buffer.buffer.iter().min().unwrap_or(&0_u64);
        let max = *buffer.buffer.iter().max().unwrap_or(&0_u64);
        let sum: u64 = buffer.buffer.iter().sum();

        let avg = sum / u64::try_from(buffer.buffer.len()).unwrap_or(1_u64);

        let count = buffer.buffer.len();
        drop(buffer);

        (min, max, avg, count)
    }

    /// Reset all tracking data
    pub fn reset(&self) {
        let mut buffer = self.samples.lock().unwrap_or_else(PoisonError::into_inner);

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
        assert_eq!(avg, 20_u64);
        assert_eq!(count, 3_usize);
    }

    #[test]
    fn test_latency_tracker_circular_buffer() {
        let tracker = LatencyTracker::new(3_usize);

        tracker.record(Duration::from_millis(10_u64));
        tracker.record(Duration::from_millis(20_u64));
        tracker.record(Duration::from_millis(30_u64));

        let (_min, _max, avg1, count1) = tracker.stats();
        assert_eq!(count1, 3_usize);
        assert_eq!(avg1, 20_u64);

        // Wraps, replacing the oldest (10).
        tracker.record(Duration::from_millis(40_u64));

        let (min, max, avg2, count2) = tracker.stats();
        assert_eq!(count2, 3_usize);
        assert_eq!(min, 20_u64);
        assert_eq!(max, 40_u64);
        assert_eq!(avg2, 30_u64);
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
