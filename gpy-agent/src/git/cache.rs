//! Git status cache, invalidated by the file watcher with a TTL backstop.
//!
//! Bounded by entry count and by total per-file detail bytes; entries over the
//! byte budget keep aggregates but lose file detail, forcing a full rescan on
//! the next incremental update.

use crate::cache::CachePolicy;
use crate::cache::bounded::{
    FILE_DETAIL_BYTES_PER_FILE, GIT_STATUS_CACHE_CAPACITY, GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES,
    drop_file_details_to_budget, evict_to_capacity,
};
use crate::debug_log;
use crate::git::native::resolve_overall_state;
use crate::git::{FileStatus, RepositoryStatus, StatusAggregate};
use crate::sync_ext::MutexExt;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
    /// When the head fields (branch, upstream counts, stash, in-progress
    /// state) were read by the full scan that set them: the time that scan
    /// started. The TTL counts from here.
    full_scan_at: Instant,
    /// Last write of any kind; the cooldown and eviction recency count from here.
    updated_at: Instant,
    /// Last incremental (pathspec) update since the entry's full scan, whose
    /// file map a full scan started earlier must not overwrite.
    incremental_at: Option<Instant>,
    /// `false` when per-file details were dropped for the byte budget;
    /// incremental updates then require a full scan.
    files_complete: bool,
    /// The full scan started while nothing reported changes to the
    /// repository; see [`GitStatusCache::get_canonical_unwatched`]. A scan
    /// taken under a watch is never trusted this way once the watch is gone.
    scanned_unwatched: bool,
}

impl CacheEntry {
    /// Flat per-file byte estimate, O(1) without inspecting paths.
    fn file_detail_bytes(&self) -> usize {
        if !self.files_complete {
            return 0;
        }
        self.data
            .files
            .len()
            .saturating_mul(FILE_DETAIL_BYTES_PER_FILE)
    }
}

