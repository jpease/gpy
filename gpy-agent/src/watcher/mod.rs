//! File system watching for live prompt updates
//!
//! Uses the notify crate with debouncing to detect changes that should
//! trigger prompt updates via the SIGURG repaint doorbell.
//!
//! # Architecture Overview
//!
//! This module implements a three-layer architecture for event processing:
//!
//! 1. **Filesystem Layer** (`filesystem::FileSystemWatcher`) - Raw file events from notify
//! 2. **Debouncing Layer** (`debouncer::DebounceEngine`) - Coalesces rapid events
//! 3. **Coordination Layer** (`WatchCoordinator`) - Orchestrates the pipeline
//!
//! ## Threading Model
//!
//! The watcher uses **one dedicated background thread** for debounce flushing,
//! plus one short-lived dispatch thread per delivered event:
//!
//! - **Main thread**: Receives filesystem events from notify, filters them, feeds debouncer
//! - **Flush thread**: Wakes every `debounce_interval` (default 100ms), locks the debouncer
//!   just long enough to drain expired events into a `Vec`, then unlocks and returns to
//!   sleeping. It never touches git/language work itself.
//! - **Dispatch threads**: One per drained event, spawned by the flush thread to invoke the
//!   agent callback (which may run unbounded git/language scans). Because dispatch happens
//!   off the flush thread and outside the debouncer lock, a slow callback for one repo can
//!   neither stall event ingestion for other repos nor delay their delivery (#306).
//!
//! **Why a dedicated flush thread?** The flush operation must happen on a timer independently
//! of incoming events. Without it, if events stop arriving, pending events would never
//! expire. The thread is lightweight (sleep-based) and only acquires the debouncer lock
//! briefly every 100ms to drain — never while a callback runs.
//!
//! ## Debouncing Strategy
//!
//! **Goal**: Prevent prompt flicker from rapid file changes (e.g., `git commit` writes
//! multiple files in quick succession).
//!
//! **Mechanism**: The `debouncer::DebounceEngine` holds events in a HashMap keyed by
//! `(event kind, repo)`. When an event arrives:
//! 1. If no entry exists for this key, create one with `first_seen = last_seen = now`.
//! 2. If an entry exists, update `last_seen = now` (its `first_seen` is preserved).
//!
//! An entry may additionally carry a due-time floor, which holds it back until that instant
//! whatever the two rules below say. Only the directory-create follow-up sets one (#416), and
//! that is what keeps the follow-up's delay inside this engine instead of on a timer thread of
//! its own: N directories created in one repository become one held entry rather than N
//! sleeping threads, and `stop` clears them like anything else pending (#569).
//!
//! The flush thread periodically drains entries whose quiet gap (`now - last_seen`) has
//! reached the debounce window, **or** whose total age (`now - first_seen`) has reached the
//! max-delay cap (and whose floor, if any, has passed). The cap exists because a pure
//! quiet-gap check can be reset indefinitely by sustained churn (e.g. a large rebase, or a
//! build emitting non-gitignored output faster than the debounce window), which would
//! otherwise starve flushes entirely (#322).
//!
//! **Default timing**: 100ms debounce window, 1s max-delay cap
//! - Debounce window fast enough to feel responsive (< 200ms)
//! - Long enough to coalesce most multi-file git operations
//! - Max-delay cap guarantees a flush within ~1s even under unbroken churn
//!
//! **Tuning**: Debounce window via the `GPY_DEBOUNCE_MS` environment variable
//!
//! ## Cache Cooldown vs Debouncing
//!
//! Two separate timing mechanisms work together:
//!
//! - **Debouncing** (100ms): Prevents redundant signals to Fish processes
//! - **Cache cooldown** (150ms): Prevents redundant git queries after watcher events
//!
//! When a file event triggers, we check if the cache was updated within the cooldown
//! window. If so, we skip re-running git status and just send signals. This avoids
//! the race where a watcher event arrives before the IPC request that triggered the
//! change has updated the cache.
//!
//! ## Why This Design?
//!
//! **Alternative considered**: Use tokio tasks instead of threads
//! - **Rejected because**: The notify crate's event channel is synchronous, not async.
//!   Bridging sync→async adds complexity without benefit.
//!
//! **Alternative considered**: Single-shot timers per event
//! - **Rejected because**: Would spawn many short-lived timers for busy repos.
//!   A single periodic flush thread is simpler and more efficient.

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
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Which paths one Git event covers.
///
/// A raw watcher event always carries a single path ([`GitPaths::single`]).
/// [`DebounceEngine`](debouncer::DebounceEngine) folds every path observed for
/// one repository inside a debounce window into the same event, so a coalesced
/// burst can no longer discard all but the last change (#466).
///
/// Accumulation is bounded by [`GitPaths::MAX_PATHS`]; past the bound the event
/// degrades to [`GitPaths::WholeRepo`], which the agent services with a full
/// scan. That keeps a pending entry's memory flat under churn, and past a few
/// dozen paths a full scan is the cheaper answer anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitPaths {
    /// The specific changed paths, deduplicated and in the order observed.
    /// Never empty, never longer than [`GitPaths::MAX_PATHS`].
    Paths(Vec<PathBuf>),
    /// The whole repository changed: accumulation exceeded
    /// [`GitPaths::MAX_PATHS`], or a backend-reported overflow/rescan (#616)
    /// could not name which paths changed. [`PendingEvent::git_whole_repo`]
    /// is the constructor a rescan uses; the root-path handling in
    /// `agent::events::git_paths_hint` remains as defence for any other
    /// root-path `Git` event, not as the rescan's own shape.
    WholeRepo,
}

impl GitPaths {
    /// Most paths one event may accumulate before it degrades to
    /// [`GitPaths::WholeRepo`].
    pub const MAX_PATHS: usize = 32;

    /// The single path a raw filesystem event carries.
    #[must_use]
    pub fn single(path: PathBuf) -> Self {
        Self::Paths(vec![path])
    }

    /// The accumulated paths, or `None` for a whole-repository event — the same
    /// "`None` means full scan" shape the agent's `paths_hint` already uses.
    #[must_use]
    pub fn as_slice(&self) -> Option<&[PathBuf]> {
        match self {
            Self::Paths(paths) => Some(paths),
            Self::WholeRepo => None,
        }
    }

    /// Fold `other` into `self`, deduplicating paths and degrading to
    /// [`GitPaths::WholeRepo`] once either side is repo-wide or the merged set
    /// would exceed [`GitPaths::MAX_PATHS`].
    ///
    /// Linear `contains` rather than a `HashSet`: the set is capped at
    /// [`GitPaths::MAX_PATHS`], and a `Vec` keeps the order deterministic for
    /// both the emitted pathspec and the tests.
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
        /// Paths that triggered the update. Exactly one as emitted by the
        /// filesystem filter; possibly several after debounce coalescing.
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
    /// A `Git` pending event covering the single changed `path`, as emitted
    /// by every ordinary filesystem-filter classification/attribution arm.
    #[must_use]
    pub fn git(repo: PathBuf, path: PathBuf) -> Self {
        Self {
            event: FileEvent::Git {
                paths: GitPaths::single(path),
            },
            repo,
        }
    }

    /// A `Git` pending event covering the whole repository, as emitted by a
    /// rescan (#616): a backend-reported overflow can't identify which paths
    /// changed, so there is nothing to name — the repo just gets a full scan.
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

/// Every piece of watcher state that used to live in a process-global
/// `OnceLock`, owned by whoever built the watcher instead (#617).
///
/// One registry is shared by a [`WatchCoordinator`], the
/// [`filesystem::FileSystemWatcher`] beneath it, and — in the agent — by the
/// repo, config and theme coordinators alike, which is what preserves the
/// old one-process-one-registry semantics exactly. Two watchers built
/// without an explicit registry get one each and share nothing, so tests no
/// longer have to serialise on a process-wide `AtomicBool`/`HashMap`.
///
/// All five pieces are behind their own lock rather than one lock over a
/// struct: they are read on independent hot paths (`classify_event` reads
/// `config_paths` for every event; `attribute_repo_detailed` reads `roots`;
/// the gitdir maps are read only for repositories that have submodules or
/// linked worktrees), and nothing ever needs two of them consistent with
/// each other.
pub struct WatchRegistry {
    /// Snapshot of the git roots currently being watched.
    ///
    /// Mirrors the key set of `MultiRepoWatcher::watched_repos` rather than
    /// sharing that map directly: the filesystem layer only ever needs the
    /// root paths, and keeping it decoupled from `WatchedRepo` (module-private
    /// bookkeeping: client PIDs, gitdir identity, etc.) avoids leaking that
    /// type across the module boundary. It lets raw notify events be
    /// attributed to the watched repository whose watch produced them via a
    /// longest-prefix match instead of an ancestor `.git` stat walk (#344).
    /// Kept in sync at the same two points `watched_repos`'s key set changes:
    /// insertion in `track_new_repo` and removal in `unregister_client_locked`.
    /// `rearm_repo_watch` never touches this set because it only updates a
    /// tracked repo's `git_dir`/identity, not its `git_root` key.
    roots: Mutex<HashSet<PathBuf>>,
    /// See [`ExternalGitDirs`].
    external_gitdirs: RwLock<ExternalGitDirs>,
    /// See [`CommonGitDirs`].
    common_gitdirs: RwLock<CommonGitDirs>,
    /// Config-file paths whose changes classify as [`FileEvent::Config`]
    /// regardless of filename, in both their canonical and absolute spellings
    /// (see [`WatchRegistry::is_registered_config_path`]).
    config_paths: RwLock<HashSet<PathBuf>>,
    /// The effective `git.watch_worktree` setting (see
    /// [`WatchRegistry::set_worktree_enabled`]).
    worktree_enabled: AtomicBool,
    /// The `.gitignore`-matcher and force-added caches consulted by the
    /// filesystem layer's ignore checks.
    ignore: filesystem::IgnoreCaches,
}

impl WatchRegistry {
    /// A registry with nothing registered and worktree watching resolved from
    /// the environment exactly as the old process-global flag was: an explicit
    /// `GPY_WATCH_WORKTREE` wins, otherwise ON.
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

