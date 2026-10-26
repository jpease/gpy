#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

use super::*;
use crate::watcher::DebouncedEvent;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use std::time::Duration;
use tempfile::TempDir;

impl WorktreeWatch {
    /// Whether the recursive worktree watch actually took effect.
    const fn effective(self) -> bool {
        matches!(self, Self::Armed)
    }
}

impl MultiRepoWatcherBuilder {
    /// Fail every watch for which `fault` returns `Some(err)`.
    #[must_use]
    fn watch_fault(mut self, fault: WatchFault) -> Self {
        self.watch_fault = Some(fault);
        self
    }
}

#[test]
fn parse_commondir_pointer_table() {
    let git_dir = Path::new("/repo/.git/worktrees/feature");
    let cases: &[(&str, Option<&str>)] = &[
        ("../..\n", Some("/repo/.git/worktrees/feature/../..")),
        ("../..", Some("/repo/.git/worktrees/feature/../..")),
        ("/absolute/common\n", Some("/absolute/common")),
        ("", None),
        ("   \n", None),
    ];
    for (input, expected) in cases {
        let actual = super::gitdir::parse_commondir_pointer(git_dir, input);
        assert_eq!(actual, expected.map(PathBuf::from), "input: {input:?}");
    }
}

#[test]
fn test_builder_with_callback_succeeds() {
    let result = MultiRepoWatcher::builder()
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build();

    assert!(result.is_ok(), "Builder should succeed with callback");
}

#[test]
fn test_builder_uses_default_config() {
    let result = MultiRepoWatcher::builder()
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build();

    assert!(result.is_ok(), "Builder should succeed with default config");
}

#[test]
fn test_builder_with_custom_config() {
    let custom_config = WatcherConfig::new(200, 300);

    let result = MultiRepoWatcher::builder()
        .config(custom_config)
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build();

    assert!(result.is_ok(), "Builder should succeed with custom config");
}

#[test]
fn test_builder_fails_without_callback() {
    let result = MultiRepoWatcher::builder().build();

    assert!(result.is_err(), "Builder should fail without callback");

    if let Err(err) = result {
        let err_msg = format!("{err}");
        assert!(
            err_msg.contains("callback"),
            "Error message should mention missing callback: {err_msg}"
        );
    }
}

#[test]
fn test_builder_fluent_interface() {
    let config = WatcherConfig::new(150, 200);

    let result = MultiRepoWatcher::builder()
        .config(config)
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build();

    assert!(result.is_ok(), "Fluent builder chain should succeed");
}

#[test]
fn test_builder_default_trait() {
    let builder = MultiRepoWatcherBuilder::default();

    let result = builder
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build();

    assert!(result.is_ok(), "Default builder should work");
}

#[test]
fn test_multi_repo_watcher_builder_method() {
    let builder = MultiRepoWatcher::builder();

    let result = builder
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build();

    assert!(
        result.is_ok(),
        "MultiRepoWatcher::builder() should return working builder"
    );
}

#[test]
fn register_blocks_when_ops_lock_held() {
    use std::sync::{Barrier, mpsc};

    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();

    let watcher = Arc::new(
        MultiRepoWatcher::builder()
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .unwrap(),
    );

    let ops_guard = watcher.ops_lock.lock().unwrap();
    let (tx, rx) = mpsc::channel();
    let watcher_clone = Arc::clone(&watcher);
    let repo_path = repo_root.to_path_buf();

    let barrier = Arc::new(Barrier::new(2));
    let barrier_clone = Arc::clone(&barrier);

    std::thread::spawn(move || {
        barrier_clone.wait(); // Sync with main thread
        let _ = watcher_clone.register_client(42, &repo_path);
        let _ = tx.send(());
    });

    barrier.wait();

    // Negative proof: give the thread a chance to wrongly pass the lock.
    std::thread::sleep(Duration::from_millis(10));

    assert!(
        rx.try_recv().is_err(),
        "register_client finished while ops_lock was held"
    );

    drop(ops_guard);

    // Long timeout: this asserts eventual progress, not latency, under CI load.
    assert!(
        rx.recv_timeout(Duration::from_secs(20)).is_ok(),
        "register_client did not complete after lock release"
    );
}

