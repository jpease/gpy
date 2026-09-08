//! Unit tests for MultiRepoWatcher

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)] // watcher API paths include socket filenames that aren't Markdown links
#![allow(clippy::tests_outside_test_module)] // top-level test functions are intentional for this module

use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
use gpy_agent::watcher::{DebouncedEvent, FileEvent, WatchCoordinator, WatcherConfig};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::tempdir;

/// Poll `predicate` until it returns true or `timeout` elapses, sleeping
/// `poll_interval` between attempts. Returns the final predicate result.
fn wait_until(timeout: Duration, poll_interval: Duration, predicate: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if predicate() {
            return true;
        }
        std::thread::sleep(poll_interval);
    }
    predicate()
}

fn init_git_repo(path: &Path) {
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(path)
        .status()
        .expect("git init should run");
    assert!(status.success(), "git init should succeed");
}

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn test_find_git_root_finds_root() {
    // This test uses the actual gpy repository as test data
    let current_dir = std::env::current_dir().expect("Failed to get current dir");
    let git_root = MultiRepoWatcher::find_git_root(&current_dir);

    assert!(git_root.is_some(), "Should find git root for gpy project");
    let root = git_root.unwrap();
    assert!(
        root.join(".git").exists(),
        "Git root should have .git directory"
    );
}

#[test]
fn test_find_git_root_no_repo() {
    let non_git_path = Path::new("/tmp");
    let _git_root = MultiRepoWatcher::find_git_root(non_git_path);

    // /tmp might or might not be in a git repo, but we can test the function doesn't crash
    // The actual behavior depends on the system
}

#[test]
#[cfg(unix)]
fn test_find_git_root_symlink_path() {
    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();

    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");

    let symlink_dir = repo_dir.path().join("symlink");
    symlink(repo_path, &symlink_dir).expect("create symlink");

    let git_root = MultiRepoWatcher::find_git_root(&symlink_dir).expect("git root from symlink");
    let canonical_repo = std::fs::canonicalize(repo_path).expect("canonical repo path");

    assert_eq!(git_root, canonical_repo);
}

#[test]
fn test_watcher_creation() {
    let events = Arc::new(Mutex::new(Vec::<FileEvent>::new()));
    let events_clone = Arc::clone(&events);

    let callback = Box::new(move |event: DebouncedEvent| {
        if let Ok(mut e) = events_clone.lock() {
            e.push(event.event);
        }
    });

    let watcher_result = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build();
    assert!(watcher_result.is_ok(), "Watcher creation should succeed");
}

