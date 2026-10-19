//! Git status cache, invalidated by the file watcher with a TTL backstop.
//!
//! Bounded by entry count and by total per-file detail bytes; entries over the
//! byte budget keep aggregates but lose file detail, forcing a full rescan on
//! the next incremental update.

use crate::cache::CachePolicy;
use crate::cache::bounded::{
    GIT_STATUS_CACHE_CAPACITY, GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES, drop_file_details_to_budget,
    evict_to_capacity,
};
use crate::debug_log;
use crate::git::native::resolve_overall_state;
use crate::git::{FileStatus, RepositoryStatus, StatusAggregate};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

/// Cached status plus file-level details for incremental updates.
#[derive(Clone, Debug)]
pub struct CachedRepoStatus {
    /// Aggregated repository status.
    pub aggregates: RepositoryStatus,
    /// Per-file status map.
    pub files: HashMap<PathBuf, FileStatus>,
}

#[derive(Clone)]
struct CacheEntry {
    data: CachedRepoStatus,
    cached_at: Instant,
    /// `false` when per-file details were dropped for the byte budget;
    /// incremental updates then require a full scan.
    files_complete: bool,
}

impl CacheEntry {
    /// Flat per-file byte estimate, O(1) without inspecting paths.
    fn file_detail_bytes(&self) -> usize {
        if !self.files_complete {
            return 0;
        }
        self.data.files.len().saturating_mul(80)
    }
}

/// Recompute aggregates from the file map and refresh the timestamp.
fn reaggregate(entry: &mut CacheEntry) -> RepositoryStatus {
    let aggregate = StatusAggregate::from_file_statuses(entry.data.files.values());
    entry.data.aggregates.staged = aggregate.staged;
    entry.data.aggregates.unstaged = aggregate.unstaged;
    entry.data.aggregates.untracked = aggregate.untracked;
    entry.data.aggregates.conflicts = aggregate.conflicts;

    // Preserve in-progress states from the last full scan: the incremental
    // path cannot detect `.git/MERGE_HEAD` and friends.
    let special_state = entry.data.aggregates.state.as_preserved_special_state();
    entry.data.aggregates.state = resolve_overall_state(
        aggregate.conflicts,
        special_state,
        aggregate.staged,
        aggregate.unstaged,
        aggregate.untracked,
    );

    entry.cached_at = Instant::now();

    entry.data.aggregates.clone()
}

/// `path.starts_with(prefix)`, optionally comparing components
/// ASCII-case-insensitively (a `core.ignorecase=true` repository).
fn path_starts_with(path: &Path, prefix: &Path, ignore_case: bool) -> bool {
    if !ignore_case {
        return path.starts_with(prefix);
    }
    let mut path_components = path.components();
    prefix.components().all(|wanted| {
        path_components
            .next()
            .is_some_and(|actual| actual.as_os_str().eq_ignore_ascii_case(wanted.as_os_str()))
    })
}

/// Whether `left` and `right` name the same path under the comparison
/// [`path_starts_with`] uses.
fn paths_equal(left: &Path, right: &Path, ignore_case: bool) -> bool {
    path_starts_with(left, right, ignore_case) && path_starts_with(right, left, ignore_case)
}

/// Git status cache with policy-driven TTL and cooldown
#[derive(Clone)]
pub struct GitStatusCache {
    entries: Arc<Mutex<HashMap<PathBuf, CacheEntry>>>,
    policy: CachePolicy,
}

