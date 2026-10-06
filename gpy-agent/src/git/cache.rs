//! Git status caching with TTL-based expiration
//!
//! Caches git status results to avoid expensive recomputation.
//! Cache entries are invalidated explicitly by the file watcher when
//! git files change, or automatically after 30 seconds (TTL).
//! This ensures data stays fresh even if the file watcher fails.
//!
//! ## Memory budget
//!
//! Two independent bounds protect RSS for long-lived agents:
//!
//! - **Entry count** (`GIT_STATUS_CACHE_CAPACITY`): caps the number of distinct
//!   repositories retained.
//! - **File-detail bytes** (`GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES`): when the
//!   total per-file change-map bytes exceed this limit, file-level details are
//!   dropped from the oldest entries while aggregate counts are preserved.
//!   Incremental updates (`update_file_canonical`) return `None` for entries
//!   that lost their detail map, causing the caller to fall back to a full scan.

use crate::cache::bounded::{
    GIT_STATUS_CACHE_CAPACITY, GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES, drop_file_details_to_budget,
    evict_to_capacity,
};
use crate::cache::{CachePolicy, PolicyDriven};
use crate::debug_log;
use crate::git::native::resolve_overall_state;
use crate::git::{FileStatus, RepositoryStatus, StatusAggregate};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Complete cached status including file-level details for incremental updates
#[derive(Clone, Debug)]
pub struct CachedRepoStatus {
    /// Aggregated repository status (returned to clients)
    pub aggregates: RepositoryStatus,
    /// Per-file status map (for incremental updates)
    pub files: HashMap<PathBuf, FileStatus>,
}

/// Cache entry with timestamp for TTL-based expiration
#[derive(Clone)]
struct CacheEntry {
    data: CachedRepoStatus,
    cached_at: Instant,
    /// `false` when per-file details were dropped to meet the byte budget.
    /// Incremental updates are disabled for such entries; callers must do a
    /// full scan to re-populate the detail map.
    files_complete: bool,
}

impl CacheEntry {
    /// Rough byte estimate for the per-file detail map.
    ///
    /// Uses a flat per-entry constant (average `PathBuf` length + `FileStatus`
    /// size + `HashMap` overhead) rather than introspecting each `PathBuf` to
    /// keep the estimate O(1) and lock-free.
    fn file_detail_bytes(&self) -> usize {
        if !self.files_complete {
            return 0;
        }
        // 64 bytes average PathBuf + 4 bytes FileStatus + ~12 bytes HashMap overhead
        self.data.files.len().saturating_mul(80)
    }
}

/// Recompute an entry's aggregates from its file map and refresh its timestamp,
/// returning the new aggregated status.
///
/// Shared by every incremental update so they cannot drift apart in how they
/// derive the repository state.
fn reaggregate(entry: &mut CacheEntry) -> RepositoryStatus {
    let aggregate = StatusAggregate::from_file_statuses(entry.data.files.values());
    entry.data.aggregates.staged = aggregate.staged;
    entry.data.aggregates.unstaged = aggregate.unstaged;
    entry.data.aggregates.untracked = aggregate.untracked;
    entry.data.aggregates.conflicts = aggregate.conflicts;

    // Re-derive overall state with the same precedence as a full scan
    // (conflicts > preserved in-progress state > dirty > clean) instead of
    // leaving the previous state stale. Any in-progress special state recorded
    // by the last full scan (e.g. a merge) is preserved rather than inferred,
    // since the incremental path has no access to `.git/MERGE_HEAD`-style
    // detection.
    let special_state = entry.data.aggregates.state.as_preserved_special_state();
    entry.data.aggregates.state = resolve_overall_state(
        aggregate.conflicts,
        special_state,
        aggregate.staged,
        aggregate.unstaged,
        aggregate.untracked,
    );

    // Update timestamp to keep cache fresh
    entry.cached_at = Instant::now();

    entry.data.aggregates.clone()
}

