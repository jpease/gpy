//! Bounded eviction for path-keyed caches.
//!
//! GPY's path-keyed caches (git status, language detection, language versions)
//! are keyed by directory and were previously evicted only by TTL or explicit
//! invalidation. A long-lived agent in a shell that navigates many directories
//! would accumulate one entry per distinct path indefinitely, working against
//! the documented <50MB RSS budget for long-running sessions.
//!
//! Two complementary bounds are enforced:
//!
//! - **Entry count** (`evict_to_capacity`): hard cap on the number of cache
//!   entries, using a least-recently-updated policy.
//! - **File-detail bytes** (`drop_file_details_to_budget`): for the git-status
//!   cache, the per-file change map can dwarf the aggregate status in large
//!   dirty repositories. When the total retained bytes across all entries exceed
//!   `GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES`, file-level details are dropped
//!   from the oldest entries while aggregates are always kept.

use std::collections::HashMap;
use std::hash::Hash;

/// Maximum number of repositories tracked by the git status cache.
pub const GIT_STATUS_CACHE_CAPACITY: usize = 512;

/// Maximum bytes of per-file change detail retained across all git-status cache
/// entries.
///
/// When this budget is exceeded, file-level path maps are dropped from
/// the oldest entries (aggregate counts are always retained). Sized to keep a
/// long-lived agent under the <50 MB RSS target even when visiting repositories
/// with thousands of dirty files.
///
/// Budget breakdown (worst-case per entry): 10 000 dirty files × ~150 bytes
/// (`PathBuf` + `FileStatus`) ≈ 1.5 `MiB`; 6 such repositories fit inside this limit.
pub const GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES: usize = 10 * 1024 * 1024; // 10 MiB

/// Maximum number of directories tracked by the language detection cache.
pub const DETECTION_CACHE_CAPACITY: usize = 512;

/// Maximum number of entries tracked by the language version cache.
///
/// Keys are `language:path`, so a single directory may hold several entries.
pub const VERSION_CACHE_CAPACITY: usize = 1024;

/// Maximum entries retained in the compiled `.gitignore`-matcher cache.
/// Each compiled `Gitignore` object is typically a few kilobytes; 256 entries
/// keeps the cache well under 1 MB for the common case.
pub const IGNORE_CACHE_CAPACITY: usize = 256;

/// Maximum entries retained in the `InstantPromptCache::last_written` map.
///
/// Each entry is a (cache-key string, rendered ANSI string) pair; 1 024 entries
/// caps the in-memory dedup table at a few megabytes in the worst case.
pub const INSTANT_PROMPT_LAST_WRITTEN_CAPACITY: usize = 1024;

/// Evict the least-recently-updated entries from `map` until it holds at most
/// `capacity` entries.
///
/// `recency` extracts a monotonic "last updated" marker from each value; entries
/// with the smallest markers are removed first. The scan only runs when the map
/// is over capacity, which in steady state means evicting at most one entry per
/// insert. A `capacity` of `0` disables eviction (unbounded).
pub fn evict_to_capacity<K, V, R>(
    map: &mut HashMap<K, V>,
    capacity: usize,
    recency: impl Fn(&V) -> R,
) where
    K: Eq + Hash + Clone,
    R: Ord,
{
    if capacity == 0 || map.len() <= capacity {
        return;
    }

    let excess = map.len().saturating_sub(capacity);

    // Collect keys ordered by recency (oldest first), then drop the excess.
    let mut keyed: Vec<(R, K)> = map
        .iter()
        .map(|(key, value)| (recency(value), key.clone()))
        .collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0));

    for (_, key) in keyed.into_iter().take(excess) {
        map.remove(&key);
    }
}

