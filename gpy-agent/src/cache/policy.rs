//! Cache Policy Abstraction
//!
//! Provides a unified policy system for managing cache timing parameters across
//! different cache implementations (git status, language versions, themes).
//!
//! ## Three-Way Timing Relationship
//!
//! GPY's caching system uses three coordinated timing mechanisms that work together
//! to optimize performance while maintaining data freshness:
//!
//! ### 1. Debounce (File Watcher → Agent)
//!
//! **What it does**: Batches rapid file system events into a single notification
//!
//! **Default**: 100ms
//!
//! **Example**: When you save 5 files in quick succession:
//! - Without debounce: 5 separate cache invalidations → 5 git status calls
//! - With 100ms debounce: 1 cache invalidation → 1 git status call
//!
//! **Location**: File watcher (notify crate's debouncer)
//!
//! ### 2. Cooldown (Cache → Data Source)
//!
//! **What it does**: Prevents re-querying the data source when cache was recently updated
//!
//! **Default**: 150ms
//!
//! **Example**: After file watcher updates the cache:
//! - T+0ms: File change detected, cache updated with fresh git status
//! - T+50ms: User prompt renders, sees fresh cached data (no git call)
//! - T+100ms: User prompt renders again, still using cached data (no git call)
//! - T+200ms: Cooldown expired, next prompt can query git if needed
//!
//! **Location**: Agent's cache lookup logic (checks `is_fresh()`)
//!
//! **Why necessary**: File watcher events can arrive while a prompt is rendering.
//! Without cooldown, the prompt would immediately re-query git even though the
//! cache was just updated with fresh data.
//!
//! ### 3. TTL (Time-To-Live) (Cache Entry → Expiration)
//!
//! **What it does**: Automatically expires stale cache entries after a period of inactivity
//!
//! **Defaults**:
//! - Git status: 30 seconds
//! - Language versions: 24 hours
//! - Theme data: (loaded from disk on demand, not cached in memory)
//!
//! **Example**: You're in a git repo, then leave for lunch:
//! - T+0s: Git status cached
//! - T+15s: Still fresh, cache hit
//! - T+29s: Still fresh, cache hit
//! - T+31s: TTL expired, cache miss → fresh git query
//!
//! **Location**: Cache entry timestamp checking
//!
//! **Why necessary**: Catches changes that bypass the file watcher:
//! - `git fetch` on another machine
//! - Manual edits outside the watched directory
//! - File watcher failures or delays
//!
//! ## Timing Relationship
//!
//! ```text
//! Debounce (100ms)    Cooldown (150ms)      TTL (30s / 24h)
//! ─────────────────   ─────────────────     ─────────────────
//! Event batching      Anti-thrashing        Staleness guard
//! ↓                   ↓                     ↓
//! Watcher → Agent     Agent → Data Source   Cache → Expiration
//! ```
//!
//! **Key insight**: Debounce ≤ Cooldown is optimal
//! - If debounce (100ms) > cooldown (150ms), the cooldown becomes ineffective
//! - Current settings: 100ms debounce, 150ms cooldown → 50ms safety margin
//!
//! ## Usage Example
//!
//! ```rust
//! use gpy_agent::cache::CachePolicy;
//! use std::time::Duration;
//!
//! // Use predefined policy
//! let git_policy = CachePolicy::git_status();
//! assert_eq!(git_policy.ttl().as_secs(), 30);
//! assert_eq!(git_policy.cooldown().as_millis(), 150);
//!
//! // Create custom policy with builder
//! let custom = CachePolicy::builder()
//!     .ttl(Duration::from_secs(60))
//!     .cooldown(Duration::from_millis(200))
//!     .build();
//!
//! // Or use the constructor directly (discouraged — prefer the builder above)
//! let custom = CachePolicy::new(
//!     Duration::from_secs(60),    // 60s TTL
//!     Duration::from_millis(200), // 200ms cooldown
//! );
//! ```

use std::time::Duration;

use crate::{Error, Result};

/// Cache timing policy that defines TTL and cooldown parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePolicy {
    /// Time-to-live: Maximum age before cache entry is considered stale
    ttl: Duration,
    /// Cooldown window: Minimum time before re-querying data source after cache update
    cooldown: Duration,
}