/// Recompute aggregates from the file map and stamp the entry as updated.
/// `full_scan_at` is untouched, so incremental updates never age the TTL.
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

    entry.updated_at = Instant::now();

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
    /// When each key last changed in a way a full scan must see: an
    /// invalidation, or a watcher event that needs a full scan. A full scan
    /// that started before it is discarded instead of storing the old view,
    /// and an entry scanned before it is never fresh. Locked after
    /// `entries`, never before.
    changed: Arc<Mutex<HashMap<PathBuf, Instant>>>,
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
            changed: Arc::new(Mutex::new(HashMap::new())),
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
        self.lookup(key, None)
    }

    /// [`get_canonical`](Self::get_canonical) that also misses when the
    /// entry's full scan started before `since`, the start of its
    /// repository's watch: changes made before then were never reported.
    #[must_use]
    pub fn get_canonical_scanned_since(
        &self,
        key: &Path,
        since: Instant,
    ) -> Option<RepositoryStatus> {
        self.lookup(key, Some(since))
    }

    /// A within-TTL status whose full scan started no earlier than `since`.
    fn lookup(&self, key: &Path, since: Option<Instant>) -> Option<RepositoryStatus> {
        let (aggregates, full_scan_at) = self
            .entries
            .lock_or_recover()
            .get(key)
            .map(|e| (e.data.aggregates.clone(), e.full_scan_at))?;

        if full_scan_at.elapsed() >= self.policy.ttl() {
            debug_log!(
                "cache",
                "Cache EXPIRED for {} (age: {:?}, TTL: {:?})",
                key.display(),
                full_scan_at.elapsed(),
                self.policy.ttl()
            );
            return None;
        }
        if since.is_some_and(|watch_start| full_scan_at < watch_start) {
            debug_log!(
                "cache",
                "Cache UNTRUSTED for {}: scanned before its watch went live",
                key.display()
            );
            return None;
        }

        debug_log!(
            "cache",
            "Cache HIT for {} (age: {:?})",
            key.display(),
            full_scan_at.elapsed()
        );
        Some(aggregates)
    }

    /// A status of a repository nothing watches, trusted for at most
    /// `max_age` (and the policy's TTL) without a rescan: only an entry whose
    /// full scan itself started while the repository was unwatched (see
    /// [`Self::apply_unwatched_full_scan`]) and with no change noted since
    /// (see [`Self::note_change_canonical`]). A scan taken under a watch that
    /// has since ended never qualifies: changes after the teardown went
    /// unreported.
    #[must_use]
    pub fn get_canonical_unwatched(
        &self,
        key: &Path,
        max_age: std::time::Duration,
    ) -> Option<RepositoryStatus> {
        let (aggregates, full_scan_at) = self
            .entries
            .lock_or_recover()
            .get(key)
            .filter(|entry| entry.scanned_unwatched)
            .map(|e| (e.data.aggregates.clone(), e.full_scan_at))?;
        let age = full_scan_at.elapsed();
        if age >= max_age.min(self.policy.ttl()) || self.changed_since_canonical(key, full_scan_at)
        {
            return None;
        }
        debug_log!(
            "cache",
            "Cache HIT for unwatched {} (age: {age:?})",
            key.display()
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
            .lock_or_recover()
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
            .lock_or_recover()
            .get(key)
            .map(|e| (e.data.aggregates.clone(), e.full_scan_at.elapsed()))
    }

    /// Cache a full-scan status under an already-canonical key; enforces the
    /// entry-count and file-detail byte bounds.
    pub fn set_canonical(
        &self,
        key: &Path,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) {
        let mut entries = self.entries.lock_or_recover();
        Self::insert_entry(&mut entries, key, status, files, Instant::now(), false);
    }

    /// Publish a full scan that began at `started`. Entries are ordered by
    /// when their scan started, since that is when its view was taken.
    ///
    /// - A key changed (see [`Self::note_change_canonical`]) after `started`
    ///   is left as it is and `None` is returned: the scan predates the
    ///   change, so its view is already out of date.
    /// - An entry holding a full scan that started after `started` is kept
    ///   and `None` returned.
    /// - An entry that only took incremental updates since `started` keeps its
    ///   newer file map; the scan's branch, upstream counts, stash count and
    ///   in-progress state replace the entry's, the counts are recomputed from
    ///   the map, and the merged status is returned. The scan's own file map
    ///   is discarded; the next full scan picks up anything only it saw.
    /// - Otherwise, including an entry whose file details were dropped for the
    ///   byte budget, the entry is replaced by the scan.
    #[must_use]
    pub fn apply_full_scan(
        &self,
        key: &Path,
        started: Instant,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) -> Option<RepositoryStatus> {
        self.apply_scan(key, started, false, status, files)
    }

    /// [`Self::apply_full_scan`] for a scan that started while nothing
    /// reported changes to the repository (no live watch, live updates off,
    /// or no watcher), which [`Self::get_canonical_unwatched`] may then trust.
    #[must_use]
    pub fn apply_unwatched_full_scan(
        &self,
        key: &Path,
        started: Instant,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) -> Option<RepositoryStatus> {
        self.apply_scan(key, started, true, status, files)
    }

    fn apply_scan(
        &self,
        key: &Path,
        started: Instant,
        unwatched: bool,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
    ) -> Option<RepositoryStatus> {
        let mut entries = self.entries.lock_or_recover();
        if self.changed_since_canonical(key, started) {
            debug_log!(
                "cache",
                "Discarded full scan started before a change to {}",
                key.display()
            );
            return None;
        }
        if let Some(entry) = entries.get_mut(key) {
            if entry.full_scan_at > started {
                debug_log!("cache", "Kept newer full scan for {}", key.display());
                return None;
            }
            if entry.incremental_at.is_some_and(|at| at > started) && entry.files_complete {
                debug_log!(
                    "cache",
                    "Merged full scan into incremental entry for {}",
                    key.display()
                );
                entry.data.aggregates = status;
                let merged = reaggregate(entry);
                entry.full_scan_at = started;
                entry.scanned_unwatched = unwatched;
                return Some(merged);
            }
        }
        Some(Self::insert_entry(
            &mut entries,
            key,
            status,
            files,
            started,
            unwatched,
        ))
    }

    /// Insert a full-scan entry whose head fields were read at `full_scan_at`,
    /// then enforce the entry-count and file-detail byte bounds. Returns the
    /// stored aggregates.
    fn insert_entry(
        entries: &mut HashMap<PathBuf, CacheEntry>,
        key: &Path,
        status: RepositoryStatus,
        files: HashMap<PathBuf, FileStatus>,
        full_scan_at: Instant,
        scanned_unwatched: bool,
    ) -> RepositoryStatus {
        debug_log!("cache", "Caching status for {}", key.display());
        entries.insert(
            key.to_path_buf(),
            CacheEntry {
                data: CachedRepoStatus {
                    aggregates: status.clone(),
                    files,
                },
                full_scan_at,
                updated_at: Instant::now(),
                incremental_at: None,
                files_complete: true,
                scanned_unwatched,
            },
        );
        evict_to_capacity(entries, GIT_STATUS_CACHE_CAPACITY, |entry| entry.updated_at);
        drop_file_details_to_budget(
            entries,
            GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES,
            |entry| entry.updated_at,
            CacheEntry::file_detail_bytes,
            |entry| {
                entry.data.files.clear();
                entry.data.files.shrink_to_fit();
                entry.files_complete = false;
            },
        );
        status
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
        let mut guard = self.entries.lock_or_recover();
        let entry = guard.get_mut(key)?;

        // A stale entry's head fields are unknown; only a full scan clears it.
        if !entry.files_complete || entry.data.aggregates.stale_error {
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
            status.untracked && path.parent().is_some_and(|parent| !parent.is_empty())
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
        entry.incremental_at = Some(entry.updated_at);

        drop(guard);
        Some(aggregates)
    }

    /// Record that the latest status query for `key` failed, so the status
    /// the cache serves and publishes renders its upstream counts as unknown.
    ///
    /// An existing entry keeps its data and timestamps, only gaining the
    /// `stale_error` flag; the next full scan replaces it. With no entry,
    /// `last_good` (the status read before the failed query invalidated it)
    /// is reinstated without file details, so the next incremental update
    /// falls back to a full scan. Returns the stale status, or `None` when
    /// there is neither an entry nor a `last_good`.
    #[must_use]
    pub fn mark_stale_error(
        &self,
        key: &Path,
        last_good: Option<RepositoryStatus>,
    ) -> Option<RepositoryStatus> {
        let mut entries = self.entries.lock_or_recover();
        if let Some(entry) = entries.get_mut(key) {
            entry.data.aggregates.stale_error = true;
            return Some(entry.data.aggregates.clone());
        }
        let mut status = last_good?;
        status.stale_error = true;
        let stale = Self::insert_entry(
            &mut entries,
            key,
            status,
            HashMap::new(),
            Instant::now(),
            false,
        );
        if let Some(entry) = entries.get_mut(key) {
            entry.files_complete = false;
        }
        Some(stale)
    }

    /// Whether the entry for an already-canonical key was updated within the
    /// policy's cooldown window and its full scan started after the key's
    /// last noted change.
    #[must_use]
    pub fn is_fresh_canonical(&self, key: &Path) -> bool {
        let entries = self.entries.lock_or_recover();
        let Some(entry) = entries.get(key) else {
            return false;
        };
        entry.updated_at.elapsed() < self.policy.cooldown()
            && !self.changed_since_canonical(key, entry.full_scan_at)
    }

    /// Record that `key` changed in a way only a full scan sees (HEAD, refs,
    /// index), so full scans that started before now are discarded and the
    /// cached entry stops counting as fresh until a newer scan lands.
    pub fn note_change_canonical(&self, key: &Path) {
        let mut changed = self.changed.lock_or_recover();
        changed.insert(key.to_path_buf(), Instant::now());
        evict_to_capacity(&mut changed, GIT_STATUS_CACHE_CAPACITY, |at| *at);
    }

    /// Whether `key` changed (see [`Self::note_change_canonical`]) after `at`.
    #[must_use]
    pub fn changed_since_canonical(&self, key: &Path, at: Instant) -> bool {
        self.changed
            .lock_or_recover()
            .get(key)
            .is_some_and(|changed_at| *changed_at > at)
    }

    /// Drop the cached status for an already-canonical key and note the
    /// change, so a full scan that started before this cannot reinstate it.
    pub fn invalidate_canonical(&self, key: &Path) {
        debug_log!("cache", "Invalidating cache for {}", key.display());
        let mut entries = self.entries.lock_or_recover();
        entries.remove(key);
        self.note_change_canonical(key);
        drop(entries);
    }

    /// Number of cached repositories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock_or_recover().len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for GitStatusCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::GitStatusCache;
    use crate::cache::CachePolicy;
    use crate::git::{FileStatus, RepositoryState, RepositoryStatus};
    use crate::sync_ext::MutexExt;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    impl GitStatusCache {
        /// Backdate an entry's timestamps: `full_scan_age` for the last full
        /// scan, `update_age` for the last write of any kind.
        pub(crate) fn age_entry_for_test(
            &self,
            key: &Path,
            full_scan_age: Duration,
            update_age: Duration,
        ) {
            if let Some(entry) = self.entries.lock_or_recover().get_mut(key) {
                let now = Instant::now();
                entry.full_scan_at = now.checked_sub(full_scan_age).unwrap_or(now);
                entry.updated_at = now.checked_sub(update_age).unwrap_or(now);
            }
        }

        /// The cached per-file map for `key`.
        pub(crate) fn files_for_test(&self, key: &Path) -> Option<HashMap<PathBuf, FileStatus>> {
            self.entries
                .lock_or_recover()
                .get(key)
                .map(|entry| entry.data.files.clone())
        }

        /// [`GitStatusCache::set_canonical`] after canonicalizing `repo_path`.
        pub(crate) fn set(
            &self,
            repo_path: &Path,
            status: RepositoryStatus,
            files: HashMap<PathBuf, FileStatus>,
        ) {
            self.set_canonical(&Self::canonical_key(repo_path), status, files);
        }

        /// [`GitStatusCache::is_fresh_canonical`] after canonicalizing `repo_path`.
        pub(crate) fn is_fresh(&self, repo_path: &Path) -> bool {
            self.is_fresh_canonical(&Self::canonical_key(repo_path))
        }

        /// [`GitStatusCache::invalidate_canonical`] after canonicalizing `repo_path`.
        pub(crate) fn invalidate(&self, repo_path: &Path) {
            self.invalidate_canonical(&Self::canonical_key(repo_path));
        }
    }

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
            stale_error: false,
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
            cooldown: Duration::from_hours(1),
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
    fn apply_full_scan_keeps_the_newer_full_scan() {
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

        assert!(
            cache
                .apply_full_scan(&key, started, stale, HashMap::new())
                .is_none()
        );
        assert_eq!(cache.get_any_canonical(&key), Some(newer));
    }

    #[test]
    fn apply_full_scan_writes_when_older() {
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
        cache.age_entry_for_test(&key, Duration::from_secs(5), Duration::from_secs(5));
        let started = Instant::now();

        assert!(
            cache
                .apply_full_scan(&key, started, newer.clone(), HashMap::new())
                .is_some()
        );
        assert_eq!(cache.get_any_canonical(&key), Some(newer));
    }

    /// A scan that started before an invalidation must not reinstate the
    /// status the invalidation dropped.
    #[test]
    fn apply_full_scan_discards_a_scan_started_before_invalidation() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/invalidated-mid-scan");
        let started = Instant::now();
        // Coarse clocks can tie; the discard needs the change after `started`.
        std::thread::sleep(Duration::from_millis(2));
        cache.invalidate_canonical(&key);

        assert!(
            cache
                .apply_full_scan(&key, started, dummy_status(), HashMap::new())
                .is_none()
        );
        assert_eq!(cache.get_any_canonical(&key), None);

        // A scan started after the invalidation is stored.
        let rescan = Instant::now();
        assert!(
            cache
                .apply_full_scan(&key, rescan, dummy_status(), HashMap::new())
                .is_some()
        );
        assert_eq!(cache.get_any_canonical(&key), Some(dummy_status()));
    }

    /// A noted change (a watcher event needing a full scan) discards scans
    /// that started before it and stops the entry counting as fresh, so a
    /// cooldown cannot skip the rescan the change needs.
    #[test]
    fn noted_change_discards_older_scans_and_ends_freshness() {
        let cache = GitStatusCache::with_policy(CachePolicy {
            ttl: Duration::from_secs(30),
            cooldown: Duration::from_hours(1),
        });
        let key = PathBuf::from("/repo/noted-change");
        let before = RepositoryStatus {
            branch: "main".to_owned(),
            ..dummy_status()
        };
        let started = Instant::now();
        assert!(
            cache
                .apply_full_scan(&key, started, before.clone(), HashMap::new())
                .is_some()
        );
        assert!(cache.is_fresh_canonical(&key));

        std::thread::sleep(Duration::from_millis(2));
        cache.note_change_canonical(&key);
        assert!(cache.changed_since_canonical(&key, started));
        assert!(!cache.is_fresh_canonical(&key));
        let in_flight = RepositoryStatus {
            branch: "in-flight".to_owned(),
            ..dummy_status()
        };
        assert!(
            cache
                .apply_full_scan(&key, started, in_flight, HashMap::new())
                .is_none()
        );
        assert_eq!(cache.get_any_canonical(&key), Some(before));

        std::thread::sleep(Duration::from_millis(2));
        let after = RepositoryStatus {
            branch: "feature".to_owned(),
            ..dummy_status()
        };
        assert!(
            cache
                .apply_full_scan(&key, Instant::now(), after.clone(), HashMap::new())
                .is_some()
        );
        assert_eq!(cache.get_any_canonical(&key), Some(after));
        assert!(cache.is_fresh_canonical(&key));
    }

    /// A status scanned before its repository's watch went live is not
    /// trusted; one scanned since is.
    #[test]
    fn scanned_since_misses_a_status_from_before_the_watch() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/scanned-since");
        let scanned = Instant::now();
        assert!(
            cache
                .apply_full_scan(&key, scanned, dummy_status(), HashMap::new())
                .is_some()
        );
        std::thread::sleep(Duration::from_millis(2));
        let watch_start = Instant::now();

        assert!(cache.get_canonical(&key).is_some());
        assert_eq!(cache.get_canonical_scanned_since(&key, watch_start), None);
        assert_eq!(
            cache.get_canonical_scanned_since(&key, scanned),
            Some(dummy_status())
        );
    }

    /// Only a scan taken while the repository was unwatched is trusted
    /// unwatched, and only within `max_age` and until a change is noted.
    #[test]
    fn unwatched_trust_needs_an_unwatched_scan_young_and_unchanged() {
        const MAX_AGE: Duration = Duration::from_secs(2);
        let cache = GitStatusCache::new();
        let watched = PathBuf::from("/repo/scanned-under-watch");
        let unwatched = PathBuf::from("/repo/scanned-unwatched");
        let started = Instant::now();
        assert!(
            cache
                .apply_full_scan(&watched, started, dummy_status(), HashMap::new())
                .is_some()
        );
        assert!(
            cache
                .apply_unwatched_full_scan(&unwatched, started, dummy_status(), HashMap::new())
                .is_some()
        );

        // A scan taken under a since-ended watch never qualifies.
        assert_eq!(cache.get_canonical_unwatched(&watched, MAX_AGE), None);
        assert_eq!(
            cache.get_canonical_unwatched(&unwatched, MAX_AGE),
            Some(dummy_status())
        );

        // Past `max_age` it rescans, though well within the TTL.
        cache.age_entry_for_test(&unwatched, MAX_AGE, Duration::ZERO);
        assert_eq!(cache.get_canonical_unwatched(&unwatched, MAX_AGE), None);
        assert!(cache.get_canonical(&unwatched).is_some());

        // A noted change ends the trust at once.
        cache.age_entry_for_test(&unwatched, Duration::ZERO, Duration::ZERO);
        assert!(cache.get_canonical_unwatched(&unwatched, MAX_AGE).is_some());
        std::thread::sleep(Duration::from_millis(2));
        cache.note_change_canonical(&unwatched);
        assert_eq!(cache.get_canonical_unwatched(&unwatched, MAX_AGE), None);

        // A later scan under a watch replaces the flag.
        assert!(
            cache
                .apply_full_scan(&unwatched, Instant::now(), dummy_status(), HashMap::new())
                .is_some()
        );
        assert_eq!(cache.get_canonical_unwatched(&unwatched, MAX_AGE), None);
    }

    /// Scans are ordered by when they started: an older scan finishing after
    /// a newer one started cannot make that newer one look stale.
    #[test]
    fn apply_full_scan_orders_scans_by_start() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/overlapping-scans");
        let old_view = RepositoryStatus {
            branch: "main".to_owned(),
            ..dummy_status()
        };
        let new_view = RepositoryStatus {
            branch: "feature".to_owned(),
            ..dummy_status()
        };
        let older_start = Instant::now()
            .checked_sub(Duration::from_secs(2))
            .expect("instant");
        let newer_start = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("instant");

        // The older scan finishes first, then the newer one.
        assert!(
            cache
                .apply_full_scan(&key, older_start, old_view, HashMap::new())
                .is_some()
        );
        assert!(
            cache
                .apply_full_scan(&key, newer_start, new_view.clone(), HashMap::new())
                .is_some()
        );
        assert_eq!(cache.get_any_canonical(&key), Some(new_view));
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
        use crate::cache::bounded::{
            FILE_DETAIL_BYTES_PER_FILE, GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES,
        };

        let cache = GitStatusCache::new();

        let files_per_repo = 2_000_usize;
        let repo_count = (GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES
            / (files_per_repo * FILE_DETAIL_BYTES_PER_FILE))
            + 10;

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
        use crate::cache::bounded::{
            FILE_DETAIL_BYTES_PER_FILE, GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES,
        };

        let cache = GitStatusCache::new();

        // One entry alone exceeds the byte budget, so the older repo_a loses details.
        let files_per_repo =
            GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES / FILE_DETAIL_BYTES_PER_FILE + 1;
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

    fn modified(path: &str) -> (PathBuf, FileStatus) {
        (
            PathBuf::from(path),
            FileStatus {
                unstaged: true,
                ..FileStatus::default()
            },
        )
    }

    fn incremental_update(cache: &GitStatusCache, key: &Path, path: &str) {
        let (file, status) = modified(path);
        assert!(
            cache
                .update_paths_canonical(
                    key,
                    std::slice::from_ref(&file),
                    &HashMap::from([(file.clone(), status)]),
                    false,
                )
                .is_some()
        );
    }

    #[test]
    fn ttl_counts_from_last_full_scan_despite_incremental_updates() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/ttl-full-scan");
        cache.set_canonical(&key, dummy_status(), HashMap::new());
        let beyond_ttl = cache.policy.ttl().saturating_add(Duration::from_secs(1));
        cache.age_entry_for_test(&key, beyond_ttl, Duration::ZERO);

        incremental_update(&cache, &key, "a.rs");

        assert_eq!(cache.get_canonical(&key), None);
        assert!(cache.get_any_canonical(&key).is_some());
    }

    #[test]
    fn cooldown_counts_from_incremental_update() {
        let cache = GitStatusCache::with_policy(CachePolicy {
            ttl: Duration::from_hours(1),
            cooldown: Duration::from_hours(1),
        });
        let key = PathBuf::from("/repo/cooldown-incremental");
        cache.set_canonical(&key, dummy_status(), HashMap::new());
        cache.age_entry_for_test(&key, Duration::from_hours(2), Duration::from_hours(2));
        assert!(!cache.is_fresh_canonical(&key));

        incremental_update(&cache, &key, "a.rs");

        assert!(cache.is_fresh_canonical(&key));
    }

    #[test]
    fn full_scan_overlapping_an_incremental_update_keeps_both() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/overlap");
        cache.set_canonical(
            &key,
            RepositoryStatus {
                branch: "a".to_owned(),
                ..dummy_status()
            },
            HashMap::new(),
        );
        let started = Instant::now();
        // Coarse clocks can tie; the merge needs an incremental update after `started`.
        std::thread::sleep(Duration::from_millis(2));
        incremental_update(&cache, &key, "x.rs");

        let scan = RepositoryStatus {
            branch: "b".to_owned(),
            ahead: 3,
            stash_count: 2,
            ..dummy_status()
        };
        let merged = cache
            .apply_full_scan(&key, started, scan, HashMap::new())
            .expect("merged status");

        assert_eq!(merged.branch, "b");
        assert_eq!((merged.ahead, merged.stash_count), (3, 2));
        assert_eq!(merged.unstaged, 1, "x.rs stays modified");
        assert_eq!(cache.get_any_canonical(&key), Some(merged));
        assert!(
            cache
                .files_for_test(&key)
                .is_some_and(|files| files.contains_key(&PathBuf::from("x.rs")))
        );
        assert!(cache.get_canonical(&key).is_some(), "still a TTL hit");
    }

    #[test]
    fn newer_full_scan_is_kept() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/newer-full-scan");
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(2));
        let newer = RepositoryStatus {
            branch: "newer".to_owned(),
            ..dummy_status()
        };
        cache.set_canonical(&key, newer.clone(), HashMap::new());

        let stale = RepositoryStatus {
            branch: "stale".to_owned(),
            ..dummy_status()
        };
        assert!(
            cache
                .apply_full_scan(&key, started, stale, HashMap::new())
                .is_none()
        );
        assert_eq!(cache.get_any_canonical(&key), Some(newer));
    }

    #[test]
    fn full_scan_replaces_an_entry_whose_file_details_were_dropped() {
        use crate::cache::bounded::{
            FILE_DETAIL_BYTES_PER_FILE, GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES,
        };

        let cache = GitStatusCache::new();
        let repo_a = PathBuf::from("/repo/dropped-a");
        let repo_b = PathBuf::from("/repo/dropped-b");
        cache.set_canonical(&repo_a, dummy_status(), dirty_files(10));
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(2));
        incremental_update(&cache, &repo_a, "x.rs");

        // One entry alone exceeds the byte budget, so the older repo_a loses details.
        let files_per_repo =
            GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES / FILE_DETAIL_BYTES_PER_FILE + 1;
        cache.set_canonical(&repo_b, dummy_status(), dirty_files(files_per_repo));
        assert_eq!(cache.files_for_test(&repo_a), Some(HashMap::new()));

        let scan = RepositoryStatus {
            unstaged: 7,
            branch: "scan".to_owned(),
            ..dummy_status()
        };
        let applied = cache
            .apply_full_scan(&repo_a, started, scan.clone(), dirty_files(7))
            .expect("scan replaces the entry");

        assert_eq!(applied, scan);
        assert_eq!(cache.get_any_canonical(&repo_a), Some(scan));
        assert_eq!(cache.files_for_test(&repo_a), Some(dirty_files(7)));
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
        use crate::cache::bounded::{
            FILE_DETAIL_BYTES_PER_FILE, GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES,
        };

        let cache = GitStatusCache::new();
        let files_per_repo =
            GIT_STATUS_CACHE_MAX_FILE_DETAIL_BYTES / FILE_DETAIL_BYTES_PER_FILE + 1;
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

    #[test]
    fn mark_stale_error_flags_the_live_entry_and_keeps_its_files() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/stale-live");
        let files = HashMap::from([(PathBuf::from("a.rs"), unstaged())]);
        cache.set_canonical(&key, dummy_status(), files.clone());

        let stale = cache.mark_stale_error(&key, None).expect("live entry");

        assert!(stale.stale_error);
        assert_eq!(cache.get_canonical(&key), Some(stale));
        assert_eq!(cache.files_for_test(&key), Some(files));
    }

    #[test]
    fn mark_stale_error_reinstates_last_good_after_invalidation() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/stale-restored");
        let last_good = RepositoryStatus {
            ahead: 2,
            ..dummy_status()
        };
        cache.set_canonical(&key, last_good.clone(), HashMap::new());
        cache.invalidate_canonical(&key);

        let stale = cache
            .mark_stale_error(&key, Some(last_good.clone()))
            .expect("reinstated");

        assert_eq!(
            stale,
            RepositoryStatus {
                stale_error: true,
                ..last_good
            }
        );
        assert_eq!(cache.get_any_canonical(&key), Some(stale));
    }

    #[test]
    fn mark_stale_error_without_any_status_invents_nothing() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/stale-none");

        assert_eq!(cache.mark_stale_error(&key, None), None);
        assert_eq!(cache.get_any_canonical(&key), None);
    }

    #[test]
    fn stale_entry_refuses_incremental_updates_and_a_full_scan_clears_it() {
        let cache = GitStatusCache::new();
        let key = PathBuf::from("/repo/stale-incremental");
        cache.set_canonical(&key, dummy_status(), HashMap::new());
        assert!(cache.mark_stale_error(&key, None).is_some());

        let results = HashMap::from([(PathBuf::from("a.rs"), unstaged())]);
        assert!(
            cache
                .update_paths_canonical(&key, &[PathBuf::from("a.rs")], &results, false)
                .is_none(),
            "a stale entry must force a full scan"
        );

        cache.set_canonical(&key, dummy_status(), HashMap::new());
        assert_eq!(cache.get_canonical(&key), Some(dummy_status()));
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