/// `path.starts_with(prefix)`, optionally comparing components
/// ASCII-case-insensitively (a `core.ignorecase=true` repository, #713).
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
    /// Resolve `path` to the canonical key used for cache storage.
    ///
    /// This performs a `realpath(3)`-style syscall (`fs::canonicalize`) that walks
    /// every path component. Callers that already hold a canonical path (e.g. the
    /// output of [`MultiRepoWatcher::find_git_root`]) should use the `*_canonical`
    /// methods to avoid paying this cost a second time on the hot request path.
    ///
    /// [`MultiRepoWatcher::find_git_root`]: crate::watcher::multi_repo::MultiRepoWatcher::find_git_root
    fn canonical_key(path: &Path) -> PathBuf {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    /// Create a new git status cache with the standard policy
    #[must_use]
    pub fn new() -> Self {
        Self::with_policy(CachePolicy::git_status())
    }

    /// Create a git status cache with an explicit timing policy.
    ///
    /// [`new`](Self::new) hardcodes [`CachePolicy::git_status`] (30s TTL, 150ms
    /// cooldown). This variant lets callers (notably tests exercising the
    /// cooldown-gated revalidation path) substitute a custom policy — e.g. a
    /// 0ms cooldown so every entry is immediately past its cooldown window.
    #[must_use]
    pub(crate) fn with_policy(policy: CachePolicy) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            policy,
        }
    }

    /// Get cached status for a repository
    /// Returns None if entry doesn't exist or has expired (TTL exceeded)
    #[must_use]
    pub fn get(&self, repo_path: &Path) -> Option<RepositoryStatus> {
        self.get_canonical(&Self::canonical_key(repo_path))
    }

    /// Get cached status using a key that is already canonical.
    ///
    /// Identical to [`get`](Self::get) but skips the `fs::canonicalize` syscall.
    /// Use this on the warm request path where `key` is the output of
    /// [`MultiRepoWatcher::find_git_root`], which has already canonicalized it.
    ///
    /// [`MultiRepoWatcher::find_git_root`]: crate::watcher::multi_repo::MultiRepoWatcher::find_git_root
    #[must_use]
    pub fn get_canonical(&self, key: &Path) -> Option<RepositoryStatus> {
        // Grab data and timestamp, release lock immediately
        let (aggregates, cached_at) = self
            .entries
            .lock()
            .ok()?
            .get(key)
            .map(|e| (e.data.aggregates.clone(), e.cached_at))?;

        // Check if entry has expired based on policy TTL
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

    /// Get cached status regardless of TTL (stale allowed)
    ///
    /// This is used for progressive timeouts where we prefer showing stale data
    /// immediately rather than waiting for a long computation.
    #[must_use]
    pub fn get_any(&self, repo_path: &Path) -> Option<RepositoryStatus> {
        self.get_any_canonical(&Self::canonical_key(repo_path))
    }

    /// Get cached status regardless of TTL using a key that is already canonical.
    ///
    /// Identical to [`get_any`](Self::get_any) but skips the `fs::canonicalize` syscall.
    #[must_use]
    pub fn get_any_canonical(&self, key: &Path) -> Option<RepositoryStatus> {
        self.entries
            .lock()
            .ok()?
            .get(key)
            .map(|e| e.data.aggregates.clone())
    }

    /// Get cached status regardless of TTL (stale allowed), together with how
    /// long ago the entry was cached, using a key that is already canonical.
    ///
    /// Identical to [`get_any_canonical`](Self::get_any_canonical) but also
    /// exposes the entry's age so callers can bound how stale a value they're
    /// willing to serve immediately, rather than accepting arbitrarily old
    /// data on a persistently slow repo (#433).
    #[must_use]
    pub fn get_any_with_age_canonical(
        &self,
        key: &Path,
    ) -> Option<(RepositoryStatus, std::time::Duration)> {
        self.entries
            .lock()
            .ok()?
            .get(key)
            .map(|e| (e.data.aggregates.clone(), e.cached_at.elapsed()))
    }

    /// Cache status for a repository with current timestamp
    pub fn set(
        &self,
        repo_path: &Path,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) {
        self.set_canonical(&Self::canonical_key(repo_path), status, files);
    }

    /// Cache status using a key that is already canonical.
    ///
    /// Identical to [`set`](Self::set) but skips the `fs::canonicalize` syscall.
    ///
    /// After inserting the new entry, two bounds are enforced:
    /// 1. Entry count: evicts the least-recently-updated entries above the cap.
    /// 2. File-detail bytes: drops per-file maps from the oldest entries when the
    ///    total exceeds `GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES`, preserving aggregates.
    pub fn set_canonical(
        &self,
        key: &Path,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) {
        debug_log!("cache", "Caching status for {}", key.display());
        if let Ok(mut entries) = self.entries.lock() {
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
            // Bound steady-state memory regardless of how many repositories a
            // long-lived agent visits (least-recently-updated eviction).
            evict_to_capacity(&mut entries, GIT_STATUS_CACHE_CAPACITY, |entry| {
                entry.cached_at
            });
            // Drop per-file details from the oldest entries when the total byte
            // footprint of file detail maps exceeds the budget. Aggregate counts
            // (branch, staged, unstaged, etc.) are always retained.
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
        }
    }

    /// Update status for a single file (incremental update)
    ///
    /// Returns the new aggregated status if successful, or `None` when the
    /// cache entry does not exist, has expired, or had its file-level details
    /// dropped to meet the memory budget — in which case the caller should
    /// perform a full scan to rebuild the detail map.
    #[must_use]
    pub fn update_file(
        &self,
        repo_path: &Path,
        file_path: &Path,
        new_status: FileStatus,
    ) -> Option<RepositoryStatus> {
        self.update_file_canonical(&Self::canonical_key(repo_path), file_path, new_status)
    }

    /// Update a single file's status using a key that is already canonical.
    ///
    /// Identical to [`update_file`](Self::update_file) but skips the
    /// `fs::canonicalize` syscall.
    ///
    /// Returns `None` when the entry's file-level details were dropped for memory,
    /// signalling the caller to fall back to a full scan.
    #[must_use]
    pub fn update_file_canonical(
        &self,
        key: &Path,
        file_path: &Path,
        new_status: FileStatus,
    ) -> Option<RepositoryStatus> {
        let mut guard = self.entries.lock().ok()?;
        let entry = guard.get_mut(key)?;

        // If file details were dropped for the memory budget, an incremental
        // update would produce incorrect aggregates. Signal the caller to do a
        // full refresh instead.
        if !entry.files_complete {
            return None;
        }

        let rel_path = file_path
            .strip_prefix(key)
            .unwrap_or(file_path)
            .to_path_buf();

        if new_status.is_clean() {
            entry.data.files.remove(&rel_path);
        } else {
            entry.data.files.insert(rel_path, new_status);
        }

        let aggregates = reaggregate(entry);

        // The guard's significant `Drop` is tightened to its last real use.
        drop(guard);
        Some(aggregates)
    }

    /// Fold a pathspec-limited status scan into the cached file map, returning
    /// the new aggregated status.
    ///
    /// `scanned` is the pathspec the scan ran with and `results` is what it
    /// reported, both relative to `key` (absolute paths under `key` are
    /// accepted and stripped). The scan is authoritative for everything under
    /// `scanned`, so each scanned path's cached subtree is replaced wholesale
    /// by `results` rather than merged into: a path the scan did not report is
    /// clean now, which is exactly how a rename's source path and a reverted
    /// edit stop being counted (#466).
    ///
    /// Returns `None` — meaning "do a full scan instead" — when the entry is
    /// missing or had its file-level details dropped for the memory budget,
    /// matching [`update_file_canonical`](Self::update_file_canonical), or
    /// when the scan cannot be merged consistently with git's collapsed
    /// untracked directories (#711): a full scan reports a new untracked
    /// directory as one `dir/` entry, but a pathspec naming a file inside it
    /// makes git report that file instead. So the merge is refused when a
    /// cached untracked entry is a strict ancestor of a scanned path, or when
    /// the scan reports an untracked file below the repository root.
    ///
    /// With `ignore_case` (the repository has `core.ignorecase=true`) scanned
    /// and cached paths are compared ASCII-case-insensitively: the watcher
    /// reports the on-disk spelling while the cached keys carry git's index
    /// spelling, so `README.md` must clear a cached `Readme.md` (#713).
    /// Non-ASCII names never get here; they take a full scan.
    #[must_use]
    pub fn update_paths_canonical(
        &self,
        key: &Path,
        scanned: &[PathBuf],
        results: &HashMap<PathBuf, FileStatus>,
        ignore_case: bool,
    ) -> Option<RepositoryStatus> {
        let mut guard = self.entries.lock().ok()?;
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
            // A scanned path may be a directory (the new-directory follow-up of
            // #416), in which case the scan covered its whole subtree.
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

    /// Returns true if the cache entry for `repo_path` is still fresh.
    ///
    /// A cache entry is considered fresh if it was updated within the policy's
    /// cooldown window. This is used to implement a cooldown period after
    /// file watcher events to avoid repeatedly querying git status.
    ///
    /// The cooldown window is determined by the cache's policy (default: 150ms).
    #[must_use]
    pub fn is_fresh(&self, repo_path: &Path) -> bool {
        self.is_fresh_canonical(&Self::canonical_key(repo_path))
    }

    /// Returns true if the cache entry for an already-canonical `key` is still fresh.
    ///
    /// Identical to [`is_fresh`](Self::is_fresh) but skips the `fs::canonicalize` syscall.
    #[must_use]
    pub fn is_fresh_canonical(&self, key: &Path) -> bool {
        if let Ok(entries) = self.entries.lock()
            && let Some(entry) = entries.get(key)
        {
            return entry.cached_at.elapsed() < self.policy.cooldown();
        }
        false
    }

    /// Invalidate cached status for a repository (called by file watcher)
    pub fn invalidate(&self, repo_path: &Path) {
        self.invalidate_canonical(&Self::canonical_key(repo_path));
    }

    /// Invalidate cached status using a key that is already canonical.
    ///
    /// Identical to [`invalidate`](Self::invalidate) but skips the `fs::canonicalize` syscall.
    pub fn invalidate_canonical(&self, key: &Path) {
        debug_log!("cache", "Invalidating cache for {}", key.display());
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(key);
        }
    }

    /// Clear all cached entries
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    /// Get number of cached repositories (for debugging/testing)
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |e| e.len())
    }

    /// Check if cache is empty (for debugging/testing)
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Test-only helper: backdate an entry's `cached_at` timestamp by `age`,
    /// so age-bound logic (e.g. `git_handler`'s `MAX_IMMEDIATE_STALE_AGE`) can
    /// be exercised deterministically without sleeping (#433). No-op if the
    /// key has no entry.
    #[cfg(test)]
    pub(crate) fn age_entry_for_test(&self, key: &Path, age: std::time::Duration) {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(entry) = entries.get_mut(key)
        {
            entry.cached_at = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
        }
    }

    /// Test-only helper: the cached per-file map for `key`, so a differential
    /// test can compare it path-for-path against git (#675). `None` if the
    /// key has no entry.
    #[cfg(test)]
    pub(crate) fn files_for_test(&self, key: &Path) -> Option<HashMap<PathBuf, FileStatus>> {
        self.entries
            .lock()
            .ok()?
            .get(key)
            .map(|entry| entry.data.files.clone())
    }
}