impl CachePolicy {
    /// Create a new cache policy with explicit timing parameters
    ///
    /// # Arguments
    ///
    /// * `ttl` - Maximum age before cache entry expires
    /// * `cooldown` - Minimum time before allowing re-query after cache update
    ///
    /// # Deprecated (Soft)
    ///
    /// **Prefer using predefined factories** ([`CachePolicy::git_status()`], [`CachePolicy::language_version()`])
    /// **or the builder** ([`CachePolicy::builder()`]) for improved readability and maintainability.
    ///
    /// This constructor remains available for cases where neither pattern applies, but most
    /// use cases should use the factory methods or builder.
    #[must_use]
    pub const fn new(ttl: Duration, cooldown: Duration) -> Self {
        Self { ttl, cooldown }
    }

    /// Create a builder for constructing a cache policy
    ///
    /// # Example
    ///
    /// ```rust
    /// use gpy_agent::cache::CachePolicy;
    /// use std::time::Duration;
    ///
    /// let policy = CachePolicy::builder()
    ///     .ttl(Duration::from_secs(60))
    ///     .cooldown(Duration::from_millis(200))
    ///     .build();
    /// ```
    #[must_use]
    pub const fn builder() -> CachePolicyBuilder {
        CachePolicyBuilder::new()
    }

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

    /// Policy for git status caching
    ///
    /// **TTL**: 30 seconds - Git repos change frequently during development
    /// **Cooldown**: 150ms - Matches watcher throttle to prevent redundant queries
    ///
    /// **Rationale**:
    /// - Short TTL catches changes that bypass file watcher (remote fetches, etc.)
    /// - Cooldown prevents thrashing when file watcher triggers cache updates
    #[must_use]
    pub const fn git_status() -> Self {
        Self {
            ttl: Duration::from_secs(30),
            cooldown: Duration::from_millis(150),
        }
    }

    /// Policy for language version caching
    ///
    /// **TTL**: 24 hours - Language versions change infrequently
    /// **Cooldown**: 0ms - No file watcher, no thrashing risk
    ///
    /// **Rationale**:
    /// - Long TTL because version checks are expensive (subprocess calls)
    /// - No cooldown needed since there's no file watcher triggering updates
    /// - Persistent disk cache survives agent restarts
    #[must_use]
    pub const fn language_version() -> Self {
        Self {
            ttl: Duration::from_hours(24),
            cooldown: Duration::from_millis(0),
        }
    }

    /// Policy for theme caching (future use)
    ///
    /// **TTL**: 5 seconds - Allow hot-reload during theme development
    /// **Cooldown**: 100ms - Prevent thrashing during rapid theme edits
    ///
    /// **Rationale**:
    /// - Short TTL enables live theme development workflow
    /// - Cooldown prevents multiple reloads during multi-file theme edits
    /// - Not currently used (theme loaded on demand), reserved for future hot-reload
    #[must_use]
    pub const fn theme() -> Self {
        Self {
            ttl: Duration::from_secs(5),
            cooldown: Duration::from_millis(100),
        }
    }
}

/// Builder for constructing cache policies with named parameters
///
/// Provides a fluent API for creating cache policies, making it clearer what each
/// duration parameter represents.
///
/// # Example
///
/// ```rust
/// use gpy_agent::cache::CachePolicy;
/// use std::time::Duration;
///
/// // Build a custom policy
/// let policy = CachePolicy::builder()
///     .ttl(Duration::from_secs(300))
///     .cooldown(Duration::from_millis(150))
///     .build();
/// ```
#[derive(Debug, Clone, Copy)]
pub struct CachePolicyBuilder {
    ttl: Option<Duration>,
    cooldown: Option<Duration>,
}

impl CachePolicyBuilder {
    /// Create a new builder with no fields set
    #[must_use]
    const fn new() -> Self {
        Self {
            ttl: None,
            cooldown: None,
        }
    }

