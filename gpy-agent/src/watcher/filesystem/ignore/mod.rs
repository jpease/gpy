//! Ignore matching: built-in noise tokens, per-repo gitignore caches, and the
//! force-added file set ([`force_added`]).
//!
//! A path is judged like git does: each `.gitignore` is matched relative to
//! its own directory, the deepest one that decides wins, then the repository's
//! `info/exclude`, then the global excludes file (`core.excludesFile`).

pub(super) mod force_added;

use super::{RepoAttribution, WatchRegistry, attribute_repo_detailed};
use crate::cache::bounded::{IGNORE_CACHE_CAPACITY, evict_to_capacity};
use crate::git::native::NativeGitBackend;
use crate::sync_ext::MutexExt;
use ::ignore::Match;
use ::ignore::gitignore::{Gitignore, GitignoreBuilder};
use force_added::{ForceAddedCache, is_force_added};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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

/// How long the event thread trusts a cached ignore file or git-path lookup
/// before checking its fingerprint again. An event for an ignore file drops
/// its entry at once ([`IgnoreCaches::invalidate_ignore_file`]); this bounds
/// staleness for edits no watch sees, such as the global excludes file.
const EVENT_REVALIDATE: Duration = Duration::from_secs(2);

/// Compiled ignore files kept, present or absent; an absent entry is a few bytes.
const IGNORE_FILE_CACHE_CAPACITY: usize = 4096;

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

/// Whether a lookup may run slow work (`git`) on the calling thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Lookup {
    /// Run a missing lookup inline and re-check every fingerprint: arming
    /// walks and other callers off the event thread.
    Wait,
    /// The shared event thread: never run `git`. A missing lookup starts in
    /// the background and is served empty meanwhile; cached files are
    /// trusted for [`EVENT_REVALIDATE`].
    NoWait,
}

/// File mtime and length, or `None` if absent; length catches same-second edits
/// on coarse-mtime filesystems.
type FileFingerprint = Option<(std::time::SystemTime, u64)>;

/// One compiled ignore file and the fingerprint it was compiled from.
struct CachedIgnoreFile {
    /// `None` when the file is absent or holds no patterns.
    matcher: Option<Arc<Gitignore>>,
    fingerprint: FileFingerprint,
    /// When `fingerprint` was last compared with the file.
    checked_at: Instant,
    /// Recency tick for eviction.
    last_used: u64,
}

/// Compiled ignore files keyed by `(file, root its patterns are relative to)`.
/// A global excludes file is shared by repositories with different roots.
type IgnoreFileCache = Mutex<HashMap<(PathBuf, PathBuf), CachedIgnoreFile>>;

/// Lists a repo's tracked files that match an ignore rule (`git ls-files -i`);
/// `None` when the listing failed or timed out.
pub(crate) type LsFiles = Arc<dyn Fn(&Path) -> Option<Vec<PathBuf>> + Send + Sync>;

/// Resolves a repo's global excludes file (`core.excludesFile` or git's default).
pub(crate) type ExcludesFile = Arc<dyn Fn(&Path) -> Option<PathBuf> + Send + Sync>;

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

/// [`RepoGitPaths`] and when its index was last seen to exist.
struct CachedGitPaths {
    paths: RepoGitPaths,
    checked_at: Instant,
}

/// A repo's global excludes file: being resolved, or resolved.
enum GlobalExcludes {
    Resolving,
    Resolved(Option<PathBuf>),
}

/// The ignore-related memoization caches owned by a [`WatchRegistry`].
pub(crate) struct IgnoreCaches {
    /// Compiled ignore files.
    ignore_files: IgnoreFileCache,
    /// Per-repo cache of force-added file sets.
    pub(super) force_added: ForceAddedCache,
    /// Per-repo git file locations; one small entry per repository.
    git_paths: Mutex<HashMap<PathBuf, CachedGitPaths>>,
    /// Per-repo global excludes file.
    global_excludes: Mutex<HashMap<PathBuf, GlobalExcludes>>,
    /// Source of recency ticks.
    tick: AtomicU64,
    /// Source of the force-added sets.
    ls_files: LsFiles,
    /// Source of the global excludes file.
    excludes_file: ExcludesFile,
}

