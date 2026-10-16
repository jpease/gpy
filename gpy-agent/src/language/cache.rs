//! Version-lookup cache result type.

/// Version cache lookup result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheLookup {
    /// Cached value (may be a cached absence).
    Hit(Option<String>),
    /// Absent or expired.
    Miss,
}