/// Real notify watcher: a nested `.git` must get one recursive watch, since a
/// second macOS `watch_directory` rebuilds the `FSEventStream` and can drop events.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn worktree_level_file_creation_is_delivered() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_root)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");
    std::process::Command::new("git")
        .args(["config", "core.fsmonitor", "false"])
        .current_dir(repo_root)
        .status()
        .expect("disable fsmonitor");

    let events: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let watcher = MultiRepoWatcher::builder()
        .registry(test_registry(true))
        .callback(Box::new(move |event: DebouncedEvent| {
            events_clone.lock().unwrap().push(event.repo);
        }))
        .build()
        .expect("create watcher");

    watcher
        .register_client(std::process::id(), repo_root)
        .expect("register client");

    let canonical_root = std::fs::canonicalize(repo_root).expect("canonicalize repo root");

    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(repo_root.join("untracked.txt"), b"content").expect("write worktree-level file");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut delivered = false;
    while std::time::Instant::now() < deadline {
        if events.lock().unwrap().contains(&canonical_root) {
            delivered = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert!(
        delivered,
        "worktree-level file creation should be delivered through the real watcher"
    );
}

/// `mkdir` + immediate write must surface an event without the reconcile.
/// Best-effort; the deterministic proof is in `filesystem.rs`.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn mkdir_then_immediate_write_is_delivered() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_root)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");
    std::process::Command::new("git")
        .args(["config", "core.fsmonitor", "false"])
        .current_dir(repo_root)
        .status()
        .expect("disable fsmonitor");

    let events: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let watcher = MultiRepoWatcher::builder()
        .registry(test_registry(true))
        .callback(Box::new(move |event: DebouncedEvent| {
            events_clone.lock().unwrap().push(event.repo);
        }))
        .build()
        .expect("create watcher");

    watcher
        .register_client(std::process::id(), repo_root)
        .expect("register client");

    let canonical_root = std::fs::canonicalize(repo_root).expect("canonicalize repo root");

    std::thread::sleep(Duration::from_millis(100));

    // Write before the OS has necessarily armed the new subdirectory.
    let new_dir = repo_root.join("newmod");
    std::fs::create_dir_all(&new_dir).expect("create subdir");
    std::fs::write(new_dir.join("mod.rs"), b"pub fn f() {}\n").expect("write file in subdir");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut delivered = false;
    while std::time::Instant::now() < deadline {
        if events.lock().unwrap().contains(&canonical_root) {
            delivered = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        delivered,
        "mkdir + immediate file write should surface a git event for the repo"
    );
}

/// Unregister unwatches exactly the armed paths: `git_root` alone for an
/// in-tree `.git`, never `git_dir`.
#[test]
fn unregister_unwatches_exactly_the_armed_paths() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();

    let watcher = MultiRepoWatcher::builder()
        .registry(test_registry(true))
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .unwrap();

    watcher.register_client(99, repo_root).unwrap();

    // Precondition: `armed` is `[git_root]` alone for this layout.
    let armed = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&canonical_root)
        .expect("repo tracked")
        .armed
        .clone();
    assert_eq!(
        armed,
        vec![canonical_root.clone()],
        "worktree watching with git_dir under git_root should arm git_root alone"
    );

    watcher.unregister_client(99);

    let unwatched = watcher
        .watcher
        .lock()
        .unwrap()
        .as_ref()
        .expect("coordinator still present")
        .unwatch_calls();
    assert_eq!(
        unwatched,
        vec![canonical_root],
        "teardown must unwatch exactly the recorded armed paths -- git_root once, \
         and never git_dir, which was never armed as its own watch"
    );
}

/// A failed worktree watch degrades to `.git`-only; the repo stays tracked.
#[test]
fn worktree_watch_failure_degrades_to_gitdir_only() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();

    // Fault only `git_root`; the essential .git watch must still succeed.
    let faulty_root = canonical_root.clone();
    let watcher = MultiRepoWatcher::builder()
        .registry(test_registry(true))
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .watch_fault(Arc::new(move |path: &Path| {
            (path == faulty_root).then(|| Error::watcher("injected worktree watch failure (test)"))
        }))
        .build()
        .unwrap();

    let result = watcher.register_client(7, repo_root);

    assert!(
        result.is_ok(),
        "registration must succeed despite worktree watch failure: {result:?}"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "repo should still be tracked after the worktree fallback"
    );

    // One statement so the guard drops before `watched_roots()` relocks.
    let degraded_worktree = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&canonical_root)
        .expect("repo tracked under canonical root")
        .worktree
        .effective();
    assert!(
        !degraded_worktree,
        "worktree flag must degrade to false after fallback"
    );
    assert!(
        watcher.watched_roots().contains(&canonical_root),
        "degraded repo must remain visible to the periodic reconcile"
    );
}