impl IgnoreCaches {
    /// Empty caches whose force-added sets come from `ls_files`, with no global
    /// excludes file (so tests never read the user's).
    #[cfg(test)]
    pub(crate) fn new(ls_files: LsFiles) -> Self {
        Self::with_sources(ls_files, Arc::new(|_: &Path| -> Option<PathBuf> { None }))
    }

    /// Empty caches over the given force-added and global-excludes sources.
    pub(crate) fn with_sources(ls_files: LsFiles, excludes_file: ExcludesFile) -> Self {
        Self {
            ignore_files: Mutex::new(HashMap::new()),
            force_added: Mutex::new(HashMap::new()),
            git_paths: Mutex::new(HashMap::new()),
            global_excludes: Mutex::new(HashMap::new()),
            tick: AtomicU64::new(0),
            ls_files,
            excludes_file,
        }
    }

    /// A strictly increasing recency tick.
    fn next_tick(&self) -> u64 {
        self.tick.fetch_add(1, Ordering::Relaxed)
    }

    /// Git file locations of `repo_root`, re-resolved once the cached index is
    /// found missing (worktree moved, index not created yet); the index is
    /// checked at most every [`EVENT_REVALIDATE`].
    fn repo_git_paths(&self, repo_root: &Path) -> RepoGitPaths {
        let now = Instant::now();
        let mut cache = self.git_paths.lock_or_recover();
        if let Some(cached) = cache.get_mut(repo_root) {
            let fresh = now.saturating_duration_since(cached.checked_at) < EVENT_REVALIDATE;
            if fresh || fingerprint_file(&cached.paths.index).is_some() {
                if !fresh {
                    cached.checked_at = now;
                }
                return cached.paths.clone();
            }
        }
        let resolved = RepoGitPaths::resolve(repo_root);
        if cache.len() >= IGNORE_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(
            repo_root.to_path_buf(),
            CachedGitPaths {
                paths: resolved.clone(),
                checked_at: now,
            },
        );
        resolved
    }

    /// The ignore file `file` compiled with patterns relative to `root`;
    /// `None` when it is absent or empty. File I/O runs with no lock held.
    fn ignore_file(&self, file: &Path, root: &Path, lookup: Lookup) -> Option<Arc<Gitignore>> {
        let key = (file.to_path_buf(), root.to_path_buf());
        let tick = self.next_tick();
        let now = Instant::now();
        if lookup == Lookup::NoWait {
            let mut cache = self.ignore_files.lock_or_recover();
            if let Some(entry) = cache.get_mut(&key)
                && now.saturating_duration_since(entry.checked_at) < EVENT_REVALIDATE
            {
                entry.last_used = tick;
                return entry.matcher.clone();
            }
        }

        let fingerprint = fingerprint_file(file);
        let unchanged = self
            .ignore_files
            .lock_or_recover()
            .get(&key)
            .filter(|entry| entry.fingerprint == fingerprint)
            .map(|entry| entry.matcher.clone());
        let matcher = match unchanged {
            Some(matcher) => matcher,
            None if fingerprint.is_some() => compile_ignore_file(file, root),
            None => None,
        };

        let mut cache = self.ignore_files.lock_or_recover();
        cache.insert(
            key,
            CachedIgnoreFile {
                matcher: matcher.clone(),
                fingerprint,
                checked_at: now,
                last_used: tick,
            },
        );
        evict_to_capacity(&mut cache, IGNORE_FILE_CACHE_CAPACITY, |entry| {
            entry.last_used
        });
        drop(cache);
        matcher
    }

    /// Drop every cached compilation of `file`, so the next lookup re-reads it.
    /// Called for the watcher's own events on `.gitignore` and `info/exclude`.
    pub(super) fn invalidate_ignore_file(&self, file: &Path) {
        self.ignore_files
            .lock_or_recover()
            .retain(|(cached, _), _| cached != file);
    }

