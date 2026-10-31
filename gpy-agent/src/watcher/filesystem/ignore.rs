//! Ignore matching: built-in noise tokens, per-repo gitignore caches, and the
//! force-added file set.

use super::{RepoAttribution, WatchRegistry, attribute_repo_detailed};
use crate::cache::bounded::{IGNORE_CACHE_CAPACITY, evict_to_capacity};
use crate::git::native::NativeGitBackend;
use crate::sync_ext::MutexExt;
use ::ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// Directory names that are high-churn hints: a hint that defers to gitignore
/// inside a trusted repo root. Generic words (`tmp`, `env`) are excluded since
/// they collide with ordinary paths.
const NOISE_DIR_NAMES: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    ".next",
    "target",
    "vendor",
    "__pycache__",
    "venv",
    ".venv",
];

/// OS metadata file names, suppressed unconditionally.
const NOISE_FILE_NAMES: &[&str] = &[".DS_Store", "Thumbs.db"];

/// Match `path` against the built-in high-churn names (`node_modules`, `build`, ...)
/// as path components.
///
/// With a trusted `repo_root` only the in-repo portion is tested, so a repo under
/// `/work/build/` isn't suppressed; otherwise the whole path is tested.
pub(super) fn path_matches_ignored_patterns(
    path: &Path,
    repo_root: Option<&Path>,
) -> Option<IgnoreTokenHit> {
    let scoped_path = repo_root.map_or(path, |root| path.strip_prefix(root).unwrap_or(path));
    let mut directory_hit = false;

    for component in scoped_path.components() {
        let Some(name) = component.as_os_str().to_str() else {
            continue;
        };
        if NOISE_FILE_NAMES.contains(&name) {
            return Some(IgnoreTokenHit::Noise);
        }
        if NOISE_DIR_NAMES.contains(&name) {
            directory_hit = true;
        }
    }

    directory_hit.then_some(IgnoreTokenHit::Directory)
}

/// Which kind of built-in name `path_matches_ignored_patterns` hit.
pub(super) enum IgnoreTokenHit {
    /// Directory-name hint (`vendor`, `dist`, ...): defers to gitignore.
    Directory,
    /// OS metadata file, suppressed unconditionally.
    Noise,
}

/// File mtime and length, or `None` if absent; length catches same-second edits
/// on coarse-mtime filesystems.
type FileFingerprint = Option<(std::time::SystemTime, u64)>;

/// Fingerprints of the ignore chain: root `.gitignore`, the repository's
/// `info/exclude`, then nested `.gitignore`s root->leaf.
type NestedIgnoreSignature = Vec<FileFingerprint>;

/// A compiled ignore matcher and the fingerprint it was built from.
struct CachedIgnore {
    matcher: Arc<Gitignore>,
    signature: NestedIgnoreSignature,
    /// Recency tick for eviction.
    last_used: u64,
}

/// Compiled `.gitignore` matchers keyed by directory (not repo root), so nested
/// directories get their own matcher.
type RepoIgnoreCache = Mutex<HashMap<PathBuf, CachedIgnore>>;

/// Lists a repo's tracked files that match an ignore rule (`git ls-files -i`).
pub(crate) type LsFiles = Arc<dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync>;

/// Where git keeps a repository's index and exclude file. In a linked worktree
/// or submodule `<root>/.git` is a pointer file, so these cannot be derived
/// from the root alone.
#[derive(Clone)]
struct RepoGitPaths {
    /// `<gitdir>/index`.
    index: PathBuf,
    /// `<common dir or gitdir>/info/exclude`.
    info_exclude: PathBuf,
}

impl RepoGitPaths {
    /// Resolve from the filesystem; falls back to `<root>/.git` when unresolvable.
    fn resolve(repo_root: &Path) -> Self {
        let git_dir =
            crate::git::resolve_gitdir(repo_root).unwrap_or_else(|| repo_root.join(".git"));
        let exclude_dir =
            crate::git::resolve_common_dir(&git_dir).unwrap_or_else(|| git_dir.clone());
        Self {
            index: git_dir.join("index"),
            info_exclude: exclude_dir.join("info/exclude"),
        }
    }
}