    /// Set the time-to-live duration
    ///
    /// # Arguments
    ///
    /// * `ttl` - Maximum age before cache entry expires
    #[must_use]
    pub const fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Set the cooldown duration
    ///
    /// # Arguments
    ///
    /// * `cooldown` - Minimum time before re-querying data source after cache update
    #[must_use]
    pub const fn cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown = Some(cooldown);
        self
    }

    /// Build the cache policy
    ///
    /// # Errors
    ///
    /// Returns an error if required fields are missing:
    /// - `ttl` must be set
    /// - `cooldown` must be set
    ///
    /// # Example
    ///
    /// ```rust
    /// use gpy_agent::cache::CachePolicy;
    /// use std::time::Duration;
    ///
    /// // This succeeds - all required fields set
    /// let policy = CachePolicy::builder()
    ///     .ttl(Duration::from_secs(60))
    ///     .cooldown(Duration::from_millis(100))
    ///     .build();
    /// assert!(policy.is_ok());
    ///
    /// // This fails - missing ttl
    /// let policy = CachePolicy::builder()
    ///     .cooldown(Duration::from_millis(100))
    ///     .build();
    /// assert!(policy.is_err());
    /// ```
    pub fn build(self) -> Result<CachePolicy> {
        let ttl = self
            .ttl
            .ok_or_else(|| Error::config("CachePolicy builder: ttl is required"))?;
        let cooldown = self
            .cooldown
            .ok_or_else(|| Error::config("CachePolicy builder: cooldown is required"))?;

        Ok(CachePolicy { ttl, cooldown })
    }
}

/// Trait for caches that follow a policy
pub trait PolicyDriven {
    /// Get the cache policy
    fn policy(&self) -> CachePolicy;
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn git_status_policy_has_correct_values() {
        let policy = CachePolicy::git_status();
        assert_eq!(policy.ttl(), Duration::from_secs(30));
        assert_eq!(policy.cooldown(), Duration::from_millis(150));
    }

    #[test]
    fn language_version_policy_has_long_ttl() {
        let policy = CachePolicy::language_version();
        assert_eq!(policy.ttl(), Duration::from_hours(24));
        assert_eq!(policy.cooldown(), Duration::from_millis(0));
    }

    #[test]
    fn theme_policy_enables_hot_reload() {
        let policy = CachePolicy::theme();
        assert_eq!(policy.ttl(), Duration::from_secs(5));
        assert_eq!(policy.cooldown(), Duration::from_millis(100));
    }

    #[test]
    fn custom_policy_construction() {
        let custom = CachePolicy::new(Duration::from_secs(60), Duration::from_millis(200));
        assert_eq!(custom.ttl(), Duration::from_secs(60));
        assert_eq!(custom.cooldown(), Duration::from_millis(200));
    }

    #[test]
    fn builder_with_all_fields() {
        let policy = CachePolicy::builder()
            .ttl(Duration::from_secs(120))
            .cooldown(Duration::from_millis(300))
            .build()
            .expect("builder should succeed with all fields set");

        assert_eq!(policy.ttl(), Duration::from_secs(120));
        assert_eq!(policy.cooldown(), Duration::from_millis(300));
    }

    #[test]
    fn builder_missing_ttl() {
        let result = CachePolicy::builder()
            .cooldown(Duration::from_millis(100))
            .build();

        assert!(result.is_err(), "builder should fail when ttl is not set");
    }

    #[test]
    fn builder_missing_cooldown() {
        let result = CachePolicy::builder().ttl(Duration::from_secs(60)).build();

        assert!(
            result.is_err(),
            "builder should fail when cooldown is not set"
        );
    }

    #[test]
    fn builder_missing_all_fields() {
        let result = CachePolicy::builder().build();

        assert!(
            result.is_err(),
            "builder should fail when no fields are set"
        );
    }

    #[test]
    fn builder_equals_constructor() {
        let from_builder = CachePolicy::builder()
            .ttl(Duration::from_secs(60))
            .cooldown(Duration::from_millis(200))
            .build()
            .expect("builder should succeed");

        let from_constructor =
            CachePolicy::new(Duration::from_secs(60), Duration::from_millis(200));

        assert_eq!(from_builder, from_constructor);
    }
}
