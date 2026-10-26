//! The notify backend behind a [`FileSystemWatcher`].

use super::super::WatchRegistry;
use super::super::watch_set::{OsOp, WatchMode, WatchSet};
use super::{
    FileSystemWatcher, is_directory_create, is_in_git_dir, is_poll_directory_mtime_noise,
    path_is_gitignored,
};
use crate::sync_ext::MutexExt;
use crate::{Error, Result};
use notify::event::{EventKind, RemoveKind};
use notify::{Event, PollWatcher, RecommendedWatcher, Watcher};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Which notify backend is armed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BackendKind {
    /// `notify::RecommendedWatcher`.
    Native,
    /// `notify::PollWatcher`, used only when native proved silent.
    Poll,
}

impl BackendKind {
    /// Debug-log label.
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Poll => "poll",
        }
    }

    /// Whether removing one watch can remove overlapping ones.
    pub(super) const fn removal_cascades(self) -> bool {
        match self {
            Self::Native => NATIVE_REMOVAL_CASCADES,
            Self::Poll => false,
        }
    }
}

/// Whether native removal of one path also drops overlapping watches
/// (inotify/kqueue share descriptors; `FSEvents`/`ReadDirectoryChangesW` don't).
pub(super) const NATIVE_REMOVAL_CASCADES: bool =
    !cfg!(any(target_os = "macos", target_os = "windows"));

/// One logical `(path, mode)` registration.
#[derive(Debug)]
pub(super) struct Registration {
    /// Unmatched `watch` calls.
    pub(super) owners: u32,
    /// OS-level targets held; ordered so a removed subtree is a contiguous range.
    pub(super) targets: BTreeSet<(PathBuf, WatchMode)>,
}

/// The armed notify backend plus the paths it must watch.
///
/// One mutex so a backend swap and a concurrent `watch()` can't interleave.
/// Leaf lock: never held while taking `MultiRepoWatcher` locks or invoking callbacks.
pub(super) struct WatcherBackend {
    /// `None` once stopped.
    pub(super) watcher: Option<Box<dyn Watcher + Send>>,
    /// Logical registrations; single source of truth for what callers asked to watch.
    pub(super) registrations: BTreeMap<(PathBuf, WatchMode), Registration>,
    /// Ref-counted OS watches shared across overlapping registrations.
    pub(super) os: WatchSet,
    pub(super) kind: BackendKind,
}

impl WatcherBackend {
    /// A freshly armed backend with no registrations.
    pub(super) fn armed(watcher: Box<dyn Watcher + Send>, kind: BackendKind) -> Self {
        Self {
            watcher: Some(watcher),
            registrations: BTreeMap::new(),
            os: WatchSet::new(kind.removal_cascades()),
            kind,
        }
    }

    /// OS-level targets a `(path, mode)` registration arms on a `kind` backend.
    pub(super) fn targets_for(
        kind: BackendKind,
        path: &Path,
        mode: WatchMode,
        poll_targets: Option<Vec<PathBuf>>,
    ) -> BTreeSet<(PathBuf, WatchMode)> {
        match (kind, mode, poll_targets) {
            // Already expanded, so each directory is armed shallow.
            (BackendKind::Poll, WatchMode::Recursive, Some(targets)) => targets
                .into_iter()
                .map(|target| (target, WatchMode::Shallow))
                .collect(),
            _ => BTreeSet::from([(path.to_path_buf(), mode)]),
        }
    }

    /// Add one owner of `(path, mode)`, arming any newly required OS watches.
    ///
    /// # Errors
    /// Returns an error, with OS changes rolled back, if a target is rejected.
    pub(super) fn acquire(
        &mut self,
        path: &Path,
        mode: WatchMode,
        poll_targets: Option<Vec<PathBuf>>,
    ) -> Result<()> {
        if self.watcher.is_none() {
            return Ok(());
        }
        let key = (path.to_path_buf(), mode);
        if let Some(registration) = self.registrations.get_mut(&key) {
            registration.owners = registration.owners.saturating_add(1);
            return Ok(());
        }

        let targets = Self::targets_for(self.kind, path, mode, poll_targets);
        let mut ops = Vec::new();
        for (target, target_mode) in &targets {
            ops.extend(self.os.acquire(target, *target_mode));
        }
        let failures = self.apply(ops);
        let own_failure = failures
            .into_iter()
            .find(|(failed, _)| targets.iter().any(|(target, _)| target == failed));
        if let Some((_, error)) = own_failure {
            let mut rollback = Vec::new();
            for (target, target_mode) in &targets {
                rollback.extend(self.os.release(target, *target_mode).unwrap_or_default());
            }
            self.apply(rollback);
            return Err(Error::watcher(format!("Failed to watch path: {error}")));
        }

        self.registrations
            .insert(key, Registration { owners: 1, targets });
        Ok(())
    }

