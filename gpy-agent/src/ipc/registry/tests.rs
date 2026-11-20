#[cfg(unix)]
use super::IDENTITY_TRUST;
use super::{ClientDirectory, ClientInfo, PidIdentity};
use crate::sync_ext::MutexExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

impl ClientDirectory {
    #[cfg(unix)]
    fn client_cwd(&self, pid: u32) -> Option<PathBuf> {
        self.inner
            .lock_or_recover()
            .get(&pid)
            .and_then(|info| info.cwd.clone())
    }

    /// Registered PIDs.
    #[must_use]
    pub fn clients(&self) -> Vec<u32> {
        self.inner.lock_or_recover().keys().copied().collect()
    }

    /// Register with an explicit `started_at`, to test PID-recycle handling.
    /// The start time has never been verified, so the next ring looks it up.
    pub(crate) fn register_with_started_at_for_test(
        &self,
        pid: u32,
        cwd: Option<PathBuf>,
        started_at: Option<u64>,
    ) {
        self.inner.lock_or_recover().insert(
            pid,
            ClientInfo {
                cwd,
                started_at,
                verified_at: None,
            },
        );
    }

    /// When `pid`'s start time last matched, if it is registered.
    fn verified_at(&self, pid: u32) -> Option<Instant> {
        self.inner
            .lock_or_recover()
            .get(&pid)
            .and_then(|info| info.verified_at)
    }

    /// Overwrite when `pid`'s start time last matched.
    #[cfg(unix)]
    fn set_verified_at_for_test(&self, pid: u32, verified_at: Option<Instant>) {
        if let Some(info) = self.inner.lock_or_recover().get_mut(&pid) {
            info.verified_at = verified_at;
        }
    }

    /// A directory whose doorbell counts rings into the returned counter
    /// instead of signalling, with this process as its one client,
    /// subscribed to every repository.
    #[cfg(unix)]
    pub(crate) fn counting_rings() -> (Self, Arc<AtomicU64>) {
        let rings = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&rings);
        let directory = Self {
            doorbell: Box::new(move |_pid| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            ..Self::new()
        };
        directory.register(std::process::id(), None);
        (directory, rings)
    }
}

#[cfg(unix)]
#[test]
/// Every related client is signaled in one broadcast, not just the first one
/// `HashMap` iteration visits; both nonexistent PIDs get ESRCH and are pruned.
fn notify_delivers_to_both_clients_in_same_repo_within_one_broadcast() {
    let directory = ClientDirectory::new();
    let repo = PathBuf::from("/repo");

    directory.register_with_started_at_for_test(999_990, Some(repo.clone()), None);
    directory.register_with_started_at_for_test(999_991, Some(repo.clone()), None);
    assert_eq!(directory.len(), 2, "Should have 2 clients registered");

    directory.notify_repaint(Some(&repo));

    assert_eq!(
        directory.len(),
        0,
        "both clients in the same repo must be signaled (and pruned via ESRCH) \
         within one broadcast"
    );
}

#[test]
/// Shells rely on this `false` to detect an agent restart and re-register.
fn update_workspace_fails_when_not_registered() {
    let directory = ClientDirectory::new();
    let unregistered_pid = 99999_u32;
    let cwd = Path::new("/test/path");

    let result = directory.update_workspace(unregistered_pid, cwd);
    assert!(
        !result,
        "update_workspace should return false for unregistered PID"
    );
}

#[test]
fn update_workspace_succeeds_when_registered() {
    let directory = ClientDirectory::new();
    let pid = 12345_u32;
    let initial_cwd = Path::new("/initial/path");
    let new_cwd = Path::new("/new/path");

    directory.register(pid, Some(initial_cwd.to_path_buf()));

    let result = directory.update_workspace(pid, new_cwd);
    assert!(
        result,
        "update_workspace should return true for registered PID"
    );
}

#[test]
fn update_workspace_fails_after_unregister() {
    let directory = ClientDirectory::new();
    let pid = 12345_u32;
    let cwd = Path::new("/test/path");

    directory.register(pid, Some(cwd.to_path_buf()));
    directory.unregister(pid);

    let result = directory.update_workspace(pid, cwd);
    assert!(
        !result,
        "update_workspace should return false after unregister"
    );
}

#[test]
fn is_registered_tracks_registration_membership() {
    let directory = ClientDirectory::new();
    let pid = 4242_u32;

    assert!(
        !directory.is_registered(pid),
        "an unknown PID must not report as registered"
    );

    directory.register(pid, Some(Path::new("/test/path").to_path_buf()));
    assert!(
        directory.is_registered(pid),
        "a registered PID must report as registered"
    );

    directory.unregister(pid);
    assert!(
        !directory.is_registered(pid),
        "an unregistered PID must not report as registered"
    );
}

