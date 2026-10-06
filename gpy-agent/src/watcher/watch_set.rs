//! Reference-counted OS watch bookkeeping for [`super::filesystem`] (#687).
//!
//! `notify` backends keep no owner information: one `unwatch(path)` undoes
//! every `watch(path)` ever made, and on some backends more than that. Several
//! gpy owners routinely watch overlapping paths -- a nested repository or
//! submodule inside another registered repository, a linked worktree's shared
//! common directory that is also its main checkout's gitdir -- so forwarding
//! each owner's `unwatch` straight to the backend silently removes watches
//! another owner still relies on.
//!
//! [`WatchSet`] is the single record of what is wanted and what is armed. Each
//! `(path, mode)` pair carries an owner count; the OS sees at most one watch per
//! path, in the strongest mode any owner still holds. Every change is returned
//! as a list of [`OsOp`]s for the caller to apply to the backend in one batch
//! (one `FSEventStream` rebuild on macOS, #388), so this module stays pure and
//! is unit-testable without a real backend.

use notify::RecursiveMode;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How a path is watched.
///
/// Ordered so the stronger mode compares greater: a path held both ways is
/// armed [`WatchMode::Recursive`], which also delivers everything a shallow
/// watch would.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WatchMode {
    /// The directory itself and its direct entries only.
    Shallow,
    /// The directory and every descendant.
    Recursive,
}

impl WatchMode {
    /// The `notify` mode this maps to.
    pub const fn recursive_mode(self) -> RecursiveMode {
        match self {
            Self::Shallow => RecursiveMode::NonRecursive,
            Self::Recursive => RecursiveMode::Recursive,
        }
    }
}

/// One change to apply to the OS backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OsOp {
    /// Arm `path` in `mode`.
    Add(PathBuf, WatchMode),
    /// Remove the backend's watch on `path`.
    Remove(PathBuf),
}

/// Owner counts and armed state for one path.
#[derive(Debug, Default)]
struct Entry {
    recursive: u32,
    shallow: u32,
    /// What the backend currently has armed for this exact path, as far as
    /// this set knows. `None` when never armed or when arming failed.
    armed: Option<WatchMode>,
}

impl Entry {
    /// The strongest mode any owner still holds.
    const fn desired(&self) -> Option<WatchMode> {
        if self.recursive > 0 {
            Some(WatchMode::Recursive)
        } else if self.shallow > 0 {
            Some(WatchMode::Shallow)
        } else {
            None
        }
    }

    const fn count_mut(&mut self, mode: WatchMode) -> &mut u32 {
        match mode {
            WatchMode::Recursive => &mut self.recursive,
            WatchMode::Shallow => &mut self.shallow,
        }
    }
}

/// Reference-counted set of OS-level watches. See the module docs.
#[derive(Debug)]
pub struct WatchSet {
    entries: BTreeMap<PathBuf, Entry>,
    /// Whether removing a watch on the backend also removes watches on other,
    /// overlapping paths. True for inotify: `notify` keeps one watch
    /// descriptor per directory, shared by every registration that walked
    /// it, and removing a recursive entry also drops every descriptor under
    /// it. False for `FSEvents` (each stream entry is independent; only
    /// entries equal to the path are removed) and for `PollWatcher` (exact
    /// key removal). The same per-directory walk is why a failed recursive
    /// add can leave part of a tree armed (#722, see
    /// [`WatchSet::record_failed_add`]).
    removal_cascades: bool,
}

impl WatchSet {
    /// An empty set for a backend with the given removal semantics.
    pub const fn new(removal_cascades: bool) -> Self {
        Self {
            entries: BTreeMap::new(),
            removal_cascades,
        }
    }

    /// Add one owner of `(path, mode)`, returning the backend changes needed.
    pub fn acquire(&mut self, path: &Path, mode: WatchMode) -> Vec<OsOp> {
        let entry = self.entries.entry(path.to_path_buf()).or_default();
        let count = entry.count_mut(mode);
        *count = count.saturating_add(1);
        self.reconcile(path)
    }

    /// Drop one owner of `(path, mode)`, returning the backend changes needed,
    /// or `None` when no owner holds that pair.
    pub fn release(&mut self, path: &Path, mode: WatchMode) -> Option<Vec<OsOp>> {
        let count = self.entries.get_mut(path)?.count_mut(mode);
        if *count == 0 {
            return None;
        }
        *count = count.saturating_sub(1);
        Some(self.reconcile(path))
    }