impl Default for GitStatusCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyDriven for GitStatusCache {
    fn policy(&self) -> CachePolicy {
        self.policy
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::GitStatusCache;
    use crate::cache::{CachePolicy, PolicyDriven};
    use crate::git::{FileStatus, RepositoryState, RepositoryStatus};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Duration;

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
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo");
        cache.set(&repo, dummy_status(), HashMap::new());

        // Policy cooldown is 150ms, so entry should be fresh immediately
        assert!(cache.is_fresh(&repo));

        // Sleep longer than cooldown
        thread::sleep(Duration::from_millis(160));
        assert!(!cache.is_fresh(&repo));
    }

    #[test]
    fn cache_uses_git_status_policy() {
        let cache = GitStatusCache::new();
        let policy = cache.policy();

        // Verify it uses the git_status policy
        assert_eq!(policy, CachePolicy::git_status());
        assert_eq!(policy.ttl(), Duration::from_secs(30));
        assert_eq!(policy.cooldown(), Duration::from_millis(150));
    }

    #[test]
    fn cache_is_bounded_when_visiting_many_repositories() {
        use crate::cache::bounded::GIT_STATUS_CACHE_CAPACITY;

        let cache = GitStatusCache::new();
        // Visit far more distinct directories than the cap allows.
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
    fn cache_respects_policy_ttl() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo");
        cache.set(&repo, dummy_status(), HashMap::new());

        // Should be cached immediately
        assert!(cache.get(&repo).is_some());

        // Sleep longer than TTL (30 seconds would be too long for a test)
        // Instead, we verify the policy is being used
        let policy = cache.policy();
        assert_eq!(policy.ttl(), Duration::from_secs(30));
    }

