//! Tests for the `CacheLookup` result type used by language version caching
//!
//! `VersionStore` (the TTL-backed cache these tests previously exercised) was
//! removed as dead code (#581): production code caches versions via the
//! module-level `VERSION_CACHE` static in `language::version`, which bypassed
//! `VersionStore` entirely. `CacheLookup` itself is still live (used by
//! `language::version`), so its behavior is still tested here.

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::string_add)]
#![allow(clippy::default_numeric_fallback)]

use gpy_agent::language::cache::CacheLookup;

// ===== CacheLookup Equality Tests =====

#[test]
fn test_cache_lookup_equality() {
    let hit1 = CacheLookup::Hit(Some("1.0".to_owned()));
    let hit2 = CacheLookup::Hit(Some("1.0".to_owned()));
    let hit3 = CacheLookup::Hit(Some("2.0".to_owned()));
    let hit_none = CacheLookup::Hit(None);
    let miss = CacheLookup::Miss;

    assert_eq!(hit1, hit2);
    assert_ne!(hit1, hit3);
    assert_ne!(hit1, hit_none);
    assert_ne!(hit1, miss);
}

// ===== Clone Tests =====

#[test]
fn test_cache_lookup_clone() {
    let original = CacheLookup::Hit(Some("1.0".to_owned()));
    let cloned = original.clone();

    assert_eq!(original, cloned);
}

// ===== Debug Formatting =====

#[test]
fn test_cache_lookup_debug() {
    let hit = CacheLookup::Hit(Some("1.0".to_owned()));
    let debug_str = format!("{hit:?}");

    assert!(debug_str.contains("Hit"));
    assert!(debug_str.contains("1.0"));
}
