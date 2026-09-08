//! Version detection caching with 24-hour TTL
//!
//! Caches language version detection results to avoid expensive
//! subprocess calls on every prompt render.

/// Cache lookup result
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheLookup {
    /// Found in cache (value may be Some or None)
    Hit(Option<String>),
    /// Not found in cache or expired
    Miss,
}
