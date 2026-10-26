//! Ignore matching: built-in noise tokens, per-repo gitignore caches, and the
//! force-added file set.

use super::{WatchRegistry, attribute_repo_detailed};
use crate::cache::bounded::IGNORE_CACHE_CAPACITY;
use crate::git::native::NativeGitBackend;
use crate::sync_ext::MutexExt;
use ::ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

pub(super) fn ignored_tokens() -> &'static [String] {
    static TOKENS: OnceLock<Vec<String>> = OnceLock::new();
    TOKENS.get_or_init(|| {
        get_ignore_patterns()
            .into_iter()
            .map(|pattern| pattern.trim_matches('*').trim_matches('/').to_owned())
            .filter(|token| !token.is_empty())
            .collect()
    })
}

/// Match `path` against the built-in high-churn tokens (`node_modules`, `build`, ...).
///
/// With a trusted `repo_root` only the in-repo portion is tested, so a repo under
/// `/work/build/` isn't suppressed; otherwise the whole path is tested.
pub(super) fn path_matches_ignored_patterns(
    path: &Path,
    repo_root: Option<&Path>,
) -> Option<IgnoreTokenHit> {
    let scoped_path = repo_root.map_or(path, |root| path.strip_prefix(root).unwrap_or(path));
    let path_str = scoped_path.to_string_lossy();
    let file_name = scoped_path.file_name();
    let mut directory_hit = false;

    for token in ignored_tokens() {
        if token.contains('/') {
            if path_str.contains(token) {
                return Some(IgnoreTokenHit::Noise);
            }
            continue;
        }

        let desired = OsStr::new(token);
        let hit = file_name == Some(desired)
            || scoped_path
                .components()
                .any(|component| component.as_os_str() == desired);
        if !hit {
            continue;
        }
        if is_noise_file_token(token) {
            return Some(IgnoreTokenHit::Noise);
        }
        directory_hit = true;
    }

    directory_hit.then_some(IgnoreTokenHit::Directory)
}

/// Which kind of built-in token `path_matches_ignored_patterns` hit.
pub(super) enum IgnoreTokenHit {
    /// Directory-name token (`vendor`, `dist`, ...): a hint, deferring to gitignore.
    Directory,
    /// OS/git noise, suppressed unconditionally.
    Noise,
}

/// Whether a slash-free token names an OS metadata file.
fn is_noise_file_token(token: &str) -> bool {
    matches!(token, ".DS_Store" | "Thumbs.db")
}

/// File mtime and length, or `None` if absent; length catches same-second edits
/// on coarse-mtime filesystems.
type FileFingerprint = Option<(std::time::SystemTime, u64)>;

/// Fingerprints of the ignore chain: root `.gitignore`, `.git/info/exclude`,
/// then nested `.gitignore`s root->leaf.
type NestedIgnoreSignature = Vec<FileFingerprint>;

/// A compiled ignore matcher and the fingerprint it was built from.
struct CachedIgnore {
    matcher: Arc<Gitignore>,
    signature: NestedIgnoreSignature,
}

/// Compiled `.gitignore` matchers keyed by directory (not repo root), so nested
/// directories get their own matcher.
type RepoIgnoreCache = Mutex<HashMap<PathBuf, CachedIgnore>>;

/// The ignore-related memoization caches owned by a [`WatchRegistry`].
#[derive(Default)]
pub(crate) struct IgnoreCaches {
    /// Per-directory cache of compiled `.gitignore` matchers.
    repo_ignore: RepoIgnoreCache,
    /// Per-repo cache of force-added file sets.
    pub(super) force_added: ForceAddedCache,
}

/// A cached force-added set and the `.git/index` fingerprint it came from.
pub(super) struct CachedForceAdded {
    files: Arc<HashSet<PathBuf>>,
    index_signature: FileFingerprint,
    /// Distinguishes a placeholder from a genuinely empty set.
    populated: bool,
    /// A run is in flight; prevents one `git ls-files` per event.
    refreshing: bool,
}