    /// The OS watches currently armed, by path. Test-only observability of
    /// what the backend holds (#718).
    #[cfg(test)]
    pub fn armed_paths(&self) -> BTreeMap<PathBuf, WatchMode> {
        self.entries
            .iter()
            .filter_map(|(path, entry)| Some((path.clone(), entry.armed?)))
            .collect()
    }

    /// Record that arming `path` on the backend failed, so the set no longer
    /// believes it armed. A later [`WatchSet::release`] or
    /// [`WatchSet::acquire`] of the path then re-adds whatever mode is still
    /// wanted.
    fn mark_unarmed(&mut self, path: &Path) {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.armed = None;
        }
    }

    /// Record that the backend rejected [`OsOp::Add`] of `(path, mode)`.
    ///
    /// Usually nothing was armed, so `path` is marked unarmed and the next
    /// reconcile re-adds whatever mode is still wanted. A recursive add on a
    /// backend whose removal cascades is the exception (#722): inotify and
    /// kqueue arm the tree one directory at a time and stop at the first one
    /// they cannot watch, without undoing the ones already armed. `path` then
    /// stays armed here, so the reconcile that drops its last owner removes
    /// that partial tree -- one removal of `path` takes every descriptor under
    /// it -- and re-arms the overlapping watches the removal took with it,
    /// exactly as for a fully armed watch. When nothing was armed at all, that
    /// removal finds no watch, which the caller only logs.
    pub fn record_failed_add(&mut self, path: &Path, mode: WatchMode) {
        if mode == WatchMode::Recursive && self.removal_cascades {
            return;
        }
        self.mark_unarmed(path);
    }

    /// Bring the backend's watch on `path` in line with its owner counts.
    ///
    /// A path whose effective mode changes is removed and re-added rather
    /// than added on top: `FSEvents` would otherwise keep a stale duplicate
    /// stream entry, and inotify would keep the recursive descriptors of a
    /// downgraded watch. When the backend's removal cascades, every other
    /// armed path the removal may have taken with it is re-added too: paths
    /// nested under `path`, and recursive ancestors whose walk shared
    /// `path`'s descriptors.
    fn reconcile(&mut self, path: &Path) -> Vec<OsOp> {
        let Some(entry) = self.entries.get_mut(path) else {
            return Vec::new();
        };
        let desired = entry.desired();
        let previously_armed = entry.armed;
        if desired == previously_armed {
            if desired.is_none() {
                self.entries.remove(path);
            }
            return Vec::new();
        }

        let mut ops = Vec::new();
        if previously_armed.is_some() {
            ops.push(OsOp::Remove(path.to_path_buf()));
        }
        if let Some(mode) = desired {
            entry.armed = Some(mode);
            ops.push(OsOp::Add(path.to_path_buf(), mode));
        } else {
            self.entries.remove(path);
        }

        if previously_armed.is_some() && self.removal_cascades {
            ops.extend(self.overlapping_rearms(path));
        }
        ops
    }

    /// Re-add every armed path, other than `removed` itself, whose backend
    /// watch overlaps `removed` (see [`WatchSet::reconcile`]).
    fn overlapping_rearms(&self, removed: &Path) -> Vec<OsOp> {
        self.entries
            .iter()
            .filter(|(path, _)| path.as_path() != removed)
            .filter_map(|(path, entry)| {
                let mode = entry.armed?;
                let nested = path.starts_with(removed);
                let covering_ancestor = mode == WatchMode::Recursive && removed.starts_with(path);
                (nested || covering_ancestor).then(|| OsOp::Add(path.clone(), mode))
            })
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[allow(clippy::expect_used)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;

    fn add(path: &str, mode: WatchMode) -> OsOp {
        OsOp::Add(PathBuf::from(path), mode)
    }

    fn remove(path: &str) -> OsOp {
        OsOp::Remove(PathBuf::from(path))
    }

    const REC: WatchMode = WatchMode::Recursive;
    const SHALLOW: WatchMode = WatchMode::Shallow;

    #[test]
    fn second_owner_of_a_path_changes_nothing_on_the_backend() {
        let mut set = WatchSet::new(false);
        assert_eq!(set.acquire(Path::new("/r"), REC), vec![add("/r", REC)]);
        assert_eq!(set.acquire(Path::new("/r"), REC), Vec::<OsOp>::new());
        assert_eq!(set.release(Path::new("/r"), REC), Some(Vec::new()));
        assert_eq!(set.release(Path::new("/r"), REC), Some(vec![remove("/r")]));
        assert_eq!(set.release(Path::new("/r"), REC), None);
    }

    #[test]
    fn release_of_a_pair_nobody_holds_is_rejected() {
        let mut set = WatchSet::new(false);
        set.acquire(Path::new("/r"), SHALLOW);
        assert_eq!(set.release(Path::new("/r"), REC), None);
        assert_eq!(set.release(Path::new("/other"), SHALLOW), None);
    }

    /// The `FSEvents` failure in #687: a worktree holds `<common>` shallow, the
    /// main checkout then holds the same path recursively. Releasing the
    /// recursive owner must leave the shallow watch armed.
    #[test]
    fn releasing_recursive_owner_downgrades_to_remaining_shallow_owner() {
        let mut set = WatchSet::new(false);
        assert_eq!(
            set.acquire(Path::new("/c"), SHALLOW),
            vec![add("/c", SHALLOW)]
        );
        assert_eq!(
            set.acquire(Path::new("/c"), REC),
            vec![remove("/c"), add("/c", REC)]
        );
        assert_eq!(
            set.release(Path::new("/c"), REC),
            Some(vec![remove("/c"), add("/c", SHALLOW)])
        );
        assert_eq!(
            set.release(Path::new("/c"), SHALLOW),
            Some(vec![remove("/c")])
        );
    }

    #[test]
    fn non_cascading_removal_leaves_overlapping_paths_alone() {
        let mut set = WatchSet::new(false);
        set.acquire(Path::new("/outer"), REC);
        set.acquire(Path::new("/outer/inner"), REC);
        assert_eq!(
            set.release(Path::new("/outer"), REC),
            Some(vec![remove("/outer")])
        );
    }

    /// inotify: removing the outer recursive watch drops the nested
    /// repository's descriptors too, so the nested watch must be re-added.
    #[test]
    fn cascading_removal_of_outer_rearms_nested_paths() {
        let mut set = WatchSet::new(true);
        set.acquire(Path::new("/outer"), REC);
        set.acquire(Path::new("/outer/inner"), REC);
        set.acquire(Path::new("/outer/inner/sub"), SHALLOW);
        set.acquire(Path::new("/elsewhere"), REC);
        assert_eq!(
            set.release(Path::new("/outer"), REC),
            Some(vec![
                remove("/outer"),
                add("/outer/inner", REC),
                add("/outer/inner/sub", SHALLOW),
            ])
        );
    }

    /// inotify: removing a nested watch re-adds recursive ancestors only.
    ///
    /// The removal drops descriptors the outer recursive walk shares. A
    /// shallow ancestor shares no descriptor below itself and is left alone.
    #[test]
    fn cascading_removal_of_nested_rearms_recursive_ancestors_only() {
        let mut set = WatchSet::new(true);
        set.acquire(Path::new("/outer"), REC);
        set.acquire(Path::new("/outer/mid"), SHALLOW);
        set.acquire(Path::new("/outer/mid/inner"), REC);
        assert_eq!(
            set.release(Path::new("/outer/mid/inner"), REC),
            Some(vec![remove("/outer/mid/inner"), add("/outer", REC)])
        );
    }

    #[test]
    fn sibling_with_shared_name_prefix_is_not_treated_as_nested() {
        let mut set = WatchSet::new(true);
        set.acquire(Path::new("/repo"), REC);
        set.acquire(Path::new("/repo-two"), REC);
        assert_eq!(
            set.release(Path::new("/repo"), REC),
            Some(vec![remove("/repo")])
        );
    }

    #[test]
    fn failed_arm_is_retried_by_the_next_reconcile() {
        let mut set = WatchSet::new(false);
        set.acquire(Path::new("/c"), SHALLOW);
        assert_eq!(
            set.acquire(Path::new("/c"), REC),
            vec![remove("/c"), add("/c", REC)]
        );
        // The recursive add failed: the backend now holds nothing for `/c`.
        set.mark_unarmed(Path::new("/c"));
        assert_eq!(
            set.release(Path::new("/c"), REC),
            Some(vec![add("/c", SHALLOW)])
        );
    }

    /// #722: a partially armed recursive tree is still removed with its owner.
    ///
    /// inotify arms a recursive watch one directory at a time and stops at the
    /// first one it cannot watch, without undoing the ones already armed.
    /// Releasing the failed path must remove that partial tree, and re-arm
    /// the nested watch the removal takes with it.
    #[test]
    fn failed_recursive_add_on_cascading_backend_is_still_removed() {
        let mut set = WatchSet::new(true);
        set.acquire(Path::new("/r/inner"), REC);
        assert_eq!(set.acquire(Path::new("/r"), REC), vec![add("/r", REC)]);
        set.record_failed_add(Path::new("/r"), REC);
        assert_eq!(
            set.release(Path::new("/r"), REC),
            Some(vec![remove("/r"), add("/r/inner", REC)])
        );
    }
}
