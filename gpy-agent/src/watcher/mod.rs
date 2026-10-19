//! File system watching for live prompt updates.
//!
//! Pipeline: `filesystem::FileSystemWatcher` (raw notify events) →
//! `debouncer::DebounceEngine` (coalescing) → `WatchCoordinator`.
//!
//! A flush thread wakes every debounce interval, drains expired events under
//! the debouncer lock, and hands each to its own dispatch thread, so a slow
//! callback for one repo never stalls ingestion or delivery for others. The
//! timer thread is needed because expiry must happen even when events stop.
//!
//! Entries flush once quiet for the debounce window (default 100ms,
//! `GPY_DEBOUNCE_MS`) or once older than the max-delay cap (1s), so sustained
//! churn cannot starve flushes. The agent's separate cache cooldown (150ms)
//! skips redundant git scans when an IPC request already refreshed the cache.

pub mod debouncer;
pub mod filesystem;
pub(crate) mod hot_reload;
pub mod multi_repo;
mod watch_set;

use crate::Result;
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Which paths one Git event covers. Coalesced bursts accumulate paths up to
/// [`GitPaths::MAX_PATHS`], then degrade to [`GitPaths::WholeRepo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitPaths {
    /// Changed paths, deduplicated, in observed order; never empty.
    Paths(Vec<PathBuf>),
    /// Whole repository changed (overflow, rescan, or too many paths).
    WholeRepo,
}

impl GitPaths {
    /// Most paths one event may accumulate before degrading to `WholeRepo`.
    pub const MAX_PATHS: usize = 32;

    /// The single path a raw filesystem event carries.
    #[must_use]
    pub fn single(path: PathBuf) -> Self {
        Self::Paths(vec![path])
    }

    /// The accumulated paths, or `None` for a whole-repository event.
    #[must_use]
    pub fn as_slice(&self) -> Option<&[PathBuf]> {
        match self {
            Self::Paths(paths) => Some(paths),
            Self::WholeRepo => None,
        }
    }

    /// Fold `other` into `self`, deduplicating and degrading to `WholeRepo`
    /// past [`GitPaths::MAX_PATHS`]. A `Vec` keeps order deterministic.
    pub fn merge(&mut self, other: &Self) {
        let (Self::Paths(existing), Self::Paths(incoming)) = (&mut *self, other) else {
            *self = Self::WholeRepo;
            return;
        };
        for path in incoming {
            if existing.contains(path) {
                continue;
            }
            if existing.len() >= Self::MAX_PATHS {
                *self = Self::WholeRepo;
                return;
            }
            existing.push(path.clone());
        }
    }
}

/// File system events that should trigger prompt updates
#[derive(Debug, Clone)]
pub enum FileEvent {
    /// Git repository changed
    Git {
        /// Paths that triggered the update.
        paths: GitPaths,
    },
    /// Language files changed (package.json, Cargo.toml, etc.)
    Language {
        /// Path that triggered the update.
        path: PathBuf,
    },
    /// Configuration changed
    Config {
        /// Path that triggered the update.
        path: PathBuf,
    },
    /// Theme file changed
    Theme {
        /// Path that triggered the update.
        path: PathBuf,
    },
}

/// Pending event emitted by the filesystem filter before debouncing.
#[derive(Debug, Clone)]
pub struct PendingEvent {
    /// Original filesystem event.
    pub event: FileEvent,
    /// Canonical repository root associated with the event.
    pub repo: PathBuf,
}

impl PendingEvent {
    /// A `Git` pending event covering the single changed `path`.
    #[must_use]
    pub fn git(repo: PathBuf, path: PathBuf) -> Self {
        Self {
            event: FileEvent::Git {
                paths: GitPaths::single(path),
            },
            repo,
        }
    }

    /// A `Git` pending event covering the whole repository (rescan).
    #[must_use]
    pub const fn git_whole_repo(repo: PathBuf) -> Self {
        Self {
            event: FileEvent::Git {
                paths: GitPaths::WholeRepo,
            },
            repo,
        }
    }
}