    /// Drop one owner of `(path, mode)`; OS watches go once no registration targets them.
    ///
    /// # Errors
    /// Returns an error if no owner holds `(path, mode)`.
    pub(super) fn release(&mut self, path: &Path, mode: WatchMode) -> Result<()> {
        if self.watcher.is_none() {
            return Ok(());
        }
        let key = (path.to_path_buf(), mode);
        let Some(registration) = self.registrations.get_mut(&key) else {
            return Err(Error::watcher(format!(
                "Failed to unwatch path: {} is not watched ({mode:?})",
                path.display()
            )));
        };
        if registration.owners > 1 {
            registration.owners = registration.owners.saturating_sub(1);
            return Ok(());
        }
        let Some(released) = self.registrations.remove(&key) else {
            return Ok(());
        };
        let mut ops = Vec::new();
        for (target, target_mode) in &released.targets {
            ops.extend(self.os.release(target, *target_mode).unwrap_or_default());
        }
        self.apply(ops);
        Ok(())
    }

    /// Apply `ops` as one batch (one `FSEventStream` rebuild on macOS), returning
    /// failed adds. Removal failures are only logged.
    pub(super) fn apply(&mut self, ops: Vec<OsOp>) -> Vec<(PathBuf, notify::Error)> {
        let mut failures = Vec::new();
        if ops.is_empty() {
            return failures;
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return failures;
        };
        let mut batch = watcher.paths_mut();
        for op in ops {
            match op {
                OsOp::Add(path, mode) => {
                    if let Err(error) = batch.add(&path, mode.recursive_mode()) {
                        crate::debug_log!(
                            "watcher",
                            "failed to watch {} ({mode:?}): {error}",
                            path.display()
                        );
                        self.os.record_failed_add(&path, mode);
                        failures.push((path, error));
                    }
                }
                OsOp::Remove(path) => {
                    if let Err(error) = batch.remove(&path) {
                        crate::debug_log!(
                            "watcher",
                            "failed to unwatch {}: {error}",
                            path.display()
                        );
                    }
                }
            }
        }
        if let Err(error) = batch.commit() {
            crate::debug_log!("watcher", "failed to commit watch changes: {error}");
        }
        failures
    }

    /// Whether a registration delivers events for `target`.
    pub(super) fn covers(&self, target: &Path) -> bool {
        self.registrations.keys().any(|(watched, mode)| match mode {
            WatchMode::Recursive => target.starts_with(watched),
            WatchMode::Shallow => watched == target,
        })
    }

    /// Recursive registrations whose poll expansion holds `dir`'s parent but not
    /// `dir`. Empty off the poll backend.
    pub(super) fn poll_expansion_owners(&self, dir: &Path) -> Vec<PathBuf> {
        if self.kind != BackendKind::Poll || self.watcher.is_none() {
            return Vec::new();
        }
        let Some(parent) = dir.parent() else {
            return Vec::new();
        };
        let parent_target = (parent.to_path_buf(), WatchMode::Shallow);
        let dir_target = (dir.to_path_buf(), WatchMode::Shallow);
        self.registrations
            .iter()
            .filter(|((_, mode), registration)| {
                *mode == WatchMode::Recursive
                    && registration.targets.contains(&parent_target)
                    && !registration.targets.contains(&dir_target)
            })
            .map(|((root, _), _)| root.clone())
            .collect()
    }

    /// Add each root's newly found directories to its poll expansion, skipping
    /// released roots and already-held directories.
    pub(super) fn extend_poll_expansion(&mut self, expansions: Vec<(PathBuf, Vec<PathBuf>)>) {
        if self.kind != BackendKind::Poll || self.watcher.is_none() {
            return;
        }
        let mut ops = Vec::new();
        for (root, dirs) in expansions {
            let Some(registration) = self.registrations.get_mut(&(root, WatchMode::Recursive))
            else {
                continue;
            };
            for dir in dirs {
                let target = (dir, WatchMode::Shallow);
                if registration.targets.contains(&target) {
                    continue;
                }
                ops.extend(self.os.acquire(&target.0, WatchMode::Shallow));
                registration.targets.insert(target);
            }
        }
        // Kept in the expansion on failure; the parent's scan reports removal.
        for (path, error) in self.apply(ops) {
            crate::debug_log!(
                "watcher",
                "poll backend could not arm new {}: {error}",
                path.display()
            );
        }
    }

    /// Drop `removed` and its subtree from every recursive poll expansion.
    /// Roots stay: `PollWatcher` watches by path and survives recreation.
    pub(super) fn drop_poll_expansion(&mut self, removed: &Path) {
        if self.kind != BackendKind::Poll || self.watcher.is_none() {
            return;
        }
        let mut ops = Vec::new();
        for ((root, mode), registration) in &mut self.registrations {
            if *mode != WatchMode::Recursive {
                continue;
            }
            // Component-wise ordering makes the subtree a contiguous range.
            let gone: Vec<(PathBuf, WatchMode)> = registration
                .targets
                .range((removed.to_path_buf(), WatchMode::Shallow)..)
                .take_while(|(target, _)| target.starts_with(removed))
                .filter(|(target, _)| target != root)
                .cloned()
                .collect();
            for gone_target in &gone {
                registration.targets.remove(gone_target);
                let (target, target_mode) = gone_target;
                ops.extend(self.os.release(target, *target_mode).unwrap_or_default());
            }
        }
        self.apply(ops);
    }
}

