//! Bounded eviction for path-keyed caches, keeping long-lived agents under the
//! <50MB RSS budget: an entry-count cap plus a byte budget for git file details.

use std::collections::HashMap;
use std::hash::Hash;

/// Maximum number of repositories tracked by the git status cache.
pub const GIT_STATUS_CACHE_CAPACITY: usize = 512;

/// Estimated bytes of change detail held per file in a git-status entry.
pub const FILE_DETAIL_BYTES_PER_FILE: usize = 80;

/// Byte budget for per-file change detail across all git-status entries; the
/// oldest entries lose file maps first. At [`FILE_DETAIL_BYTES_PER_FILE`] per
/// file, that is about 0.8 MB per repo with 10,000 dirty files.
pub const GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES: usize = 10 * 1024 * 1024;

/// Maximum number of directories tracked by the language detection cache.
pub const DETECTION_CACHE_CAPACITY: usize = 512;

/// Maximum number of entries tracked by the language version cache.
///
/// Keys are `language:path`, so a single directory may hold several entries.
pub const VERSION_CACHE_CAPACITY: usize = 1024;

/// Maximum entries retained in the compiled `.gitignore`-matcher cache.
pub const IGNORE_CACHE_CAPACITY: usize = 256;

/// Maximum repositories tracked by each `NativeGitBackend` config cache.
pub const NATIVE_GIT_CONFIG_CACHE_CAPACITY: usize = 512;

/// Maximum entries retained in the `InstantPromptCache::last_written` map.
///
/// 32 repositories × 16 `prev_bg` contexts (`MAX_PREV_BG_CONTEXTS_PER_KEY`) ×
/// 4 segment positions × 3 prompt dialects × 2 kinds (git and language), so 32
/// repositories at the context maximum keep every record. A record is about
/// 100 B of name plus at most 400 B of content: about 6 MB worst case.
pub const INSTANT_PROMPT_LAST_WRITTEN_CAPACITY: usize = 12_288;

/// Maximum keys retained in the `InstantPromptCache` `seen_contexts` and
/// `unwritable_keys` tables. Language keys include every non-git directory a
/// shell visits, so this table holds far more keys than are written at once.
pub const INSTANT_PROMPT_KEY_CAPACITY: usize = 1024;

/// Evict the least-recently-updated entries (smallest `recency`) until `map`
/// holds at most `capacity` entries; `0` disables eviction.
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

    if excess == 1 {
        let oldest = map
            .iter()
            .min_by_key(|(_, value)| recency(value))
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            map.remove(&key);
        }
        return;
    }

    let mut keyed: Vec<(R, K)> = map
        .iter()
        .map(|(key, value)| (recency(value), key.clone()))
        .collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0));

    for (_, key) in keyed.into_iter().take(excess) {
        map.remove(&key);
    }
}

/// Drop file details from the oldest entries until estimated bytes fit
/// `budget_bytes`; aggregates are kept. `0` disables detail eviction.
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
