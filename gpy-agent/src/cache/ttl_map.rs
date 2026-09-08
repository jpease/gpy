//! A bounded, TTL-expiring cache primitive (#589).
//!
//! Four independent hand-rolled copies of this shape existed across
//! `language/version.rs` (`VERSION_CACHE`, `TOOL_CONFIG_CACHE`,
//! `MISE_NEGATIVE_CACHE`) and `language/venv.rs` (`VENV_STASH`) before this
//! consolidation — each with its own get/insert/evict/freshness logic, and
//! one (`venv.rs`'s stash) hardcoding its own TTL instead of sharing the one
//! knob (`config.language.cache_ttl_hours`) the other three already honored.
//!
//! [`TtlMap`] replaces all four: a bounded `OnceLock<Mutex<HashMap<K, (V,
//! SystemTime)>>>` that shares a single, runtime-adjustable TTL
//! ([`shared_cache_ttl_seconds`] / [`set_shared_cache_ttl`]) and delegates
//! capacity enforcement to [`crate::cache::bounded::evict_to_capacity`].
//! Freshness itself is the pure, clock-injectable [`is_fresh`].

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use super::bounded::evict_to_capacity;

/// Shared TTL (seconds) governing every [`TtlMap`] instance.
///
/// One knob for all four caches, per #589's acceptance criteria — deliberately
/// a single crate-wide atomic rather than a per-instance field, so "one TTL
/// knob" is literally true rather than four independently-set copies that
/// happen to start equal.
static SHARED_CACHE_TTL_SECONDS: AtomicU64 = AtomicU64::new(24 * 60 * 60);

/// Update the shared cache TTL (in hours), clamped to `[1, 720]`.
///
/// Governs every [`TtlMap`] instance in the process, including the venv
/// stash, which previously had its own hardcoded 24h TTL independent of this
/// knob (#589).
pub fn set_shared_cache_ttl(hours: u64) {
    let clamped = hours.clamp(1, 720);
    SHARED_CACHE_TTL_SECONDS.store(clamped.saturating_mul(3600), Ordering::SeqCst);
}

/// Current shared cache TTL, in seconds.
#[must_use]
pub fn shared_cache_ttl_seconds() -> u64 {
    SHARED_CACHE_TTL_SECONDS.load(Ordering::SeqCst)
}

/// Whether an entry inserted at `inserted_at` is still fresh at `now`, given `ttl`.
///
/// Pure and clock-injectable so freshness logic is unit-testable without real
/// sleeps (#589) — every production call site passes `SystemTime::now()` for
/// `now`; tests pass a fixed, controlled instant instead.
///
/// A backwards clock (`now` earlier than `inserted_at`) makes
/// `now.duration_since(inserted_at)` return `Err`; that reads as "not fresh",
/// matching the fail-safe direction of the hand-rolled callers this replaces
/// (each treated a clock error as "definitely stale, evict it" by inflating
/// the effective elapsed time past the TTL rather than reading it as zero
/// elapsed / definitely fresh).
fn is_fresh(inserted_at: SystemTime, now: SystemTime, ttl: Duration) -> bool {
    match now.duration_since(inserted_at) {
        Ok(elapsed) => elapsed < ttl,
        Err(_clock_went_backwards) => false,
    }
}

/// A cache entry's value paired with the [`SystemTime`] it was inserted at.
type Entry<V> = (V, SystemTime);

/// The lazily-initialized, mutex-guarded backing map for a [`TtlMap`].
type Entries<K, V> = OnceLock<Mutex<HashMap<K, Entry<V>>>>;

/// A bounded, TTL-expiring cache: `OnceLock<Mutex<HashMap<K, (V, SystemTime)>>>`
/// plus a capacity bound and the shared, runtime-adjustable TTL (#589).
///
/// `get` evicts a stale entry as part of the lookup ("evict on stale read",
/// matching all four caches this replaces). `insert` stamps the entry with
/// the current time and then bounds the map via
/// [`crate::cache::bounded::evict_to_capacity`]. `remove`/`retain` cover the
/// explicit invalidation each of the four caches needed; domain-specific,
/// key-pattern-aware invalidation (e.g. `version.rs`'s path-prefix matching
/// across multiple caches) stays in its own module rather than being pushed
/// down into this domain-agnostic type.
pub struct TtlMap<K, V> {
    entries: Entries<K, V>,
    capacity: usize,
}

impl<K, V> TtlMap<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create an empty cache bounded to `capacity` entries (`0` disables
    /// eviction, see [`evict_to_capacity`]).
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            entries: OnceLock::new(),
            capacity,
        }
    }

    fn map(&self) -> &Mutex<HashMap<K, Entry<V>>> {
        self.entries.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Look up `key`, returning a clone of the value if present and fresh
    /// under the shared TTL. A stale entry is removed as part of the lookup.
    #[must_use]
    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let ttl = Duration::from_secs(shared_cache_ttl_seconds());
        let now = SystemTime::now();
        let Ok(mut map) = self.map().lock() else {
            return None;
        };
        let entry = map.get(key)?;
        let (value, inserted_at) = (entry.0.clone(), entry.1);
        if is_fresh(inserted_at, now, ttl) {
            return Some(value);
        }
        map.remove(key);
        None
    }

    /// Insert `value` for `key`, stamped with the current time, then evict
    /// down to capacity (least-recently-updated first).
    pub fn insert(&self, key: K, value: V) {
        let Ok(mut map) = self.map().lock() else {
            return;
        };
        map.insert(key, (value, SystemTime::now()));
        evict_to_capacity(&mut map, self.capacity, |(_value, inserted_at)| {
            *inserted_at
        });
    }

    /// Remove `key` unconditionally, regardless of freshness.
    pub fn remove<Q>(&self, key: &Q)
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Ok(mut map) = self.map().lock() {
            map.remove(key);
        }
    }

    /// Retain only entries whose key satisfies `predicate`.
    ///
    /// Used for pattern-based invalidation (e.g. by path prefix); the
    /// pattern-matching logic itself stays with the caller.
    pub fn retain<F>(&self, mut predicate: F)
    where
        F: FnMut(&K) -> bool,
    {
        if let Ok(mut map) = self.map().lock() {
            map.retain(|key, _value| predicate(key));
        }
    }
}