#[test]
#[cfg(unix)]
fn is_client_alive_current_process() {
    let pid = std::process::id();
    assert!(
        ClientDirectory::is_client_alive(pid),
        "Current process should be detected as alive"
    );
}

#[test]
#[cfg(unix)]
fn prune_dead_clients_removes_dead() {
    let directory = ClientDirectory::new();
    let current_pid = std::process::id();
    let fake_pid = 999_999_u32;

    directory.register(current_pid, Some(Path::new("/test1").to_path_buf()));
    directory.register(fake_pid, Some(Path::new("/test2").to_path_buf()));

    assert_eq!(directory.len(), 2, "Should have 2 clients registered");

    assert_eq!(
        directory.prune_dead_clients(),
        vec![fake_pid],
        "only the dead client is pruned"
    );
    assert_eq!(
        directory.clients(),
        vec![current_pid],
        "only the live client remains registered"
    );
}

#[test]
#[cfg(unix)]
fn prune_dead_clients_no_pruning_needed() {
    let directory = ClientDirectory::new();
    let current_pid = std::process::id();

    directory.register(current_pid, Some(Path::new("/test").to_path_buf()));

    assert_eq!(directory.len(), 1, "Should have 1 client registered");

    let pruned_pids = directory.prune_dead_clients();
    assert_eq!(pruned_pids.len(), 0, "Should have pruned 0 clients");
    assert!(pruned_pids.is_empty(), "Pruned list should be empty");
    assert_eq!(directory.len(), 1, "Should still have 1 client");
}

#[test]
fn prune_dead_clients_empty_registry() {
    let directory = ClientDirectory::new();

    assert_eq!(directory.len(), 0, "Should start with 0 clients");

    let pruned_pids = directory.prune_dead_clients();
    assert_eq!(pruned_pids.len(), 0, "Should have pruned 0 clients");
    assert!(pruned_pids.is_empty(), "Pruned list should be empty");
    assert_eq!(directory.len(), 0, "Should still have 0 clients");
}

#[test]
/// A failed lookup at registration must not permanently disable notifications.
fn classify_identity_matches_when_recorded_is_none() {
    assert_eq!(
        ClientDirectory::classify_identity(None, Some(123)),
        PidIdentity::Matches
    );
    assert_eq!(
        ClientDirectory::classify_identity(None, None),
        PidIdentity::Matches
    );
}

#[test]
fn classify_identity_matches_when_start_times_match() {
    assert_eq!(
        ClientDirectory::classify_identity(Some(100), Some(100)),
        PidIdentity::Matches
    );
}

#[test]
fn classify_identity_recycled_only_when_both_known_and_differ() {
    assert_eq!(
        ClientDirectory::classify_identity(Some(5), Some(9)),
        PidIdentity::Recycled
    );
}

#[test]
/// A transient current-lookup failure on a still-registered PID is not a recycle.
fn classify_identity_unverified_when_current_lookup_fails() {
    assert_eq!(
        ClientDirectory::classify_identity(Some(100), None),
        PidIdentity::Unverified
    );
}

#[test]
#[cfg(unix)]
/// A live PID with a mismatched start time is pruned before `kill()` is reached.
fn notify_prunes_pid_whose_identity_no_longer_matches() {
    let directory = ClientDirectory::new();
    let current_pid = std::process::id();

    // No process started in 1970 is still running.
    directory.register_with_started_at_for_test(current_pid, None, Some(1));
    assert_eq!(directory.len(), 1, "Should have 1 client registered");

    directory.notify_repaint(None);

    assert!(
        !directory.clients().contains(&current_pid),
        "PID with mismatched identity must be pruned, not signaled"
    );
}

#[test]
#[cfg(unix)]
/// Uses a nonexistent PID so it is pruned via ESRCH, not signaled.
fn notify_does_not_block_signal_when_identity_unknown() {
    let directory = ClientDirectory::new();
    let fake_pid = 999_999_u32;

    directory.register_with_started_at_for_test(fake_pid, None, None);
    assert_eq!(directory.len(), 1, "Should have 1 client registered");

    directory.notify_repaint(None);

    assert!(
        !directory.clients().contains(&fake_pid),
        "Nonexistent PID should still be pruned via normal ESRCH handling"
    );
}

#[test]
#[cfg(unix)]
fn prune_dead_clients_prunes_confirmed_recycle() {
    let directory = ClientDirectory::new();
    let pid = std::process::id();

    directory.register_with_started_at_for_test(pid, None, Some(1));

    assert_eq!(directory.prune_dead_clients(), vec![pid]);
    assert_eq!(directory.len(), 0);
}