    /// Ignore files with a cached compilation.
    #[cfg(test)]
    fn cached_ignore_files(&self) -> Vec<PathBuf> {
        self.ignore_files
            .lock_or_recover()
            .keys()
            .map(|(file, _)| file.clone())
            .collect()
    }
}

impl Default for IgnoreCaches {
    /// Force-added sets and the global excludes file from the repository's own git.
    fn default() -> Self {
        Self::with_sources(
            Arc::new(NativeGitBackend::tracked_ignored_files),
            Arc::new(NativeGitBackend::global_excludes_file),
        )
    }
}

/// Compile `file` with patterns relative to `root`; `None` if it holds none.
fn compile_ignore_file(file: &Path, root: &Path) -> Option<Arc<Gitignore>> {
    let mut builder = GitignoreBuilder::new(root);
    if let Some(err) = builder.add(file) {
        crate::debug_log!(
            "watcher",
            "Failed to load ignore file {}: {err}",
            file.display()
        );
    }
    match builder.build() {
        Ok(matcher) => (!matcher.is_empty()).then(|| Arc::new(matcher)),
        Err(err) => {
            crate::debug_log!(
                "watcher",
                "Failed to build ignore matcher for {}: {err}",
                file.display()
            );
            None
        }
    }
}

/// The global excludes file of `repo_root`. Resolved once per repo: inline for
/// [`Lookup::Wait`], else on a background thread (none meanwhile).
fn global_excludes(
    repo_root: &Path,
    registry: &Arc<WatchRegistry>,
    lookup: Lookup,
) -> Option<PathBuf> {
    let ignore = registry.ignore();
    {
        let mut cache = ignore.global_excludes.lock_or_recover();
        match cache.get(repo_root) {
            Some(GlobalExcludes::Resolved(file)) => return file.clone(),
            Some(GlobalExcludes::Resolving) => return None,
            None => {
                if cache.len() >= IGNORE_CACHE_CAPACITY {
                    cache.clear();
                }
                cache.insert(repo_root.to_path_buf(), GlobalExcludes::Resolving);
            }
        }
    }
    match lookup {
        Lookup::Wait => resolve_global_excludes(repo_root, ignore),
        Lookup::NoWait => {
            let repo_root_owned = repo_root.to_path_buf();
            // Owned handle: the thread outlives this call.
            let registry_for_lookup = Arc::clone(registry);
            thread::spawn(move || {
                drop(resolve_global_excludes(
                    &repo_root_owned,
                    registry_for_lookup.ignore(),
                ));
            });
            None
        }
    }
}

/// Run the global-excludes source with no lock held and store the answer.
fn resolve_global_excludes(repo_root: &Path, ignore: &IgnoreCaches) -> Option<PathBuf> {
    let file = (ignore.excludes_file)(repo_root);
    ignore.global_excludes.lock_or_recover().insert(
        repo_root.to_path_buf(),
        GlobalExcludes::Resolved(file.clone()),
    );
    file
}

/// The matchers that can decide one path, in precedence order.
struct IgnoreLayers {
    /// `(directory, matcher)` for each existing `.gitignore` from the path's
    /// directory up to the repo root, deepest first.
    nested: Vec<(PathBuf, Arc<Gitignore>)>,
    /// `info/exclude`, then the global excludes file; both rooted at the repo root.
    repo_wide: Vec<Arc<Gitignore>>,
}

impl IgnoreLayers {
    /// The layers for `path` under `repo_root`.
    fn load(repo_root: &Path, path: &Path, registry: &Arc<WatchRegistry>, lookup: Lookup) -> Self {
        let ignore = registry.ignore();
        let mut nested = Vec::new();
        let dir = path.parent().unwrap_or(repo_root);
        for ancestor in dir.ancestors() {
            if !ancestor.starts_with(repo_root) {
                break;
            }
            if let Some(matcher) =
                ignore.ignore_file(&ancestor.join(".gitignore"), ancestor, lookup)
            {
                nested.push((ancestor.to_path_buf(), matcher));
            }
            if ancestor == repo_root {
                break;
            }
        }

        let mut repo_wide = Vec::with_capacity(2);
        let git_paths = ignore.repo_git_paths(repo_root);
        if let Some(matcher) = ignore.ignore_file(&git_paths.info_exclude, repo_root, lookup) {
            repo_wide.push(matcher);
        }
        if let Some(global) = global_excludes(repo_root, registry, lookup)
            && let Some(matcher) = ignore.ignore_file(&global, repo_root, lookup)
        {
            repo_wide.push(matcher);
        }
        Self { nested, repo_wide }
    }

