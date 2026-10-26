//! Shared watcher state: watched roots, external git directories, config paths.

use super::filesystem;
use crate::sync_ext::{MutexExt, RwLockExt};
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

/// All watcher state, shared by the coordinators and filesystem watcher of one
/// process. Fields are independently locked: they are read on separate hot
/// paths and never need mutual consistency.
pub struct WatchRegistry {
    /// Watched git roots; mirrors `MultiRepoWatcher::watched_repos` keys and is
    /// used for longest-prefix event attribution.
    roots: Mutex<HashSet<PathBuf>>,
    /// See [`ExternalGitDirs`].
    external_gitdirs: RwLock<ExternalGitDirs>,
    /// See [`CommonGitDirs`].
    common_gitdirs: RwLock<CommonGitDirs>,
    /// Config-file paths classified as [`FileEvent::Config`], canonical and absolute.
    config_paths: RwLock<HashSet<PathBuf>>,
    /// The effective `git.watch_worktree` setting.
    worktree_enabled: AtomicBool,
    /// Ignore caches consulted by the filesystem layer.
    ignore: filesystem::IgnoreCaches,
}

impl WatchRegistry {
    /// An empty registry; worktree watching from `GPY_WATCH_WORKTREE`, else on.
    #[must_use]
    pub fn new() -> Self {
        Self {
            roots: Mutex::new(HashSet::new()),
            external_gitdirs: RwLock::new(ExternalGitDirs::new()),
            common_gitdirs: RwLock::new(CommonGitDirs::new()),
            config_paths: RwLock::new(HashSet::new()),
            worktree_enabled: AtomicBool::new(env_watch_worktree_override().unwrap_or(true)),
            ignore: filesystem::IgnoreCaches::default(),
        }
    }

    /// Whether worktree watching is currently in effect.
    pub(crate) fn worktree_enabled(&self) -> bool {
        self.worktree_enabled.load(Ordering::Relaxed)
    }

    /// Apply `git.watch_worktree`; `GPY_WATCH_WORKTREE` overrides it.
    pub(crate) fn set_worktree_enabled(&self, config_enabled: bool) {
        let effective = env_watch_worktree_override().unwrap_or(config_enabled);
        self.worktree_enabled.store(effective, Ordering::Relaxed);
    }

    /// The `.gitignore`/force-added caches for the watcher this registry backs.
    pub(crate) const fn ignore(&self) -> &filesystem::IgnoreCaches {
        &self.ignore
    }

    /// Record `root` as a watched git root.
    pub(crate) fn insert_root(&self, root: PathBuf) {
        self.roots.lock_or_recover().insert(root);
    }