    /// Apply the effective worktree-watching setting.
    ///
    /// `config_enabled` comes from `git.watch_worktree`; an explicit
    /// `GPY_WATCH_WORKTREE` env var overrides it. Newly registered
    /// repositories pick up the new value; already-active ones are brought
    /// into line by [`multi_repo::MultiRepoWatcher::resync_watch_worktree`],
    /// which the config-reload path calls right after this (#444).
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
        if let Ok(mut guard) = self.roots.lock() {
            guard.insert(root);
        }
    }

    /// Drop every root in `roots` from the watched set, under one lock.
    pub(crate) fn remove_roots<'a>(&self, roots: impl IntoIterator<Item = &'a Path>) {
        if let Ok(mut guard) = self.roots.lock() {
            for root in roots {
                guard.remove(root);
            }
        }
    }

    /// Forget every watched root.
    pub(crate) fn clear_roots(&self) {
        if let Ok(mut guard) = self.roots.lock() {
            guard.clear();
        }
    }

    /// Whether `root` is a registered watched root, or `None` when the lock is
    /// poisoned so callers can pick their own fail-open/fail-closed answer.
    pub(crate) fn root_is_registered(&self, root: &Path) -> Option<bool> {
        Some(self.roots.lock().ok()?.contains(root))
    }

    /// Every watched root, or an empty vector when the lock is poisoned.
    pub(crate) fn root_snapshot(&self) -> Vec<PathBuf> {
        self.roots
            .lock()
            .map_or_else(|_| Vec::new(), |guard| guard.iter().cloned().collect())
    }

    /// The longest watched root that is a prefix of `path`.
    ///
    /// See [`filesystem`]'s `attribute_repo_detailed` for why longest-prefix
    /// rather than an ancestor walk.
    pub(crate) fn longest_root_prefix(&self, path: &Path) -> Option<PathBuf> {
        let guard = self.roots.lock().ok()?;
        guard
            .iter()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.as_os_str().len())
            .cloned()
    }

    /// The subset of `candidates` that are registered watched roots, or `None`
    /// when the lock is poisoned. The lock is released before returning, so a
    /// caller may deliver events for the result without holding it.
    pub(crate) fn filter_registered_roots(&self, candidates: Vec<PathBuf>) -> Option<Vec<PathBuf>> {
        let guard = self.roots.lock().ok()?;
        let registered = candidates
            .into_iter()
            .filter(|candidate| guard.contains(candidate))
            .collect();
        drop(guard);
        Some(registered)
    }

    pub(crate) fn register_config_path(&self, path: &Path) {
        if let Ok(mut guard) = self.config_paths.write() {
            for entry in normalize_config_paths(path) {
                guard.insert(entry);
            }
        }
    }

    pub(crate) fn unregister_config_path(&self, path: &Path) {
        if let Ok(mut guard) = self.config_paths.write() {
            for entry in normalize_config_paths(path) {
                guard.remove(&entry);
            }
        }
    }

    /// Whether `path` is a registered config path — a pure lookup, deliberately
    /// doing zero normalization (no `canonicalize`, no `current_dir`).
    ///
    /// This is called once per classified filesystem event (the first check in
    /// [`classify_event`]), so unlike [`WatchRegistry::register_config_path`]/
    /// [`WatchRegistry::unregister_config_path`] (infrequent —
    /// startup/config-change), it sits on the hot path. It relies on two things
    /// holding together:
    ///
    /// 1. Watcher-delivered event paths are always already-absolute. The
    ///    underlying OS file-watching APIs (inotify, `FSEvents`,
    ///    `ReadDirectoryChangesW`) always deliver absolute paths for a changed
    ///    file, and this crate's watches are themselves registered against
    ///    already-canonicalized roots elsewhere in this module.
    /// 2. `register_config_path` stores BOTH the canonical and the absolute
    ///    spelling of a config path at registration time specifically so this
    ///    plain `HashSet::contains` can match either spelling without having to
    ///    resolve one into the other itself.
    pub(crate) fn is_registered_config_path(&self, path: &Path) -> bool {
        self.config_paths
            .read()
            .is_ok_and(|guard| guard.contains(path))
    }

    /// Record an external gitdir → working-root mapping. Both paths MUST already be
    /// canonical (the registry stores what the watcher registered, and the mapped
    /// root is path-validated downstream).
    pub(crate) fn register_external_gitdir(&self, git_dir: &Path, git_root: &Path) {
        if let Ok(mut guard) = self.external_gitdirs.write() {
            guard.insert(git_dir.to_path_buf(), git_root.to_path_buf());
        }
    }

    /// Remove a previously registered external gitdir mapping (no-op if absent).
    pub(crate) fn unregister_external_gitdir(&self, git_dir: &Path) {
        if let Ok(mut guard) = self.external_gitdirs.write() {
            guard.remove(git_dir);
        }
    }

    /// Attribute a raw event `path` to a registered external gitdir's working root.
    ///
    /// Returns the canonical working root when `path` is a git-significant leaf
    /// (HEAD, `refs/{heads,remotes}/**`, index, config, …) under the longest
    /// registered external gitdir that is a prefix of `path`. Non-significant
    /// internal churn (`objects/`, `logs/`, `hooks/`, `refs/tags/`) yields `None`,
    /// so this stays as quiet as the normal `.git` classification.
    ///
    /// The longest-prefix scan mirrors `attribute_repo`: it is a linear scan over a
    /// handful of registered gitdirs with zero filesystem stats (the mapping is
    /// populated at repo-registration time), so the hot path stays allocation- and
    /// stat-free apart from cloning the matched root once on a hit.
    pub(crate) fn attribute_external_gitdir(&self, path: &Path) -> Option<PathBuf> {
        let guard = self.external_gitdirs.read().ok()?;
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
        if let Ok(mut guard) = self.common_gitdirs.write() {
            guard
                .entry(common_dir.to_path_buf())
                .or_default()
                .insert(git_root.to_path_buf());
        }
    }

    /// Drop one dependent working root, removing the whole entry once the last one
    /// is gone (no-op if absent).
    pub(crate) fn unregister_common_gitdir(&self, common_dir: &Path, git_root: &Path) {
        if let Ok(mut guard) = self.common_gitdirs.write()
            && let Some(dependents) = guard.get_mut(common_dir)
        {
            dependents.remove(git_root);
            if dependents.is_empty() {
                guard.remove(common_dir);
            }
        }
    }

    /// Every linked-worktree working root that must refresh because shared git
    /// metadata at `path` changed.
    ///
    /// Returns an empty vector — allocating nothing — for the overwhelming majority
    /// of paths: those under no registered common directory, and those whose
    /// git-relative remainder is not shared state. Longest-prefix-wins mirrors
    /// [`WatchRegistry::attribute_external_gitdir`], so a submodule's own common
    /// directory (`<super>/.git/modules/<name>`) beats the superproject's
    /// `<super>/.git` it sits under.
    pub(crate) fn attribute_common_gitdir(&self, path: &Path) -> Vec<PathBuf> {
        let Ok(guard) = self.common_gitdirs.read() else {
            return Vec::new();
        };
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
        dependents.iter().cloned().collect()
    }

    /// Whether `path` lies under any registered common git directory (#468).
    ///
    /// Allocation- and stat-free: a linear `starts_with` over the handful of
    /// registered common directories, over an empty map for every repository
    /// without a registered linked worktree. Used by the filesystem layer to keep
    /// the shallow common-directory watch this change arms from delivering
    /// *cross-repository* refreshes for roots no client is registered for.
    pub(crate) fn is_under_registered_common_gitdir(&self, path: &Path) -> bool {
        self.common_gitdirs
            .read()
            .is_ok_and(|guard| guard.keys().any(|common_dir| path.starts_with(common_dir)))
    }

    /// Every registered-or-not *parent* repository working root whose gitlink status
    /// can change because submodule git metadata at `path` changed (#467).
    ///
    /// Returns an empty vector — allocating nothing — for any path outside a
    /// submodule git directory, which is every path in a repository without
    /// submodules. Callers are responsible for dropping roots no client is
    /// registered for; this function knows only the repository topology.
    ///
    /// The superproject root comes from [`split_at_git_dir_modules`], never from
    /// `<name>`. For a submodule of a submodule
    /// (`<super>/.git/modules/a/modules/b/…`) that split yields `b`'s
    /// *grandparent*, which does need refreshing but is not the whole answer: `a`'s
    /// gitlink to `b` moved too. `a`'s own git directory (`<super>/.git/modules/a`)
    /// is a proper ancestor of `b`'s, so any intermediate parent that is registered
    /// is recoverable from [`ExternalGitDirs`] — and one that is not registered
    /// would be filtered out by the caller regardless.
    pub(crate) fn attribute_submodule_parents(&self, path: &Path) -> Vec<PathBuf> {
        let Some((super_root, modules_root)) = split_at_git_dir_modules(path) else {
            return Vec::new();
        };

        let Some(owning_gitdir) = owning_submodule_gitdir(path, &modules_root) else {
            return Vec::new();
        };

        let mut parents = vec![super_root];
        if let Ok(guard) = self.external_gitdirs.read() {
            parents.extend(
                guard
                    .iter()
                    .filter(|(git_dir, _)| {
                        // A *proper* ancestor of the owning gitdir: the owning
                        // gitdir itself maps to the submodule that changed, which
                        // `attribute_external_gitdir` already delivers.
                        owning_gitdir.starts_with(git_dir) && git_dir.as_path() != owning_gitdir
                    })
                    .map(|(_, git_root)| git_root.clone()),
            );
        }
        parents
    }
}