#[test]
fn test_watcher_initial_state() {
    let callback = Box::new(|_event: DebouncedEvent| {
        // No-op callback for testing
    });

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    // Initially no repos should be watched
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_register_client_non_git_directory() {
    let callback = Box::new(|_event: DebouncedEvent| {});
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    // Register client in /tmp (not a git repo)
    let result = watcher.register_client(12345, Path::new("/tmp"));

    // Should succeed but not watch anything
    assert!(result.is_ok());
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_register_client_in_git_repo() {
    let callback = Box::new(|_event: DebouncedEvent| {});
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // Register client in current directory (should be gpy git repo)
    let result = watcher.register_client(12345, &current_dir);

    if MultiRepoWatcher::find_git_root(&current_dir).is_some() {
        assert!(result.is_ok());
        assert_eq!(
            watcher.watched_repo_count(),
            1,
            "Should be watching one repo"
        );
    }
}

#[test]
fn test_register_multiple_clients_same_repo() {
    let callback = Box::new(|_event: DebouncedEvent| {});
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // Register multiple clients in the same repo
    let _ = watcher.register_client(100, &current_dir);
    let _ = watcher.register_client(200, &current_dir);
    let _ = watcher.register_client(300, &current_dir);

    if MultiRepoWatcher::find_git_root(&current_dir).is_some() {
        // Should only watch the repo once, not three times
        assert_eq!(watcher.watched_repo_count(), 1);
    }
}

#[test]
fn test_unregister_client() {
    let callback = Box::new(|_event: DebouncedEvent| {});
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // Skip if not in a git repo
    if MultiRepoWatcher::find_git_root(&current_dir).is_none() {
        return;
    }

    // Register and then unregister a client
    let _ = watcher.register_client(12345, &current_dir);
    assert_eq!(watcher.watched_repo_count(), 1);

    let result = watcher.unregister_client(12345);
    assert!(result.is_ok());

    // Should stop watching when last client unregisters
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_unregister_with_multiple_clients() {
    let callback = Box::new(|_event: DebouncedEvent| {});
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // Skip if not in a git repo
    if MultiRepoWatcher::find_git_root(&current_dir).is_none() {
        return;
    }

    // Register multiple clients
    let _ = watcher.register_client(100, &current_dir);
    let _ = watcher.register_client(200, &current_dir);
    let _ = watcher.register_client(300, &current_dir);

    assert_eq!(watcher.watched_repo_count(), 1);

    // Unregister one client - should still be watching
    let _ = watcher.unregister_client(100);
    assert_eq!(watcher.watched_repo_count(), 1);

    // Unregister second client - should still be watching
    let _ = watcher.unregister_client(200);
    assert_eq!(watcher.watched_repo_count(), 1);

    // Unregister last client - should stop watching
    let _ = watcher.unregister_client(300);
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_update_client() {
    let callback = Box::new(|_event: DebouncedEvent| {});
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // Skip if not in a git repo
    if MultiRepoWatcher::find_git_root(&current_dir).is_none() {
        return;
    }

    // Register client in current directory
    let _ = watcher.register_client(12345, &current_dir);
    assert_eq!(watcher.watched_repo_count(), 1);

    // Update to /tmp (not a git repo)
    let result = watcher.update_client(12345, Path::new("/tmp"));
    assert!(result.is_ok());

    // Should have stopped watching original repo
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_watcher_stop() {
    // Test that stop() properly cleans up without hanging
    // The watcher now uses an explicit stop flag so shutdown no longer depends on
    // notify closing its sender promptly on every platform.

    let callback = Box::new(|_event: DebouncedEvent| {});
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    let current_dir = std::env::current_dir().expect("Failed to get current dir");

    // Register client
    let _ = watcher.register_client(12345, &current_dir);

    watcher.stop();
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_unregister_nonexistent_client() {
    let callback = Box::new(|_event: DebouncedEvent| {});
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    // Unregistering a client that was never registered should not error
    let result = watcher.unregister_client(99999);
    assert!(result.is_ok());
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_filesystem_watcher_drop_without_stop() {
    // Test that FileSystemWatcher cleanup happens via Drop even without calling stop()
    use gpy_agent::watcher::filesystem::FileSystemWatcher;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let callback = Box::new(move |event| {
        if let Ok(mut guard) = events_clone.lock() {
            guard.push(event);
        }
    });

    // Create watcher and let it drop without calling stop()
    {
        let _watcher = FileSystemWatcher::new(
            callback,
            None,
            Duration::from_millis(100),
            Arc::new(|_, _| {}),
        )
        .expect("Should create watcher");
        // Watcher goes out of scope here without stop() being called
    }

    // Give a moment for cleanup to complete
    std::thread::sleep(Duration::from_millis(100));

    // Test passes if we get here without hanging
    // The Drop implementation should have cleaned up the background thread
}

#[test]
fn test_watch_coordinator_drop_without_stop() {
    // Test that WatchCoordinator cleanup happens via Drop even without calling stop()
    use std::time::Duration;

    let callback = Box::new(|_event| {});

    // Create coordinator and let it drop without calling stop()
    {
        let _coordinator =
            gpy_agent::watcher::WatchCoordinator::new(Duration::from_millis(100), callback, None)
                .expect("Should create coordinator");
        // Coordinator goes out of scope here without stop() being called
    }

    // Give a moment for cleanup to complete
    std::thread::sleep(Duration::from_millis(100));

    // Test passes if we get here without hanging
    // The Drop implementation should have cleaned up all background threads
}

#[test]
fn test_multi_repo_watcher_drop_without_stop() {
    // Test that MultiRepoWatcher cleanup happens via Drop even without calling stop()
    use gpy_agent::watcher::{WatcherConfig, multi_repo::MultiRepoWatcher};

    let callback = Box::new(|_event| {});

    // Create watcher and let it drop without calling stop()
    {
        let _watcher = MultiRepoWatcher::builder()
            .config(WatcherConfig::default())
            .callback(callback)
            .build()
            .expect("Should create watcher");
        // Watcher goes out of scope here without stop() being called
    }

    // Give a moment for cleanup to complete
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Test passes if we get here without hanging
    // The Drop implementation should have cleaned up all background threads
}

#[test]
#[cfg(unix)]
// Test that unregister_client cleans up watcher when last client for repo exits
//
// This test verifies that the MultiRepoWatcher correctly stops watching a repository
// when the last client for that repo unregisters (either via explicit unregister or
// via dead client pruning).
//
// # Panics
//
// Panics if watcher cleanup doesn't work correctly.
fn test_unregister_client_cleans_up_watcher() {
    use gpy_agent::ipc::ClientDirectory;
    use std::sync::Arc;

    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();

    // Initialize git repo
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");

    let config = WatcherConfig::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let watcher = MultiRepoWatcher::builder()
        .config(config)
        .callback(callback)
        .build()
        .expect("create watcher");
    let client_directory = Arc::new(ClientDirectory::new());

    // Register a client (using a fake PID that definitely doesn't exist)
    let fake_pid = 999_999_u32;
    client_directory.register(fake_pid, Some(repo_path.to_path_buf()));
    let _ = watcher.register_client(fake_pid, repo_path);

    // Verify watcher is tracking this repo
    let watched_count_before = watcher.watched_repo_count();
    assert_eq!(
        watched_count_before, 1,
        "Watcher should be tracking 1 repository"
    );

    // Simulate dead client pruning: unregister the client
    let _ = watcher.unregister_client(fake_pid);

    // Verify watcher stopped tracking this repo
    let watched_count_after = watcher.watched_repo_count();
    assert_eq!(
        watched_count_after, 0,
        "Watcher should stop tracking repository after last client unregisters"
    );

    // Cleanup
    watcher.stop();
}

#[test]
// gpy-agent#384: nextest runs each test as its own OS process, and this test
// plus `test_stop_does_not_block_on_in_flight_slow_callback` both stand up a
// real `notify` FSEvents stream. When both happen to be scheduled at nearly
// the same wall-clock instant (they're commonly the last two tests, and
// nextest retries failures for both together), the two concurrently-launched
// FSEventStreams can end up never delivering their first event within any
// budget tried (2s through 15s) -- not "late", genuinely absent for that
// process's lifetime -- while an immediately-following solo run passes in
// well under 300ms. `#[serial]` alone would not help here since it only
// guards against same-process interleaving; nextest's per-test-process model
// needs the cross-process file lock `file_serial` provides so these two
// never bootstrap an FSEventStream concurrently with each other.
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn test_slow_callback_in_one_repo_does_not_delay_another() {
    // Regression test for #306: the flush thread used to invoke callbacks
    // while holding the debouncer mutex, so a slow callback for one repo
    // stalled event delivery (and ingestion) for every other watched repo.
    // Dispatch must be decoupled per event so a slow repo cannot block a
    // fast one.
    let slow_repo = tempdir().expect("tempdir");
    let fast_repo = tempdir().expect("tempdir");
    init_git_repo(slow_repo.path());
    init_git_repo(fast_repo.path());

    let slow_root =
        MultiRepoWatcher::find_git_root(slow_repo.path()).expect("slow repo should resolve");

    let fast_completed_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let fast_completed_at_cb = Arc::clone(&fast_completed_at);

    // The slow callback sleeps far longer than the generous delivery budget
    // below (2s, calibrated against `theme_manager_tests.rs`'s equivalent
    // "first FS event observed" wait) so there's still a clear, load-tolerant
    // gap between "fast delivery is prompt" and "fast got stuck behind slow".
    let slow_callback_duration = Duration::from_secs(5);
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |event| {
        if event.repo == slow_root {
            std::thread::sleep(slow_callback_duration);
        } else {
            *fast_completed_at_cb.lock().expect("lock") = Some(Instant::now());
        }
    });

    let mut coordinator = WatchCoordinator::new(Duration::from_millis(20), callback, None)
        .expect("create coordinator");
    coordinator
        .watch_directory(slow_repo.path())
        .expect("watch slow repo");
    coordinator
        .watch_directory(fast_repo.path())
        .expect("watch fast repo");

    // Give the watcher a moment to establish its OS-level subscriptions.
    std::thread::sleep(Duration::from_millis(100));

    let start = Instant::now();
    std::fs::write(slow_repo.path().join(".git/HEAD"), "ref: refs/heads/main\n")
        .expect("write slow HEAD");
    std::fs::write(fast_repo.path().join(".git/HEAD"), "ref: refs/heads/main\n")
        .expect("write fast HEAD");

    // 2s matches the wait_until budget `theme_manager_tests.rs` uses for the
    // same underlying wait (a single notify-crate FS event reaching a
    // callback under full-suite load) -- OS file-watch delivery latency, not
    // anything specific to slow-callback isolation, dominates this budget.
    let delivery_budget = Duration::from_secs(2);
    let delivered = wait_until(delivery_budget, Duration::from_millis(10), || {
        fast_completed_at.lock().expect("lock").is_some()
    });

    coordinator.stop();

    assert!(
        delivered,
        "fast repo's event should be delivered without waiting on the slow repo's callback"
    );
    let elapsed = fast_completed_at
        .lock()
        .expect("lock")
        .expect("fast callback should have recorded a timestamp")
        - start;
    assert!(
        elapsed < delivery_budget,
        "fast repo delivery took {elapsed:?}, expected well under the slow repo's {slow_callback_duration:?} callback"
    );
}

#[test]
// gpy-agent#384: see the matching comment on
// `test_slow_callback_in_one_repo_does_not_delay_another` -- this test stands
// up its own FSEvents stream too, and must not bootstrap it concurrently with
// that test's in a separate nextest process.
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn test_stop_does_not_block_on_in_flight_slow_callback() {
    // Regression test for #306: WatchCoordinator::stop() joins the flush
    // thread. Once dispatch moved off that thread, stop() should return
    // promptly even while a slow callback is still running in the
    // background, instead of blocking behind an in-flight scan.
    let repo = tempdir().expect("tempdir");
    init_git_repo(repo.path());

    let dispatch_started: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let dispatch_started_cb = Arc::clone(&dispatch_started);

    let in_flight_callback_duration = Duration::from_secs(3);
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |_event| {
        *dispatch_started_cb.lock().expect("lock") = Some(Instant::now());
        std::thread::sleep(in_flight_callback_duration);
    });

    let mut coordinator = WatchCoordinator::new(Duration::from_millis(20), callback, None)
        .expect("create coordinator");
    coordinator
        .watch_directory(repo.path())
        .expect("watch repo");

    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("write HEAD");

    // 2s matches the wait_until budget `theme_manager_tests.rs` uses for the
    // same underlying wait (a single notify-crate FS event reaching a
    // callback under full-suite load) -- OS file-watch delivery latency
    // dominates this budget, independent of the coordinator's own dispatch
    // path.
    let dispatched = wait_until(Duration::from_secs(2), Duration::from_millis(10), || {
        dispatch_started.lock().expect("lock").is_some()
    });
    assert!(dispatched, "callback should have started dispatching");

    let stop_started = Instant::now();
    coordinator.stop();
    let stop_elapsed = stop_started.elapsed();

    // stop() only needs to join the flush thread (which wakes every 20ms to
    // check the stop flag) -- it must not block behind the in-flight 3s
    // callback. 1.5s leaves comfortable headroom over that ~20ms join cost
    // for scheduler contention while still being clearly less than the
    // callback's full duration, so a regression that makes stop() wait on
    // the callback would still be caught.
    assert!(
        stop_elapsed < Duration::from_millis(1500),
        "stop() took {stop_elapsed:?}, expected it not to block on the in-flight {in_flight_callback_duration:?} callback"
    );
}

#[test]
// Stands up its own real FS watch stream, so it takes the same bootstrap lock
// as the two tests above.
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn test_no_callback_fires_after_stop_for_a_pending_directory_followup() {
    // Regression test for #569: the directory-create follow-up used to be a
    // detached thread that slept past stop() and then re-entered the stopped
    // coordinator's debouncer -- state arriving after stop() had already
    // cleared it, deliverable to the callback by any later flush. The
    // follow-up now lives inside the debouncer from the moment it is
    // scheduled, so stop() clears it along with everything else pending.
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let repo = tempdir().expect("tempdir");
    init_git_repo(repo.path());

    let stopped = Arc::new(AtomicBool::new(false));
    let fired_after_stop = Arc::new(AtomicUsize::new(0));
    let delivered = Arc::new(AtomicUsize::new(0));

    let stopped_cb = Arc::clone(&stopped);
    let fired_after_stop_cb = Arc::clone(&fired_after_stop);
    let delivered_cb = Arc::clone(&delivered);
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |_event| {
        if stopped_cb.load(Ordering::SeqCst) {
            fired_after_stop_cb.fetch_add(1, Ordering::SeqCst);
        }
        delivered_cb.fetch_add(1, Ordering::SeqCst);
    });

    // The coordinator uses its debounce window as the directory-create
    // follow-up delay, so one window is both the flush period and the
    // follow-up's due offset. 400ms is long enough to stop() well inside the
    // window a follow-up would still be waiting out -- the interval in which
    // the old detached thread was unreachable from stop().
    let debounce = Duration::from_millis(400);
    let mut coordinator =
        WatchCoordinator::new(debounce, callback, None).expect("create coordinator");
    coordinator
        .watch_directory(repo.path())
        .expect("watch repo");
    std::thread::sleep(Duration::from_millis(150));

    // Prove the watch is live and delivering worktree events through this
    // coordinator before relying on it below -- otherwise a mkdir whose event
    // never arrived would make the assertions vacuous.
    std::fs::write(repo.path().join("warmup.txt"), b"warm").expect("write warmup file");
    assert!(
        wait_until(Duration::from_secs(5), Duration::from_millis(10), || {
            delivered.load(Ordering::SeqCst) > 0
        }),
        "the watch should deliver an ordinary worktree write before the real scenario runs"
    );

    // The scenario: create a directory (scheduling a follow-up due one
    // debounce window out), then stop well inside that window.
    std::fs::create_dir_all(repo.path().join("newdir")).expect("mkdir");
    std::thread::sleep(Duration::from_millis(120));
    coordinator.stop();
    stopped.store(true, Ordering::SeqCst);

    // Wait past the follow-up's due time and several flush periods, giving any
    // leftover mechanism every chance to misfire.
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(
        fired_after_stop.load(Ordering::SeqCst),
        0,
        "no callback may fire once stop() has returned, including a pending directory-create \
         follow-up (#569)"
    );

    // And nothing may have been *queued* after stop() either: a follow-up that
    // re-entered the debouncer behind stop()'s clear would sit there until the
    // next flush and be delivered then, which is the same bug one call later.
    coordinator.flush_pending_events();
    assert_eq!(
        fired_after_stop.load(Ordering::SeqCst),
        0,
        "a stopped coordinator must hold nothing to flush: a follow-up that landed after \
         stop() cleared the debouncer would surface here (#569)"
    );
}