/// The ignore-related memoization caches owned by a [`WatchRegistry`].
pub(crate) struct IgnoreCaches {
    /// Per-directory cache of compiled `.gitignore` matchers.
    repo_ignore: RepoIgnoreCache,
    /// Per-repo cache of force-added file sets.
    pub(super) force_added: ForceAddedCache,
    /// Per-repo git file locations; one small entry per repository.
    git_paths: Mutex<HashMap<PathBuf, RepoGitPaths>>,
    /// Source of recency ticks.
    tick: AtomicU64,
    /// Source of the force-added sets.
    ls_files: LsFiles,
}

impl IgnoreCaches {
    /// Empty caches whose force-added sets come from `ls_files`.
    pub(crate) fn new(ls_files: LsFiles) -> Self {
        Self {
            repo_ignore: Mutex::new(HashMap::new()),
            force_added: Mutex::new(HashMap::new()),
            git_paths: Mutex::new(HashMap::new()),
            tick: AtomicU64::new(0),
            ls_files,
        }
    }

    /// A strictly increasing recency tick.
    fn next_tick(&self) -> u64 {
        self.tick.fetch_add(1, Ordering::Relaxed)
    }

    /// Git file locations of `repo_root`, re-resolved while the cached index
    /// does not exist (worktree moved, index not created yet).
    fn repo_git_paths(&self, repo_root: &Path) -> RepoGitPaths {
        let mut cache = self.git_paths.lock_or_recover();
        let cached = cache
            .get(repo_root)
            .filter(|paths| fingerprint_file(&paths.index).is_some())
            .cloned();
        cached.unwrap_or_else(|| {
            let resolved = RepoGitPaths::resolve(repo_root);
            if cache.len() >= IGNORE_CACHE_CAPACITY {
                cache.clear();
            }
            cache.insert(repo_root.to_path_buf(), resolved.clone());
            resolved
        })
    }

    /// Directories with a cached matcher.
    #[cfg(test)]
    fn cached_ignore_dirs(&self) -> Vec<PathBuf> {
        self.repo_ignore.lock_or_recover().keys().cloned().collect()
    }
}

impl Default for IgnoreCaches {
    /// Force-added sets from the repository's own `git ls-files`.
    fn default() -> Self {
        Self::new(Arc::new(NativeGitBackend::tracked_ignored_files))
    }
}

/// A cached force-added set and the index fingerprint it came from.
pub(super) struct CachedForceAdded {
    files: Arc<HashSet<PathBuf>>,
    index_signature: FileFingerprint,
    /// Distinguishes a placeholder from a genuinely empty set.
    populated: bool,
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
            refreshing: false,
            last_used: 0,
        }
    }
}

/// What [`force_added_set`] must do about the entry it just inspected.
enum ForceAddedRefresh {
    /// Entry is current, or a run is already in flight: serve what we have.
    None,
    /// Nothing computed yet: run now on this thread, no lock held.
    Blocking,
    /// Index changed: serve the stale set and refresh off-thread.
    Background,
}

/// Cache of force-added sets, keyed by repo root.
pub(super) type ForceAddedCache = Mutex<HashMap<PathBuf, CachedForceAdded>>;

/// Fingerprint one file; absent maps to `None`.
fn fingerprint_file(path: &Path) -> FileFingerprint {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some((modified, metadata.len()))
}

/// Directories from `repo_root` down to `dir`, root->leaf; just `[repo_root]` if
/// `dir` is outside it.
fn ancestor_gitignore_dirs(repo_root: &Path, dir: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = Some(dir);
    while let Some(component_dir) = current {
        chain.push(component_dir.to_path_buf());
        if component_dir == repo_root {
            break;
        }
        current = component_dir.parent();
    }

    if chain.last().map(PathBuf::as_path) != Some(repo_root) {
        return vec![repo_root.to_path_buf()];
    }

    chain.reverse();
    chain
}

/// Fingerprint the ancestor ignore chain, in [`build_nested_ignore`] order.
fn nested_ignore_signature(
    repo_root: &Path,
    chain: &[PathBuf],
    git_paths: &RepoGitPaths,
) -> NestedIgnoreSignature {
    let mut signature = Vec::with_capacity(chain.len().saturating_add(1));
    signature.push(fingerprint_file(&repo_root.join(".gitignore")));
    signature.push(fingerprint_file(&git_paths.info_exclude));
    for dir in chain.iter().skip(1) {
        signature.push(fingerprint_file(&dir.join(".gitignore")));
    }
    signature
}