impl CachedForceAdded {
    /// Unpopulated entry; `None` signature never matches a real fingerprint.
    fn placeholder() -> Self {
        Self {
            files: Arc::new(HashSet::new()),
            index_signature: None,
            populated: false,
            refreshing: false,
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
fn nested_ignore_signature(repo_root: &Path, chain: &[PathBuf]) -> NestedIgnoreSignature {
    let mut signature = Vec::with_capacity(chain.len().saturating_add(1));
    signature.push(fingerprint_file(&repo_root.join(".gitignore")));
    signature.push(fingerprint_file(&repo_root.join(".git/info/exclude")));
    for dir in chain.iter().skip(1) {
        signature.push(fingerprint_file(&dir.join(".gitignore")));
    }
    signature
}

/// Build a matcher from the chain, lowest precedence first so deeper rules
/// (including negations) win. Empty matcher on error.
fn build_nested_ignore(repo_root: &Path, chain: &[PathBuf]) -> Gitignore {
    let mut builder = GitignoreBuilder::new(repo_root);
    let _ = builder.add(repo_root.join(".gitignore"));
    let _ = builder.add(repo_root.join(".git/info/exclude"));
    for dir in chain.iter().skip(1) {
        let _ = builder.add(dir.join(".gitignore"));
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Cap a cache to [`IGNORE_CACHE_CAPACITY`], never evicting `keep`.
fn evict_cache_overflow<V>(cache: &mut HashMap<PathBuf, V>, keep: &Path) {
    if cache.len() <= IGNORE_CACHE_CAPACITY {
        return;
    }
    let excess = cache.len().saturating_sub(IGNORE_CACHE_CAPACITY);
    let to_remove: Vec<PathBuf> = cache
        .keys()
        .filter(|k| k.as_path() != keep)
        .take(excess)
        .cloned()
        .collect();
    for k in to_remove {
        cache.remove(&k);
    }
}

/// Force-added files (`git add -f`) for `repo_root`, cached by `.git/index` fingerprint.
///
/// The shared mutex is never held across `git ls-files`. A stale set is served while
/// one background thread refreshes; only the first lookup per repo runs inline, so a
/// force-added file is never suppressed.
pub(super) fn force_added_set(
    repo_root: &Path,
    registry: &Arc<WatchRegistry>,
) -> Arc<HashSet<PathBuf>> {
    let index_signature = fingerprint_file(&repo_root.join(".git/index"));

    let (files, refresh) = {
        let mut cache = registry.ignore().force_added.lock_or_recover();
        let cached = cache
            .entry(repo_root.to_path_buf())
            .or_insert_with(CachedForceAdded::placeholder);
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
        evict_cache_overflow(&mut cache, repo_root);
        drop(cache);
        (files, refresh)
    };

    match refresh {
        ForceAddedRefresh::None => files,
        ForceAddedRefresh::Blocking => {
            refresh_force_added(repo_root, &registry.ignore().force_added)
        }
        ForceAddedRefresh::Background => {
            let repo_root_owned = repo_root.to_path_buf();
            // Owned handle: the thread outlives this call.
            let registry_for_refresh = Arc::clone(registry);
            thread::spawn(move || {
                drop(refresh_force_added(
                    &repo_root_owned,
                    &registry_for_refresh.ignore().force_added,
                ));
            });
            files
        }
    }
}

/// Run `git ls-files` with no lock held, store the result, and clear `refreshing`.
/// Fingerprinting before the run means a racing index write leaves it stale.
fn refresh_force_added(repo_root: &Path, cache_slot: &ForceAddedCache) -> Arc<HashSet<PathBuf>> {
    let index_signature = fingerprint_file(&repo_root.join(".git/index"));
    let files: Arc<HashSet<PathBuf>> = Arc::new(
        compute_tracked_ignored_files(repo_root)
            .into_iter()
            .collect(),
    );

    let mut cache = cache_slot.lock_or_recover();
    let cached = cache
        .entry(repo_root.to_path_buf())
        .or_insert_with(CachedForceAdded::placeholder);
    cached.files = Arc::clone(&files);
    cached.index_signature = index_signature;
    cached.populated = true;
    cached.refreshing = false;
    evict_cache_overflow(&mut cache, repo_root);
    drop(cache);
    files
}

/// Run `git ls-files`; tests may override per repo root.
fn compute_tracked_ignored_files(repo_root: &Path) -> Vec<PathBuf> {
    #[cfg(test)]
    {
        // Release the table lock before running the hook.
        let hook = tracked_ignored_test_hooks()
            .lock_or_recover()
            .get(repo_root)
            .map(Arc::clone);
        if let Some(hook_fn) = hook {
            return hook_fn(repo_root);
        }
    }
    NativeGitBackend::tracked_ignored_files(repo_root)
}

/// A test stand-in for `git ls-files`, registered per repo root.
#[cfg(test)]
type TrackedIgnoredHook = Arc<dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync>;

/// Per-repo table of test stand-ins consulted by [`compute_tracked_ignored_files`].
#[cfg(test)]
pub(super) fn tracked_ignored_test_hooks() -> &'static Mutex<HashMap<PathBuf, TrackedIgnoredHook>> {
    static HOOKS: OnceLock<Mutex<HashMap<PathBuf, TrackedIgnoredHook>>> = OnceLock::new();
    HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
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

/// A language marker inside a gitignored subtree of a repo; root-level
/// markers always count.
pub(super) fn language_marker_is_ignored(path: &Path, registry: &Arc<WatchRegistry>) -> bool {
    let Some(attribution) = attribute_repo_detailed(path, registry) else {
        return false;
    };
    if !attribution.is_genuine_repo() {
        return false;
    }
    let repo = attribution.into_repo();
    path.parent().is_some_and(|parent| parent != repo) && path_is_gitignored(&repo, path, registry)
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
    let signature = nested_ignore_signature(repo_root, &chain);
    let matcher = {
        let mut cache = registry.ignore().repo_ignore.lock_or_recover();
        let cached = cache
            .entry(dir.to_path_buf())
            .or_insert_with(|| CachedIgnore {
                matcher: Arc::new(build_nested_ignore(repo_root, &chain)),
                signature: signature.clone(),
            });
        if cached.signature != signature {
            cached.matcher = Arc::new(build_nested_ignore(repo_root, &chain));
            cached.signature.clone_from(&signature);
        }
        let matcher = Arc::clone(&cached.matcher);
        evict_cache_overflow(&mut cache, dir);
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

/// Built-in high-churn hints matched as exact path components. Generic words
/// (`tmp`, `env`) are excluded since they collide with ordinary paths.
#[must_use]
fn get_ignore_patterns() -> Vec<&'static str> {
    vec![
        // Node.js
        "**/node_modules/**",
        "**/dist/**",
        "**/build/**",
        "**/.next/**",
        // Rust
        "**/target/**",
        // Go
        "**/vendor/**",
        // Python
        "**/__pycache__/**",
        "**/venv/**",
        "**/.venv/**",
        // General
        "**/.git/objects/**",
        "**/.git/logs/**",
        "**/.DS_Store",
        "**/Thumbs.db",
    ]
}