#[test]
#[cfg(unix)]
fn prune_dead_clients_keeps_live_pid_without_recorded_start_time() {
    let directory = ClientDirectory::new();
    let pid = std::process::id();

    directory.register_with_started_at_for_test(pid, None, None);

    assert_eq!(directory.prune_dead_clients(), Vec::<u32>::new());
    assert_eq!(directory.len(), 1);
}

#[test]
#[cfg(unix)]
/// `paths_related` assumes canonical inputs, so symlinked cwds are canonicalized at registration.
fn register_canonicalizes_symlinked_cwd() {
    use std::fs;

    let base = std::env::temp_dir().join(format!("gpy_reg_canon_{}", std::process::id()));
    let real_dir = base.join("real");
    let link = base.join("link");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&real_dir).expect("create real dir");
    std::os::unix::fs::symlink(&real_dir, &link).expect("create symlink");

    let canonical = fs::canonicalize(&real_dir).expect("canonicalize real dir");

    let directory = ClientDirectory::new();
    let pid = 4242_u32;
    directory.register(pid, Some(link));

    let stored = directory.client_cwd(pid).expect("cwd should be stored");
    assert_eq!(
        stored, canonical,
        "register must store the canonical cwd, not the raw symlink path"
    );
    assert!(
        ClientDirectory::paths_related(&stored, &canonical),
        "a canonical notify target for the real dir must match the stored cwd"
    );

    let _ = fs::remove_dir_all(&base);
}

/// A cleanly unregistered shell leaves the shared `System` once it exits.
#[test]
#[cfg(unix)]
fn prune_evicts_exited_unregistered_processes_from_sysinfo() {
    let directory = ClientDirectory::new();
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn stand-in shell");
    let pid = child.id();

    directory.register(pid, None);
    assert!(
        directory.sysinfo.lock_or_recover().holds(pid),
        "registration records the process"
    );
    directory.unregister(pid);
    let _ = child.kill();
    let _ = child.wait();

    let _ = directory.prune_dead_clients();

    assert!(
        !directory.sysinfo.lock_or_recover().holds(pid),
        "an exited, unregistered process must not stay in the shared System"
    );
}

/// A start time that matched within `IDENTITY_TRUST` is trusted without a
/// lookup: a client recorded with an impossible start time is still rung.
#[test]
#[cfg(unix)]
fn recent_identity_match_skips_the_start_time_lookup() {
    let (directory, rings) = ClientDirectory::counting_rings();
    let pid = std::process::id();
    // No process started in 1970 is still running, so a lookup would prune.
    directory.register_with_started_at_for_test(pid, None, Some(1));
    directory.set_verified_at_for_test(pid, Some(Instant::now()));

    directory.notify_repaint(None);

    assert_eq!(rings.load(Ordering::SeqCst), 1, "a trusted client is rung");
    assert!(
        directory.is_registered(pid),
        "a recently verified client must not be looked up again"
    );
}

/// Past the trust window the start time is looked up again, and a mismatch
/// prunes the client before it is rung.
#[test]
#[cfg(unix)]
fn expired_identity_match_is_looked_up_again() {
    let (directory, rings) = ClientDirectory::counting_rings();
    let pid = std::process::id();
    directory.register_with_started_at_for_test(pid, None, Some(1));
    // `None` (an uptime shorter than the offset) forces the lookup as well.
    let expired = Instant::now().checked_sub(IDENTITY_TRUST.saturating_mul(2));
    directory.set_verified_at_for_test(pid, expired);

    directory.notify_repaint(None);

    assert_eq!(
        rings.load(Ordering::SeqCst),
        0,
        "a recycled PID is not rung"
    );
    assert!(
        !directory.is_registered(pid),
        "an expired match must be looked up again and the recycle pruned"
    );
}

/// A lookup that matches renews the trust window for the next ring.
#[test]
#[cfg(unix)]
fn matching_lookup_renews_the_trust_window() {
    let (directory, rings) = ClientDirectory::counting_rings();
    let pid = std::process::id();
    assert!(
        directory.verified_at(pid).is_some(),
        "registration counts as a match"
    );
    directory.set_verified_at_for_test(pid, None);
    let before = Instant::now();

    directory.notify_repaint(None);

    assert_eq!(rings.load(Ordering::SeqCst), 1);
    assert!(
        directory.verified_at(pid).is_some_and(|at| at >= before),
        "the matching lookup must be recorded"
    );
}

/// A match for one start time never vouches for a re-registration of the
/// same PID with another.
#[test]
fn mark_verified_ignores_a_replaced_registration() {
    let directory = ClientDirectory::new();
    directory.register_with_started_at_for_test(4242, None, Some(5));

    directory.mark_verified(4242, 4);
    assert_eq!(directory.verified_at(4242), None);

    directory.mark_verified(4242, 5);
    assert!(directory.verified_at(4242).is_some());
}
