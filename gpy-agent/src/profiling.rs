//! Opt-in operation timing, enabled by setting `GPY_PROFILE` to a true flag
//! value (`1`, `true`, `yes`, `on`).

use crate::env_config::{self, PROFILE};
use crate::warn_log;
use std::sync::OnceLock;
use std::time::Instant;

static PROFILE_ENABLED: OnceLock<bool> = OnceLock::new();

/// Whether profiling is on (`GPY_PROFILE` set to a true flag value); read once and cached for the process.
#[must_use]
pub fn is_profile_enabled() -> bool {
    *PROFILE_ENABLED.get_or_init(|| {
        if PROFILE.flag() {
            return true;
        }
        // Any other value used to enable profiling; say so once.
        if let Some(raw) = PROFILE
            .raw()
            .map(|value| value.to_string_lossy().into_owned())
            && matches!(
                env_config::parse_flag(Some(&raw)),
                env_config::Reading::Value(false) | env_config::Reading::Unset
            )
        {
            env_config::warn_once(
                "GPY_PROFILE",
                &format!("GPY_PROFILE={raw} no longer enables profiling; use 1 to enable"),
                true,
            );
        }
        false
    })
}

/// Scope guard: on drop, logs `name` and the elapsed milliseconds when profiling is enabled.
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
}
