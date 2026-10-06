//! In-memory cache for language detection results
//!
//! Stores the result of expensive directory language scans so they can be reused
//! for instant prompt generation.

use crate::cache::bounded::{DETECTION_CACHE_CAPACITY, evict_to_capacity};
use crate::language::detector::DetectedLanguage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Rate limit for re-detection of a cached language entry.
///
/// Shared by the git-status-driven refresh in `agent::events` and the language
/// handler's stale-hit revalidation for non-git directories, so both agree on
/// how old an entry must be before it is re-detected.
pub(crate) const LANGUAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

struct DetectionEntry {
    languages: Vec<DetectedLanguage>,
    last_refresh: Instant,
    signature: Option<u64>,
}

type DetectionMap = HashMap<PathBuf, DetectionEntry>;

/// Thread-safe cache for language detection results
#[derive(Default, Clone)]
pub struct DetectionCache {
    cache: Arc<RwLock<DetectionMap>>,
}

impl DetectionCache {
    /// Create a new language detection cache
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get cached detection results for a directory
    #[must_use]
    pub fn get(&self, path: &Path) -> Option<Vec<DetectedLanguage>> {
        let cache = self.cache.read().ok()?;
        // Clone the results because DetectedLanguage is relatively small
        // and we want to avoid holding the lock
        cache.get(path).map(|entry| entry.languages.clone())
    }

    /// Get cached detection results together with the time since the entry was
    /// last refreshed, so callers can revalidate stale hits.
    #[must_use]
    pub fn get_with_age(&self, path: &Path) -> Option<(Vec<DetectedLanguage>, Duration)> {
        let cache = self.cache.read().ok()?;
        cache
            .get(path)
            .map(|entry| (entry.languages.clone(), entry.last_refresh.elapsed()))
    }

    /// Test-only helper: pretend the entry for `path` was last refreshed `by`
    /// ago, so stale-hit behaviour can be exercised without sleeping. No-op if
    /// the path has no entry.
    #[cfg(test)]
    pub(crate) fn age_entry_for_test(&self, path: &Path, by: Duration) {
        if let Ok(mut cache) = self.cache.write()
            && let Some(entry) = cache.get_mut(path)
        {
            entry.last_refresh = Instant::now().checked_sub(by).unwrap_or_else(Instant::now);
        }
    }

    /// Set cached detection results for a directory
    pub fn set(&self, path: &Path, results: Vec<DetectedLanguage>) {
        self.set_entry(path, results, None);
    }

    /// Set cached detection results with a refresh signature.
    pub fn set_with_signature(&self, path: &Path, results: Vec<DetectedLanguage>, signature: u64) {
        self.set_entry(path, results, Some(signature));
    }

    fn set_entry(&self, path: &Path, results: Vec<DetectedLanguage>, signature: Option<u64>) {
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                path.to_path_buf(),
                DetectionEntry {
                    languages: results,
                    last_refresh: Instant::now(),
                    signature,
                },
            );
            // Cap memory for agents that visit many directories
            // (least-recently-updated eviction).
            evict_to_capacity(&mut cache, DETECTION_CACHE_CAPACITY, |entry| {
                entry.last_refresh
            });
        }
    }

    /// Invalidate cache for a directory
    pub fn invalidate(&self, path: &Path) {
        if let Ok(mut cache) = self.cache.write() {
            cache.remove(path);
        }
    }

    /// Returns true if a refresh should run based on signature change and interval.
    #[must_use]
    pub fn should_refresh(&self, path: &Path, signature: u64, min_interval: Duration) -> bool {
        let Ok(cache) = self.cache.read() else {
            return true;
        };

        let Some(entry) = cache.get(path) else {
            return true;
        };

        if entry.last_refresh.elapsed() < min_interval {
            return false;
        }

        entry.signature != Some(signature)
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use crate::language::DetectedLanguage;

    fn sample_lang() -> Vec<DetectedLanguage> {
        vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 1,
        }]
    }

    #[test]
    fn should_refresh_respects_signature_match() {
        let cache = DetectionCache::new();
        let path = Path::new("/tmp/repo");
        cache.set_with_signature(path, sample_lang(), 42);

        assert!(!cache.should_refresh(path, 42, Duration::from_secs(0)));
    }

    #[test]
    fn cache_is_bounded_when_visiting_many_directories() {
        use crate::cache::bounded::DETECTION_CACHE_CAPACITY;

        let cache = DetectionCache::new();
        let visits = DETECTION_CACHE_CAPACITY.saturating_mul(2);
        for i in 0..visits {
            cache.set(&PathBuf::from(format!("/dir/{i}")), sample_lang());
        }

        let len = cache.cache.read().expect("cache read lock").len();
        assert!(
            len <= DETECTION_CACHE_CAPACITY,
            "cache grew to {len} entries, exceeding cap {DETECTION_CACHE_CAPACITY}"
        );
    }

    #[test]
    fn should_refresh_when_signature_changes() {
        let cache = DetectionCache::new();
        let path = Path::new("/tmp/repo");
        cache.set_with_signature(path, sample_lang(), 1);

        assert!(cache.should_refresh(path, 2, Duration::from_secs(0)));
    }
}
