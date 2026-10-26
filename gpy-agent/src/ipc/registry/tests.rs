#[cfg(unix)]
use super::ThrottleRecord;
use super::{ClientDirectory, ClientInfo, PidIdentity};
use crate::sync_ext::MutexExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

impl ClientDirectory {
    #[cfg(unix)]
    fn client_cwd(&self, pid: u32) -> Option<PathBuf> {
        self.inner
            .lock_or_recover()
            .get(&pid)
            .and_then(|info| info.cwd.clone())
    }

    #[cfg(unix)]
    fn throttle_allows(&self, repo: &Path) -> bool {
        self.should_notify_repo(repo, None)
    }

    #[cfg(unix)]
    fn throttle_allows_token(&self, repo: &Path, token: u64) -> bool {
        self.should_notify_repo(repo, Some(token))
    }

    /// Registered PIDs.
    #[must_use]
    pub fn clients(&self) -> Vec<u32> {
        self.inner.lock_or_recover().keys().copied().collect()
    }

    /// Register with an explicit `started_at`, to test PID-recycle handling.
    pub(crate) fn register_with_started_at_for_test(
        &self,
        pid: u32,
        cwd: Option<PathBuf>,
        started_at: Option<u64>,
    ) {
        self.inner
            .lock_or_recover()
            .insert(pid, ClientInfo { cwd, started_at });
    }

    /// A directory whose doorbell counts rings into the returned counter
    /// instead of signalling. Unthrottled, with this process as its one
    /// client, subscribed to every repository.
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
        directory.set_throttle_ms(0);
        directory.register(std::process::id(), None);
        (directory, rings)
    }
}

#[cfg(unix)]
#[test]
fn throttle_suppresses_within_window() {
    let directory = ClientDirectory::new();
    directory.set_throttle_ms(60_000);
    let repo = Path::new("/repo");

    assert!(directory.throttle_allows(repo));
    assert!(!directory.throttle_allows(repo));
}

#[cfg(unix)]
#[test]
/// Latest-wins: a distinct token inside the window is delivered; duplicates coalesce.
fn throttle_delivers_distinct_token_within_window() {
    let directory = ClientDirectory::new();
    directory.set_throttle_ms(60_000);
    let repo = Path::new("/repo");

    assert!(directory.throttle_allows_token(repo, 1));
    assert!(!directory.throttle_allows_token(repo, 1));
    assert!(directory.throttle_allows_token(repo, 2));
    assert!(!directory.throttle_allows_token(repo, 2));
}

#[cfg(unix)]
#[test]
fn throttle_decision_no_prior_record_allows_and_stamps() {
    let now = Instant::now();
    let window = Duration::from_millis(50);

    let (allow, record) = ClientDirectory::throttle_decision(None, now, window, Some(7));

    assert!(allow, "a first push with no prior record must be allowed");
    assert_eq!(record.at, now);
    assert_eq!(record.token, Some(7));
}

#[cfg(unix)]
#[test]
/// The suppressed record must not be bumped to `now`.
fn throttle_decision_inside_window_same_token_suppresses_and_keeps_record() {
    let recorded_at = Instant::now();
    let prev = ThrottleRecord {
        at: recorded_at,
        token: Some(3),
    };
    let now = recorded_at + Duration::from_millis(10);
    let window = Duration::from_millis(50);

    let (allow, record) = ClientDirectory::throttle_decision(Some(prev), now, window, Some(3));

    assert!(
        !allow,
        "a same-token duplicate inside the window must be suppressed"
    );
    assert_eq!(
        record.at, recorded_at,
        "a suppressed decision must not refresh the record's timestamp"
    );
    assert_eq!(record.token, Some(3));
}

#[cfg(unix)]
#[test]
fn throttle_decision_inside_window_distinct_token_allows_and_refreshes() {
    let recorded_at = Instant::now();
    let prev = ThrottleRecord {
        at: recorded_at,
        token: Some(1),
    };
    let now = recorded_at + Duration::from_millis(10);
    let window = Duration::from_millis(50);

    let (allow, record) = ClientDirectory::throttle_decision(Some(prev), now, window, Some(2));

    assert!(
        allow,
        "a distinct token inside the window must still be delivered"
    );
    assert_eq!(
        record.at, now,
        "an allowed decision must refresh the record's timestamp"
    );
    assert_eq!(record.token, Some(2));
}

#[cfg(unix)]
#[test]
/// Tokenless pushes fall back to time-based suppression.
fn throttle_decision_tokenless_inside_window_suppresses() {
    let recorded_at = Instant::now();
    let prev = ThrottleRecord {
        at: recorded_at,
        token: Some(1),
    };
    let now = recorded_at + Duration::from_millis(10);
    let window = Duration::from_millis(50);

    let (allow, record) = ClientDirectory::throttle_decision(Some(prev), now, window, None);

    assert!(
        !allow,
        "a tokenless push inside the window must be suppressed"
    );
    assert_eq!(record.at, recorded_at);
    assert_eq!(record.token, Some(1));
}

#[cfg(unix)]
#[test]
fn throttle_decision_outside_window_allows_and_refreshes() {
    let recorded_at = Instant::now();
    let prev = ThrottleRecord {
        at: recorded_at,
        token: Some(1),
    };
    let now = recorded_at + Duration::from_millis(60);
    let window = Duration::from_millis(50);

    let (allow, record) = ClientDirectory::throttle_decision(Some(prev), now, window, Some(1));

    assert!(
        allow,
        "a push outside the window must be delivered even with the same token"
    );
    assert_eq!(
        record.at, now,
        "an allowed decision must refresh the record's timestamp"
    );
    assert_eq!(record.token, Some(1));
}

#[cfg(unix)]
#[test]
/// The throttle is decided once per broadcast, so a second client in the same
/// repo is not starved; both nonexistent PIDs get ESRCH and are pruned.
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
         within one broadcast, not just the first one HashMap iteration visits"
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
fn is_client_alive_nonexistent_pid() {
    let fake_pid = 999_999_u32;
    assert!(
        !ClientDirectory::is_client_alive(fake_pid),
        "Non-existent PID should be detected as dead"
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

    let pruned_pids = directory.prune_dead_clients();
    assert_eq!(pruned_pids.len(), 1, "Should have pruned 1 dead client");
    assert!(
        pruned_pids.contains(&fake_pid),
        "Pruned list should contain fake PID"
    );
    assert_eq!(directory.len(), 1, "Should have 1 client remaining");

    let clients = directory.clients();
    assert!(
        clients.contains(&current_pid),
        "Current process should still be registered"
    );
    assert!(
        !clients.contains(&fake_pid),
        "Fake PID should have been removed"
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