    /// The decision for `candidate` alone: `Some(true)` ignored, `Some(false)`
    /// re-included by a negation, `None` when no pattern matches. Within one
    /// file the last matching line wins.
    fn decide(&self, repo_root: &Path, candidate: &Path, is_dir: bool) -> Option<bool> {
        let nested = self.nested.iter().filter_map(|(dir, matcher)| {
            candidate
                .strip_prefix(dir)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
                .map(|relative| (relative, matcher))
        });
        let relative_to_root = candidate.strip_prefix(repo_root).ok()?;
        let repo_wide = self
            .repo_wide
            .iter()
            .map(|matcher| (relative_to_root, matcher));
        for (relative, matcher) in nested.chain(repo_wide) {
            match matcher.matched(relative, is_dir) {
                Match::Ignore(_) => return Some(true),
                Match::Whitelist(_) => return Some(false),
                Match::None => {}
            }
        }
        None
    }

    /// Whether `path` is ignored: decided by the path itself, else by its
    /// nearest decided ancestor directory (an ignored directory hides its
    /// contents), like `Gitignore::matched_path_or_any_parents`.
    fn ignores(&self, repo_root: &Path, path: &Path, is_dir: bool) -> bool {
        if self.nested.is_empty() && self.repo_wide.is_empty() {
            return false;
        }
        let mut candidate = path;
        let mut candidate_is_dir = is_dir;
        while candidate != repo_root {
            if let Some(ignored) = self.decide(repo_root, candidate, candidate_is_dir) {
                return ignored;
            }
            let Some(parent) = candidate.parent() else {
                return false;
            };
            candidate = parent;
            candidate_is_dir = true;
        }
        false
    }
}