/// Debounced event delivered to the agent.
#[derive(Debug, Clone)]
pub struct DebouncedEvent {
    /// Original filesystem event.
    pub event: FileEvent,
    /// Canonical repository root associated with the event.
    pub repo: PathBuf,
}

impl DebouncedEvent {
    /// Whether every path behind this event lies in a `gpy/themes` tree, which
    /// the theme watcher alone reloads.
    #[must_use]
    pub fn is_theme_only(&self) -> bool {
        match &self.event {
            FileEvent::Theme { .. } => true,
            FileEvent::Git {
                paths: GitPaths::Paths(paths),
            } => paths
                .iter()
                .all(|path| !path_has_git_component(path) && is_in_gpy_themes(path)),
            FileEvent::Git {
                paths: GitPaths::WholeRepo,
            }
            | FileEvent::Language { .. }
            | FileEvent::Config { .. } => false,
        }
    }
}

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
        self.roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(root);
    }

    /// Drop every root in `roots` from the watched set, under one lock.
    pub(crate) fn remove_roots<'a>(&self, roots: impl IntoIterator<Item = &'a Path>) {
        let mut guard = self.roots.lock().unwrap_or_else(PoisonError::into_inner);
        for root in roots {
            guard.remove(root);
        }
    }

    /// Forget every watched root.
    pub(crate) fn clear_roots(&self) {
        self.roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Whether `root` is a registered watched root.
    pub(crate) fn root_is_registered(&self, root: &Path) -> bool {
        self.roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(root)
    }

    /// Every watched root.
    pub(crate) fn root_snapshot(&self) -> Vec<PathBuf> {
        self.roots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// The longest watched root that is a prefix of `path`.
    pub(crate) fn longest_root_prefix(&self, path: &Path) -> Option<PathBuf> {
        let guard = self.roots.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.as_os_str().len())
            .cloned()
    }

    /// The subset of `candidates` that are registered watched roots.
    pub(crate) fn filter_registered_roots(&self, candidates: Vec<PathBuf>) -> Vec<PathBuf> {
        let guard = self.roots.lock().unwrap_or_else(PoisonError::into_inner);
        let registered = candidates
            .into_iter()
            .filter(|candidate| guard.contains(candidate))
            .collect();
        drop(guard);
        registered
    }

    pub(crate) fn register_config_path(&self, path: &Path) {
        let mut guard = self
            .config_paths
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        for entry in normalize_config_paths(path) {
            guard.insert(entry);
        }
    }

    pub(crate) fn unregister_config_path(&self, path: &Path) {
        let mut guard = self
            .config_paths
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        for entry in normalize_config_paths(path) {
            guard.remove(&entry);
        }
    }

    /// Whether `path` is a registered config path. Hot path, so no
    /// normalization: event paths are absolute and registration stores both
    /// canonical and absolute spellings.
    pub(crate) fn is_registered_config_path(&self, path: &Path) -> bool {
        self.config_paths
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(path)
    }

    /// Record an external gitdir → working-root mapping; both paths canonical.
    pub(crate) fn register_external_gitdir(&self, git_dir: &Path, git_root: &Path) {
        self.external_gitdirs
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(git_dir.to_path_buf(), git_root.to_path_buf());
    }

    /// Remove a previously registered external gitdir mapping (no-op if absent).
    pub(crate) fn unregister_external_gitdir(&self, git_dir: &Path) {
        self.external_gitdirs
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(git_dir);
    }

    /// The working root of the longest registered external gitdir prefixing
    /// `path`, if the remainder is git-significant.
    pub(crate) fn attribute_external_gitdir(&self, path: &Path) -> Option<PathBuf> {
        let guard = self
            .external_gitdirs
            .read()
            .unwrap_or_else(PoisonError::into_inner);
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
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(common_dir.to_path_buf())
            .or_default()
            .insert(git_root.to_path_buf());
    }

    /// Drop one dependent working root, removing the whole entry once the last one
    /// is gone (no-op if absent).
    pub(crate) fn unregister_common_gitdir(&self, common_dir: &Path, git_root: &Path) {
        let mut guard = self
            .common_gitdirs
            .write()
            .unwrap_or_else(PoisonError::into_inner);
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
        let guard = self
            .common_gitdirs
            .read()
            .unwrap_or_else(PoisonError::into_inner);
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
            .read()
            .unwrap_or_else(PoisonError::into_inner)
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
        let guard = self
            .external_gitdirs
            .read()
            .unwrap_or_else(PoisonError::into_inner);
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
fn is_git_significant_relative(relative: &Path) -> bool {
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
fn is_common_gitdir_significant_relative(relative: &Path) -> bool {
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
fn split_at_git_dir_modules(path: &Path) -> Option<(PathBuf, PathBuf)> {
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

/// Callback invoked once an event has passed through debouncing.
pub type EventCallback = Box<dyn Fn(DebouncedEvent) + Send + Sync>;

/// Callback invoked before debouncing (raw pending events).
pub type PendingCallback = Box<dyn Fn(PendingEvent) + Send + Sync>;

/// Schedules a [`PendingEvent`] for delivery no earlier than `due`, coalescing
/// like an immediate event. Synchronous and cheap; the flush thread waits.
pub type DelayedEventScheduler = Arc<dyn Fn(PendingEvent, Instant) + Send + Sync>;

/// The longest a flush thread sleeps before re-checking its stop flag.
const FLUSH_STOP_POLL: Duration = Duration::from_millis(25);

/// Sleep for `total`, waking early once `stop` is set.
pub(crate) fn sleep_unless_stopped(total: Duration, stop: &AtomicBool) {
    let deadline = Instant::now().checked_add(total);
    while !stop.load(Ordering::Relaxed) {
        let remaining = deadline.map_or(total, |at| at.saturating_duration_since(Instant::now()));
        if remaining.is_zero() {
            return;
        }
        std::thread::sleep(remaining.min(FLUSH_STOP_POLL));
    }
}

/// Single-repository file system watcher with debouncing.
pub struct WatchCoordinator {
    watcher: Option<filesystem::FileSystemWatcher>,
    debouncer: Arc<Mutex<debouncer::DebounceEngine>>,
    callback: Arc<EventCallback>,
    stop_flag: Arc<AtomicBool>,
    flush_handle: Option<JoinHandle<()>>,
    /// Registry shared with the filesystem layer.
    registry: Arc<WatchRegistry>,

    /// Every unwatched path, in call order.
    #[cfg(test)]
    unwatch_calls: Mutex<Vec<PathBuf>>,
}

impl WatchCoordinator {
    /// Create a new file watcher with callback. `registry: None` gives a
    /// private, empty registry.
    ///
    /// # Errors
    /// Returns an error if the file system watcher cannot be created.
    pub fn new(
        debounce: Duration,
        callback: EventCallback,
        registry: Option<Arc<WatchRegistry>>,
    ) -> Result<Self> {
        let shared_registry = registry.unwrap_or_else(|| Arc::new(WatchRegistry::new()));
        let event_debouncer = debouncer::DebounceEngine::new(debounce);
        let callback_arc = Arc::new(callback);

        // Feed watcher events into the debouncer.
        let debouncer_arc = Arc::new(Mutex::new(event_debouncer));
        let debouncer_for_watcher = Arc::clone(&debouncer_arc);
        let debounce_interval = debounce;

        let ingress_callback: PendingCallback = Box::new(move |pending| {
            debouncer_for_watcher
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .handle_event(pending);
        });

        // Directory-create follow-up uses a due-time floor in the same
        // debouncer instead of a timer thread per directory.
        let debouncer_for_schedule = Arc::clone(&debouncer_arc);
        let schedule_delayed: DelayedEventScheduler = Arc::new(move |pending, due| {
            debouncer_for_schedule
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .handle_event_due(pending, due);
        });

        // Follow-up delay = debounce window, so contents created before the
        // new directory's watch was armed still get rescanned.
        let watcher = filesystem::FileSystemWatcher::new(
            ingress_callback,
            Some(Arc::clone(&shared_registry)),
            debounce,
            schedule_delayed,
        )?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_thread = Arc::clone(&stop_flag);
        let debouncer_for_thread = Arc::clone(&debouncer_arc);
        let callback_for_thread = Arc::clone(&callback_arc);
        let flush_handle = std::thread::spawn(move || {
            while !stop_flag_thread.load(Ordering::Relaxed) {
                // Short sleep slices keep `stop` fast: a long debounce would
                // otherwise exceed the CLI's 2s reload budget.
                sleep_unless_stopped(debounce_interval, &stop_flag_thread);
                if stop_flag_thread.load(Ordering::Relaxed) {
                    break;
                }
                let expired = debouncer_for_thread
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .drain_expired_events();
                // Dispatch each event on its own thread, off the flush loop
                // and outside the debouncer lock, so a slow callback for one
                // repo cannot delay flushing or delivery for another.
                for event in expired {
                    let callback_for_event = Arc::clone(&callback_for_thread);
                    std::thread::spawn(move || {
                        (callback_for_event)(event);
                    });
                }
            }
        });

        Ok(Self {
            watcher: Some(watcher),
            debouncer: debouncer_arc,
            callback: callback_arc,
            stop_flag,
            flush_handle: Some(flush_handle),
            registry: shared_registry,
            #[cfg(test)]
            unwatch_calls: Mutex::new(Vec::new()),
        })
    }

    /// The [`WatchRegistry`] this coordinator and its filesystem layer share.
    #[must_use]
    pub const fn registry(&self) -> &Arc<WatchRegistry> {
        &self.registry
    }

    /// Start watching a directory
    ///
    /// # Errors
    /// Returns an error if the directory cannot be watched.
    pub fn watch_directory<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if let Some(watcher) = &mut self.watcher {
            watcher.watch(path)?;
        }
        Ok(())
    }

    /// Start watching a directory non-recursively.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be watched.
    pub fn watch_directory_shallow<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if let Some(watcher) = &mut self.watcher {
            watcher.watch_shallow(path)?;
        }
        Ok(())
    }

    /// Whether an already-armed watch delivers events for `path`
    /// (see [`filesystem::FileSystemWatcher::covers`]).
    #[must_use]
    pub fn covers_directory<P: AsRef<Path>>(&self, path: P) -> bool {
        self.watcher
            .as_ref()
            .is_some_and(|watcher| watcher.covers(path))
    }

    /// Release one recursive [`WatchCoordinator::watch_directory`] of `path`
    /// (see [`filesystem::FileSystemWatcher::unwatch`]).
    ///
    /// # Errors
    /// Returns an error if the directory cannot be unwatched.
    pub fn unwatch_directory<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        #[cfg(test)]
        self.unwatch_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.as_ref().to_path_buf());
        if let Some(watcher) = &mut self.watcher {
            watcher.unwatch(path)?;
        }
        Ok(())
    }

    /// Release one [`WatchCoordinator::watch_directory_shallow`] of `path`
    /// (see [`filesystem::FileSystemWatcher::unwatch_shallow`]).
    ///
    /// # Errors
    /// Returns an error if the directory cannot be unwatched.
    pub fn unwatch_directory_shallow<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        #[cfg(test)]
        self.unwatch_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.as_ref().to_path_buf());
        if let Some(watcher) = &mut self.watcher {
            watcher.unwatch_shallow(path)?;
        }
        Ok(())
    }

    /// Every path this instance was asked to unwatch, in call order.
    #[cfg(test)]
    pub fn unwatch_calls(&self) -> Vec<PathBuf> {
        self.unwatch_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Logical registrations (with owner counts) and the OS watches beneath them.
    #[cfg(test)]
    pub(crate) fn held_watches(&self) -> filesystem::HeldWatches {
        self.watcher.as_ref().map_or_else(
            Default::default,
            filesystem::FileSystemWatcher::held_watches,
        )
    }

    /// Stop all watching
    pub fn stop(&mut self) {
        if let Some(watcher) = &mut self.watcher {
            watcher.stop();
        }
        self.watcher = None;

        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.flush_handle.take() {
            let _ = handle.join();
        }

        self.debouncer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Deliver pending debounced events synchronously; the callback runs
    /// after the debouncer lock is released.
    pub fn flush_pending_events(&self) {
        let expired = self
            .debouncer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain_expired_events();
        for event in expired {
            (self.callback)(event);
        }
    }
}

impl Drop for WatchCoordinator {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Determine if a file path should trigger prompt updates.
#[must_use]
pub fn should_trigger_update(path: &Path, registry: &WatchRegistry) -> Option<PendingEvent> {
    filter_ignored(path)?;
    let event = classify_event(path, registry)?;
    let repo = multi_repo::MultiRepoWatcher::find_git_root(path)
        .or_else(|| path.parent().map(Path::to_path_buf))?;

    Some(PendingEvent { event, repo })
}

/// Whether `path` has a `.git` component immediately followed by
/// `subdir_components`. Component-based so it works with Windows separators.
fn path_contains_git_subdir(path: &Path, subdir_components: &[&str]) -> bool {
    let components: Vec<_> = path.components().collect();
    let window_len = subdir_components.len().saturating_add(1);
    components.windows(window_len).any(|window| {
        window
            .first()
            .is_some_and(|head| head.as_os_str() == OsStr::new(".git"))
            && window.get(1..).is_some_and(|rest| {
                rest.iter()
                    .zip(subdir_components)
                    .all(|(component, expected)| component.as_os_str() == OsStr::new(*expected))
            })
    })
}

fn filter_ignored(path: &Path) -> Option<()> {
    // Match file names, not path substrings (`foo.socket.rs` must pass).
    if path.file_name() == Some(OsStr::new("gpy-agent.log")) {
        return None;
    }
    if path.extension() == Some(OsStr::new("sock")) {
        return None;
    }

    // `.git/info/` stays unfiltered: `info/exclude` is git-significant.
    if path_contains_git_subdir(path, &["objects"])
        || path_contains_git_subdir(path, &["logs"])
        || path_contains_git_subdir(path, &["hooks"])
        || path_contains_git_subdir(path, &["refs", "tags"])
    {
        return None;
    }

    Some(())
}

/// Whether `path` has a `.git` component anywhere.
fn path_has_git_component(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == OsStr::new(".git"))
}

/// Whether `path` has consecutive components `first` then `second`.
fn path_has_consecutive_components(path: &Path, first: &str, second: &str) -> bool {
    let components: Vec<_> = path.components().collect();
    components.windows(2).any(|window| {
        window
            .first()
            .is_some_and(|component| component.as_os_str() == OsStr::new(first))
            && window
                .get(1)
                .is_some_and(|component| component.as_os_str() == OsStr::new(second))
    })
}

/// Whether `path` ends with components `first` then `second`.
fn path_ends_with_components(path: &Path, first: &str, second: &str) -> bool {
    let mut components = path.components().rev();
    let Some(last) = components.next() else {
        return false;
    };
    let Some(second_last) = components.next() else {
        return false;
    };
    last.as_os_str() == OsStr::new(second) && second_last.as_os_str() == OsStr::new(first)
}

/// The components after the last `.git` component, or `None` without one.
fn path_after_last_git_component(path: &Path) -> Option<PathBuf> {
    let last_git_index = path
        .components()
        .enumerate()
        .filter(|(_, component)| component.as_os_str() == OsStr::new(".git"))
        .map(|(index, _)| index)
        .last()?;
    Some(
        path.components()
            .skip(last_git_index.saturating_add(1))
            .collect(),
    )
}

/// Whether `path` is a `gpy/themes` directory or lies under one.
fn is_in_gpy_themes(path: &Path) -> bool {
    path_has_consecutive_components(path, "gpy", "themes")
        || path_ends_with_components(path, "gpy", "themes")
}

fn classify_event(path: &Path, registry: &WatchRegistry) -> Option<FileEvent> {
    if registry.is_registered_config_path(path) {
        return Some(FileEvent::Config {
            path: path.to_path_buf(),
        });
    }

    let file_name = path.file_name()?.to_str()?;

    let is_in_git_dir = path_has_git_component(path);

    // Theme saves often go via temp file + rename, so any change under
    // gpy/themes invalidates.
    if !is_in_git_dir && is_in_gpy_themes(path) {
        return Some(FileEvent::Theme {
            path: path.to_path_buf(),
        });
    }

    // In-tree `.git/…` metadata; external gitdirs go through
    // `attribute_external_gitdir`.
    if is_in_git_dir
        && let Some(relative) = path_after_last_git_component(path)
        && is_git_significant_relative(&relative)
    {
        return Some(FileEvent::Git {
            paths: GitPaths::single(path.to_path_buf()),
        });
    }

    let event = if is_language_cache_trigger(file_name) {
        FileEvent::Language {
            path: path.to_path_buf(),
        }
    } else if file_name.ends_with("gpy.toml") || file_name.eq_ignore_ascii_case("config.toml") {
        FileEvent::Config {
            path: path.to_path_buf(),
        }
    } else {
        return None;
    };

    Some(event)
}

/// Lockfiles and version pins that invalidate the language cache but are not
/// language-detection markers.
const LANGUAGE_CACHE_LOCKFILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "rust-toolchain",
    "rust-toolchain.toml",
    "go.work",
    ".tool-versions",
    ".python-version",
    ".node-version",
    ".nvmrc",
    "mise.toml",
    ".mise.toml",
    "mise.local.toml",
    ".mise.local.toml",
    "poetry.lock",
    "Gemfile.lock",
    "mix.lock",
    "Package.resolved",
    "settings.gradle",
];