/// A failed `.git` watch surfaces the error and tracks nothing.
#[test]
fn gitdir_watch_failure_rolls_back_without_tracking() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();

    // Fault every path: recursive root watch fails, then the .git fallback.
    let watcher = MultiRepoWatcher::builder()
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .watch_fault(Arc::new(|_path: &Path| {
            Some(Error::watcher("injected git-dir watch failure (test)"))
        }))
        .build()
        .unwrap();

    let result = watcher.register_client(8, repo_root);

    assert!(
        result.is_err(),
        "registration should surface the essential .git watch failure"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        0,
        "no repo should be tracked after a rolled-back watch setup"
    );
}

/// Registering after `stop()` must error rather than track an unwatched repo.
#[test]
fn register_client_errors_when_coordinator_is_gone() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();

    let watcher = MultiRepoWatcher::builder()
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .unwrap();

    watcher.stop();

    let result = watcher.register_client(9, repo_root);
    assert!(
        result.is_err(),
        "registration must fail rather than silently record an unwatched repo"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        0,
        "no repo should be tracked when the coordinator is gone"
    );
}

/// Re-register after `.git` recreation (new inode) must re-arm. Swapped via
/// `rename` because Linux often reuses a just-freed inode number.
#[cfg(unix)]
#[test]
fn recreated_gitdir_rearms_watch_on_reregister() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();

    let watcher = MultiRepoWatcher::builder()
        .registry(test_registry(false))
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .unwrap();

    watcher.register_client(1, repo_root).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();

    let first_id = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&canonical_root)
        .expect("repo tracked after first registration")
        .git_dir_id;
    assert!(first_id.is_some(), "unix identity should be captured");

    // Coexisting dirs guarantee distinct inodes (see doc comment).
    let replacement = repo_root.join(".git-replacement");
    std::fs::create_dir_all(&replacement).unwrap();
    std::fs::remove_dir_all(repo_root.join(".git")).unwrap();
    std::fs::rename(&replacement, repo_root.join(".git")).unwrap();

    watcher.register_client(1, repo_root).unwrap();

    let second_id = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&canonical_root)
        .expect("repo still tracked after re-register")
        .git_dir_id;

    assert_ne!(
        first_id, second_id,
        "recreated gitdir must update the tracked identity (watch re-armed)"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "repo should remain tracked exactly once after re-arm"
    );
}

/// Effective worktree flag for `root`; one statement so the guard drops at once.
fn tracked_worktree(watcher: &MultiRepoWatcher, root: &Path) -> bool {
    watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(root)
        .expect("repo tracked")
        .worktree
        .effective()
}

fn tracked_clients(watcher: &MultiRepoWatcher, root: &Path) -> HashSet<u32> {
    watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(root)
        .expect("repo tracked")
        .clients
        .clone()
}

/// A private registry with `worktree_enabled` applied.
fn test_registry(worktree_enabled: bool) -> Arc<WatchRegistry> {
    let registry = Arc::new(WatchRegistry::new());
    registry.set_worktree_enabled(worktree_enabled);
    registry
}

/// Build a watcher over a fresh temp repo with a bare `.git` directory.
fn watcher_over_temp_repo(worktree_enabled: bool) -> (TempDir, MultiRepoWatcher) {
    watcher_over_temp_repo_with_fault(worktree_enabled, |_canonical_root| {
        Arc::new(|_path: &Path| None)
    })
}

/// Like [`watcher_over_temp_repo`], with a fault built from the canonical root.
fn watcher_over_temp_repo_with_fault(
    worktree_enabled: bool,
    fault_for_root: impl FnOnce(&Path) -> WatchFault,
) -> (TempDir, MultiRepoWatcher) {
    let temp_dir = TempDir::new().unwrap();
    std::fs::create_dir_all(temp_dir.path().join(".git")).unwrap();
    let canonical_root = std::fs::canonicalize(temp_dir.path()).unwrap();
    let watcher = MultiRepoWatcher::builder()
        .registry(test_registry(worktree_enabled))
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .watch_fault(fault_for_root(&canonical_root))
        .build()
        .unwrap();
    (temp_dir, watcher)
}

/// Re-register after `git.watch_worktree` flipped on must re-arm the repo.
#[test]
fn reregister_after_worktree_enabled_rearms_repo() {
    let (temp_dir, watcher) = watcher_over_temp_repo(false);
    let repo_root = temp_dir.path();

    watcher.register_client(1, repo_root).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();
    assert!(
        !tracked_worktree(&watcher, &canonical_root),
        "repo should start out .git-only"
    );

    watcher.set_watch_worktree(true);
    watcher.register_client(1, repo_root).unwrap();

    assert!(
        tracked_worktree(&watcher, &canonical_root),
        "re-register after the flag flipped on must install the worktree watch"
    );
}