    /// Drop every root in `roots` from the watched set, under one lock.
    pub(crate) fn remove_roots<'a>(&self, roots: impl IntoIterator<Item = &'a Path>) {
        let mut guard = self.roots.lock_or_recover();
        for root in roots {
            guard.remove(root);
        }
    }

    /// Forget every watched root.
    pub(crate) fn clear_roots(&self) {
        self.roots.lock_or_recover().clear();
    }

    /// Whether `root` is a registered watched root.
    pub(crate) fn root_is_registered(&self, root: &Path) -> bool {
        self.roots.lock_or_recover().contains(root)
    }

    /// Every watched root.
    pub(crate) fn root_snapshot(&self) -> Vec<PathBuf> {
        self.roots.lock_or_recover().iter().cloned().collect()
    }

    /// The longest watched root that is a prefix of `path`.
    pub(crate) fn longest_root_prefix(&self, path: &Path) -> Option<PathBuf> {
        let guard = self.roots.lock_or_recover();
        guard
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.as_os_str().len())
            .cloned()
    }

    /// The subset of `candidates` that are registered watched roots.
    pub(crate) fn filter_registered_roots(&self, candidates: Vec<PathBuf>) -> Vec<PathBuf> {
        let guard = self.roots.lock_or_recover();
        let registered = candidates
            .into_iter()
            .filter(|candidate| guard.contains(candidate))
            .collect();
        drop(guard);
        registered
    }

    pub(crate) fn register_config_path(&self, path: &Path) {
        let mut guard = self.config_paths.write_or_recover();
        for entry in normalize_config_paths(path) {
            guard.insert(entry);
        }
    }

    pub(crate) fn unregister_config_path(&self, path: &Path) {
        let mut guard = self.config_paths.write_or_recover();
        for entry in normalize_config_paths(path) {
            guard.remove(&entry);
        }
    }

    /// Whether `path` is a registered config path. Hot path, so no
    /// normalization: event paths are absolute and registration stores both
    /// canonical and absolute spellings.
    pub(crate) fn is_registered_config_path(&self, path: &Path) -> bool {
        self.config_paths.read_or_recover().contains(path)
    }

    /// Record an external gitdir → working-root mapping; both paths canonical.
    pub(crate) fn register_external_gitdir(&self, git_dir: &Path, git_root: &Path) {
        self.external_gitdirs
            .write_or_recover()
            .insert(git_dir.to_path_buf(), git_root.to_path_buf());
    }

    /// Remove a previously registered external gitdir mapping (no-op if absent).
    pub(crate) fn unregister_external_gitdir(&self, git_dir: &Path) {
        self.external_gitdirs.write_or_recover().remove(git_dir);
    }

    /// The working root of the longest registered external gitdir prefixing
    /// `path`, if the remainder is git-significant.
    pub(crate) fn attribute_external_gitdir(&self, path: &Path) -> Option<PathBuf> {
        let guard = self.external_gitdirs.read_or_recover();
        let (git_dir, git_root) = guard
            .iter()
            .filter(|(git_dir, _)| path.starts_with(git_dir))
            .max_by_key(|(git_dir, _)| git_dir.as_os_str().len())?;
        let relative = path.strip_prefix(git_dir).ok()?;
        let is_significant = is_git_significant_relative(relative);
        let root = git_root.clone();
        drop(guard);
        is_significant.then_some(root)
    }

    /// Record that `git_root` (a canonical linked-worktree working root) depends on
    /// the shared metadata in `common_dir` (canonical). Idempotent.
    pub(crate) fn register_common_gitdir(&self, common_dir: &Path, git_root: &Path) {
        self.common_gitdirs
            .write_or_recover()
            .entry(common_dir.to_path_buf())
            .or_default()
            .insert(git_root.to_path_buf());
    }

    /// Drop one dependent working root, removing the whole entry once the last one
    /// is gone (no-op if absent).
    pub(crate) fn unregister_common_gitdir(&self, common_dir: &Path, git_root: &Path) {
        let mut guard = self.common_gitdirs.write_or_recover();
        if let Some(dependents) = guard.get_mut(common_dir) {
            dependents.remove(git_root);
            if dependents.is_empty() {
                guard.remove(common_dir);
            }
        }
    }

    /// Linked-worktree roots that must refresh because shared metadata at `path`
    /// changed. Longest common-dir prefix wins, so a submodule's beats its super's.
    pub(crate) fn attribute_common_gitdir(&self, path: &Path) -> Vec<PathBuf> {
        let guard = self.common_gitdirs.read_or_recover();
        let Some((common_dir, dependents)) = guard
            .iter()
            .filter(|(common_dir, _)| path.starts_with(common_dir))
            .max_by_key(|(common_dir, _)| common_dir.as_os_str().len())
        else {
            return Vec::new();
        };
        let Ok(relative) = path.strip_prefix(common_dir) else {
            return Vec::new();
        };
        if !is_common_gitdir_significant_relative(relative) {
            return Vec::new();
        }
        let attributed = dependents.iter().cloned().collect();
        drop(guard);
        attributed
    }

    /// Whether `path` lies under any registered common git directory.
    pub(crate) fn is_under_registered_common_gitdir(&self, path: &Path) -> bool {
        self.common_gitdirs
            .read_or_recover()
            .keys()
            .any(|common_dir| path.starts_with(common_dir))
    }

    /// Parent repository roots whose gitlink status can change because
    /// submodule metadata at `path` changed; empty outside submodule gitdirs.
    /// Includes registered intermediate parents for nested submodules.
    pub(crate) fn attribute_submodule_parents(&self, path: &Path) -> Vec<PathBuf> {
        let Some((super_root, modules_root)) = split_at_git_dir_modules(path) else {
            return Vec::new();
        };

        let Some(owning_gitdir) = owning_submodule_gitdir(path, &modules_root) else {
            return Vec::new();
        };

        let mut parents = vec![super_root];
        let guard = self.external_gitdirs.read_or_recover();
        parents.extend(
            guard
                .iter()
                .filter(|(git_dir, _)| {
                    // Proper ancestor only: the owning gitdir is delivered by
                    // `attribute_external_gitdir`.
                    owning_gitdir.starts_with(git_dir) && git_dir.as_path() != owning_gitdir
                })
                .map(|(_, git_root)| git_root.clone()),
        );
        drop(guard);
        parents
    }
}

impl Default for WatchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse `GPY_WATCH_WORKTREE`: `None` when unset, `Some(false)` for falsey spellings.
fn env_watch_worktree_override() -> Option<bool> {
    env::var("GPY_WATCH_WORKTREE").ok().map(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
    })
}

fn normalize_config_paths(path: &Path) -> Vec<PathBuf> {
    let mut normalized = Vec::with_capacity(2);
    let absolute = crate::paths::absolutize(path);

    if let Ok(canonical) = absolute.canonicalize() {
        normalized.push(canonical);
    }

    if !normalized.iter().any(|candidate| candidate == &absolute) {
        normalized.push(absolute);
    }

    normalized
}