/// Whether a change to `file_name` should invalidate the language cache
/// (detection markers plus [`LANGUAGE_CACHE_LOCKFILES`]). Case-sensitive.
fn is_language_cache_trigger(file_name: &str) -> bool {
    crate::language::metadata::marker_file_names().contains(file_name)
        || LANGUAGE_CACHE_LOCKFILES.contains(&file_name)
}
/// Watcher tuning parsed at startup.
#[derive(Clone, Copy, Debug)]
pub struct WatcherConfig {
    debounce: u64,
    throttle: u64,
}

impl WatcherConfig {
    /// Create a new watcher configuration with explicit values in milliseconds.
    #[must_use]
    pub const fn new(debounce_ms: u64, throttle_ms: u64) -> Self {
        Self {
            debounce: debounce_ms,
            throttle: throttle_ms,
        }
    }

    /// Load watcher configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        fn parse_env(name: &str, default: u64) -> u64 {
            env::var(name)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(default)
        }

        let debounce = parse_env("GPY_DEBOUNCE_MS", 100);
        let throttle = parse_env("GPY_SIGURG_THROTTLE_MS", 150);

        Self::new(debounce, throttle)
    }

    /// Debounce duration as a `Duration`.
    #[must_use]
    pub const fn debounce_duration(self) -> Duration {
        Duration::from_millis(self.debounce)
    }

    /// Per-repository throttle duration in milliseconds.
    #[must_use]
    pub const fn throttle_ms(self) -> u64 {
        self.throttle
    }
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self::new(100, 150)
    }
}

#[cfg(test)]
mod tests;