/// Fingerprint one file; absent maps to `None`.
fn fingerprint_file(path: &Path) -> FileFingerprint {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some((modified, metadata.len()))
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

    /// Whether the path is gitignored and not force-added, judged without
    /// running git (this is the event thread).
    pub(super) fn is_gitignored(&self) -> bool {
        self.attribution().is_some_and(|attribution| {
            *self.ignored.get_or_init(|| {
                gitignore_verdict(attribution.repo(), self.path, self.registry, Lookup::NoWait)
            })
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

/// Whether `path` is gitignored and not force-added, waiting for any lookup it
/// needs (see [`Lookup::Wait`]).
pub(super) fn path_is_gitignored(
    repo_root: &Path,
    path: &Path,
    registry: &Arc<WatchRegistry>,
) -> bool {
    gitignore_verdict(repo_root, path, registry, Lookup::Wait)
}

/// Whether `path` under `repo_root` is gitignored and not force-added.
///
/// A path not under `repo_root` (e.g. reported through a symlinked spelling of
/// a canonical root) is never ignored: the matchers are rooted at `repo_root`.
/// Directories skip the force-added set, which lists only tracked files.
fn gitignore_verdict(
    repo_root: &Path,
    path: &Path,
    registry: &Arc<WatchRegistry>,
    lookup: Lookup,
) -> bool {
    let Ok(relative) = path.strip_prefix(repo_root) else {
        return false;
    };
    if relative.as_os_str().is_empty() {
        return false;
    }
    let is_dir = path.is_dir();
    let layers = IgnoreLayers::load(repo_root, path, registry, lookup);
    if !layers.ignores(repo_root, path, is_dir) {
        return false;
    }
    if is_dir {
        return true;
    }
    // Force-added files are tracked, so edits still change `git status`.
    !is_force_added(repo_root, path, registry, lookup)
}

#[cfg(test)]
mod tests {
    use super::{IGNORE_FILE_CACHE_CAPACITY, IgnoreCaches, path_is_gitignored};
    use crate::watcher::WatchRegistry;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    /// A registry over `excludes` as the global excludes file and an empty
    /// force-added set.
    fn registry_with_global(excludes: Option<PathBuf>) -> Arc<WatchRegistry> {
        Arc::new(WatchRegistry::with_ignore(IgnoreCaches::with_sources(
            Arc::new(|_: &Path| -> Option<Vec<PathBuf>> { Some(Vec::new()) }),
            Arc::new(move |_: &Path| excludes.clone()),
        )))
    }

    #[test]
    fn hot_ignore_file_survives_capacity_overflow() {
        let registry = registry_with_global(None);
        let repo = PathBuf::from("/gpy-ignore-cache-test-repo");
        let hot_dir = repo.join("hot");
        let hot_file = hot_dir.join("file.txt");
        assert!(!path_is_gitignored(&repo, &hot_file, &registry));

        for index in 0..IGNORE_FILE_CACHE_CAPACITY.saturating_add(16) {
            let other = repo.join(format!("other-{index}")).join("file.txt");
            assert!(!path_is_gitignored(&repo, &other, &registry));
            if index.is_multiple_of(256) {
                assert!(
                    registry
                        .ignore()
                        .cached_ignore_files()
                        .contains(&hot_dir.join(".gitignore")),
                    "hot ignore file evicted after inserting directory {index}"
                );
            }
            assert!(!path_is_gitignored(&repo, &hot_file, &registry));
        }
    }

    /// `/dist` in a nested `.gitignore` is anchored to that directory.
    #[test]
    fn nested_anchored_pattern_matches_relative_to_its_directory() {
        let registry = registry_with_global(None);
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path();
        let package = repo.join("packages").join("foo");
        std::fs::create_dir_all(package.join("dist")).expect("dist dir");
        std::fs::create_dir_all(package.join("docs")).expect("docs dir");
        std::fs::write(package.join(".gitignore"), "/dist\ndocs/*.md\n").expect("gitignore");

        assert!(path_is_gitignored(
            repo,
            &package.join("dist").join("bundle.js"),
            &registry
        ));
        assert!(path_is_gitignored(
            repo,
            &package.join("docs").join("api.md"),
            &registry
        ));
        assert!(
            !path_is_gitignored(repo, &repo.join("dist").join("bundle.js"), &registry),
            "a nested anchored rule must not reach the repo root"
        );
    }

    /// A `.gitignore` negation outranks `info/exclude`, which outranks the
    /// global excludes file.
    #[test]
    fn gitignore_outranks_info_exclude_which_outranks_global_excludes() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git").join("info")).expect("info dir");
        let global = tmp.path().join("global-ignore");
        std::fs::write(&global, "*.swp\n*.keep\n.idea/\n").expect("global ignore");
        std::fs::write(
            repo.join(".git").join("info").join("exclude"),
            "!*.keep\nlocal.txt\n",
        )
        .expect("info/exclude");
        std::fs::write(repo.join(".gitignore"), "!local.txt\n").expect("gitignore");
        let registry = registry_with_global(Some(global));

        assert!(path_is_gitignored(&repo, &repo.join("a.swp"), &registry));
        assert!(path_is_gitignored(
            &repo,
            &repo.join(".idea").join("workspace.xml"),
            &registry
        ));
        assert!(
            !path_is_gitignored(&repo, &repo.join("a.keep"), &registry),
            "info/exclude must outrank the global excludes file"
        );
        assert!(
            !path_is_gitignored(&repo, &repo.join("local.txt"), &registry),
            ".gitignore must outrank info/exclude"
        );
    }

    /// A path outside the (canonical) root is judged not ignored instead of
    /// tripping the matcher's "path is expected to be under the root" assert.
    #[test]
    fn path_outside_the_root_is_not_ignored() {
        let registry = registry_with_global(None);
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        std::fs::write(repo.join(".gitignore"), "*\n").expect("gitignore");

        assert!(!path_is_gitignored(
            &repo,
            &tmp.path().join("elsewhere").join("config.toml"),
            &registry
        ));
    }
}