impl Default for WatchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse the `GPY_WATCH_WORKTREE` override. Returns `None` when unset, otherwise
/// `Some(false)` for the usual falsey spellings and `Some(true)` for anything else.
fn env_watch_worktree_override() -> Option<bool> {
    env::var("GPY_WATCH_WORKTREE").ok().map(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
    })
}

fn normalize_config_paths(path: &Path) -> Vec<PathBuf> {
    let mut normalized = Vec::with_capacity(2);

    if let Ok(canonical) = path.canonicalize() {
        normalized.push(canonical);
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(path)
    } else {
        path.to_path_buf()
    };

    if !normalized.iter().any(|candidate| candidate == &absolute) {
        normalized.push(absolute);
    }

    normalized
}

/// Registry mapping a repository's *external* git directory (canonical) to its
/// canonical working-tree root.
///
/// For submodules (`<super>/.git/modules/<name>`) and linked worktrees
/// (`<super>/.git/worktrees/<name>`) the git metadata lives outside the working
/// tree, so a HEAD/refs write there is not a descendant of the working root.
/// Neither the filesystem layer's longest-prefix match over watched roots nor
/// [`MultiRepoWatcher::find_git_root`]'s ancestor walk can recover the correct
/// working root from such a path — and the gitdir *name* need not equal the
/// working-dir path, so it cannot be derived from the string either. This
/// registration-time mapping is the only source of truth (#428).
///
/// Lives in [`WatchRegistry`] rather than being derivable from the event path:
/// the gitdir *name* need not equal the working-dir path, so registration time
/// is the only moment the mapping is known. Only *external* gitdirs are ever
/// inserted, so ordinary repos never touch this map.
type ExternalGitDirs = std::collections::HashMap<PathBuf, PathBuf>;