/// Build a matcher from the chain, lowest precedence first so deeper rules
/// (including negations) win. Empty matcher on error.
fn build_nested_ignore(repo_root: &Path, chain: &[PathBuf], git_paths: &RepoGitPaths) -> Gitignore {
    let mut builder = GitignoreBuilder::new(repo_root);
    let root_ignore = repo_root.join(".gitignore");
    if let Some(err) = builder.add(&root_ignore) {
        crate::debug_log!(
            "watcher",
            "Failed to load .gitignore at {}: {err}",
            root_ignore.display()
        );
    }
    if let Some(err) = builder.add(&git_paths.info_exclude) {
        crate::debug_log!(
            "watcher",
            "Failed to load exclude file at {}: {err}",
            git_paths.info_exclude.display()
        );
    }
    for dir in chain.iter().skip(1) {
        let nested_ignore = dir.join(".gitignore");
        if let Some(err) = builder.add(&nested_ignore) {
            crate::debug_log!(
                "watcher",
                "Failed to load nested .gitignore at {}: {err}",
                nested_ignore.display()
            );
        }
    }
    builder.build().unwrap_or_else(|err| {
        crate::debug_log!(
            "watcher",
            "Failed to build gitignore matcher for {}: {err}",
            repo_root.display()
        );
        Gitignore::empty()
    })
}

/// Force-added files (`git add -f`) for `repo_root`, cached by index fingerprint.
///
/// The shared mutex is never held across `git ls-files`. A stale set is served while
/// one background thread refreshes; only the first lookup per repo runs inline, so a
/// force-added file is never suppressed.
pub(super) fn force_added_set(
    repo_root: &Path,
    registry: &Arc<WatchRegistry>,
) -> Arc<HashSet<PathBuf>> {
    let index_signature = fingerprint_file(&registry.ignore().repo_git_paths(repo_root).index);
    let tick = registry.ignore().next_tick();

    let (files, refresh) = {
        let mut cache = registry.ignore().force_added.lock_or_recover();
        let cached = cache
            .entry(repo_root.to_path_buf())
            .or_insert_with(CachedForceAdded::placeholder);
        cached.last_used = tick;
        let refresh = if cached.refreshing {
            ForceAddedRefresh::None
        } else if !cached.populated {
            cached.refreshing = true;
            ForceAddedRefresh::Blocking
        } else if cached.index_signature == index_signature {
            ForceAddedRefresh::None
        } else {
            cached.refreshing = true;
            ForceAddedRefresh::Background
        };
        let files = Arc::clone(&cached.files);
        evict_to_capacity(&mut cache, IGNORE_CACHE_CAPACITY, |entry| entry.last_used);
        drop(cache);
        (files, refresh)
    };

    match refresh {
        ForceAddedRefresh::None => files,
        ForceAddedRefresh::Blocking => refresh_force_added(repo_root, registry.ignore()),
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
/// Fingerprinting before the run means a racing index write leaves it stale.
fn refresh_force_added(repo_root: &Path, ignore: &IgnoreCaches) -> Arc<HashSet<PathBuf>> {
    let index_signature = fingerprint_file(&ignore.repo_git_paths(repo_root).index);
    let files: Arc<HashSet<PathBuf>> = Arc::new((ignore.ls_files)(repo_root).into_iter().collect());
    let tick = ignore.next_tick();

    let mut cache = ignore.force_added.lock_or_recover();
    let cached = cache
        .entry(repo_root.to_path_buf())
        .or_insert_with(CachedForceAdded::placeholder);
    cached.files = Arc::clone(&files);
    cached.index_signature = index_signature;
    cached.populated = true;
    cached.refreshing = false;
    cached.last_used = tick;
    evict_to_capacity(&mut cache, IGNORE_CACHE_CAPACITY, |entry| entry.last_used);
    drop(cache);
    files
}

/// Whether `path` is a force-added tracked file despite matching an ignore rule.
pub(super) fn is_force_added(repo_root: &Path, path: &Path, registry: &Arc<WatchRegistry>) -> bool {
    let set = force_added_set(repo_root, registry);
    if set.is_empty() {
        return false;
    }
    // Match the canonical form stored by `tracked_ignored_files`.
    let candidate = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    set.contains(&candidate)
}

/// Attribution and ignore verdict of one event path, each computed at most once
/// so a path consulted by several decisions is attributed and stat-ed once.
pub(super) struct EventPathVerdict<'a> {
    path: &'a Path,
    registry: &'a Arc<WatchRegistry>,
    attribution: OnceCell<Option<RepoAttribution>>,
    ignored: OnceCell<bool>,
}

impl<'a> EventPathVerdict<'a> {
    /// A verdict for `path` with nothing computed yet.
    pub(super) const fn new(path: &'a Path, registry: &'a Arc<WatchRegistry>) -> Self {
        Self {
            path,
            registry,
            attribution: OnceCell::new(),
            ignored: OnceCell::new(),
        }
    }

    /// The event path.
    pub(super) const fn path(&self) -> &'a Path {
        self.path
    }

    /// The repo the path belongs to, if any.
    pub(super) fn attribution(&self) -> Option<&RepoAttribution> {
        self.attribution
            .get_or_init(|| attribute_repo_detailed(self.path, self.registry))
            .as_ref()
    }

    /// Whether the path is gitignored (nested-aware chain) and not force-added.
    pub(super) fn is_gitignored(&self) -> bool {
        self.attribution().is_some_and(|attribution| {
            *self
                .ignored
                .get_or_init(|| path_is_gitignored(attribution.repo(), self.path, self.registry))
        })
    }

    /// A language marker inside a gitignored subtree of a genuine repo;
    /// root-level markers always count.
    pub(super) fn language_marker_is_ignored(&self) -> bool {
        self.attribution().is_some_and(|attribution| {
            attribution.is_genuine_repo()
                && self
                    .path
                    .parent()
                    .is_some_and(|parent| parent != attribution.repo())
        }) && self.is_gitignored()
    }
}