#[cfg(test)]
impl<K, V> TtlMap<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Remove every entry. Test-only: production code has no need to
    /// wholesale-clear a cache.
    pub fn clear(&self) {
        if let Ok(mut map) = self.map().lock() {
            map.clear();
        }
    }

    /// Current entry count, ignoring freshness. Test-only: used to assert
    /// eviction behavior without racing real TTL expiry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map().lock().map_or(0, |map| map.len())
    }

    /// Insert `value` for `key` stamped with an explicit `inserted_at`, then
    /// evict down to capacity — mirroring [`TtlMap::insert`] but bypassing
    /// `SystemTime::now()`. Test-only: lets freshness/staleness and eviction
    /// ordering be exercised deterministically instead of via real sleeps.
    pub fn insert_at(&self, key: K, value: V, inserted_at: SystemTime) {
        let Ok(mut map) = self.map().lock() else {
            return;
        };
        map.insert(key, (value, inserted_at));
        evict_to_capacity(&mut map, self.capacity, |(_value, entry_inserted_at)| {
            *entry_inserted_at
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;

    #[test]
    fn fresh_entry_within_ttl_is_fresh() {
        let inserted = SystemTime::UNIX_EPOCH;
        let now = inserted + Duration::from_secs(30);
        assert!(is_fresh(inserted, now, Duration::from_secs(60)));
    }

    #[test]
    fn entry_past_ttl_is_not_fresh() {
        let inserted = SystemTime::UNIX_EPOCH;
        let now = inserted + Duration::from_secs(90);
        assert!(!is_fresh(inserted, now, Duration::from_secs(60)));
    }

    #[test]
    fn entry_exactly_at_ttl_boundary_is_not_fresh() {
        // matches the existing `< ttl` (strict) semantics every current cache uses
        let inserted = SystemTime::UNIX_EPOCH;
        let now = inserted + Duration::from_secs(60);
        assert!(!is_fresh(inserted, now, Duration::from_secs(60)));
    }

    #[test]
    fn clock_going_backwards_is_treated_as_stale_not_fresh() {
        // now < inserted_at: the existing caches' `.elapsed()`-based checks treat
        // this failure mode as "definitely stale" (fail safe toward re-fetching
        // rather than serving a value with an unverifiable age); the pure
        // function must preserve that direction.
        let inserted = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let now = SystemTime::UNIX_EPOCH; // clock went backwards
        assert!(!is_fresh(inserted, now, Duration::from_secs(60)));
    }

    #[test]
    #[serial_test::serial(global_ttl)]
    fn get_evicts_and_returns_none_once_ttl_has_elapsed() {
        set_shared_cache_ttl(1); // 1 hour
        let map: TtlMap<&str, u32> = TtlMap::new(0);

        map.insert_at("k", 1, SystemTime::now() - Duration::from_secs(3600 + 5));
        assert_eq!(map.len(), 1, "entry present before the stale read");

        assert_eq!(map.get("k"), None, "stale entry should read as a miss");
        assert_eq!(map.len(), 0, "stale entry should be evicted on read");

        set_shared_cache_ttl(24);
    }

    #[test]
    fn get_returns_fresh_value_and_leaves_it_in_place() {
        let map: TtlMap<&str, u32> = TtlMap::new(0);
        map.insert("k", 7);
        assert_eq!(map.get("k"), Some(7));
        assert_eq!(map.len(), 1, "fresh entry should not be evicted on read");
    }

    #[test]
    fn insert_evicts_down_to_capacity() {
        let map: TtlMap<u32, u32> = TtlMap::new(4);
        // Distinct, monotonically increasing timestamps a few seconds in the
        // past (not `UNIX_EPOCH`, and not the future): they establish recency
        // order for eviction while staying well within even the tightest TTL
        // and strictly before "now", so the freshness check inside `get()`
        // below never itself evicts a just-inserted entry as backwards-clock
        // stale.
        let now = SystemTime::now();
        for key in 0..10_u32 {
            let seconds_ago = 10_u64.saturating_sub(u64::from(key));
            map.insert_at(key, key, now - Duration::from_secs(seconds_ago));
        }
        assert!(
            map.len() <= 4,
            "map should be bounded to capacity, has {} entries",
            map.len()
        );
        // The four most-recently-inserted (largest timestamp) keys survive.
        for key in 6..10_u32 {
            assert_eq!(
                map.get(&key),
                Some(key),
                "recent key {key} should remain after eviction"
            );
        }
    }

    #[test]
    fn retain_drops_entries_failing_the_predicate() {
        let map: TtlMap<u32, u32> = TtlMap::new(0);
        map.insert(1, 1);
        map.insert(2, 2);
        map.insert(3, 3);

        map.retain(|key| *key != 2);

        assert_eq!(map.get(&1), Some(1));
        assert_eq!(map.get(&2), None);
        assert_eq!(map.get(&3), Some(3));
    }

    #[test]
    fn remove_drops_a_specific_key() {
        let map: TtlMap<&str, u32> = TtlMap::new(0);
        map.insert("a", 1);
        map.insert("b", 2);

        map.remove("a");

        assert_eq!(map.get("a"), None);
        assert_eq!(map.get("b"), Some(2));
    }
}