/// Drop file-level details from the oldest entries in `map` until the total
/// estimated bytes across all entries is at or below `budget_bytes`.
///
/// `estimate_bytes` returns the current detail-bytes cost for a value; `drop_details`
/// mutates the value in place to discard file details (setting its byte cost to zero).
/// Entries are processed oldest-first via `recency`. Aggregate status fields are
/// always preserved — only the per-file detail payload is dropped.
///
/// A `budget_bytes` of `0` disables detail eviction (unbounded).
pub fn drop_file_details_to_budget<K, V, R>(
    map: &mut HashMap<K, V>,
    budget_bytes: usize,
    recency: impl Fn(&V) -> R,
    estimate_bytes: impl Fn(&V) -> usize,
    drop_details: impl Fn(&mut V),
) where
    K: Eq + Hash + Clone,
    R: Ord,
{
    if budget_bytes == 0 {
        return;
    }

    let total: usize = map.values().map(&estimate_bytes).sum();
    if total <= budget_bytes {
        return;
    }

    // Sort keys oldest-first so we drain detail from the least-recently-used entries.
    let mut ordered: Vec<(R, K)> = map.iter().map(|(k, v)| (recency(v), k.clone())).collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    let mut remaining = total;
    for (_, key) in ordered {
        if remaining <= budget_bytes {
            break;
        }
        if let Some(entry) = map.get_mut(&key) {
            let cost = estimate_bytes(entry);
            if cost > 0 {
                drop_details(entry);
                remaining = remaining.saturating_sub(cost);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::{drop_file_details_to_budget, evict_to_capacity};
    use std::collections::HashMap;

    fn marker_map(count: u64) -> HashMap<u64, u64> {
        (0..count).map(|key| (key, key)).collect()
    }

    #[test]
    fn no_eviction_when_within_capacity() {
        let mut map = marker_map(5);
        evict_to_capacity(&mut map, 8, |&recency| recency);
        assert_eq!(map.len(), 5);
    }

    #[test]
    fn evicts_oldest_entries_down_to_capacity() {
        // Key doubles as the recency marker: smaller == older.
        let mut map = marker_map(10);
        evict_to_capacity(&mut map, 4, |&recency| recency);

        assert_eq!(map.len(), 4);
        // The four most-recent (largest) markers survive: keys 6,7,8,9.
        for key in 0..6 {
            assert!(!map.contains_key(&key), "old key {key} should be evicted");
        }
        for key in 6..10 {
            assert!(map.contains_key(&key), "recent key {key} should remain");
        }
    }

    #[test]
    fn zero_capacity_disables_eviction() {
        let mut map = marker_map(3);
        evict_to_capacity(&mut map, 0, |&recency| recency);
        assert_eq!(map.len(), 3);
    }

    /// Each entry holds `(recency_marker, detail_bytes)`.
    /// `estimate_bytes` returns `detail_bytes`; `drop_details` zeroes it.
    fn detail_map(entries: &[(u64, usize)]) -> HashMap<u64, (u64, usize)> {
        entries.iter().map(|&(k, b)| (k, (k, b))).collect()
    }

    #[test]
    fn no_drop_when_within_budget() {
        let mut map = detail_map(&[(0, 100), (1, 100), (2, 100)]);
        drop_file_details_to_budget(&mut map, 1000, |v| v.0, |v| v.1, |v| v.1 = 0);
        assert!(map.values().all(|v| v.1 == 100));
    }

    #[test]
    fn drops_oldest_details_first_until_within_budget() {
        // 4 entries × 100 bytes = 400 bytes total; budget = 250 bytes.
        // Oldest two (keys 0, 1) should have details dropped (200 bytes freed).
        let mut map = detail_map(&[(0, 100), (1, 100), (2, 100), (3, 100)]);
        drop_file_details_to_budget(&mut map, 250, |v| v.0, |v| v.1, |v| v.1 = 0);
        assert_eq!(
            map.get(&0).expect("entry 0 present").1,
            0,
            "oldest entry should have details dropped"
        );
        assert_eq!(
            map.get(&1).expect("entry 1 present").1,
            0,
            "second-oldest entry should have details dropped"
        );
        assert_eq!(
            map.get(&2).expect("entry 2 present").1,
            100,
            "entry 2 should be retained"
        );
        assert_eq!(
            map.get(&3).expect("entry 3 present").1,
            100,
            "newest entry should be retained"
        );
    }

    #[test]
    fn zero_budget_disables_detail_drop() {
        let mut map = detail_map(&[(0, 1000), (1, 1000)]);
        drop_file_details_to_budget(&mut map, 0, |v| v.0, |v| v.1, |v| v.1 = 0);
        assert!(map.values().all(|v| v.1 == 1000));
    }
}