/// Maps a canonical external git directory (submodule or linked worktree) to its
/// working root. Recorded at registration because the gitdir name need not match
/// the working path, so it cannot be derived from event paths.
type ExternalGitDirs = std::collections::HashMap<PathBuf, PathBuf>;

/// Whether `relative` (a path inside a git directory) names metadata whose
/// change should refresh the prompt.
pub(super) fn is_git_significant_relative(relative: &Path) -> bool {
    // `refs/tags/` is excluded: no segment displays tags.
    if relative.starts_with("refs/heads")
        || relative.starts_with("refs/remotes")
        || relative == Path::new("refs/stash")
    {
        return true;
    }
    // Per-repo `.gitignore` equivalent.
    if relative == Path::new("info/exclude") {
        return true;
    }
    // Rebase operation dirs plus only the files the status readers use; the
    // whole subtree is rewritten on every step (`git am` once per patch).
    // `git-rebase-todo` matters because `--edit-todo` changes the step total.
    if relative == Path::new("rebase-merge")
        || relative == Path::new("rebase-merge/msgnum")
        || relative == Path::new("rebase-merge/end")
        || relative == Path::new("rebase-merge/git-rebase-todo")
        || relative == Path::new("rebase-apply")
        || relative == Path::new("rebase-apply/applying")
    {
        return true;
    }
    // `get_repo_state` reads `sequencer/todo`; reftable repos rewrite
    // `tables.list` on every ref transaction.
    if relative == Path::new("sequencer")
        || relative == Path::new("sequencer/todo")
        || relative == Path::new("reftable/tables.list")
    {
        return true;
    }
    // Remaining significant files sit directly under the git dir.
    let is_top_level = relative.parent().is_none_or(|p| p.as_os_str().is_empty());
    if !is_top_level {
        return false;
    }
    matches!(
        relative.file_name().and_then(std::ffi::OsStr::to_str),
        Some(
            "HEAD"
                | "index"
                | "config"
                | "MERGE_HEAD"
                | "REBASE_HEAD"
                | "CHERRY_PICK_HEAD"
                | "REVERT_HEAD"
                | "BISECT_LOG"
                | "ORIG_HEAD"
                | "packed-refs"
                | "FETCH_HEAD"
        )
    )
}

/// Maps a canonical common git directory to the working roots of every
/// registered linked worktree sharing it. Kept separate from
/// [`ExternalGitDirs`] so a per-worktree `HEAD` write can never fan out to
/// siblings.
type CommonGitDirs = std::collections::HashMap<PathBuf, std::collections::HashSet<PathBuf>>;

/// Whether `relative` (inside a common git directory) names state shared by all
/// worktrees: branch/remote refs, `packed-refs`, `config`. `HEAD` and `index`
/// are per-checkout and excluded, else every `git add` would wake every worktree.
pub(super) fn is_common_gitdir_significant_relative(relative: &Path) -> bool {
    if relative.starts_with("refs/heads") || relative.starts_with("refs/remotes") {
        return true;
    }
    let is_top_level = relative.parent().is_none_or(|p| p.as_os_str().is_empty());
    is_top_level
        && matches!(
            relative.file_name().and_then(std::ffi::OsStr::to_str),
            Some("packed-refs" | "config")
        )
}

/// Split `path` at its first `.git`/`modules` pair into `(super_root,
/// modules_root)`. Only submodules produce this shape; nested repos and linked
/// worktrees (`.git/worktrees`) cannot, so the parent fan-out stays structural.
pub(super) fn split_at_git_dir_modules(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let components: Vec<_> = path.components().collect();
    let git_index = components.windows(2).position(|window| {
        window
            .first()
            .is_some_and(|component| component.as_os_str() == OsStr::new(".git"))
            && window
                .get(1)
                .is_some_and(|component| component.as_os_str() == OsStr::new("modules"))
    })?;
    let super_root: PathBuf = components.iter().take(git_index).collect();
    let modules_root: PathBuf = components
        .iter()
        .take(git_index.saturating_add(2))
        .collect();
    Some((super_root, modules_root))
}

/// The git directory of the submodule whose metadata `path` names, or `None`
/// for internal churn. Submodule names may contain `/`, so ancestors are
/// scanned deepest-first for one whose remainder is git-significant. A
/// `<gitdir>/logs` match is tolerated: reflog writes coalesce with the ref update.
fn owning_submodule_gitdir(path: &Path, modules_root: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors().skip(1) {
        if ancestor == modules_root || !ancestor.starts_with(modules_root) {
            return None;
        }
        if let Ok(relative) = path.strip_prefix(ancestor)
            && is_git_significant_relative(relative)
        {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}
