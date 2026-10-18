//! A bounded, TTL-expiring cache primitive. Instances share one
//! runtime-adjustable TTL unless built with their own fixed TTL.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use super::bounded::evict_to_capacity;

/// Shared TTL (seconds) governing every [`TtlMap`] built with [`TtlMap::new`].
static SHARED_CACHE_TTL_SECONDS: AtomicU64 = AtomicU64::new(24 * 60 * 60);

/// Update the shared cache TTL (in hours), clamped to `[1, 720]`.
pub fn set_shared_cache_ttl(hours: u64) {
    let clamped = hours.clamp(1, 720);
    SHARED_CACHE_TTL_SECONDS.store(clamped.saturating_mul(3600), Ordering::SeqCst);
}

/// Current shared cache TTL, in seconds.
#[must_use]
pub fn shared_cache_ttl_seconds() -> u64 {
    SHARED_CACHE_TTL_SECONDS.load(Ordering::SeqCst)
}

/// Whether an entry inserted at `inserted_at` is still fresh at `now`.
/// A backwards clock reads as stale, failing safe toward re-fetching.
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

/// A bounded, TTL-expiring cache; `get` evicts stale entries on read and
/// `insert` bounds the map via [`evict_to_capacity`].
pub struct TtlMap<K, V> {
    entries: Entries<K, V>,
    capacity: usize,
    /// Fixed TTL; `None` follows the shared knob.
    ttl: Option<Duration>,
}

impl<K, V> TtlMap<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create an empty cache bounded to `capacity` entries (`0` = unbounded),
    /// following the shared TTL.
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            entries: OnceLock::new(),
            capacity,
            ttl: None,
        }
    }

    /// Like [`TtlMap::new`], but entries expire after `ttl` regardless of the
    /// shared knob.
    #[must_use]
    pub const fn with_ttl(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: OnceLock::new(),
            capacity,
            ttl: Some(ttl),
        }
    }

    fn map(&self) -> &Mutex<HashMap<K, Entry<V>>> {
        self.entries.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn ttl(&self) -> Duration {
        self.ttl
            .unwrap_or_else(|| Duration::from_secs(shared_cache_ttl_seconds()))
    }

    /// Return a clone of a fresh value for `key`; a stale entry is removed.
    #[must_use]
    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let ttl = self.ttl();
        let now = SystemTime::now();
        let mut map = self.map().lock().unwrap_or_else(PoisonError::into_inner);
        let entry = map.get(key)?;
        let (value, inserted_at) = (entry.0.clone(), entry.1);
        if is_fresh(inserted_at, now, ttl) {
            return Some(value);
        }
        map.remove(key);
        None
    }

    /// Insert `value` stamped with now, then evict down to capacity.
    pub fn insert(&self, key: K, value: V) {
        let mut map = self.map().lock().unwrap_or_else(PoisonError::into_inner);
        map.insert(key, (value, SystemTime::now()));
        evict_to_capacity(&mut map, self.capacity, |(_value, inserted_at)| {
            *inserted_at
        });
        drop(map);
    }

    /// Remove `key` unconditionally, regardless of freshness.
    pub fn remove<Q>(&self, key: &Q)
    where
        K: std::borrow::Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }

    /// Retain only entries whose key satisfies `predicate`.
    pub fn retain<F>(&self, mut predicate: F)
    where
        F: FnMut(&K) -> bool,
    {
        self.map()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|key, _value| predicate(key));
    }
}

#[cfg(test)]
impl<K, V> TtlMap<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Remove every entry.
    pub fn clear(&self) {
        self.map()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Current entry count, ignoring freshness.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Like [`TtlMap::insert`] but with an explicit timestamp.
    pub fn insert_at(&self, key: K, value: V, inserted_at: SystemTime) {
        let mut map = self.map().lock().unwrap_or_else(PoisonError::into_inner);
        map.insert(key, (value, inserted_at));
        evict_to_capacity(&mut map, self.capacity, |(_value, entry_inserted_at)| {
            *entry_inserted_at
        });
        drop(map);
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
        let inserted = SystemTime::UNIX_EPOCH;
        let now = inserted + Duration::from_secs(60);
        assert!(!is_fresh(inserted, now, Duration::from_secs(60)));
    }

    #[test]
    fn clock_going_backwards_is_treated_as_stale_not_fresh() {
        let inserted = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let now = SystemTime::UNIX_EPOCH;
        assert!(!is_fresh(inserted, now, Duration::from_secs(60)));
    }

    #[test]
    #[serial_test::serial(global_ttl)]
    fn get_evicts_and_returns_none_once_ttl_has_elapsed() {
        set_shared_cache_ttl(1);
        let map: TtlMap<&str, u32> = TtlMap::new(0);

        map.insert_at("k", 1, SystemTime::now() - Duration::from_secs(3600 + 5));
        assert_eq!(map.len(), 1, "entry present before the stale read");

        assert_eq!(map.get("k"), None, "stale entry should read as a miss");
        assert_eq!(map.len(), 0, "stale entry should be evicted on read");

        set_shared_cache_ttl(24);
    }

    #[test]
    fn fixed_ttl_expires_independently_of_the_shared_knob() {
        let map: TtlMap<&str, u32> = TtlMap::with_ttl(0, Duration::from_secs(60));
        let now = SystemTime::now();

        map.insert_at("young", 1, now - Duration::from_secs(30));
        map.insert_at("old", 2, now - Duration::from_secs(61));

        assert_eq!(map.get("young"), Some(1));
        assert_eq!(
            map.get("old"),
            None,
            "past the fixed TTL despite a 1h+ knob"
        );
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
        // Past, increasing timestamps: set eviction order without tripping the
        // backwards-clock check in `get()`.
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
