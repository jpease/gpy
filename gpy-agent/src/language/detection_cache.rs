//! In-memory cache for directory language detection results.

use crate::cache::bounded::{DETECTION_CACHE_CAPACITY, evict_to_capacity};
use crate::language::detector::DetectedLanguage;
use crate::sync_ext::RwLockExt;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Minimum age before a cached language entry is re-detected.
pub(crate) const LANGUAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

struct DetectionEntry {
    languages: Vec<DetectedLanguage>,
    last_refresh: Instant,
    signature: Option<u64>,
}

type DetectionMap = HashMap<PathBuf, DetectionEntry>;

/// Per-directory detection results shared across threads.
#[derive(Default, Clone)]
pub struct DetectionCache {
    cache: Arc<RwLock<DetectionMap>>,
}

impl DetectionCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Cached results for `path`.
    #[must_use]
    pub fn get(&self, path: &Path) -> Option<Vec<DetectedLanguage>> {
        let cache = self.cache.read_or_recover();
        cache.get(path).map(|entry| entry.languages.clone())
    }

    /// Get cached detection results plus time since last refresh.
    #[must_use]
    pub fn get_with_age(&self, path: &Path) -> Option<(Vec<DetectedLanguage>, Duration)> {
        let cache = self.cache.read_or_recover();
        cache
            .get(path)
            .map(|entry| (entry.languages.clone(), entry.last_refresh.elapsed()))
    }

    /// Store `results` for `path` without a refresh signature.
    pub fn set(&self, path: &Path, results: Vec<DetectedLanguage>) {
        self.set_entry(path, results, None);
    }

    /// Store `results` for `path` with a refresh signature.
    pub fn set_with_signature(&self, path: &Path, results: Vec<DetectedLanguage>, signature: u64) {
        self.set_entry(path, results, Some(signature));
    }

    fn set_entry(&self, path: &Path, results: Vec<DetectedLanguage>, signature: Option<u64>) {
        let mut cache = self.cache.write_or_recover();
        cache.insert(
            path.to_path_buf(),
            DetectionEntry {
                languages: results,
                last_refresh: Instant::now(),
                signature,
            },
        );
        evict_to_capacity(&mut cache, DETECTION_CACHE_CAPACITY, |entry| {
            entry.last_refresh
        });
        drop(cache);
    }

    /// Drop the entry for `path`.
    pub fn invalidate(&self, path: &Path) {
        self.cache.write_or_recover().remove(path);
    }

    /// Whether a refresh should run: no entry, or the entry is older than
    /// `min_interval` and its signature differs.
    #[must_use]
    pub fn should_refresh(&self, path: &Path, signature: u64, min_interval: Duration) -> bool {
        self.cache.read_or_recover().get(path).is_none_or(|entry| {
            entry.last_refresh.elapsed() >= min_interval && entry.signature != Some(signature)
        })
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use crate::language::DetectedLanguage;

    impl DetectionCache {
        /// Backdate the entry for `path` by `by`; no-op if absent.
        pub(crate) fn age_entry_for_test(&self, path: &Path, by: Duration) {
            if let Some(entry) = self.cache.write_or_recover().get_mut(path) {
                entry.last_refresh = Instant::now().checked_sub(by).unwrap_or_else(Instant::now);
            }
        }
    }

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