/// Re-register after it flipped off drops the worktree watch, keeps `.git`.
#[test]
fn reregister_after_worktree_disabled_rearms_repo() {
    let (temp_dir, watcher) = watcher_over_temp_repo(true);
    let repo_root = temp_dir.path();

    watcher.register_client(1, repo_root).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();
    assert!(
        tracked_worktree(&watcher, &canonical_root),
        "repo should start out watching the worktree"
    );

    watcher.set_watch_worktree(false);
    watcher.register_client(1, repo_root).unwrap();

    assert!(
        !tracked_worktree(&watcher, &canonical_root),
        "re-register after the flag flipped off must drop the worktree watch"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "repo must stay tracked (gitdir coverage preserved)"
    );
}

/// A degraded repo must not re-arm on an unchanged-config re-register
/// (compare against requested, not effective, state).
#[test]
fn degraded_worktree_repo_does_not_rearm_on_reregister() {
    let (temp_dir, watcher) = watcher_over_temp_repo_with_fault(true, |canonical_root| {
        let faulty_root = canonical_root.to_path_buf();
        Arc::new(move |path: &Path| {
            (path == faulty_root).then(|| Error::watcher("injected worktree watch failure (test)"))
        })
    });
    let repo_root = temp_dir.path();
    watcher.register_client(1, repo_root).unwrap();

    let canonical_root = std::fs::canonicalize(repo_root).unwrap();
    assert!(
        !tracked_worktree(&watcher, &canonical_root),
        "worktree watch failure should have degraded to .git-only"
    );

    // Injection is off now, so a spurious re-arm would flip the flag to true.
    watcher.register_client(1, repo_root).unwrap();

    assert!(
        !tracked_worktree(&watcher, &canonical_root),
        "a degraded repo must not re-arm on an unchanged-config re-register"
    );
}

/// Resync re-arms every active repo when the flag flips on.
#[test]
fn resync_installs_worktree_watch_on_active_repos() {
    let (temp_dir, watcher) = watcher_over_temp_repo(false);
    let repo_root = temp_dir.path();

    watcher.register_client(1, repo_root).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();
    assert!(!tracked_worktree(&watcher, &canonical_root));

    watcher.set_watch_worktree(true);
    watcher.resync_watch_worktree();

    assert!(
        tracked_worktree(&watcher, &canonical_root),
        "config reload must re-arm active repos with the worktree watch"
    );
}

/// Resync drops the worktree watch when the flag flips off; repo stays tracked.
#[test]
fn resync_removes_worktree_watch_on_active_repos() {
    let (temp_dir, watcher) = watcher_over_temp_repo(true);
    let repo_root = temp_dir.path();

    watcher.register_client(1, repo_root).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();
    assert!(tracked_worktree(&watcher, &canonical_root));

    watcher.set_watch_worktree(false);
    watcher.resync_watch_worktree();

    assert!(
        !tracked_worktree(&watcher, &canonical_root),
        "config reload must drop the worktree watch when disabled"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "repo must stay tracked with its essential gitdir watch"
    );
}

/// Re-arm must not drop any client from a shared repo's refcount.
#[test]
fn resync_preserves_multiple_registered_clients() {
    let (temp_dir, watcher) = watcher_over_temp_repo(false);
    let repo_root = temp_dir.path();

    for pid in [11_u32, 22_u32, 33_u32] {
        watcher.register_client(pid, repo_root).unwrap();
    }
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();
    let before = tracked_clients(&watcher, &canonical_root);
    assert_eq!(before.len(), 3, "all three clients should be registered");

    watcher.set_watch_worktree(true);
    watcher.resync_watch_worktree();

    assert!(tracked_worktree(&watcher, &canonical_root));
    assert_eq!(
        tracked_clients(&watcher, &canonical_root),
        before,
        "reconfiguration must preserve the full client set"
    );
    assert_eq!(watcher.watched_repo_count(), 1);
}

/// A failed resync worktree watch degrades to `.git`-only and is not re-armed
/// on the next resync. The fault only fires on resync (initial register has
/// worktree watching off).
#[test]
fn resync_worktree_failure_degrades_to_gitdir_only() {
    let (temp_dir, watcher) = watcher_over_temp_repo_with_fault(false, |canonical_root| {
        let faulty_root = canonical_root.to_path_buf();
        Arc::new(move |path: &Path| {
            (path == faulty_root).then(|| Error::watcher("injected worktree watch failure (test)"))
        })
    });
    let repo_root = temp_dir.path();
    watcher.register_client(1, repo_root).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();

    watcher.set_watch_worktree(true);
    watcher.resync_watch_worktree();

    assert!(
        !tracked_worktree(&watcher, &canonical_root),
        "a failed worktree watch must degrade to .git-only"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "the degraded repo must remain tracked and reconcilable"
    );

    // Second pass must be a no-op; a re-arm would flip the flag to true.
    watcher.resync_watch_worktree();
    assert!(
        !tracked_worktree(&watcher, &canonical_root),
        "a degraded repo must not thrash its watch on every resync"
    );
}