/// Whether `relative` (a path relative to a git directory) names git metadata whose change
/// should refresh the prompt.
///
/// HEAD-family pointers, the index, config, `packed-refs`, `FETCH_HEAD`, any ref under
/// `refs/heads` or `refs/remotes`, the single `refs/stash` ref, and the per-repo
/// `info/exclude`.
///
/// Shared by [`classify_event`] (for in-tree `.git/…` paths) and
/// [`attribute_external_gitdir`] (for submodule/worktree external gitdirs) so
/// both recognize exactly the same set of significant leaves. O(path length),
/// no filesystem access.
fn is_git_significant_relative(relative: &Path) -> bool {
    // Ref updates: any file under refs/heads or refs/remotes (e.g. a branch or
    // remote-tracking ref move), plus the single stash ref.
    //
    // `refs/tags/` is intentionally NOT classified as significant (#435):
    // `RepositoryStatus` carries no tag field and no segment surfaces git tags,
    // so a tag create/delete has no visible effect on the prompt. Waking a
    // refresh for it would only cost a spurious (and change-detection-suppressed)
    // scan. Revisit if a tag-displaying segment is ever added.
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
    // Interactive-rebase / am-style-rebase operation metadata (#469, #481):
    // the bare directory entries — a notify event for a new directory carries
    // the directory path itself, so this is what makes create/remove of the
    // operation visible without waiting for an incidental HEAD write — and
    // the specific files `get_rebase_progress` (`rebase-merge/msgnum`,
    // `rebase-merge/git-rebase-todo`) and `get_repo_state`
    // (`rebase-apply/applying`) read.
    //
    // `rebase-merge/git-rebase-todo` joined this list in #481:
    // `get_rebase_progress` no longer computes `total` from `end` (`end` goes
    // stale after `git rebase --edit-todo` rewrites the plan — git never
    // rewrites it until the next `--continue`) but from `msgnum` plus the
    // current todo's remaining step count instead. That means a
    // `--edit-todo` write to the todo file is the only signal that the plan —
    // and thus the displayed total — changed; without classifying it, the
    // corrected total would sit uncomputed until some unrelated event
    // happened to trigger a rescan. `rebase-merge/end` stays in this list
    // too even though `get_rebase_progress` no longer reads it: it is still
    // written once at rebase start and its create/first-write remains a
    // useful (harmless if redundant) rebase-start signal.
    //
    // Deliberately NOT the whole subtree: both directories are rewritten on
    // every step (`patch`, `message`, `stopped-sha`, …), and `git am`
    // rewrites `rebase-apply/next` once per patch in the series — admitting
    // the subtree would turn a 200-patch `am` into 200 classified events for
    // no additional visible effect.
    if relative == Path::new("rebase-merge")
        || relative == Path::new("rebase-merge/msgnum")
        || relative == Path::new("rebase-merge/end")
        || relative == Path::new("rebase-merge/git-rebase-todo")
        || relative == Path::new("rebase-apply")
        || relative == Path::new("rebase-apply/applying")
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

/// Registry mapping a linked worktree's *common* git directory (canonical) to
/// the canonical working roots of every registered linked worktree that reads
/// it.
///
/// A linked worktree has two git directories: the per-worktree admin directory
/// (`<common>/worktrees/<name>`, tracked by [`ExternalGitDirs`]) holding its own
/// `HEAD`/`index`, and the *common* directory holding state shared by every
/// worktree of the repository — branch refs, remote-tracking refs,
/// `packed-refs` and `config`. A change to that shared state made from the main
/// checkout or a sibling worktree can move a linked worktree's ahead/behind or
/// upstream output without touching anything the worktree itself watches (#468).
///
/// Deliberately a *second* registry rather than widening [`ExternalGitDirs`] to
/// one-to-many:
///
/// - One common directory has many dependent working roots, while an external
///   gitdir has exactly one. Merging them would put both keys into a single
///   longest-prefix scan, where `<common>/worktrees/<name>` is a descendant of
///   `<common>` and one predicate would have to serve both — the failure mode
///   being a per-worktree `HEAD` write fanned out to every sibling.
/// - The two significance sets genuinely differ. See
///   [`is_common_gitdir_significant_relative`].
///
/// The two maps are consulted for different purposes at different points in
/// `process_event`, which makes that failure mode structurally impossible
/// instead of merely test-guarded.
type CommonGitDirs = std::collections::HashMap<PathBuf, std::collections::HashSet<PathBuf>>;

/// Whether `relative` (a path relative to a *common* git directory) names state shared by
/// every worktree of the repository.
///
/// A change to it must refresh every linked worktree rather than only the checkout that wrote
/// it.
///
/// Strictly narrower than [`is_git_significant_relative`], and deliberately not
/// a flag on it — the two sets differ in a way that would drift if they shared
/// one function:
///
/// - `refs/heads/**` — branch refs are shared (git refuses to check the same
///   branch out twice precisely because there is one ref). A commit made in a
///   sibling worktree moves the ref a linked worktree's ahead/behind may be
///   computed against. The highest-churn member of this set, but every one of
///   those writes is a real repository state change, not per-checkout noise.
/// - `refs/remotes/**` — remote-tracking refs, read by every worktree's
///   ahead/behind.
/// - `packed-refs` — the packed form of both of the above.
/// - `config` — where `branch.<name>.remote`/`.merge` upstream tracking lives.
///
/// Explicitly **excluded**, and this is the point of the separate predicate:
/// `HEAD` and `index`. For a linked worktree those are per-worktree state kept
/// in the admin directory, so a common-directory `HEAD`/`index` write belongs to
/// the main checkout alone. Fanning them out would wake a full status capture in
/// every registered linked worktree on every `git add` in the main checkout,
/// permanently, for no visible change. Everything else under the common
/// directory — `worktrees/<name>/**`, `objects/**`, `logs/**`, `refs/tags/**` —
/// falls through to `false` for the same reason.
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

/// The component pair that identifies a *submodule's* external git directory, and
/// separates the superproject's working root from it (#467).
///
/// Git puts a submodule's git directory at `<super>/.git/modules/<name>`, where
/// `<name>` is the submodule's name — not necessarily its working path, and it
/// may itself contain `/`. So the superproject root is recoverable by splitting
/// at the `.git`/`modules` component pair, while the submodule's working path is
/// not recoverable from the path at all (which is why [`ExternalGitDirs`] exists).
///
/// Keying the parent fan-out on this exact component pair is what makes it
/// structural rather than heuristic:
///
/// - An **ordinary nested repository** keeps an in-tree `.git`, so nothing it
///   writes can produce a `<super>/.git/modules/…` path, and its container has
///   no gitlink to it. It can never reach this code.
/// - A **linked worktree**'s admin directory is `<main>/.git/worktrees/<name>` —
///   also an external gitdir under a `.git`, and also *not* a submodule: the
///   main checkout holds no gitlink to a worktree, so refreshing it would be a
///   spurious cross-repository wake. Keying on "any external gitdir" would
///   conflate the two; keying on the `modules` component cannot.
///
/// Splits at the FIRST `.git`/`modules` pair, matching the byte-offset
/// `str::find` semantics this replaced — see [`attribute_submodule_parents`]'s
/// doc comment for why first (not last) is deliberate for nested submodules.
/// Returns `(super_root, modules_root)`: `super_root` is the path prefix
/// before `.git`, and `modules_root` is the prefix through `.git/modules`
/// inclusive. `None` when `path` has no such component pair.
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
/// when `path` is internal churn rather than a significant leaf.
///
/// A submodule's *name* may contain `/` (git names the directory after the
/// submodule's configured path by default, so `libs/foo` becomes
/// `.git/modules/libs/foo`), so where the git directory ends and the
/// git-relative remainder begins cannot be read off the string. Resolve it by
/// scanning `path`'s ancestors deepest-first for the first one whose remainder
/// [`is_git_significant_relative`] accepts, stopping at `modules_root`.
///
/// Deliberately reuses [`is_git_significant_relative`] rather than adding a
/// third predicate. Every leaf it accepts can move the parent's gitlink or the
/// dirty flags git renders beside it: `HEAD`/`index`/`refs/heads/**` move the
/// recorded commit, and `info/exclude` changes what counts as untracked content
/// *inside* the submodule, which the superproject reports in the porcelain-v2
/// `S<c><m><u>` field. The handful it accepts that cannot — `config`,
/// `refs/remotes/**`, `FETCH_HEAD` — are written only by `submodule add`,
/// `submodule update` and `fetch`, and cost at most one change-detection-
/// suppressed scan.
///
/// One over-approximation, kept on purpose: git's reflog lives at
/// `<gitdir>/logs/HEAD` and `<gitdir>/logs/refs/{heads,remotes}/…`, so the scan
/// can accept `<gitdir>/logs` as the git directory. Excluding it would instead
/// silently drop a submodule literally named `logs`, and it costs nothing:
/// git writes a reflog entry only alongside the ref update it records, so the
/// two land in the same debounce window and coalesce into one parent refresh.
/// `objects/**`, `hooks/**` and `refs/tags/**` have no such shape and are
/// rejected outright.
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

/// A hook that schedules a [`PendingEvent`] for delivery no earlier than
/// `due`, coalescing with any other pending event sharing its debounce key
/// exactly like an immediate one.
///
/// See [`debouncer::DebounceEngine::handle_event_due`] (#569).
///
/// The filesystem layer uses this for the directory-create follow-up, which
/// used to be a detached sleeping thread per created directory. Invoking it is
/// synchronous and cheap: it mutates the debouncer's in-memory map and returns,
/// leaving the waiting to the flush thread [`WatchCoordinator`] already owns.
pub type DelayedEventScheduler = Arc<dyn Fn(PendingEvent, Instant) + Send + Sync>;

/// The longest a flush thread sleeps before re-checking its stop flag.
const FLUSH_STOP_POLL: Duration = Duration::from_millis(25);

/// Sleep for `total`, waking early once `stop` is set.
///
/// Bounds how long [`WatchCoordinator::stop`] can block on joining the flush
/// thread to roughly [`FLUSH_STOP_POLL`], independent of the debounce window.
fn sleep_unless_stopped(total: Duration, stop: &AtomicBool) {
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
///
/// Wraps a `FileSystemWatcher` and `DebounceEngine` to coordinate
/// event filtering and debouncing for a single repository. This is
/// the low-level watcher component; for managing multiple repositories,
/// see `multi_repo::MultiRepoWatcher`.
pub struct WatchCoordinator {
    watcher: Option<filesystem::FileSystemWatcher>,
    debouncer: Arc<Mutex<debouncer::DebounceEngine>>,
    callback: Arc<EventCallback>,
    stop_flag: Arc<AtomicBool>,
    flush_handle: Option<JoinHandle<()>>,
    /// The state this coordinator and its filesystem layer share, so callers
    /// that built the coordinator can register config paths and gitdir
    /// mappings on the same registry the event path reads (#617).
    registry: Arc<WatchRegistry>,
    /// Every path this instance was asked to unwatch, in call order.
    /// Instance-scoped test observability for #616's teardown-precision
    /// tests: proving `unregister_client_locked`/`rearm_repo_watch` unwatch
    /// exactly the paths they armed (no more, no less) needs to see what the
    /// coordinator was actually asked to do, not just what `WatchedRepo`
    /// claims it armed.
    #[cfg(test)]
    unwatch_calls: Mutex<Vec<PathBuf>>,
}

impl WatchCoordinator {
    /// Create a new file watcher with callback.
    ///
    /// `registry` is the [`WatchRegistry`] this coordinator shares with the
    /// filesystem layer beneath it: the watched git roots events are
    /// attributed against (#344), the external/common gitdir maps, the
    /// registered config paths, the worktree-watch flag and the ignore
    /// caches. Pass `Some` to share one registry with other coordinators (the
    /// agent shares one across its repo, config and theme watchers); pass
    /// `None` for a standalone coordinator, which gets a private registry of
    /// its own and therefore an empty watched-root set — the same
    /// ancestor-walk attribution the old `None` handle produced.
    ///
    /// # Errors
    ///
    /// Returns an error if the file system watcher cannot be created.
    pub fn new(
        debounce: Duration,
        callback: EventCallback,
        registry: Option<Arc<WatchRegistry>>,
    ) -> Result<Self> {
        let shared_registry = registry.unwrap_or_else(|| Arc::new(WatchRegistry::new()));
        let event_debouncer = debouncer::DebounceEngine::new(debounce);
        let callback_arc = Arc::new(callback);

        // Create a callback that feeds events into the debouncer
        let debouncer_arc = Arc::new(Mutex::new(event_debouncer));
        let debouncer_for_watcher = Arc::clone(&debouncer_arc);
        let debounce_interval = debounce;

        let ingress_callback: PendingCallback = Box::new(move |pending| {
            if let Ok(mut debouncer_guard) = debouncer_for_watcher.lock() {
                debouncer_guard.handle_event(pending);
            }
        });

        // The directory-create follow-up (#416) rides the same debouncer as
        // ordinary ingress, just with a due-time floor, so the flush thread
        // below does its waiting and coalescing. No timer thread per created
        // directory, and nothing left running once `stop` clears the map (#569).
        let debouncer_for_schedule = Arc::clone(&debouncer_arc);
        let schedule_delayed: DelayedEventScheduler = Arc::new(move |pending, due| {
            if let Ok(mut debouncer_guard) = debouncer_for_schedule.lock() {
                debouncer_guard.handle_event_due(pending, due);
            }
        });

        // Use the debounce window as the directory-create follow-up delay so a
        // brand-new directory's contents are rescanned even if the OS watch on
        // it wasn't armed in time to deliver their create events (#416).
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
                // Sleep in short slices so `stop` (which joins this thread)
                // returns within one slice rather than one full debounce
                // window. With the theme coordinator's 5 s debounce, a plain
                // `sleep(debounce_interval)` here made every theme switch
                // block its IPC connection for up to 5 s, long enough for the
                // CLI's 2 s reload budget to expire and report "Agent not
                // reloaded" for a reload the agent then completed (#656).
                sleep_unless_stopped(debounce_interval, &stop_flag_thread);
                if stop_flag_thread.load(Ordering::Relaxed) {
                    break;
                }
                let expired = debouncer_for_thread.lock().map_or_else(
                    |_| Vec::new(),
                    |mut debouncer_guard| debouncer_guard.drain_expired_events(),
                );
                // Dispatch each event on its own thread, off the flush loop
                // and outside the debouncer lock, so a slow callback for one
                // repo cannot delay flushing or delivery for another (#306).
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
    ///
    /// Returns an error if the directory cannot be watched.
    pub fn watch_directory<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if let Some(watcher) = &mut self.watcher {
            watcher.watch(path)?;
        }
        Ok(())
    }

    /// Start watching a directory non-recursively (#468).
    ///
    /// See [`filesystem::FileSystemWatcher::watch_shallow`] for why the common
    /// git directory of a linked worktree must not be watched recursively.
    ///
    /// # Errors
    ///
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
    ///
    /// Returns an error if the directory cannot be unwatched.
    pub fn unwatch_directory<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        #[cfg(test)]
        if let Ok(mut calls) = self.unwatch_calls.lock() {
            calls.push(path.as_ref().to_path_buf());
        }
        if let Some(watcher) = &mut self.watcher {
            watcher.unwatch(path)?;
        }
        Ok(())
    }

    /// Release one [`WatchCoordinator::watch_directory_shallow`] of `path`
    /// (see [`filesystem::FileSystemWatcher::unwatch_shallow`]).
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be unwatched.
    pub fn unwatch_directory_shallow<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        #[cfg(test)]
        if let Ok(mut calls) = self.unwatch_calls.lock() {
            calls.push(path.as_ref().to_path_buf());
        }
        if let Some(watcher) = &mut self.watcher {
            watcher.unwatch_shallow(path)?;
        }
        Ok(())
    }

    /// Every path this instance was asked to unwatch, in call order.
    /// Test-only observability (#616).
    #[cfg(test)]
    pub fn unwatch_calls(&self) -> Vec<PathBuf> {
        self.unwatch_calls
            .lock()
            .map_or_else(|_| Vec::new(), |calls| calls.clone())
    }

    /// Stop all watching
    pub fn stop(&mut self) {
        // Stop the file watcher
        if let Some(watcher) = &mut self.watcher {
            watcher.stop();
        }
        self.watcher = None;

        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.flush_handle.take() {
            let _ = handle.join();
        }

        if let Ok(mut debouncer_guard) = self.debouncer.lock() {
            debouncer_guard.clear();
        }
    }

    /// Process any pending debounced events, delivering them synchronously
    /// on the calling thread. Drains under the debouncer lock, then invokes
    /// the callback after releasing it (#306).
    pub fn flush_pending_events(&self) {
        let expired = self.debouncer.lock().map_or_else(
            |_| Vec::new(),
            |mut debouncer_guard| debouncer_guard.drain_expired_events(),
        );
        for event in expired {
            (self.callback)(event);
        }
    }
}

impl Drop for WatchCoordinator {
    fn drop(&mut self) {
        // Ensure cleanup happens even if stop() wasn't called
        self.stop();
    }
}