/// Whether `path` is gitignored (nested-aware chain) and not force-added.
///
/// `matched_path_or_any_parents` honors ignored parent directories and negations.
/// Matchers are cached per directory and rebuilt on chain fingerprint change.
pub(super) fn path_is_gitignored(
    repo_root: &Path,
    path: &Path,
    registry: &Arc<WatchRegistry>,
) -> bool {
    let dir = path.parent().unwrap_or(repo_root);
    let chain = ancestor_gitignore_dirs(repo_root, dir);
    let git_paths = registry.ignore().repo_git_paths(repo_root);
    let signature = nested_ignore_signature(repo_root, &chain, &git_paths);
    let tick = registry.ignore().next_tick();
    let matcher = {
        let mut cache = registry.ignore().repo_ignore.lock_or_recover();
        let cached = cache
            .entry(dir.to_path_buf())
            .or_insert_with(|| CachedIgnore {
                matcher: Arc::new(build_nested_ignore(repo_root, &chain, &git_paths)),
                signature: signature.clone(),
                last_used: tick,
            });
        if cached.signature != signature {
            cached.matcher = Arc::new(build_nested_ignore(repo_root, &chain, &git_paths));
            cached.signature.clone_from(&signature);
        }
        cached.last_used = tick;
        let matcher = Arc::clone(&cached.matcher);
        evict_to_capacity(&mut cache, IGNORE_CACHE_CAPACITY, |entry| entry.last_used);
        drop(cache);
        matcher
    };
    if !matcher
        .matched_path_or_any_parents(path, path.is_dir())
        .is_ignore()
    {
        return false;
    }
    // Force-added files are tracked, so edits still change `git status`.
    !is_force_added(repo_root, path, registry)
}

#[cfg(test)]
mod tests {
    use super::{IGNORE_CACHE_CAPACITY, path_is_gitignored};
    use crate::watcher::WatchRegistry;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn hot_matcher_survives_capacity_overflow() {
        let registry = Arc::new(WatchRegistry::new());
        let repo = PathBuf::from("/gpy-ignore-cache-test-repo");
        let hot_dir = repo.join("hot");
        let hot_file = hot_dir.join("file.txt");
        assert!(!path_is_gitignored(&repo, &hot_file, &registry));

        for index in 0..IGNORE_CACHE_CAPACITY.saturating_add(16) {
            let other = repo.join(format!("other-{index}")).join("file.txt");
            assert!(!path_is_gitignored(&repo, &other, &registry));
            assert!(
                registry.ignore().cached_ignore_dirs().contains(&hot_dir),
                "hot directory evicted after inserting directory {index}"
            );
            assert!(!path_is_gitignored(&repo, &hot_file, &registry));
        }
    }
}