/// An unchanged flag means resync touches no watch; an armed `.git` fault
/// would corrupt the entry if any re-arm ran.
#[test]
fn resync_is_a_noop_when_nothing_changed() {
    let fault_armed = Arc::new(AtomicBool::new(false));
    let fault_armed_for_closure = Arc::clone(&fault_armed);
    let (temp_dir, watcher) = watcher_over_temp_repo_with_fault(true, move |_root| {
        Arc::new(move |_path: &Path| {
            fault_armed_for_closure
                .load(Ordering::Relaxed)
                .then(|| Error::watcher("injected git-dir watch failure (test)"))
        })
    });
    let repo_root = temp_dir.path();
    watcher.register_client(1, repo_root).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();
    let identity_before = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&canonical_root)
        .expect("repo tracked")
        .git_dir_id;

    fault_armed.store(true, Ordering::Relaxed);
    watcher.resync_watch_worktree();
    fault_armed.store(false, Ordering::Relaxed);

    assert!(tracked_worktree(&watcher, &canonical_root));
    assert_eq!(
        watcher
            .watched_repos
            .lock()
            .unwrap()
            .get(&canonical_root)
            .expect("repo tracked")
            .git_dir_id,
        identity_before,
        "an unchanged flag must not touch any watch"
    );
}

/// A plain re-register keeps the tracked identity stable.
#[cfg(unix)]
#[test]
fn unchanged_gitdir_keeps_identity_on_reregister() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    std::fs::create_dir_all(repo_root.join(".git")).unwrap();

    let watcher = MultiRepoWatcher::builder()
        .registry(test_registry(false))
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .unwrap();

    watcher.register_client(1, repo_root).unwrap();
    let canonical_root = std::fs::canonicalize(repo_root).unwrap();
    let first_id = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&canonical_root)
        .expect("repo tracked")
        .git_dir_id;

    watcher.register_client(1, repo_root).unwrap();
    let second_id = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&canonical_root)
        .expect("repo still tracked")
        .git_dir_id;

    assert_eq!(
        first_id, second_id,
        "an unchanged gitdir must not have its identity churned on re-register"
    );
}

/// Two watchers in one process share no attribution or worktree state.
#[test]
fn two_watchers_do_not_share_registry_state() {
    let first = MultiRepoWatcher::builder()
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .unwrap();
    let second = MultiRepoWatcher::builder()
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .unwrap();

    // External-gitdir attribution.
    let git_dir = PathBuf::from("/super/.git/modules/sub");
    let work_root = PathBuf::from("/super/sub");
    first
        .registry()
        .register_external_gitdir(&git_dir, &work_root);
    let head = git_dir.join("HEAD");
    assert_eq!(
        first.registry().attribute_external_gitdir(&head),
        Some(work_root.clone()),
        "the registering watcher must attribute its own external gitdir"
    );
    assert_eq!(
        second.registry().attribute_external_gitdir(&head),
        None,
        "a second watcher must not see the first watcher's external gitdir"
    );

    let common = PathBuf::from("/main/.git");
    first.registry().register_common_gitdir(&common, &work_root);
    assert_eq!(
        first
            .registry()
            .attribute_common_gitdir(&common.join("packed-refs")),
        vec![work_root.clone()],
    );
    assert!(
        second
            .registry()
            .attribute_common_gitdir(&common.join("packed-refs"))
            .is_empty(),
        "a second watcher must not see the first watcher's common gitdir"
    );

    // Absolute on every host: Windows would absolutize a drive-less path.
    let config_path = std::env::temp_dir().join("nowhere/gpy/config.toml");
    first.registry().register_config_path(&config_path);
    assert!(first.registry().is_registered_config_path(&config_path));
    assert!(
        !second.registry().is_registered_config_path(&config_path),
        "a second watcher must not see the first watcher's config path"
    );

    assert!(first.registry().worktree_enabled());
    assert!(second.registry().worktree_enabled());
    first.set_watch_worktree(false);
    assert!(!first.registry().worktree_enabled());
    assert!(
        second.registry().worktree_enabled(),
        "flipping watch_worktree on one watcher must not change the other"
    );

    first.registry().unregister_external_gitdir(&git_dir);
    first
        .registry()
        .unregister_common_gitdir(&common, &work_root);
    first.registry().unregister_config_path(&config_path);
}