impl GitStatusCache {
    /// Canonicalize `path` as a cache key; `*_canonical` methods skip this syscall.
    fn canonical_key(path: &Path) -> PathBuf {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    /// Create a cache with the standard git-status policy.
    #[must_use]
    pub fn new() -> Self {
        Self::with_policy(CachePolicy::git_status())
    }

    /// Create a cache with an explicit timing policy.
    #[must_use]
    pub(crate) fn with_policy(policy: CachePolicy) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            policy,
        }
    }

    /// Get cached status; `None` if missing or past TTL.
    #[must_use]
    pub fn get(&self, repo_path: &Path) -> Option<RepositoryStatus> {
        self.get_canonical(&Self::canonical_key(repo_path))
    }

    /// [`get`](Self::get) for an already-canonical key.
    #[must_use]
    pub fn get_canonical(&self, key: &Path) -> Option<RepositoryStatus> {
        let (aggregates, cached_at) = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map(|e| (e.data.aggregates.clone(), e.cached_at))?;

        if cached_at.elapsed() >= self.policy.ttl() {
            debug_log!(
                "cache",
                "Cache EXPIRED for {} (age: {:?}, TTL: {:?})",
                key.display(),
                cached_at.elapsed(),
                self.policy.ttl()
            );
            return None;
        }

        debug_log!(
            "cache",
            "Cache HIT for {} (age: {:?})",
            key.display(),
            cached_at.elapsed()
        );
        Some(aggregates)
    }

    /// Get cached status regardless of TTL.
    #[must_use]
    pub fn get_any(&self, repo_path: &Path) -> Option<RepositoryStatus> {
        self.get_any_canonical(&Self::canonical_key(repo_path))
    }

    /// [`get_any`](Self::get_any) for an already-canonical key.
    #[must_use]
    pub fn get_any_canonical(&self, key: &Path) -> Option<RepositoryStatus> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map(|e| e.data.aggregates.clone())
    }

    /// [`get_any_canonical`](Self::get_any_canonical) plus the entry's age.
    #[must_use]
    pub fn get_any_with_age_canonical(
        &self,
        key: &Path,
    ) -> Option<(RepositoryStatus, std::time::Duration)> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map(|e| (e.data.aggregates.clone(), e.cached_at.elapsed()))
    }

    /// Cache status for a repository.
    pub fn set(
        &self,
        repo_path: &Path,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) {
        self.set_canonical(&Self::canonical_key(repo_path), status, files);
    }

    /// [`set`](Self::set) for an already-canonical key; enforces the entry-count
    /// and file-detail byte bounds.
    pub fn set_canonical(
        &self,
        key: &Path,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) {
        self.set_inner(key, status, files, None);
    }

    /// Like [`set_canonical`](Self::set_canonical), but skips the write and
    /// returns `false` when the entry was written after `started`.
    #[must_use]
    pub fn set_canonical_if_not_newer(
        &self,
        key: &Path,
        started: Instant,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) -> bool {
        self.set_inner(key, status, files, Some(started))
    }

    /// Compare, insert and evict under one `entries` guard; with `started`,
    /// an entry written after it is kept and `false` returned.
    fn set_inner(
        &self,
        key: &Path,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
        started: Option<Instant>,
    ) -> bool {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(started) = started
            && entries.get(key).is_some_and(|e| e.cached_at > started)
        {
            debug_log!("cache", "Kept newer status for {}", key.display());
            return false;
        }
        debug_log!("cache", "Caching status for {}", key.display());
        entries.insert(
            key.to_path_buf(),
            CacheEntry {
                data: CachedRepoStatus {
                    aggregates: status,
                    files,
                },
                cached_at: Instant::now(),
                files_complete: true,
            },
        );
        evict_to_capacity(&mut entries, GIT_STATUS_CACHE_CAPACITY, |entry| {
            entry.cached_at
        });
        drop_file_details_to_budget(
            &mut entries,
            GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES,
            |entry| entry.cached_at,
            CacheEntry::file_detail_bytes,
            |entry| {
                entry.data.files.clear();
                entry.data.files.shrink_to_fit();
                entry.files_complete = false;
            },
        );
        drop(entries);
        true
    }

    /// Fold a pathspec-limited scan into the cached file map.
    ///
    /// `scanned` and `results` are relative to `key`. The scan is authoritative
    /// under `scanned`: each scanned subtree is replaced by `results`, so
    /// unreported paths (rename sources, reverted edits) become clean.
    ///
    /// Returns `None` (do a full scan) when the entry is missing or lost its
    /// details, or when git's collapsed untracked `dir/` entries make the merge
    /// inconsistent: a cached untracked entry is an ancestor of a scanned path,
    /// or the scan reports a nested untracked file.
    ///
    /// `ignore_case` compares paths ASCII-case-insensitively, since the watcher
    /// reports on-disk spelling and the cache holds index spelling.
    #[must_use]
    pub fn update_paths_canonical(
        &self,
        key: &Path,
        scanned: &[PathBuf],
        results: &HashMap<PathBuf, FileStatus>,
        ignore_case: bool,
    ) -> Option<RepositoryStatus> {
        let mut guard = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = guard.get_mut(key)?;

        if !entry.files_complete {
            return None;
        }

        let relative_scanned: Vec<&Path> = scanned
            .iter()
            .map(|path| path.strip_prefix(key).unwrap_or(path))
            .collect();
        let collapsed_ancestor = relative_scanned.iter().any(|relative| {
            entry.data.files.iter().any(|(cached, status)| {
                status.untracked
                    && !paths_equal(cached, relative, ignore_case)
                    && path_starts_with(relative, cached, ignore_case)
            })
        });
        let nested_untracked = results.iter().any(|(path, status)| {
            status.untracked
                && path
                    .parent()
                    .is_some_and(|parent| !parent.as_os_str().is_empty())
        });
        if collapsed_ancestor || nested_untracked {
            return None;
        }

        for relative in relative_scanned {
            // A scanned directory covers its whole subtree.
            entry
                .data
                .files
                .retain(|cached, _| !path_starts_with(cached, relative, ignore_case));
        }
        for (path, status) in results {
            if status.is_clean() {
                continue;
            }
            entry.data.files.insert(path.clone(), *status);
        }

        let aggregates = reaggregate(entry);

        drop(guard);
        Some(aggregates)
    }

    /// Whether the entry was updated within the policy's cooldown window.
    #[must_use]
    pub fn is_fresh(&self, repo_path: &Path) -> bool {
        self.is_fresh_canonical(&Self::canonical_key(repo_path))
    }

    /// [`is_fresh`](Self::is_fresh) for an already-canonical key.
    #[must_use]
    pub fn is_fresh_canonical(&self, key: &Path) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .is_some_and(|entry| entry.cached_at.elapsed() < self.policy.cooldown())
    }

    /// Invalidate cached status for a repository.
    pub fn invalidate(&self, repo_path: &Path) {
        self.invalidate_canonical(&Self::canonical_key(repo_path));
    }

    /// [`invalidate`](Self::invalidate) for an already-canonical key.
    pub fn invalidate_canonical(&self, key: &Path) {
        debug_log!("cache", "Invalidating cache for {}", key.display());
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }

    /// Clear all cached entries
    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Number of cached repositories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Backdate an entry's `cached_at` by `age`.
    #[cfg(test)]
    pub(crate) fn age_entry_for_test(&self, key: &Path, age: std::time::Duration) {
        if let Some(entry) = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(key)
        {
            entry.cached_at = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
        }
    }

    /// The cached per-file map for `key`.
    #[cfg(test)]
    pub(crate) fn files_for_test(&self, key: &Path) -> Option<HashMap<PathBuf, FileStatus>> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map(|entry| entry.data.files.clone())
    }
}