/// Build the OS-native backend (`notify::RecommendedWatcher`).
///
/// # Errors
/// Returns an error if the platform backend cannot be initialized.
pub(super) fn build_native_watcher(event_tx: &Sender<Event>) -> Result<Box<dyn Watcher + Send>> {
    let tx_clone = event_tx.clone();
    let watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| FileSystemWatcher::forward_event(&tx_clone, res),
        notify::Config::default(),
    )
    .map_err(|e| Error::watcher(format!("Failed to create file watcher: {e}")))?;
    Ok(Box::new(watcher))
}

/// Build the polling fallback backend (`notify::PollWatcher`).
///
/// Content comparison is on because notify compares only whole-second mtimes,
/// missing same-second rewrites (e.g. a moved ref).
///
/// # Errors
/// Returns an error if the poll watcher cannot be initialized.
pub(super) fn build_poll_watcher(
    event_tx: &Sender<Event>,
    poll_interval: Duration,
) -> Result<Box<dyn Watcher + Send>> {
    let tx_clone = event_tx.clone();
    let watcher = PollWatcher::new(
        move |res: notify::Result<Event>| match res {
            Ok(event) if is_poll_directory_mtime_noise(&event) => {}
            other => FileSystemWatcher::forward_event(&tx_clone, other),
        },
        notify::Config::default()
            .with_poll_interval(poll_interval)
            .with_compare_contents(true),
    )
    .map_err(|e| Error::watcher(format!("Failed to create poll-fallback watcher: {e}")))?;
    Ok(Box::new(watcher))
}

/// Directories under `root` the poll backend arms individually, shallow.
///
/// `PollWatcher` has no `.gitignore` awareness and would re-walk ignored
/// subtrees every tick, pinning a core; this prunes them (never inside `.git`).
/// `root` is always first.
pub(super) fn poll_watch_targets(root: &Path, registry: &Arc<WatchRegistry>) -> Vec<PathBuf> {
    poll_targets_from(root, root, registry)
}

/// What `dir`, created after `root` was armed, adds to `root`'s poll expansion.
/// Empty if pruned, gone, or a symlink.
pub(super) fn poll_watch_targets_within(
    root: &Path,
    dir: &Path,
    registry: &Arc<WatchRegistry>,
) -> Vec<PathBuf> {
    let is_real_dir = std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir());
    if !is_real_dir || poll_prunes(root, dir, registry) {
        return Vec::new();
    }
    poll_targets_from(root, dir, registry)
}

/// `start` plus every directory below it that `root`'s poll expansion keeps.
pub(super) fn poll_targets_from(
    root: &Path,
    start: &Path,
    registry: &Arc<WatchRegistry>,
) -> Vec<PathBuf> {
    let mut targets = vec![start.to_path_buf()];
    let mut stack = vec![start.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let path = entry.path();
            if poll_prunes(root, &path, registry) {
                continue;
            }
            targets.push(path.clone());
            stack.push(path);
        }
    }

    targets
}

/// Whether `root`'s poll expansion skips `path`: gitignored and not in `.git`.
pub(super) fn poll_prunes(root: &Path, path: &Path, registry: &Arc<WatchRegistry>) -> bool {
    !is_in_git_dir(path) && path_is_gitignored(root, path, registry)
}

/// Keep the poll expansion in step with the tree: `PollWatcher` scans one
/// level deep, so created dirs must join and removed ones leave. No-op on native.
pub(super) fn track_poll_expansion(
    event: &Event,
    backend: &Mutex<WatcherBackend>,
    registry: &Arc<WatchRegistry>,
) {
    match event.kind {
        EventKind::Create(_) => {
            for path in &event.paths {
                if is_directory_create(event.kind, path) {
                    expand_poll_into(path, backend, registry);
                }
            }
        }
        // Files are never expansion targets.
        EventKind::Remove(kind) if kind != RemoveKind::File => {
            let mut backend_guard = backend.lock_or_recover();
            for path in &event.paths {
                backend_guard.drop_poll_expansion(path);
            }
        }
        _ => {}
    }
}

/// Add newly created `dir` and its contents to every owning root's poll expansion.
///
/// Walks with the backend lock released: it may run `git ls-files`.
pub(super) fn expand_poll_into(
    dir: &Path,
    backend: &Mutex<WatcherBackend>,
    registry: &Arc<WatchRegistry>,
) {
    let owners = backend.lock_or_recover().poll_expansion_owners(dir);
    if owners.is_empty() {
        return;
    }
    let expansions: Vec<(PathBuf, Vec<PathBuf>)> = owners
        .into_iter()
        .map(|root| {
            let dirs = poll_watch_targets_within(&root, dir, registry);
            (root, dirs)
        })
        .collect();
    backend.lock_or_recover().extend_poll_expansion(expansions);
}