/// The backend's unwatch log for `watcher`.
fn unwatch_log(watcher: &MultiRepoWatcher) -> Vec<PathBuf> {
    watcher
        .watcher
        .lock()
        .unwrap()
        .as_ref()
        .expect("coordinator present")
        .unwatch_calls()
}

/// Two plain repos (bare `.git` dirs), the first with a `sub` directory.
fn two_plain_repos() -> (TempDir, PathBuf, PathBuf) {
    let temp_dir = TempDir::new().unwrap();
    let base = std::fs::canonicalize(temp_dir.path()).unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir_all(base.join(name).join(".git")).unwrap();
    }
    std::fs::create_dir_all(base.join("a").join("sub")).unwrap();
    let (a, b) = (base.join("a"), base.join("b"));
    (temp_dir, a, b)
}

fn plain_watcher() -> MultiRepoWatcher {
    MultiRepoWatcher::builder()
        .registry(test_registry(true))
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .unwrap()
}

/// A `cd` within the attached repo touches no watch or tracked state.
#[test]
fn update_client_within_same_repo_does_not_rearm() {
    let (_temp, repo, _other) = two_plain_repos();
    let watcher = plain_watcher();
    watcher.register_client(1, &repo).unwrap();
    let before = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&repo)
        .unwrap()
        .clone();

    watcher.update_client(1, &repo.join("sub")).unwrap();

    assert!(
        unwatch_log(&watcher).is_empty(),
        "an intra-repo cd must not unwatch anything, got {:?}",
        unwatch_log(&watcher)
    );
    let after = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&repo)
        .unwrap()
        .clone();
    assert_eq!(after.armed, before.armed);
    assert_eq!(after.git_dir_id, before.git_dir_id);
    assert_eq!(after.clients, before.clients);
    assert_eq!(tracked_clients(&watcher, &repo), HashSet::from([1]));
}

/// Registering elsewhere detaches the pid, tearing down an emptied repo.
#[test]
fn register_client_detaches_pid_from_previous_repo() {
    let (_temp, a, b) = two_plain_repos();
    let watcher = plain_watcher();
    watcher.register_client(1, &a).unwrap();
    let a_armed = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(&a)
        .unwrap()
        .armed
        .clone();

    watcher.register_client(1, &b).unwrap();

    assert!(
        !watcher.watched_repos.lock().unwrap().contains_key(&a),
        "the previous repository must be released"
    );
    assert_eq!(unwatch_log(&watcher), a_armed);
    assert_eq!(tracked_clients(&watcher, &b), HashSet::from([1]));
}

/// Moving one pid away leaves a repo other pids still use alone.
#[test]
fn update_client_to_other_repo_keeps_shared_repo() {
    let (_temp, a, b) = two_plain_repos();
    let watcher = plain_watcher();
    watcher.register_client(1, &a).unwrap();
    watcher.register_client(2, &a).unwrap();

    watcher.update_client(1, &b).unwrap();

    assert_eq!(tracked_clients(&watcher, &a), HashSet::from([2]));
    assert_eq!(tracked_clients(&watcher, &b), HashSet::from([1]));
    assert!(
        !unwatch_log(&watcher).contains(&a),
        "a still has a client, its watch must stay: {:?}",
        unwatch_log(&watcher)
    );
}

/// A failed arm of the target still detaches the pid from its old repo.
#[test]
fn update_client_failed_arm_still_detaches_from_previous_repo() {
    let (_temp, a, b) = two_plain_repos();
    let fault_root = b.clone();
    let watcher = MultiRepoWatcher::builder()
        .registry(test_registry(true))
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .watch_fault(Arc::new(move |path: &Path| {
            path.starts_with(&fault_root)
                .then(|| Error::watcher("injected watch failure"))
        }))
        .build()
        .unwrap();
    watcher.register_client(1, &a).unwrap();

    assert!(watcher.update_client(1, &b).is_err());

    assert!(watcher.watched_repos.lock().unwrap().is_empty());
    assert_eq!(unwatch_log(&watcher), vec![a]);
}

// Randomized register/unregister/update sequences checked against a model
// derived from the fixture alone, never from the watcher's state.

struct FixtureRepo {
    root: PathBuf,
    git_dir: PathBuf,
    /// A linked worktree: its common directory is shared with `main`.
    linked: bool,
}

struct Fixture {
    _temp: TempDir,
    repos: Vec<FixtureRepo>,
    /// Pid cwds with owning repo index (`None` outside any repo).
    cwds: Vec<(PathBuf, Option<usize>)>,
    /// `main`'s `.git`: the common directory of both linked worktrees.
    common: PathBuf,
}