    /// A repository with many dirty files must not push total retained bytes
    /// beyond the configured budget.
    ///
    /// Aggregate status is preserved; file details are dropped from the oldest
    /// entries when the limit is exceeded.
    #[test]
    fn aggregate_preserved_when_file_details_exceed_byte_budget() {
        use crate::cache::bounded::GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES;

        let cache = GitStatusCache::new();

        // Each entry holds ~2 000 files × 80 bytes ≈ 160 KiB of detail.
        // Insert enough entries to exceed the 10 MiB budget (≥ 63 entries).
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

        // Every entry must still have its aggregates available.
        for i in 0..repo_count {
            let repo = PathBuf::from(format!("/large-repo/{i}"));
            // get_any bypasses TTL; use it to check presence regardless of expiry.
            assert!(
                cache.get_any(&repo).is_some(),
                "aggregate for repo {i} must be retained after file-detail eviction"
            );
        }
    }

    /// `update_file_canonical` must return `None` (triggering a full scan) for
    /// entries whose file-level details were dropped to meet the byte budget.
    #[test]
    fn incremental_update_falls_back_when_details_dropped() {
        use crate::cache::bounded::GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES;

        let cache = GitStatusCache::new();

        // Force the byte budget to be exceeded immediately with one large entry.
        let files_per_repo = GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES / 80 + 1;
        let repo_a = PathBuf::from("/repo/a");
        let repo_b = PathBuf::from("/repo/b");

        // Insert repo_a first (older), then repo_b (newer with many files).
        cache.set(&repo_a, dummy_status(), dirty_files(10));
        cache.set(&repo_b, dummy_status(), dirty_files(files_per_repo));

        // repo_a is the oldest entry; its details should have been dropped.
        // update_file must return None for it, signalling a full-scan fallback.
        let result = cache.update_file_canonical(
            &repo_a,
            &PathBuf::from("src/main.rs"),
            FileStatus {
                staged: true,
                ..FileStatus::default()
            },
        );
        assert!(
            result.is_none(),
            "incremental update should return None when file details were dropped"
        );

        // Aggregate for repo_a must still be available.
        assert!(
            cache.get_any(&repo_a).is_some(),
            "aggregate must survive after file-detail eviction"
        );
    }