/// Determine if a file path should trigger prompt updates.
///
/// `registry` supplies the config-path set [`classify_event`] consults; pass
/// the registry of the watcher the event came from (a fresh
/// [`WatchRegistry::new`] is fine for a caller that registers no config path).
#[must_use]
pub fn should_trigger_update(path: &Path, registry: &WatchRegistry) -> Option<PendingEvent> {
    filter_ignored(path)?;
    let event = classify_event(path, registry)?;
    let repo = multi_repo::MultiRepoWatcher::find_git_root(path)
        .or_else(|| path.parent().map(Path::to_path_buf))?;

    Some(PendingEvent { event, repo })
}

/// Whether `path` has a `.git` path component immediately followed by the
/// given component sequence (e.g. `&["objects"]` for `.git/objects/**`,
/// `&["refs", "tags"]` for `.git/refs/tags/**`).
///
/// Component-based rather than a string pattern so this matches on Windows
/// too: `Path::to_string_lossy` preserves `\` there, so a forward-slash
/// literal like `".git/objects/"` silently never matches a real Windows path.
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
    // The two exact log/socket filenames, and the broader `.sock` extension
    // catch-all, are matched against the FILE NAME alone (not a substring of
    // the whole path string): a bare `path_str.contains(".sock")` also matches
    // a legitimately-named file like `foo.socket.rs`, silently filtering it
    // out of every refresh forever.
    if path.file_name() == Some(OsStr::new("gpy-agent.log"))
        || path.file_name() == Some(OsStr::new("gpy-agent.sock"))
    {
        return None;
    }
    if path.extension() == Some(OsStr::new("sock")) {
        return None;
    }

    // `.git/info/` (notably `exclude`, a per-repo .gitignore) is intentionally
    // NOT blanket-ignored here: `classify_event` has a dedicated arm that
    // treats `.git/info/exclude` changes as a git status change, matching
    // `.gitignore` (#323). Filtering the whole directory here would make
    // that arm unreachable.
    if path_contains_git_subdir(path, &["objects"])
        || path_contains_git_subdir(path, &["logs"])
        || path_contains_git_subdir(path, &["hooks"])
        || path_contains_git_subdir(path, &["refs", "tags"])
    {
        return None;
    }

    Some(())
}

/// Whether `path` has a `.git` path component anywhere.
///
/// Mirrors [`filesystem::is_in_git_dir`] (kept private there; not exported) so
/// this module gets the same Windows-safe, component-based check rather than
/// a `/.git/`-literal string match.
fn path_has_git_component(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == OsStr::new(".git"))
}

/// Whether `path` has two consecutive components `first` then `second`
/// anywhere in it (e.g. a `gpy` component immediately followed by a `themes`
/// component).
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

/// Whether `path`'s own trailing two components are exactly `first` then
/// `second` (e.g. a path that itself ends `.../gpy/themes`).
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

/// The path's components after its LAST `.git` component, reconstructed as a
/// `PathBuf` — the component-based equivalent of
/// `path_str.rsplit_once("/.git/")`'s second half.
///
/// `None` when `path` has no `.git` component at all.
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

fn classify_event(path: &Path, registry: &WatchRegistry) -> Option<FileEvent> {
    if registry.is_registered_config_path(path) {
        return Some(FileEvent::Config {
            path: path.to_path_buf(),
        });
    }

    let file_name = path.file_name()?.to_str()?;

    // Check if path is within .git directory (for refs, etc.)
    let is_in_git_dir = path_has_git_component(path);

    // Theme files are often written via temp-file + rename workflows where
    // intermediate notifications may target the directory or a non-.toml temp file.
    // Treat any change under gpy/themes as a theme invalidation signal.
    if path_has_consecutive_components(path, "gpy", "themes")
        || path_ends_with_components(path, "gpy", "themes")
    {
        return Some(FileEvent::Theme {
            path: path.to_path_buf(),
        });
    }

    // Git metadata under a normal in-tree `.git/…` directory: the portion after
    // the last `.git` component is the git-relative path. `is_git_significant_relative`
    // is the single source of truth for which leaves matter, shared with the
    // submodule/worktree external-gitdir path in `attribute_external_gitdir`
    // (#428). External gitdirs (`.git/modules|worktrees/<name>/…`) aren't a
    // `.git`-descendant of the working tree, so they're handled there, not
    // here.
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
    } else if file_name.ends_with(".gpy.toml")
        || file_name.ends_with("gpy.toml")
        || file_name.eq_ignore_ascii_case("config.toml")
    {
        FileEvent::Config {
            path: path.to_path_buf(),
        }
    } else {
        return None;
    };

    Some(event)
}

/// Lockfile / version-pin filenames that must invalidate the language cache
/// but have no [`crate::language::metadata::LanguageMeta::marker_files`]
/// equivalent (gpy-agent#602).
///
/// None of these are language *detection* markers — `metadata.rs`'s
/// `marker_files` answers "what language is present in this directory" and
/// deliberately doesn't (and shouldn't) list lockfiles — but each still
/// signals a dependency/version change that the language segment's version
/// display cares about, so the watcher must still treat a change to one as a
/// cache-invalidating [`FileEvent::Language`].
///
/// Kept as an explicit residual list rather than merged into `metadata.rs`:
/// merging the two purposes ("what language is here" vs. "what changed a
/// displayed version") into one list is exactly the drift #602 fixed on the
/// *marker* side — a name that has no detection meaning shouldn't be added to
/// the detection list just to keep this list in sync.
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
    "poetry.lock",
    "Gemfile.lock",
    "mix.lock",
    "Package.resolved",
    "settings.gradle",
];