impl Fixture {
    fn root_of(&self, index: usize) -> &Path {
        &self.repos.get(index).expect("repo index in range").root
    }
}

/// A set of `(path, mode)` OS watches.
type WatchSpec = std::collections::BTreeSet<(PathBuf, WatchMode)>;

/// `main` (nested repo, external-gitdir submodule, two linked worktrees),
/// `other`, and a non-repo dir. No `git` process needed.
fn build_fixture() -> Fixture {
    let temp = TempDir::new().unwrap();
    let base = std::fs::canonicalize(temp.path()).unwrap();
    let mk = |rel: &str| {
        let path = base.join(rel);
        std::fs::create_dir_all(&path).unwrap();
        path
    };
    let main = mk("main");
    let common = mk("main/.git");
    mk("main/.git/refs");
    mk("main/src");
    let libs = mk("main/libs");
    let sub_admin = mk("main/.git/modules/sub");
    let sub = mk("main/libs/sub");
    std::fs::write(
        sub.join(".git"),
        format!("gitdir: {}\n", sub_admin.display()),
    )
    .unwrap();
    let nested = mk("main/nested");
    mk("main/nested/.git");
    mk("main/nested/src");
    let mut linked = Vec::new();
    for name in ["wt", "wt2"] {
        let admin = mk(&format!("main/.git/worktrees/{name}"));
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        let root = mk(name);
        mk(&format!("{name}/src"));
        std::fs::write(root.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        linked.push((root, admin));
    }
    let other = mk("other");
    mk("other/.git");
    mk("other/deep");
    let plain = mk("plain");

    let mut repos = vec![
        FixtureRepo {
            root: main.clone(),
            git_dir: common.clone(),
            linked: false,
        },
        FixtureRepo {
            root: nested.clone(),
            git_dir: nested.join(".git"),
            linked: false,
        },
        FixtureRepo {
            root: sub.clone(),
            git_dir: sub_admin,
            linked: false,
        },
    ];
    for (root, admin) in &linked {
        repos.push(FixtureRepo {
            root: root.clone(),
            git_dir: admin.clone(),
            linked: true,
        });
    }
    repos.push(FixtureRepo {
        root: other.clone(),
        git_dir: other.join(".git"),
        linked: false,
    });

    let wt = linked.first().unwrap().0.clone();
    let wt2 = linked.get(1).expect("two linked worktrees").0.clone();
    let cwds = vec![
        (main.clone(), Some(0)),
        (main.join("src"), Some(0)),
        (libs, Some(0)),
        (nested.clone(), Some(1)),
        (nested.join("src"), Some(1)),
        (sub, Some(2)),
        (wt.clone(), Some(3)),
        (wt.join("src"), Some(3)),
        (wt2, Some(4)),
        (other.clone(), Some(5)),
        (other.join("deep"), Some(5)),
        (plain, None),
    ];
    Fixture {
        _temp: temp,
        repos,
        cwds,
        common,
    }
}

/// `(required, optional)` watches for live repos. Optional common-dir watches
/// depend on arrival order (the main checkout's watch may cover them).
fn needed_watches(
    fx: &Fixture,
    live: &std::collections::BTreeSet<usize>,
) -> (WatchSpec, WatchSpec) {
    let mut required = std::collections::BTreeSet::new();
    let mut optional = std::collections::BTreeSet::new();
    for repo in live.iter().filter_map(|index| fx.repos.get(*index)) {
        if !repo.git_dir.starts_with(&repo.root) {
            required.insert((repo.git_dir.clone(), WatchMode::Recursive));
        }
        required.insert((repo.root.clone(), WatchMode::Recursive));
        if repo.linked {
            optional.insert((fx.common.clone(), WatchMode::Shallow));
            optional.insert((fx.common.join("refs"), WatchMode::Recursive));
        }
    }
    (required, optional)
}

fn held_watches_of(watcher: &MultiRepoWatcher) -> crate::watcher::filesystem::HeldWatches {
    watcher
        .watcher
        .lock()
        .unwrap()
        .as_ref()
        .expect("coordinator present")
        .held_watches()
}

/// Assert tracked repos and held OS watches equal the model.
fn assert_matches_model(
    watcher: &MultiRepoWatcher,
    fx: &Fixture,
    model: &std::collections::BTreeMap<u32, Option<usize>>,
    trace: &[(u8, u32, usize)],
) {
    use std::collections::{BTreeMap, BTreeSet};

    // (a) tracked repos == repos with >= 1 modelled client, same clients.
    let mut expected: BTreeMap<PathBuf, BTreeSet<u32>> = BTreeMap::new();
    let mut live = BTreeSet::new();
    for (pid, repo) in model {
        if let Some(index) = repo {
            live.insert(*index);
            let root = fx.root_of(*index).to_path_buf();
            expected.entry(root).or_default().insert(*pid);
        }
    }
    let tracked: BTreeMap<PathBuf, BTreeSet<u32>> = watcher
        .watched_repos
        .lock()
        .unwrap()
        .iter()
        .map(|(root, repo)| (root.clone(), repo.clients.iter().copied().collect()))
        .collect();
    assert_eq!(tracked, expected, "tracked repos/clients after {trace:?}");

    // (b) the held OS watches are exactly what the live repos need.
    let (owners, armed) = held_watches_of(watcher);
    let held: BTreeSet<(PathBuf, WatchMode)> = owners.keys().cloned().collect();
    let armed_pairs: BTreeSet<(PathBuf, WatchMode)> = armed.into_iter().collect();
    assert_eq!(
        armed_pairs, held,
        "OS watch set vs registrations after {trace:?}"
    );
    assert!(
        owners.values().all(|count| *count == 1),
        "every watch has exactly one owner: {owners:?} after {trace:?}"
    );
    let (required, optional) = needed_watches(fx, &live);
    let missing: Vec<_> = required.difference(&held).collect();
    assert!(
        missing.is_empty(),
        "missing watches {missing:?} after {trace:?}"
    );
    let leaked: Vec<_> = held
        .iter()
        .filter(|watch| !required.contains(*watch) && !optional.contains(*watch))
        .collect();
    assert!(
        leaked.is_empty(),
        "leaked watches {leaked:?} after {trace:?}"
    );
    for (path, mode) in &optional {
        let covered = held.iter().any(|(held_path, held_mode)| {
            path.starts_with(held_path)
                && (*held_mode == WatchMode::Recursive || held_path == path)
                && (*mode == WatchMode::Shallow || *held_mode == WatchMode::Recursive)
        });
        assert!(covered, "common watch {path:?} uncovered after {trace:?}");
    }
}

/// Held watches, unwatch-log length, and one repo's `armed`/`git_dir_id`/`clients`.
type RearmSnapshot = (
    crate::watcher::filesystem::HeldWatches,
    usize,
    Vec<PathBuf>,
    DirIdentity,
    HashSet<u32>,
);

/// What a same-repository re-attach must leave untouched.
fn rearm_snapshot(watcher: &MultiRepoWatcher, root: &Path) -> RearmSnapshot {
    let repo = watcher
        .watched_repos
        .lock()
        .unwrap()
        .get(root)
        .unwrap()
        .clone();
    (
        held_watches_of(watcher),
        unwatch_log(watcher).len(),
        repo.armed,
        repo.git_dir_id,
        repo.clients,
    )
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig {
        cases: 48,
        failure_persistence: None,
        ..proptest::prelude::ProptestConfig::default()
    })]

    /// Random op sequences leave exactly the needed watches; a same-repo
    /// attach performs no backend watch operation.
    #[test]
    fn watch_set_matches_client_model(
        steps in proptest::collection::vec((0_u8..3_u8, 1_u32..=3_u32, 0_usize..12_usize), 1..=30),
    ) {
        let fx = build_fixture();
        let watcher = plain_watcher();
        let mut model: std::collections::BTreeMap<u32, Option<usize>> =
            std::collections::BTreeMap::new();
        for (done, (kind, pid, cwd_index)) in steps.iter().copied().enumerate() {
            let trace = steps.get(..=done).expect("in range");
            let (cwd, target) = fx.cwds.get(cwd_index).expect("cwd index in range");
            if kind == 2 {
                watcher.unregister_client(pid);
                model.insert(pid, None);
            } else {
                let same_repo = target.is_some() && model.get(&pid) == Some(target);
                let snapshot_before = same_repo.then(|| {
                    rearm_snapshot(&watcher, fx.root_of(target.unwrap()))
                });
                if kind == 0 {
                    watcher.register_client(pid, cwd).unwrap();
                } else {
                    watcher.update_client(pid, cwd).unwrap();
                }
                model.insert(pid, *target);
                if let Some(snapshot) = snapshot_before {
                    let after =
                        rearm_snapshot(&watcher, fx.root_of(target.unwrap()));
                    assert_eq!(snapshot, after, "same-repo attach touched watches after {trace:?}");
                }
            }
            assert_matches_model(&watcher, &fx, &model, trace);
        }
    }
}
