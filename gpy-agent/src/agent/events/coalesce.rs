//! Per-repository single-flight for git refreshes: concurrent events coalesce
//! into one follow-up pass instead of racing stale scans.

use crate::debug_log;
use crate::sync_ext::MutexExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Serializes `refresh_and_notify_if_changed` per repository so a stale scan
/// cannot overwrite a fresher one; different repositories stay concurrent.
/// The inner lock covers only map bookkeeping, never the git scan.
#[derive(Default)]
pub struct RepoRefreshCoordinator {
    /// Present key = refresh in flight; value = follow-up pending.
    /// Entries are removed when the owner finishes, so the map stays bounded.
    state: Mutex<std::collections::HashMap<PathBuf, bool>>,
}

impl RepoRefreshCoordinator {
    /// Claim the repo's refresh. Returns `None` after recording a pending
    /// follow-up for the current owner.
    pub(super) fn try_start(&self, repo: &Path) -> Option<RefreshSlot<'_>> {
        let mut guard = self.state.lock_or_recover();
        match guard.entry(repo.to_path_buf()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                *entry.get_mut() = true;
                None
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(false);
                Some(RefreshSlot {
                    coordinator: self,
                    repo: repo.to_path_buf(),
                    held: true,
                })
            }
        }
    }
}

/// Ownership of one repository's refresh. Dropping it frees the slot, so a
/// panicking scan cannot leave the repository permanently "in flight".
pub(super) struct RefreshSlot<'a> {
    coordinator: &'a RepoRefreshCoordinator,
    repo: PathBuf,
    held: bool,
}

impl RefreshSlot<'_> {
    /// Returns `true` if exactly one more pass is owed; otherwise frees the slot.
    pub(super) fn needs_follow_up(&mut self) -> bool {
        let mut guard = self.coordinator.state.lock_or_recover();
        if let Some(pending) = guard.get_mut(&self.repo)
            && *pending
        {
            *pending = false;
            return true;
        }
        guard.remove(&self.repo);
        self.held = false;
        false
    }
}

impl Drop for RefreshSlot<'_> {
    /// Frees the slot ignoring any pending follow-up, unless
    /// [`RefreshSlot::needs_follow_up`] already did.
    fn drop(&mut self) {
        if self.held {
            self.coordinator.state.lock_or_recover().remove(&self.repo);
        }
    }
}

/// Cap on catch-up passes per coalesced refresh; the reconcile backstop handles the rest.
pub(super) const MAX_COALESCED_FOLLOW_UPS: u32 = 32;

/// Run `scan(false)`, then `scan(true)` until no follow-up is pending, bounded by
/// [`MAX_COALESCED_FOLLOW_UPS`]. Nothing else re-drives this, so drain until quiet.
pub(super) fn run_coalesced<F: FnMut(bool)>(
    coordinator: &RepoRefreshCoordinator,
    repo: &Path,
    mut scan: F,
) {
    let Some(mut slot) = coordinator.try_start(repo) else {
        // Another thread owns this repo and will service our pending change.
        return;
    };

    scan(false);

    let mut follow_ups = 0_u32;
    while slot.needs_follow_up() {
        if follow_ups >= MAX_COALESCED_FOLLOW_UPS {
            debug_log!(
                "agent",
                "Coalesced refresh hit follow-up cap ({}) for {}; deferring remaining churn to reconcile",
                MAX_COALESCED_FOLLOW_UPS,
                repo.display()
            );
            // Cap hit with a real pending change: service it once, defer further churn.
            scan(true);
            break;
        }
        follow_ups = follow_ups.saturating_add(1);
        scan(true);
    }
}