/// Whether a change to `file_name` should invalidate the language cache.
///
/// True when `file_name` is either a real language-detection marker
/// ([`crate::language::metadata::marker_file_names`]) or one of the
/// [`LANGUAGE_CACHE_LOCKFILES`] residual names.
///
/// Sharing the marker-file set with `metadata.rs` means a marker file added
/// there automatically also triggers a watcher refresh here — no second list
/// to remember to update (gpy-agent#602).
///
/// Case-sensitive, matching `classify_event`'s historical behavior: unlike
/// `metadata.rs`'s marker-file *detection* lookups (which are
/// case-insensitive, since directory scans use `eq_ignore_ascii_case`), this
/// check does exact `&str` equality via `HashSet::contains`/slice `contains`.
/// That's a pre-existing, narrower inconsistency this issue doesn't attempt
/// to fix.
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
        let throttle = parse_env("GPY_SIGUSR1_THROTTLE_MS", 150);

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
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::{
        FileEvent, PendingEvent, WatchRegistry, filter_ignored,
        is_common_gitdir_significant_relative, is_git_significant_relative,
        split_at_git_dir_modules,
    };
    use std::path::{Path, PathBuf};

    /// Classify against a fresh, empty registry.
    ///
    /// Every test using this shim asserts on classification rules that read
    /// nothing from the registry but the (empty) config-path set, so a private
    /// registry per call is equivalent to the process-global one these tests
    /// used before #617 — and independent of anything running in parallel.
    fn classify_event(path: &Path) -> Option<FileEvent> {
        super::classify_event(path, &WatchRegistry::new())
    }

    /// As [`classify_event`], through the full filter+classify pipeline.
    fn should_trigger_update(path: &Path) -> Option<PendingEvent> {
        super::should_trigger_update(path, &WatchRegistry::new())
    }

    /// `stop` must not wait out the debounce window (#656).
    ///
    /// It joins the flush thread, which used to sleep a whole debounce window
    /// per iteration -- 5 s for the theme coordinator -- so every theme
    /// switch stalled its IPC reply past the CLI's reload budget.
    #[test]
    fn stop_returns_promptly_regardless_of_debounce_window() {
        let mut coordinator = super::WatchCoordinator::new(
            std::time::Duration::from_secs(30),
            Box::new(|_event| {}),
            None,
        )
        .expect("coordinator");

        let started = std::time::Instant::now();
        coordinator.stop();
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "stop() must not wait out the debounce window, took {elapsed:?}"
        );
    }

    #[test]
    fn python_version_file_is_language_event() {
        let event = classify_event(Path::new("/tmp/project/.python-version"));
        assert!(matches!(event, Some(FileEvent::Language { .. })));
    }

    #[test]
    fn tool_versions_file_is_language_event() {
        let event = classify_event(Path::new("/tmp/project/.tool-versions"));
        assert!(matches!(event, Some(FileEvent::Language { .. })));
    }

    /// Regression test for #323: `.git/info/exclude` must classify as a git
    /// event.
    ///
    /// `.git/info/exclude` (a per-repo `.gitignore`) was previously
    /// unreachable — blanket-filtered by `filter_ignored`'s `.git/info/`
    /// pattern, and even if it had gotten through, `classify_event` had no arm
    /// to match `exclude`'s parent directory (`info`, not `.git`).
    #[test]
    fn git_info_exclude_is_git_event() {
        let event = classify_event(Path::new("/tmp/project/.git/info/exclude"));
        assert!(
            matches!(event, Some(FileEvent::Git { .. })),
            "expected a Git event, got {event:?}"
        );
    }

    /// Same regression, exercised through the full `should_trigger_update`
    /// pipeline (filter + classify), which is what the watcher actually calls.
    #[test]
    fn git_info_exclude_triggers_update() {
        let pending = should_trigger_update(Path::new("/tmp/project/.git/info/exclude"));
        assert!(
            matches!(pending.map(|p| p.event), Some(FileEvent::Git { .. })),
            "should_trigger_update must classify .git/info/exclude as a Git event"
        );
    }

    /// Regression test for #415: `git gc` / `git pack-refs` moves ref updates
    /// into `.git/packed-refs`, which previously matched no arm in
    /// `classify_event` and produced no event at all.
    #[test]
    fn packed_refs_is_git_event() {
        let event = classify_event(Path::new("/tmp/project/.git/packed-refs"));
        assert!(
            matches!(event, Some(FileEvent::Git { .. })),
            "expected a Git event, got {event:?}"
        );
    }

    /// Regression test for #415: a stash-only change (no index/worktree
    /// touch) only writes `.git/refs/stash`, which previously matched
    /// neither the `refs/heads/` nor `refs/remotes/` prefix arms.
    #[test]
    fn refs_stash_is_git_event() {
        let event = classify_event(Path::new("/tmp/project/.git/refs/stash"));
        assert!(
            matches!(event, Some(FileEvent::Git { .. })),
            "expected a Git event, got {event:?}"
        );
    }

    /// `.git/ORIG_HEAD` (set by merge/rebase/reset before they rewrite HEAD)
    /// should also classify as a Git event; see #415.
    #[test]
    fn orig_head_is_git_event() {
        let event = classify_event(Path::new("/tmp/project/.git/ORIG_HEAD"));
        assert!(
            matches!(event, Some(FileEvent::Git { .. })),
            "expected a Git event, got {event:?}"
        );
    }

    /// `refs/stash` must be matched as the exact leaf file, not as a prefix,
    /// so unrelated paths that merely contain the substring don't slip
    /// through as Git events.
    #[test]
    fn refs_stash_prefix_lookalike_is_not_a_git_event() {
        let event = classify_event(Path::new("/tmp/project/.git/refs/stash-backup/main"));
        assert!(event.is_none(), "expected no event, got {event:?}");
    }

    /// `filter_ignored` must continue to blanket-ignore loose object and reflog writes before
    /// `classify_event` ever runs.
    ///
    /// So the new `packed-refs`/`refs/stash`/`ORIG_HEAD` arms don't introduce signal churn
    /// from those high-frequency paths (#415).
    ///
    #[test]
    fn filtered_git_dirs_still_produce_no_event() {
        let ignored_paths = [
            "/tmp/project/.git/objects/ab/cdef1234",
            "/tmp/project/.git/logs/HEAD",
            "/tmp/project/.git/logs/refs/heads/main",
            "/tmp/project/.git/hooks/pre-commit",
            "/tmp/project/.git/refs/tags/v1.0.0",
        ];

        for path in ignored_paths {
            let pending = should_trigger_update(Path::new(path));
            assert!(
                pending.is_none(),
                "expected no event for ignored path {path}, got {pending:?}"
            );
        }
    }

    /// The shared git-significance predicate (#428) must accept exactly the leaves the normal
    /// `.git` arms accept.
    ///
    /// HEAD-family, index/config, `refs/{heads,remotes}/**`, `refs/stash`, `info/exclude`,
    /// `packed-refs`, `FETCH_HEAD` — and reject internal churn (objects/logs/hooks/tags) and
    /// nested top-level lookalikes.
    ///
    #[test]
    fn git_significant_relative_predicate() {
        let significant = [
            "HEAD",
            "index",
            "config",
            "ORIG_HEAD",
            "MERGE_HEAD",
            "FETCH_HEAD",
            "packed-refs",
            "refs/heads/main",
            "refs/remotes/origin/main",
            "refs/stash",
            "info/exclude",
        ];
        for rel in significant {
            assert!(
                is_git_significant_relative(Path::new(rel)),
                "{rel} should be git-significant"
            );
        }

        let insignificant = [
            "objects/ab/cdef",
            "logs/HEAD",
            "logs/refs/heads/main",
            "hooks/pre-commit",
            "refs/tags/v1.0.0",
            "refs/stash-backup/main",
            "description",
        ];
        for rel in insignificant {
            assert!(
                !is_git_significant_relative(Path::new(rel)),
                "{rel} must not be git-significant"
            );
        }
    }

    /// Interactive-rebase and am-style-rebase operation metadata (#469, #481).
    ///
    /// The bare `rebase-merge`/`rebase-apply` directory entries — whose create/remove is the
    /// `Rebasing`/`Applying` state transition — and the specific files `get_rebase_progress`
    /// (`rebase-merge/{msgnum,end,git-rebase-todo}`) and `get_repo_state`
    /// (`rebase-apply/applying`) actually read.
    ///
    /// Everything else either directory holds (`patch`, `message`, `stopped-sha`,
    /// `rebase-apply/next`, …) is rewritten on every step — every patch, for `git am` — and
    /// must stay rejected.
    ///
    /// `rebase-merge/git-rebase-todo` moved from the rejected list to the
    /// accepted one in #481: `get_rebase_progress` used to trust
    /// `rebase-merge/end` for the displayed total, but git never rewrites
    /// `end` when `git rebase --edit-todo` changes the plan, so the total
    /// went stale until the next `--continue`. The fix reads the *current*
    /// `git-rebase-todo` to count remaining steps instead, so a `--edit-todo`
    /// write to that file is now the only signal that the total changed —
    /// classifying it is what makes the corrected total actually reach the
    /// prompt without waiting for an unrelated event. `git-rebase-todo.backup`
    /// and `rebase-merge/done` stay rejected: nothing reads either.
    #[test]
    fn rebase_metadata_significant_relative_predicate() {
        let significant = [
            "rebase-merge",
            "rebase-merge/msgnum",
            "rebase-merge/end",
            "rebase-merge/git-rebase-todo",
            "rebase-apply",
            "rebase-apply/applying",
        ];
        for rel in significant {
            assert!(
                is_git_significant_relative(Path::new(rel)),
                "{rel} should be git-significant"
            );
        }

        // High-churn files under either directory. Named explicitly so a
        // later "just accept the directory" simplification fails loudly.
        let insignificant = [
            "rebase-merge/patch",
            "rebase-merge/git-rebase-todo.backup",
            "rebase-merge/message",
            "rebase-merge/stopped-sha",
            "rebase-merge/done",
            "rebase-merge/head-name",
            "rebase-merge/onto",
            "rebase-apply/next",
            "rebase-apply/patch",
            "rebase-apply/last",
            "rebase-apply/msg",
        ];
        for rel in insignificant {
            assert!(
                !is_git_significant_relative(Path::new(rel)),
                "{rel} must not be git-significant"
            );
        }
    }

    /// Full-pipeline regression: rebase step/total changes and the am-style marker must reach
    /// `Git` events.
    ///
    /// An interactive-rebase step/total change (`rebase-merge/msgnum`, `rebase-merge/end`) and
    /// an am-style-rebase operation marker (`rebase-apply/applying`) must reach `Git` events
    /// through `should_trigger_update`, matching the existing
    /// `git_info_exclude_triggers_update` regression pattern (#469 AC 1).
    ///
    #[test]
    fn rebase_progress_files_trigger_update() {
        for path in [
            "/tmp/project/.git/rebase-merge/msgnum",
            "/tmp/project/.git/rebase-merge/end",
            "/tmp/project/.git/rebase-apply/applying",
        ] {
            let pending = should_trigger_update(Path::new(path));
            assert!(
                matches!(pending.map(|p| p.event), Some(FileEvent::Git { .. })),
                "should_trigger_update must classify {path} as a Git event"
            );
        }
    }

    /// Creating or removing the `rebase-merge`/`rebase-apply` directory itself must also
    /// classify as a Git event.
    ///
    /// The notify event for a new directory carries the directory path, not a file beneath it
    /// — so the `Rebasing`/`Applying` state transition is seen without waiting for an
    /// incidental `HEAD` write (#469 AC 2).
    ///
    #[test]
    fn rebase_operation_directory_entry_is_git_event() {
        for path in [
            "/tmp/project/.git/rebase-merge",
            "/tmp/project/.git/rebase-apply",
        ] {
            let event = classify_event(Path::new(path));
            assert!(
                matches!(event, Some(FileEvent::Git { .. })),
                "expected a Git event for the {path} directory entry, got {event:?}"
            );
        }
    }

    /// High-churn rebase files must produce no event through the full `should_trigger_update`
    /// pipeline, not merely fail the predicate in isolation.
    ///
    /// `filter_ignored` does not block `rebase-merge`/`rebase-apply` (only `objects/`,
    /// `logs/`, `hooks/`, `refs/tags/`), so classification is the only gate (#469 AC 3).
    ///
    #[test]
    fn rebase_high_churn_files_produce_no_event() {
        for path in [
            "/tmp/project/.git/rebase-merge/patch",
            "/tmp/project/.git/rebase-merge/git-rebase-todo.backup",
            "/tmp/project/.git/rebase-merge/message",
            "/tmp/project/.git/rebase-merge/stopped-sha",
            "/tmp/project/.git/rebase-apply/next",
            "/tmp/project/.git/rebase-apply/patch",
        ] {
            let pending = should_trigger_update(Path::new(path));
            assert!(
                pending.is_none(),
                "expected no event for high-churn rebase path {path}, got {pending:?}"
            );
        }
    }

    /// Consumer 3 (#467's `owning_submodule_gitdir`, reused by `attribute_submodule_parents`):
    /// rebase metadata inside a submodule's gitdir must resolve to that gitdir specifically.
    ///
    /// Not a shallower or deeper ancestor — attributing to the submodule's own working root
    /// via `attribute_external_gitdir` and fanning out to the registered superproject via
    /// `attribute_submodule_parents`.
    ///
    /// High-churn rebase files under the same gitdir must resolve to no owner at all (#469).
    #[test]
    fn submodule_rebase_metadata_resolves_to_owning_gitdir() {
        let registry = WatchRegistry::new();
        let git_dir = PathBuf::from("/super/.git/modules/sub");
        let work_root = PathBuf::from("/super/sub");
        registry.register_external_gitdir(&git_dir, &work_root);

        for rel in [
            "rebase-merge",
            "rebase-merge/msgnum",
            "rebase-merge/end",
            "rebase-apply",
            "rebase-apply/applying",
        ] {
            let path = git_dir.join(rel);
            assert_eq!(
                registry.attribute_external_gitdir(&path),
                Some(work_root.clone()),
                "{} should attribute to the submodule's own working root",
                path.display()
            );
            assert_eq!(
                registry.attribute_submodule_parents(&path),
                vec![PathBuf::from("/super")],
                "{} should also fan out to the registered superproject",
                path.display()
            );
        }

        for rel in ["rebase-merge/patch", "rebase-apply/next"] {
            let path = git_dir.join(rel);
            assert_eq!(
                registry.attribute_external_gitdir(&path),
                None,
                "{} is high-churn and must not attribute",
                path.display()
            );
            assert!(
                registry.attribute_submodule_parents(&path).is_empty(),
                "{} is high-churn and must not fan out to the superproject",
                path.display()
            );
        }

        registry.unregister_external_gitdir(&git_dir);
    }

    /// `attribute_external_gitdir` (#428) must map a significant leaf to its working root.
    ///
    /// Under a registered external gitdir, prefer the longest matching gitdir, reject
    /// non-significant churn, and return `None` for unregistered paths.
    ///
    /// Isolated by construction: the mapping lives on a registry owned by this
    /// test.
    #[test]
    fn external_gitdir_attribution_maps_to_working_root() {
        let registry = WatchRegistry::new();
        let git_dir = PathBuf::from("/super/.git/modules/sub");
        let work_root = PathBuf::from("/super/sub");
        registry.register_external_gitdir(&git_dir, &work_root);

        // A significant leaf under the external gitdir resolves to the root.
        assert_eq!(
            registry.attribute_external_gitdir(Path::new("/super/.git/modules/sub/HEAD")),
            Some(work_root.clone())
        );
        assert_eq!(
            registry
                .attribute_external_gitdir(Path::new("/super/.git/modules/sub/refs/heads/other")),
            Some(work_root)
        );
        // Non-significant internal churn under the same gitdir is ignored.
        assert_eq!(
            registry.attribute_external_gitdir(Path::new("/super/.git/modules/sub/objects/ab/cd")),
            None
        );
        // An unrelated path matches no registered gitdir.
        assert_eq!(
            registry.attribute_external_gitdir(Path::new("/elsewhere/.git/HEAD")),
            None
        );

        registry.unregister_external_gitdir(&git_dir);
        assert_eq!(
            registry.attribute_external_gitdir(Path::new("/super/.git/modules/sub/HEAD")),
            None,
            "unregister must remove the mapping"
        );
    }

    /// The common-directory predicate (#468) covers state shared by every worktree and nothing
    /// else.
    ///
    /// `HEAD`/`index` are the load-bearing exclusions: for a linked worktree they are
    /// per-worktree state in the admin dir, so a common-dir write of either belongs to the
    /// main checkout alone.
    #[test]
    fn common_gitdir_significance_covers_shared_state_only() {
        let shared = [
            "refs/heads/main",
            "refs/heads/feature/nested",
            "refs/remotes/origin/main",
            "packed-refs",
            "config",
        ];
        for rel in shared {
            assert!(
                is_common_gitdir_significant_relative(Path::new(rel)),
                "{rel} is shared across worktrees and must fan out"
            );
        }

        let not_shared = [
            // Per-checkout state, not shared state.
            "HEAD",
            "index",
            "ORIG_HEAD",
            "MERGE_HEAD",
            "FETCH_HEAD",
            "info/exclude",
            "refs/stash",
            // Another worktree's admin dir is a descendant of the common dir;
            // its contents must never fan out to siblings.
            "worktrees/other/HEAD",
            "worktrees/other/index",
            "worktrees/other/refs/bisect/bad",
            // Internal churn.
            "objects/ab/cdef",
            "logs/HEAD",
            "refs/tags/v1.0.0",
            "description",
        ];
        for rel in not_shared {
            assert!(
                !is_common_gitdir_significant_relative(Path::new(rel)),
                "{rel} must not fan out to linked worktrees"
            );
        }
    }

    /// `attribute_common_gitdir` fans one shared-metadata path out to every dependent working
    /// root.
    ///
    /// Rejects per-worktree state under the same common dir, prefers the longest registered
    /// common dir, and reference-counts dependents.
    ///
    /// Isolated by construction: the mapping lives on a registry owned by this
    /// test.
    #[test]
    fn common_gitdir_attribution_fans_out_to_every_dependent() {
        let registry = WatchRegistry::new();
        let common = PathBuf::from("/main/.git");
        let first = PathBuf::from("/wt/one");
        let second = PathBuf::from("/wt/two");
        registry.register_common_gitdir(&common, &first);
        registry.register_common_gitdir(&common, &second);

        let mut fanned =
            registry.attribute_common_gitdir(Path::new("/main/.git/refs/remotes/origin/main"));
        fanned.sort();
        assert_eq!(fanned, vec![first.clone(), second.clone()]);

        assert_eq!(
            registry
                .attribute_common_gitdir(Path::new("/main/.git/config"))
                .len(),
            2,
            "a shared config change must reach every dependent worktree"
        );
        assert!(
            registry
                .attribute_common_gitdir(Path::new("/main/.git/index"))
                .is_empty(),
            "a main-checkout index write must wake zero linked worktrees"
        );
        assert!(
            registry
                .attribute_common_gitdir(Path::new("/main/.git/worktrees/one/HEAD"))
                .is_empty(),
            "a per-worktree HEAD write must not fan out to siblings"
        );
        assert!(
            registry
                .attribute_common_gitdir(Path::new("/elsewhere/.git/config"))
                .is_empty(),
            "an unregistered common dir matches nothing"
        );

        // A submodule's own common dir nests under the superproject's; the
        // longest prefix must win.
        let nested = PathBuf::from("/main/.git/modules/sub");
        let nested_root = PathBuf::from("/wt/sub");
        registry.register_common_gitdir(&nested, &nested_root);
        assert_eq!(
            registry.attribute_common_gitdir(Path::new("/main/.git/modules/sub/refs/heads/main")),
            vec![nested_root.clone()],
            "the nested common dir must win over the superproject's"
        );
        registry.unregister_common_gitdir(&nested, &nested_root);

        // Reference counting: the mapping survives the first dependent leaving.
        registry.unregister_common_gitdir(&common, &first);
        assert_eq!(
            registry.attribute_common_gitdir(Path::new("/main/.git/packed-refs")),
            vec![second.clone()],
            "the remaining dependent must keep receiving shared metadata"
        );

        registry.unregister_common_gitdir(&common, &second);
        assert!(
            registry
                .attribute_common_gitdir(Path::new("/main/.git/packed-refs"))
                .is_empty(),
            "the entry must be dropped once the last dependent unregisters"
        );
    }

    /// `attribute_submodule_parents` recovers the superproject root by splitting on
    /// `/.git/modules/`.
    ///
    /// Accepts only leaves that can move the gitlink, and never fires for the two layouts
    /// that look similar but hold no gitlink: an ordinary nested repository (in-tree `.git`)
    /// and a linked worktree's admin directory (#467).
    ///
    #[test]
    fn submodule_parents_derive_the_superproject_from_the_gitdir() {
        let registry = WatchRegistry::new();
        assert_eq!(
            registry.attribute_submodule_parents(Path::new("/super/.git/modules/sub/HEAD")),
            vec![PathBuf::from("/super")]
        );
        // The gitdir name is the submodule NAME, which may contain `/` and need
        // not match its working path -- the superproject is still recoverable.
        assert_eq!(
            registry.attribute_submodule_parents(Path::new("/super/.git/modules/libs/foo/index")),
            vec![PathBuf::from("/super")]
        );
        assert_eq!(
            registry
                .attribute_submodule_parents(Path::new("/super/.git/modules/sub/refs/heads/topic")),
            vec![PathBuf::from("/super")]
        );

        for internal in [
            "/super/.git/modules/sub/objects/ab/cdef",
            "/super/.git/modules/sub/refs/tags/v1.0.0",
            "/super/.git/modules/sub/hooks/pre-commit",
            "/super/.git/modules/sub/description",
        ] {
            assert!(
                registry
                    .attribute_submodule_parents(Path::new(internal))
                    .is_empty(),
                "{internal} cannot move the superproject's gitlink"
            );
        }

        for not_a_submodule in [
            // An ordinary nested repository keeps an in-tree `.git`.
            "/outer/nested/.git/HEAD",
            // A linked worktree's admin dir: an external gitdir under `.git`,
            // but the main checkout holds no gitlink to it.
            "/main/.git/worktrees/wt/HEAD",
            // Plain worktree content.
            "/super/sub/src/main.rs",
        ] {
            assert!(
                registry
                    .attribute_submodule_parents(Path::new(not_a_submodule))
                    .is_empty(),
                "{not_a_submodule} has no parent gitlink to refresh"
            );
        }
    }

    /// A submodule of a submodule moves TWO gitlinks: the grandparent's (via the path split)
    /// and the intermediate submodule's, which is only knowable from the external-gitdir
    /// registry.
    ///
    /// Isolated per test process by nextest.
    #[test]
    fn nested_submodule_parents_include_the_intermediate_working_root() {
        let registry = WatchRegistry::new();
        let inner_gitdir = PathBuf::from("/super/.git/modules/outer/modules/inner");
        let outer_gitdir = PathBuf::from("/super/.git/modules/outer");
        let outer_root = PathBuf::from("/super/outer");
        let inner_root = PathBuf::from("/super/outer/inner");

        // Only the grandparent is derivable while nothing is registered.
        assert_eq!(
            registry.attribute_submodule_parents(&inner_gitdir.join("HEAD")),
            vec![PathBuf::from("/super")]
        );

        registry.register_external_gitdir(&outer_gitdir, &outer_root);
        registry.register_external_gitdir(&inner_gitdir, &inner_root);

        let mut parents = registry.attribute_submodule_parents(&inner_gitdir.join("HEAD"));
        parents.sort();
        assert_eq!(
            parents,
            vec![PathBuf::from("/super"), outer_root],
            "both the grandparent and the intermediate submodule hold a moved gitlink"
        );

        // The submodule that changed is NOT its own parent: the external-gitdir
        // arm of `process_event` already delivers that one.
        assert!(
            !registry
                .attribute_submodule_parents(&inner_gitdir.join("HEAD"))
                .contains(&inner_root),
            "the changed submodule must not be fanned out to as a parent"
        );

        registry.unregister_external_gitdir(&inner_gitdir);
        registry.unregister_external_gitdir(&outer_gitdir);
    }

    /// gpy-agent#602 Part A: `is_registered_config_path` must be a pure lookup
    /// with zero `canonicalize`/`current_dir` calls in the per-event path.
    ///
    /// There is no fs-call-counting harness in this crate to assert that
    /// mechanically (unlike, say, a cache hit-counter), so this documents and
    /// exercises the invariant the same way nearby doc comments in this file
    /// justify allocation-/stat-free claims: by construction (read
    /// `is_registered_config_path`'s own doc comment — it is a bare
    /// `HashSet::contains`, no I/O of any kind) and by a behavioral proof that
    /// would fail if a `canonicalize` call had crept back in. Registering a
    /// path to a file that does NOT exist on disk means `Path::canonicalize`
    /// on it would return `Err`; if `is_registered_config_path` still called
    /// it (as it did before this fix, via `normalize_config_paths`), a
    /// canonicalize failure would still fall through to the absolute-path
    /// branch and this test wouldn't distinguish old from new behavior on its
    /// own — the decisive check is the second half below, which looks up an
    /// entirely different (never-registered) nonexistent path and confirms it
    /// is NOT matched, proving the lookup is exact set membership rather than
    /// some permissive fallback that would accept any unresolvable path.
    ///
    /// Isolated by construction: the path is registered on a registry owned by
    /// this test.
    #[test]
    fn config_path_lookup_is_pure_no_resolution() {
        let registry = WatchRegistry::new();
        let nonexistent = PathBuf::from("/tmp/gpy-agent-602-does-not-exist/gpy.toml");
        registry.register_config_path(&nonexistent);

        // Exact match: the byte-identical path used at registration is found,
        // with no canonicalize call required to succeed for a file that
        // doesn't exist.
        assert!(
            registry.is_registered_config_path(&nonexistent),
            "a registered path must be found by exact lookup, with no \
             canonicalize/current_dir call required for a file that doesn't exist"
        );

        // A different, never-registered nonexistent path must not match:
        // proves this isn't a permissive "any unresolvable path passes" bug.
        let unregistered = PathBuf::from("/tmp/gpy-agent-602-also-does-not-exist/other.toml");
        assert!(
            !registry.is_registered_config_path(&unregistered),
            "lookup must be exact HashSet::contains, matching only what was registered"
        );

        registry.unregister_config_path(&nonexistent);
        assert!(
            !registry.is_registered_config_path(&nonexistent),
            "unregister must remove the exact entry"
        );
    }

    /// gpy-agent#602 Part B: the `.sock` bug this issue names.
    ///
    /// A bare substring check (`path_str.contains(".sock")`) against the
    /// whole path incorrectly filters out a legitimately named file like
    /// `foo.socket.rs` (whose name contains the literal substring `.sock`)
    /// forever. The fix checks the file NAME's extension, not a path
    /// substring.
    #[test]
    fn socket_lookalike_filename_is_not_filtered() {
        assert!(
            filter_ignored(Path::new("/tmp/project/src/foo.socket.rs")).is_some(),
            "foo.socket.rs must NOT be filtered: `.sock` is a substring of \
             its name, not its extension"
        );
    }

    /// A real `.sock` file must still be filtered, exactly as before.
    #[test]
    fn real_sock_file_is_still_filtered() {
        assert!(
            filter_ignored(Path::new("/tmp/gpy-agent.sock")).is_none(),
            "the well-known gpy-agent.sock file must still be filtered"
        );
        assert!(
            filter_ignored(Path::new("/tmp/project/other.sock")).is_none(),
            "any other *.sock file must still be filtered"
        );
    }

    /// The exact `gpy-agent.log` filename must still be filtered, matched by
    /// file name equality rather than a path substring.
    #[test]
    fn gpy_agent_log_filename_is_still_filtered() {
        assert!(filter_ignored(Path::new("/var/log/gpy-agent.log")).is_none());
    }

    /// gpy-agent#602 Part B: `.components()`-based matching, not a hardcoded `/`.
    ///
    /// Every `filter_ignored`/`classify_event` site that used to do
    /// forward-slash-literal string matching must instead work on
    /// `.components()`, so it keeps working when the path uses the
    /// PLATFORM'S OWN separator rather than a hardcoded `/`.
    ///
    /// This crate's test-running platform is not necessarily Windows, so a
    /// literal `r"C:\Users\..."` string (as some existing tests in this crate
    /// use for Windows-specific *parsing* logic) would not actually exercise
    /// component-splitting here: on a Unix test run, `Path::new` treats a
    /// whole backslash-separated string as ONE opaque component, defeating
    /// the point. Building the path from an explicit list of path segments
    /// via `PathBuf::from_iter` instead joins them with whatever separator
    /// the compiling platform's `std::path` actually uses, so
    /// `.components()`-based logic is exercised identically on every
    /// platform `cargo test` might run on -- which is exactly the property
    /// being fixed (no more hardcoded `/`).
    #[test]
    fn classify_event_recognizes_platform_separated_paths() {
        let git_objects = PathBuf::from_iter([
            "C:", "Users", "dev", "project", ".git", "objects", "ab", "cdef1234",
        ]);
        assert!(
            filter_ignored(&git_objects).is_none(),
            "a .git/objects churn path must still be filtered regardless of platform separator"
        );

        let git_head = PathBuf::from_iter(["C:", "Users", "dev", "project", ".git", "HEAD"]);
        assert!(
            matches!(classify_event(&git_head), Some(FileEvent::Git { .. })),
            "a .git/HEAD path must still classify as a Git event regardless of platform separator"
        );

        let themes_dir = PathBuf::from_iter([
            "C:",
            "Users",
            "dev",
            ".config",
            "gpy",
            "themes",
            "default.toml",
        ]);
        assert!(
            matches!(classify_event(&themes_dir), Some(FileEvent::Theme { .. })),
            "a gpy/themes path must still classify as a Theme event regardless of platform separator"
        );
    }

    /// gpy-agent#602 Part C: previously-missing `metadata.rs` detection
    /// markers must now also trigger a language-cache refresh, via the shared
    /// `marker_file_names` set.
    #[test]
    fn previously_missing_marker_files_now_trigger_language_event() {
        for file_name in ["composer.json", "CMakeLists.txt", "tsconfig.json"] {
            let path = Path::new("/tmp/project").join(file_name);
            assert!(
                matches!(classify_event(&path), Some(FileEvent::Language { .. })),
                "{file_name} should now classify as a Language event (metadata.rs marker file)"
            );
        }
    }

    /// The residual lockfile/version-pin list (no `metadata.rs` marker
    /// equivalent) must keep triggering a language-cache refresh exactly as
    /// before.
    #[test]
    fn residual_lockfiles_still_trigger_language_event() {
        for file_name in [
            "package-lock.json",
            "Cargo.lock",
            "poetry.lock",
            "rust-toolchain",
            "go.work",
        ] {
            let path = Path::new("/tmp/project").join(file_name);
            assert!(
                matches!(classify_event(&path), Some(FileEvent::Language { .. })),
                "{file_name} must keep triggering a Language event (residual lockfile list)"
            );
        }
    }

    /// gpy-agent#602 Part B: nested submodules split at the FIRST pair.
    ///
    /// `split_at_git_dir_modules` must split at the FIRST `.git`/`modules`
    /// component pair, not the last, for a submodule of a submodule --
    /// matching `attribute_submodule_parents`'s own doc comment on why the
    /// grandparent split is deliberate.
    #[test]
    fn split_at_git_dir_modules_uses_first_occurrence_for_nested_submodules() {
        let nested = Path::new("/super/.git/modules/outer/modules/inner/HEAD");
        let (super_root, modules_root) =
            split_at_git_dir_modules(nested).expect("nested submodule path must split");
        assert_eq!(super_root, PathBuf::from("/super"));
        assert_eq!(modules_root, PathBuf::from("/super/.git/modules"));
    }

    /// A single-level submodule still splits correctly, matching the original
    /// byte-offset implementation's output exactly.
    #[test]
    fn split_at_git_dir_modules_single_level() {
        let path = Path::new("/super/.git/modules/sub/HEAD");
        let (super_root, modules_root) =
            split_at_git_dir_modules(path).expect("submodule path must split");
        assert_eq!(super_root, PathBuf::from("/super"));
        assert_eq!(modules_root, PathBuf::from("/super/.git/modules"));
    }

    /// A path with no `.git`/`modules` pair must not split.
    #[test]
    fn split_at_git_dir_modules_rejects_non_submodule_paths() {
        assert!(split_at_git_dir_modules(Path::new("/outer/nested/.git/HEAD")).is_none());
        assert!(split_at_git_dir_modules(Path::new("/main/.git/worktrees/wt/HEAD")).is_none());
    }
}
