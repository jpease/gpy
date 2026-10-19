//! Git status cache timing: cooldown avoids re-querying right after an
//! update; TTL catches changes the watcher cannot see.

use std::time::Duration;

/// Cache timing policy that defines TTL and cooldown parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePolicy {
    /// Maximum age before an entry is stale.
    pub(crate) ttl: Duration,
    /// Minimum time before re-querying after a cache update.
    pub(crate) cooldown: Duration,
}

impl CachePolicy {
    /// Get the time-to-live duration
    #[must_use]
    pub const fn ttl(self) -> Duration {
        self.ttl
    }

    /// Get the cooldown duration
    #[must_use]
    pub const fn cooldown(self) -> Duration {
        self.cooldown
    }

    /// Git status policy: 30s TTL; 150ms cooldown matches the watcher throttle.
    #[must_use]
    pub const fn git_status() -> Self {
        Self {
            ttl: Duration::from_secs(30),
            cooldown: Duration::from_millis(150),
        }
    }
}
