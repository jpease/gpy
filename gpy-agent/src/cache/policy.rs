//! Git status cache timing: the cooldown (counted from the last update of any
//! kind) avoids re-querying right after an update; the TTL (counted from the
//! last full scan) catches changes the watcher cannot see.

use std::time::Duration;

/// How long after a cache update of any kind a hit skips background
/// revalidation. Independent of `GPY_SIGURG_THROTTLE_MS`, which tunes only
/// the prompt repaint throttle.
pub const GIT_STATUS_COOLDOWN: Duration = Duration::from_millis(150);

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

    /// Git status policy: 30s TTL from the last full scan, and
    /// [`GIT_STATUS_COOLDOWN`].
    #[must_use]
    pub const fn git_status() -> Self {
        Self {
            ttl: Duration::from_secs(30),
            cooldown: GIT_STATUS_COOLDOWN,
        }
    }
}
