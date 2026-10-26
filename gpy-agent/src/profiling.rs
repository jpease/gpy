//! Opt-in operation timing, enabled by setting `GPY_PROFILE`.

use crate::warn_log;
use std::sync::OnceLock;
use std::time::Instant;

static PROFILE_ENABLED: OnceLock<bool> = OnceLock::new();

/// Check if profiling is enabled via `GPY_PROFILE`.
#[must_use]
pub fn is_profile_enabled() -> bool {
    *PROFILE_ENABLED.get_or_init(|| std::env::var("GPY_PROFILE").is_ok())
}
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
        if is_profile_enabled() {
            let elapsed = self.start.elapsed();
            warn_log!(
                "profiling",
                "[PROFILE] {}: {:.2}ms",
                self.name,
                elapsed.as_secs_f64() * 1_000.0_f64
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_constructs_and_drops_without_panic() {
        let timer = Timer::new("test_operation");
        drop(timer);
    }

    #[test]
    fn profile_enabled_is_consistent() {
        let first = is_profile_enabled();
        let second = is_profile_enabled();
        assert_eq!(first, second);
    }
}