impl Default for GitStatusCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::GitStatusCache;
    use crate::cache::CachePolicy;
    use crate::git::{FileStatus, RepositoryState, RepositoryStatus};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn dummy_status() -> RepositoryStatus {
        RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }
    }

    fn dirty_files(count: usize) -> HashMap<PathBuf, FileStatus> {
        (0..count)
            .map(|i| {
                (
                    PathBuf::from(format!("src/file_{i:05}.rs")),
                    FileStatus {
                        staged: false,
                        unstaged: true,
                        untracked: false,
                        conflicted: false,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn is_fresh_uses_policy_cooldown() {
        let repo = PathBuf::from("/repo");
        let within = GitStatusCache::with_policy(CachePolicy {
            ttl: Duration::from_secs(30),
            cooldown: Duration::from_secs(3600),
        });
        within.set(&repo, dummy_status(), HashMap::new());
        assert!(within.is_fresh(&repo));

        let past = GitStatusCache::with_policy(CachePolicy {
            ttl: Duration::from_secs(30),
            cooldown: Duration::ZERO,
        });
        past.set(&repo, dummy_status(), HashMap::new());
        assert!(!past.is_fresh(&repo));
    }

    #[test]
    fn set_canonical_if_not_newer_keeps_the_newer_entry() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/not-newer-keep");
        let newer = RepositoryStatus {
            untracked: 2,
            ..dummy_status()
        };
        let stale = RepositoryStatus {
            untracked: 1,
            ..dummy_status()
        };
        // A second back so coarse clocks cannot make the instants equal.
        let started = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("instant");
        cache.set_canonical(&key, newer.clone(), HashMap::new());

        assert!(!cache.set_canonical_if_not_newer(&key, started, stale, HashMap::new()));
        assert_eq!(cache.get_any_canonical(&key), Some(newer));
    }

    #[test]
    fn set_canonical_if_not_newer_writes_when_older() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/not-newer-write");
        let older = RepositoryStatus {
            untracked: 1,
            ..dummy_status()
        };
        let newer = RepositoryStatus {
            untracked: 2,
            ..dummy_status()
        };
        cache.set_canonical(&key, older, HashMap::new());
        cache.age_entry_for_test(&key, Duration::from_secs(5));
        let started = Instant::now();

        assert!(cache.set_canonical_if_not_newer(&key, started, newer.clone(), HashMap::new()));
        assert_eq!(cache.get_any_canonical(&key), Some(newer));
    }

    #[test]
    fn new_uses_git_status_policy() {
        assert_eq!(GitStatusCache::new().policy, CachePolicy::git_status());
    }

    #[test]
    fn cache_is_bounded_when_visiting_many_repositories() {
        use crate::cache::bounded::GIT_STATUS_CACHE_CAPACITY;

        let cache = GitStatusCache::new();
        let visits = GIT_STATUS_CACHE_CAPACITY.saturating_mul(2);
        for i in 0..visits {
            cache.set(
                &PathBuf::from(format!("/repo/{i}")),
                dummy_status(),
                HashMap::new(),
            );
        }

        assert!(
            cache.len() <= GIT_STATUS_CACHE_CAPACITY,
            "cache grew to {} entries, exceeding cap {GIT_STATUS_CACHE_CAPACITY}",
            cache.len()
        );
    }

    #[test]
    fn get_misses_once_policy_ttl_has_elapsed() {
        let repo = PathBuf::from("/repo");
        let live = GitStatusCache::new();
        live.set(&repo, dummy_status(), HashMap::new());
        assert!(live.get(&repo).is_some());

        let expired = GitStatusCache::with_policy(CachePolicy {
            ttl: Duration::ZERO,
            cooldown: Duration::ZERO,
        });
        expired.set(&repo, dummy_status(), HashMap::new());
        assert!(expired.get(&repo).is_none());
    }

    #[test]
    fn aggregate_preserved_when_file_details_exceed_byte_budget() {
        use crate::cache::bounded::GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES;

        let cache = GitStatusCache::new();

        let files_per_repo = 2_000_usize;
        let repo_count = (GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES / (files_per_repo * 80)) + 10;

        for i in 0..repo_count {
            let mut status = dummy_status();
            status.unstaged = u32::try_from(files_per_repo).unwrap_or(u32::MAX);
            cache.set(
                &PathBuf::from(format!("/large-repo/{i}")),
                status,
                dirty_files(files_per_repo),
            );
        }

        for i in 0..repo_count {
            let repo = PathBuf::from(format!("/large-repo/{i}"));
            assert!(
                cache.get_any(&repo).is_some(),
                "aggregate for repo {i} must be retained after file-detail eviction"
            );
        }
    }

    #[test]
    fn incremental_update_falls_back_when_details_dropped() {
        use crate::cache::bounded::GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES;

        let cache = GitStatusCache::new();

        // One entry alone exceeds the byte budget, so the older repo_a loses details.
        let files_per_repo = GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES / 80 + 1;
        let repo_a = PathBuf::from("/repo/a");
        let repo_b = PathBuf::from("/repo/b");

        cache.set(&repo_a, dummy_status(), dirty_files(10));
        cache.set(&repo_b, dummy_status(), dirty_files(files_per_repo));

        let file = PathBuf::from("src/main.rs");
        let result = cache.update_paths_canonical(
            &repo_a,
            std::slice::from_ref(&file),
            &HashMap::from([(
                file.clone(),
                FileStatus {
                    staged: true,
                    ..FileStatus::default()
                },
            )]),
            false,
        );
        assert!(
            result.is_none(),
            "incremental update should return None when file details were dropped"
        );

        assert!(
            cache.get_any(&repo_a).is_some(),
            "aggregate must survive after file-detail eviction"
        );
    }

    #[test]
    fn incremental_update_flips_clean_to_dirty() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/incremental-dirty");
        cache.set(&repo, dummy_status(), HashMap::new());

        let file = PathBuf::from("src/main.rs");
        let result = cache.update_paths_canonical(
            &repo,
            std::slice::from_ref(&file),
            &HashMap::from([(
                file.clone(),
                FileStatus {
                    unstaged: true,
                    ..FileStatus::default()
                },
            )]),
            false,
        );

        let status = result.expect("entry with complete file details must return Some");
        assert_eq!(status.unstaged, 1);
        assert_eq!(
            status.state,
            RepositoryState::Dirty,
            "state must be re-derived from the fresh counts, not left stale at Clean"
        );
    }

    #[test]
    fn incremental_update_flips_dirty_back_to_clean_on_revert() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/incremental-revert");
        let file = PathBuf::from("src/main.rs");

        let mut dirty_status = dummy_status();
        dirty_status.unstaged = 1;
        dirty_status.state = RepositoryState::Dirty;
        let mut files = HashMap::new();
        files.insert(
            file.clone(),
            FileStatus {
                unstaged: true,
                ..FileStatus::default()
            },
        );
        cache.set(&repo, dirty_status, files);

        let result = cache.update_paths_canonical(
            &repo,
            std::slice::from_ref(&file),
            &HashMap::from([(file.clone(), FileStatus::default())]),
            false,
        );

        let status = result.expect("entry with complete file details must return Some");
        assert_eq!(status.unstaged, 0);
        assert_eq!(
            status.state,
            RepositoryState::Clean,
            "state must flip back to Clean once the file is no longer dirty"
        );
    }

    #[test]
    fn incremental_update_reports_conflicts_with_precedence() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/incremental-conflict");
        cache.set(&repo, dummy_status(), HashMap::new());

        let file = PathBuf::from("src/main.rs");
        let result = cache.update_paths_canonical(
            &repo,
            std::slice::from_ref(&file),
            &HashMap::from([(
                file.clone(),
                FileStatus {
                    conflicted: true,
                    ..FileStatus::default()
                },
            )]),
            false,
        );

        let status = result.expect("entry with complete file details must return Some");
        assert_eq!(status.conflicts, 1);
        assert_eq!(status.state, RepositoryState::Conflicts);
    }

    /// The incremental path must preserve in-progress states like a merge.
    #[test]
    fn incremental_update_preserves_special_state() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/incremental-merging");

        let mut merging_status = dummy_status();
        merging_status.state = RepositoryState::Merging;
        cache.set(&repo, merging_status, HashMap::new());

        let file = PathBuf::from("src/main.rs");
        let result = cache.update_paths_canonical(
            &repo,
            std::slice::from_ref(&file),
            &HashMap::from([(
                file.clone(),
                FileStatus {
                    unstaged: true,
                    ..FileStatus::default()
                },
            )]),
            false,
        );

        let status = result.expect("entry with complete file details must return Some");
        assert_eq!(status.unstaged, 1);
        assert_eq!(
            status.state,
            RepositoryState::Merging,
            "special in-progress state must be preserved, not overwritten with Dirty"
        );
    }

    #[test]
    fn get_any_with_age_canonical_exposes_age_for_present_entry() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/age-present");
        cache.set(&repo, dummy_status(), HashMap::new());

        let (status, age) = cache
            .get_any_with_age_canonical(&repo)
            .expect("freshly seeded entry must be present");
        assert_eq!(status, dummy_status());
        assert!(
            age < Duration::from_secs(5),
            "a freshly seeded entry should be reported as very young, got {age:?}"
        );
    }

    #[test]
    fn get_any_with_age_canonical_returns_none_for_absent_key() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/age-absent");

        assert!(cache.get_any_with_age_canonical(&repo).is_none());
    }

    /// Paths the scan did not report are clean now, so their entries go.
    #[test]
    fn multi_path_update_clears_paths_the_scan_did_not_report() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/multi-path");
        let reverted = PathBuf::from("a.txt");
        let still_dirty = PathBuf::from("b.txt");

        let mut files = HashMap::new();
        files.insert(reverted.clone(), unstaged());
        files.insert(still_dirty.clone(), unstaged());
        cache.set(&repo, dummy_status(), files);

        let mut scan = HashMap::new();
        scan.insert(still_dirty.clone(), unstaged());

        let status = cache
            .update_paths_canonical(&repo, &[reverted, still_dirty], &scan, false)
            .expect("entry with complete file details must return Some");

        assert_eq!(
            status.unstaged, 1,
            "only the file still dirty may be counted"
        );
    }

    /// `git status` keys renames by destination, so the source must be dropped.
    #[test]
    fn multi_path_update_drops_a_renamed_source_path() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/rename");
        let source = PathBuf::from("a.txt");
        let destination = PathBuf::from("b.txt");

        let mut files = HashMap::new();
        files.insert(source.clone(), unstaged());
        cache.set(&repo, dummy_status(), files);

        let mut scan = HashMap::new();
        scan.insert(destination.clone(), staged());

        let status = cache
            .update_paths_canonical(&repo, &[source, destination], &scan, false)
            .expect("entry with complete file details must return Some");

        assert_eq!(
            status.unstaged, 0,
            "the renamed-away source must be dropped"
        );
        assert_eq!(status.staged, 1, "the rename destination must be recorded");
    }

    #[test]
    fn multi_path_update_replaces_a_whole_scanned_subtree() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/subtree");
        let scanned_dir = PathBuf::from("src");

        let mut files = HashMap::new();
        files.insert(PathBuf::from("src/gone.rs"), unstaged());
        files.insert(PathBuf::from("untouched.rs"), unstaged());
        cache.set(&repo, dummy_status(), files);

        let mut scan = HashMap::new();
        scan.insert(PathBuf::from("src/new.rs"), unstaged());

        let status = cache
            .update_paths_canonical(&repo, &[scanned_dir], &scan, false)
            .expect("entry with complete file details must return Some");

        assert_eq!(
            status.unstaged, 2,
            "src/gone.rs is replaced by src/new.rs; untouched.rs is outside the scan"
        );
    }

    /// The watcher's on-disk spelling must clear the cache's index spelling.
    #[test]
    fn ignore_case_update_clears_a_cached_entry_spelled_in_another_case() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/icase-file");
        let mut files = HashMap::new();
        files.insert(PathBuf::from("Readme.md"), unstaged());
        files.insert(PathBuf::from("Src/lib.rs"), unstaged());
        cache.set(&repo, dummy_status(), files);

        let status = cache
            .update_paths_canonical(
                &repo,
                &[PathBuf::from("README.md"), PathBuf::from("SRC")],
                &HashMap::new(),
                true,
            )
            .expect("entry with complete file details must return Some");

        assert_eq!(
            status.unstaged, 0,
            "a file and a directory spelled in another case must still be cleared"
        );
    }

    #[test]
    fn case_sensitive_update_keeps_a_differently_cased_entry() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/case-sensitive");
        let mut files = HashMap::new();
        files.insert(PathBuf::from("Readme.md"), unstaged());
        cache.set(&repo, dummy_status(), files);

        let status = cache
            .update_paths_canonical(&repo, &[PathBuf::from("README.md")], &HashMap::new(), false)
            .expect("entry with complete file details must return Some");

        assert_eq!(status.unstaged, 1, "README.md is not Readme.md here");
    }

    /// Refusing an update beneath a cached collapsed untracked directory must
    /// also see through a case difference.
    #[test]
    fn ignore_case_update_refuses_collapsed_ancestor() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/icase-collapsed");
        let mut files = HashMap::new();
        files.insert(
            PathBuf::from("Newdir"),
            FileStatus {
                untracked: true,
                ..FileStatus::default()
            },
        );
        cache.set(&repo, dummy_status(), files);

        assert!(
            cache
                .update_paths_canonical(
                    &repo,
                    &[PathBuf::from("NEWDIR/x.txt")],
                    &HashMap::new(),
                    true,
                )
                .is_none(),
            "the collapsed directory must force a full scan"
        );
    }

    #[test]
    fn multi_path_update_falls_back_when_details_dropped() {
        use crate::cache::bounded::GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES;

        let cache = GitStatusCache::new();
        let files_per_repo = GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES / 80 + 1;
        let repo_a = PathBuf::from("/repo/budget-a");
        let repo_b = PathBuf::from("/repo/budget-b");
        cache.set(&repo_a, dummy_status(), dirty_files(10));
        cache.set(&repo_b, dummy_status(), dirty_files(files_per_repo));

        assert!(
            cache
                .update_paths_canonical(
                    &repo_a,
                    &[PathBuf::from("src/main.rs")],
                    &HashMap::new(),
                    false,
                )
                .is_none(),
            "a details-dropped entry must force a full scan"
        );
    }

    fn unstaged() -> FileStatus {
        FileStatus {
            unstaged: true,
            ..FileStatus::default()
        }
    }

    fn staged() -> FileStatus {
        FileStatus {
            staged: true,
            ..FileStatus::default()
        }
    }
}
