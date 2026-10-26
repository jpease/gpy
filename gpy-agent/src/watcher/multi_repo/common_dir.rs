//! Reference-counted watches on the common git directory shared by linked worktrees.

use super::gitdir::resolve_common_dir;
use super::{ArmedWatches, MultiRepoWatcher, WatchTarget, unwatch_logged};
use crate::debug_log;
use crate::sync_ext::MutexExt;
use crate::watcher::watch_set::WatchMode;
use std::path::{Path, PathBuf};

/// Add `git_root` as a dependent of its common dir; the first arms watches.
pub(super) fn acquire_common_dir_watches<T: WatchTarget>(
    watcher: &MultiRepoWatcher<T>,
    w: &mut T,
    git_root: &Path,
    git_dir: &Path,
) {
    let Some(common_dir) = resolve_common_dir(git_dir) else {
        return;
    };

    {
        let mut guard = watcher.common_watches.lock_or_recover();
        let entry = guard.entry(common_dir.clone()).or_default();
        let first_dependent = entry.dependents.is_empty();
        entry.dependents.insert(git_root.to_path_buf());
        if first_dependent {
            entry.armed = arm_common_dir(w, &common_dir);
        }
        drop(guard);
    }

    watcher
        .registry
        .register_common_gitdir(&common_dir, git_root);
}

/// Watch `<common>` shallow (`packed-refs`, `config`; a directory watch
/// survives git's lock-then-rename) and `<common>/refs` recursive (covers a
/// future `remotes/`). Never recursive over `<common>`: the poll backend
/// would stat-walk `objects/**`. Already-covered paths are skipped.
fn arm_common_dir(w: &mut impl WatchTarget, common_dir: &Path) -> ArmedWatches {
    let mut armed = Vec::new();

    if w.covers(common_dir) {
        debug_log!(
            "watcher",
            "Common gitdir {} already covered by an existing watch",
            common_dir.display()
        );
    } else if let Err(e) = w.watch_shallow(common_dir) {
        debug_log!(
            "watcher",
            "Shallow watch of common gitdir {} failed ({}); shared metadata changes may be missed",
            common_dir.display(),
            e
        );
    } else {
        armed.push((common_dir.to_path_buf(), WatchMode::Shallow));
    }

    let refs_dir = common_dir.join("refs");
    if refs_dir.is_dir() && !w.covers(&refs_dir) {
        if let Err(e) = w.watch(&refs_dir) {
            debug_log!(
                "watcher",
                "Watch of common refs {} failed ({}); shared ref changes may be missed",
                refs_dir.display(),
                e
            );
        } else {
            armed.push((refs_dir, WatchMode::Recursive));
        }
    }

    armed
}

/// Drop `git_root` from every common dir; unwatch when the last dependent
/// leaves, and re-arm survivors whose covering ancestor watch just went.
pub(super) fn release_common_dir_watches<T: WatchTarget>(
    watcher: &MultiRepoWatcher<T>,
    git_root: &Path,
) {
    let mut to_unwatch: ArmedWatches = Vec::new();
    let mut to_rearm: Vec<PathBuf> = Vec::new();

    // Release `common_watches` before taking the watcher lock: acquire
    // takes them in the opposite order.
    {
        let mut guard = watcher.common_watches.lock_or_recover();
        guard.retain(|common_dir, entry| {
            if entry.dependents.remove(git_root) {
                watcher
                    .registry
                    .unregister_common_gitdir(common_dir, git_root);
            }
            if entry.dependents.is_empty() {
                to_unwatch.append(&mut entry.armed);
                return false;
            }
            true
        });
        // Entries that armed nothing relied on an ancestor watch (usually
        // the main checkout's), which may have just been dropped.
        to_rearm.extend(
            guard
                .iter()
                .filter(|(_, entry)| entry.armed.is_empty())
                .map(|(common_dir, _)| common_dir.clone()),
        );
    }

    if to_unwatch.is_empty() && to_rearm.is_empty() {
        return;
    }

    let mut rearmed: Vec<(PathBuf, ArmedWatches)> = Vec::new();
    if let Some(w) = watcher.watcher.lock_or_recover().as_mut() {
        for (path, mode) in &to_unwatch {
            unwatch_logged(w, path, *mode);
        }
        for common_dir in to_rearm {
            let armed = arm_common_dir(w, &common_dir);
            if !armed.is_empty() {
                rearmed.push((common_dir, armed));
            }
        }
    }

    if rearmed.is_empty() {
        return;
    }
    let mut guard = watcher.common_watches.lock_or_recover();
    for (common_dir, armed) in rearmed {
        if let Some(entry) = guard.get_mut(&common_dir) {
            entry.armed = armed;
        }
    }
}