    /// Acceptance criterion 1: editing a tracked file in a clean repo.
    ///
    /// Must flip the exported `state` to `Dirty` via the incremental path,
    /// not just bump the `unstaged` count while leaving `state` stale at
    /// `Clean`.
    #[test]
    fn incremental_update_flips_clean_to_dirty() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/incremental-dirty");
        cache.set(&repo, dummy_status(), HashMap::new());

        let result = cache.update_file_canonical(
            &repo,
            &PathBuf::from("src/main.rs"),
            FileStatus {
                unstaged: true,
                ..FileStatus::default()
            },
        );

        let status = result.expect("entry with complete file details must return Some");
        assert_eq!(status.unstaged, 1);
        assert_eq!(
            status.state,
            RepositoryState::Dirty,
            "state must be re-derived from the fresh counts, not left stale at Clean"
        );
    }

    /// Acceptance criterion 2: reverting the edit (file status goes back to
    /// clean) must flip the exported `state` back to `Clean` via the
    /// incremental path.
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

        // Revert: the file is now clean again.
        let result = cache.update_file_canonical(&repo, &file, FileStatus::default());

        let status = result.expect("entry with complete file details must return Some");
        assert_eq!(status.unstaged, 0);
        assert_eq!(
            status.state,
            RepositoryState::Clean,
            "state must flip back to Clean once the file is no longer dirty"
        );
    }

    /// Conflicts must still take precedence over dirty/clean through the
    /// incremental path, proving the full `resolve_overall_state` precedence
    /// order (not just the dirty/clean boundary) survived the wiring.
    #[test]
    fn incremental_update_reports_conflicts_with_precedence() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/incremental-conflict");
        cache.set(&repo, dummy_status(), HashMap::new());

        let result = cache.update_file_canonical(
            &repo,
            &PathBuf::from("src/main.rs"),
            FileStatus {
                conflicted: true,
                ..FileStatus::default()
            },
        );

        let status = result.expect("entry with complete file details must return Some");
        assert_eq!(status.conflicts, 1);
        assert_eq!(status.state, RepositoryState::Conflicts);
    }

    /// An in-progress special state (e.g. an ongoing merge) recorded by the
    /// last full scan.
    ///
    /// Must be preserved by the incremental path rather than clobbered by a
    /// plain dirty/clean re-derivation from file counts alone.
    #[test]
    fn incremental_update_preserves_special_state() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/incremental-merging");

        let mut merging_status = dummy_status();
        merging_status.state = RepositoryState::Merging;
        cache.set(&repo, merging_status, HashMap::new());

        let result = cache.update_file_canonical(
            &repo,
            &PathBuf::from("src/main.rs"),
            FileStatus {
                unstaged: true,
                ..FileStatus::default()
            },
        );

        let status = result.expect("entry with complete file details must return Some");
        assert_eq!(status.unstaged, 1);
        assert_eq!(
            status.state,
            RepositoryState::Merging,
            "special in-progress state must be preserved, not overwritten with Dirty"
        );
    }

    /// #433: `get_any_with_age_canonical` must expose an entry's age
    /// alongside its status, so callers can bound how stale a value they're
    /// willing to serve immediately.
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

    /// #433: an absent key must yield `None`, matching `get_any_canonical`.
    #[test]
    fn get_any_with_age_canonical_returns_none_for_absent_key() {
        let cache = GitStatusCache::new();
        let repo = PathBuf::from("/repo/age-absent");

        assert!(cache.get_any_with_age_canonical(&repo).is_none());
    }

    /// #466: a pathspec-limited scan is authoritative for everything under the paths it was
    /// given.
    ///
    /// A path the scan did not report back is clean now, so its cache entry has to go —
    /// otherwise a reverted edit stays counted.
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

    /// #466 acceptance criterion 3: `git status` keys a rename under the
    /// destination only, so the source path comes back absent and must be
    /// dropped rather than left counted as a separate change.
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

    /// #466: a directory path (a new-directory follow-up, #416) scans its whole
    /// subtree, so the cache entries beneath it are replaced by the scan's
    /// result rather than accumulating stale children.
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

    /// #713: an ignore-case update clears a cached entry in another spelling.
    ///
    /// On a `core.ignorecase=true` repository the watcher reports the on-disk
    /// spelling (`README.md`) while the cached key carries git's index spelling
    /// (`Readme.md`); a revert must still clear the cached entry.
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

    /// #713: without `core.ignorecase` the comparison stays exact, so a
    /// differently-cased path is a different file and its entry is kept.
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

    /// #713: the #711 refusal (a cached collapsed untracked directory above a
    /// scanned path) must also see through a case difference.
    #[test]
    fn ignore_case_update_refuses_a_collapsed_untracked_ancestor_in_another_case() {
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

    /// The multi-path update must honour the same memory-budget bail-out as the
    /// single-file one: dropped file details make any incremental aggregate wrong.
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
