//! Opt-in operation timing, enabled by setting `GPY_PROFILE`.

use crate::warn_log;
use std::time::Instant;

/// Logs the elapsed time between construction and drop when `GPY_PROFILE` is set.
pub struct Timer {
    name: &'static str,
    start: Instant,
}

impl Timer {
    /// Start timing an operation.
    #[must_use]
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            start: Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        if std::env::var("GPY_PROFILE").is_ok() {
            warn_log!(
                "profiling",
                "[PROFILE] {}: {:.2}ms",
                self.name,
                elapsed.as_secs_f64() * 1_000.0_f64
            );
        }
    }
}
