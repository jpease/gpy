//! Cache timing policy (TTL and cooldown) for the git status cache.
//!
//! Three timing mechanisms cooperate to keep prompts fast and fresh:
//!
//! ```text
//! Debounce (100ms)    Cooldown (150ms)      TTL (30s)
//! ─────────────────   ─────────────────     ─────────────────
//! Event batching      Anti-thrashing        Staleness guard
//! ↓                   ↓                     ↓
//! Watcher → Agent     Agent → Data Source   Cache → Expiration
//! ```
//!
//! - **Debounce** (file watcher): batches a burst of filesystem events into one
//!   invalidation, so saving five files triggers one git status call.
//! - **Cooldown** (`is_fresh()`): after the cache is updated, prompts rendered
//!   within the window use the cached data instead of re-querying git, even if
//!   a watcher event arrives mid-render.
//! - **TTL**: entries older than this are misses, catching changes the watcher
//!   cannot see (a `git fetch` from elsewhere, edits outside the watched tree,
//!   watcher failures).
//!
//! ```rust
//! use gpy_agent::cache::CachePolicy;
//!
//! let git_policy = CachePolicy::git_status();
//! assert_eq!(git_policy.ttl().as_secs(), 30);
//! assert_eq!(git_policy.cooldown().as_millis(), 150);
//! ```

use std::time::Duration;

/// Cache timing policy that defines TTL and cooldown parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePolicy {
    /// Time-to-live: Maximum age before cache entry is considered stale
    pub(crate) ttl: Duration,
    /// Cooldown window: Minimum time before re-querying data source after cache update
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

    /// Policy for git status caching: 30s TTL, 150ms cooldown.
    ///
    /// The short TTL catches changes that bypass the file watcher; the
    /// cooldown matches the watcher throttle so a watcher-triggered update is
    /// not immediately re-queried.
    #[must_use]
    pub const fn git_status() -> Self {
        Self {
            ttl: Duration::from_secs(30),
            cooldown: Duration::from_millis(150),
        }
    }
}

/// Trait for caches that follow a policy
pub trait PolicyDriven {
    /// Get the cache policy
    fn policy(&self) -> CachePolicy;
}
