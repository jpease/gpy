//! The force-added file set (`git add -f`): tracked files that match an
//! ignore rule, so their edits still change `git status`.

use super::super::WatchRegistry;
use super::{FileFingerprint, IgnoreCaches, Lookup, fingerprint_file};
use crate::cache::bounded::{IGNORE_CACHE_CAPACITY, evict_to_capacity};
use crate::sync_ext::MutexExt;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long a failed `git ls-files` is served (as the previous set, or empty)
/// before it is retried.
const FORCE_ADDED_RETRY: Duration = Duration::from_secs(30);

/// A cached force-added set and the index fingerprint it came from.
pub(in crate::watcher::filesystem) struct CachedForceAdded {
    files: Arc<HashSet<PathBuf>>,
    index_signature: FileFingerprint,
    /// A lookup has landed; until then `files` is the empty placeholder.
    populated: bool,
    /// When the last `git ls-files` failed; it is retried after
    /// [`FORCE_ADDED_RETRY`] even if the index is unchanged.
    failed_at: Option<Instant>,
    /// A run is in flight; prevents one `git ls-files` per event.
    refreshing: bool,
    /// Recency tick for eviction.
    last_used: u64,
}

impl CachedForceAdded {
    /// Unpopulated entry; `None` signature never matches a real fingerprint.
    fn placeholder() -> Self {
        Self {
            files: Arc::new(HashSet::new()),
            index_signature: None,
            populated: false,
            failed_at: None,
            refreshing: false,
            last_used: 0,
        }
    }

    /// Whether the entry needs a new `git ls-files` run.
    fn is_stale(&self, index_signature: FileFingerprint) -> bool {
        !self.populated
            || self.index_signature != index_signature
            || self
                .failed_at
                .is_some_and(|failed| failed.elapsed() >= FORCE_ADDED_RETRY)
    }
}

/// What [`force_added_set`] must do about the entry it just inspected.
enum ForceAddedRefresh {
    /// Entry is current, or a run is already in flight: serve what we have.
    None,
    /// Nothing computed yet and the caller may wait: run now, no lock held.
    Inline,
    /// Refresh off-thread, serving the current set (empty if none yet) meanwhile.
    Background,
}

/// Cache of force-added sets, keyed by repo root.
pub(in crate::watcher::filesystem) type ForceAddedCache = Mutex<HashMap<PathBuf, CachedForceAdded>>;

/// Force-added files (`git add -f`) for `repo_root`, cached by index
/// fingerprint.
///
/// The shared mutex is never held across `git ls-files`. A stale set is served
/// while one background thread refreshes. Only a [`Lookup::Wait`] caller runs
/// the first lookup inline; the event thread never runs git and is served the
/// empty set until the first listing lands, so ignored paths stay suppressed
/// meanwhile, as they are while a refresh is running.
pub(in crate::watcher::filesystem) fn force_added_set(
    repo_root: &Path,
    registry: &Arc<WatchRegistry>,
    lookup: Lookup,
) -> Arc<HashSet<PathBuf>> {
    let index_signature = fingerprint_file(&registry.ignore().repo_git_paths(repo_root).index);
    let tick = registry.ignore().next_tick();

    let (files, refresh) = {
        let mut cache = registry.ignore().force_added.lock_or_recover();
        let cached = cache
            .entry(repo_root.to_path_buf())
            .or_insert_with(CachedForceAdded::placeholder);
        cached.last_used = tick;
        let refresh = if cached.refreshing || !cached.is_stale(index_signature) {
            ForceAddedRefresh::None
        } else {
            cached.refreshing = true;
            if !cached.populated && lookup == Lookup::Wait {
                ForceAddedRefresh::Inline
            } else {
                ForceAddedRefresh::Background
            }
        };
        let files = Arc::clone(&cached.files);
        evict_to_capacity(&mut cache, IGNORE_CACHE_CAPACITY, |entry| entry.last_used);
        drop(cache);
        (files, refresh)
    };

    match refresh {
        ForceAddedRefresh::None => files,
        ForceAddedRefresh::Inline => refresh_force_added(repo_root, registry.ignore()),
        ForceAddedRefresh::Background => {
            let repo_root_owned = repo_root.to_path_buf();
            // Owned handle: the thread outlives this call.
            let registry_for_refresh = Arc::clone(registry);
            thread::spawn(move || {
                drop(refresh_force_added(
                    &repo_root_owned,
                    registry_for_refresh.ignore(),
                ));
            });
            files
        }
    }
}

/// Run `git ls-files` with no lock held, store the result, and clear `refreshing`.
/// Fingerprinting before the run means a racing index write leaves it stale. A
/// failed run keeps the previous set and is retried after [`FORCE_ADDED_RETRY`],
/// so a timeout is never cached as an answer.
fn refresh_force_added(repo_root: &Path, ignore: &IgnoreCaches) -> Arc<HashSet<PathBuf>> {
    let index_signature = fingerprint_file(&ignore.repo_git_paths(repo_root).index);
    let listed = (ignore.ls_files)(repo_root);
    let tick = ignore.next_tick();

    let mut cache = ignore.force_added.lock_or_recover();
    let cached = cache
        .entry(repo_root.to_path_buf())
        .or_insert_with(CachedForceAdded::placeholder);
    if let Some(files) = listed {
        cached.files = Arc::new(files.into_iter().collect());
        cached.index_signature = index_signature;
        cached.failed_at = None;
    } else {
        crate::debug_log!(
            "watcher",
            "git ls-files failed for {}; retrying in {}s",
            repo_root.display(),
            FORCE_ADDED_RETRY.as_secs()
        );
        cached.failed_at = Some(Instant::now());
    }
    cached.populated = true;
    cached.refreshing = false;
    cached.last_used = tick;
    let files = Arc::clone(&cached.files);
    evict_to_capacity(&mut cache, IGNORE_CACHE_CAPACITY, |entry| entry.last_used);
    drop(cache);
    files
}

/// Run the first force-added lookup for `repo_root` now, unless one has landed
/// or is running. Arming walks run off the event thread, so they prime the set
/// the event thread would otherwise only start in the background.
pub(in crate::watcher::filesystem) fn prime_force_added(
    repo_root: &Path,
    registry: &Arc<WatchRegistry>,
) {
    drop(force_added_set(repo_root, registry, Lookup::Wait));
}

/// Whether `path` is a force-added tracked file despite matching an ignore rule.
pub(in crate::watcher::filesystem) fn is_force_added(
    repo_root: &Path,
    path: &Path,
    registry: &Arc<WatchRegistry>,
    lookup: Lookup,
) -> bool {
    let set = force_added_set(repo_root, registry, lookup);
    if set.is_empty() {
        return false;
    }
    // Match the canonical form stored by `tracked_ignored_files`.
    let candidate = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    set.contains(&candidate)
}
