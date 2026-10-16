//! Unit tests for MultiRepoWatcher

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)] // watcher API paths include socket filenames that aren't Markdown links

#[path = "common/git.rs"]
mod git_helpers;
#[path = "common/wait.rs"]
mod wait;

use git_helpers::git_init;
use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
use gpy_agent::watcher::{DebouncedEvent, FileEvent, WatchCoordinator, WatcherConfig};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};
use wait::wait_until;

#[cfg(unix)]
use std::os::unix::fs::symlink;

/// A fresh git repository in a temp directory.
fn temp_repo() -> TempDir {
    let repo = tempdir().expect("create temp dir");
    git_init(repo.path());
    repo
}

/// A temp directory outside any git repository. Requires `TMPDIR` itself to
/// lie outside a repository, which every test here that needs a non-repo
/// path assumes.
fn non_repo_dir() -> TempDir {
    let dir = tempdir().expect("create temp dir");
    assert!(
        MultiRepoWatcher::find_git_root(dir.path()).is_none(),
        "TMPDIR ({}) must not be inside a git repository",
        dir.path().display()
    );
    dir
}

fn idle_watcher() -> MultiRepoWatcher {
    MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .expect("Should create watcher")
}

#[test]
fn test_find_git_root_finds_root() {
    let repo = temp_repo();
    let nested = repo.path().join("src").join("deep");
    std::fs::create_dir_all(&nested).expect("create nested dir");

    let git_root = MultiRepoWatcher::find_git_root(&nested).expect("git root from nested dir");

    assert_eq!(
        git_root,
        std::fs::canonicalize(repo.path()).expect("canonical repo path")
    );
}

#[test]
fn test_find_git_root_no_repo() {
    let dir = tempdir().expect("create temp dir");

    assert_eq!(
        MultiRepoWatcher::find_git_root(dir.path()),
        None,
        "TMPDIR ({}) must not be inside a git repository",
        dir.path().display()
    );
}

#[test]
#[cfg(unix)]
fn test_find_git_root_symlink_path() {
    let repo_dir = temp_repo();
    let repo_path = repo_dir.path();

    let symlink_dir = repo_dir.path().join("symlink");
    symlink(repo_path, &symlink_dir).expect("create symlink");

    let git_root = MultiRepoWatcher::find_git_root(&symlink_dir).expect("git root from symlink");
    let canonical_repo = std::fs::canonicalize(repo_path).expect("canonical repo path");

    assert_eq!(git_root, canonical_repo);
}

#[test]
fn test_watcher_initial_state() {
    let events = Arc::new(Mutex::new(Vec::<FileEvent>::new()));
    let events_clone = Arc::clone(&events);
    let callback = Box::new(move |event: DebouncedEvent| {
        if let Ok(mut e) = events_clone.lock() {
            e.push(event.event);
        }
    });

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("Should create watcher");

    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_register_client_non_git_directory() {
    let watcher = idle_watcher();
    let dir = non_repo_dir();

    watcher
        .register_client(12345, dir.path())
        .expect("registering outside a repo should succeed");

    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_register_client_in_git_repo() {
    let watcher = idle_watcher();
    let repo = temp_repo();

    watcher
        .register_client(12345, repo.path())
        .expect("register client");

    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "Should be watching one repo"
    );
}

#[test]
fn test_register_multiple_clients_same_repo() {
    let watcher = idle_watcher();
    let repo = temp_repo();

    for pid in [100, 200, 300] {
        watcher
            .register_client(pid, repo.path())
            .expect("register client");
    }

    // One watch per repository, not per client.
    assert_eq!(watcher.watched_repo_count(), 1);
}

#[test]
fn test_unregister_client() {
    let watcher = idle_watcher();
    let repo = temp_repo();

    watcher
        .register_client(12345, repo.path())
        .expect("register client");
    assert_eq!(watcher.watched_repo_count(), 1);

    watcher.unregister_client(12345);

    // The last client leaving stops the watch.
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_unregister_with_multiple_clients() {
    let watcher = idle_watcher();
    let repo = temp_repo();

    for pid in [100, 200, 300] {
        watcher
            .register_client(pid, repo.path())
            .expect("register client");
    }
    assert_eq!(watcher.watched_repo_count(), 1);

    watcher.unregister_client(100);
    assert_eq!(watcher.watched_repo_count(), 1);

    watcher.unregister_client(200);
    assert_eq!(watcher.watched_repo_count(), 1);

    watcher.unregister_client(300);
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_update_client() {
    let watcher = idle_watcher();
    let repo = temp_repo();
    let elsewhere = non_repo_dir();

    watcher
        .register_client(12345, repo.path())
        .expect("register client");
    assert_eq!(watcher.watched_repo_count(), 1);

    watcher
        .update_client(12345, elsewhere.path())
        .expect("update client");

    // Moving the only client out of the repo stops its watch.
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_watcher_stop() {
    // stop() uses an explicit stop flag, so shutdown does not depend on notify
    // closing its sender promptly on every platform.
    let watcher = idle_watcher();
    let repo = temp_repo();

    watcher
        .register_client(12345, repo.path())
        .expect("register client");
    assert_eq!(watcher.watched_repo_count(), 1);

    watcher.stop();
    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_unregister_nonexistent_client() {
    let watcher = idle_watcher();

    watcher.unregister_client(99999);
    assert_eq!(watcher.watched_repo_count(), 0);
}

// The drop-without-stop tests pass by returning: `Drop` must join every
// background thread, and a thread that never exits hangs the test.

#[test]
fn test_filesystem_watcher_drop_without_stop() {
    use gpy_agent::watcher::filesystem::FileSystemWatcher;

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let callback = Box::new(move |event| {
        if let Ok(mut guard) = events_clone.lock() {
            guard.push(event);
        }
    });

    let watcher = FileSystemWatcher::new(
        callback,
        None,
        Duration::from_millis(100),
        Arc::new(|_, _| {}),
    )
    .expect("Should create watcher");
    drop(watcher);
}

#[test]
fn test_watch_coordinator_drop_without_stop() {
    let callback = Box::new(|_event: DebouncedEvent| {});
    let coordinator = WatchCoordinator::new(Duration::from_millis(100), callback, None)
        .expect("Should create coordinator");
    drop(coordinator);
}

#[test]
fn test_multi_repo_watcher_drop_without_stop() {
    drop(idle_watcher());
}

#[test]
#[cfg(unix)]
fn test_unregister_client_cleans_up_watcher() {
    // Pruning a dead client goes through `unregister_client`; the repo's watch
    // must stop once its last client is gone.
    use gpy_agent::ipc::ClientDirectory;

    let repo_dir = temp_repo();
    let repo_path = repo_dir.path();

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("create watcher");
    let client_directory = Arc::new(ClientDirectory::new());

    // A PID that cannot belong to a live process.
    let fake_pid = 999_999_u32;
    client_directory.register(fake_pid, Some(repo_path.to_path_buf()));
    watcher
        .register_client(fake_pid, repo_path)
        .expect("register client");
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "Watcher should be tracking 1 repository"
    );

    watcher.unregister_client(fake_pid);
    assert_eq!(
        watcher.watched_repo_count(),
        0,
        "Watcher should stop tracking repository after last client unregisters"
    );

    watcher.stop();
}

#[test]
// nextest runs each test as its own OS process, and this test
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
    // The flush thread dispatches callbacks without holding the debouncer
    // mutex, so a slow callback for one repo cannot stall event delivery (or
    // ingestion) for another watched repo.
    let slow_repo = temp_repo();
    let fast_repo = temp_repo();

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
// See the matching comment on
// `test_slow_callback_in_one_repo_does_not_delay_another` -- this test stands
// up its own FSEvents stream too, and must not bootstrap it concurrently with
// that test's in a separate nextest process.
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn test_stop_does_not_block_on_in_flight_slow_callback() {
    // WatchCoordinator::stop() joins the flush thread, which does not run
    // callbacks itself, so stop() returns promptly even while a slow callback
    // is still running in the background.
    let repo = temp_repo();

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

    // Give the watcher a moment to establish its OS-level subscriptions;
    // there is no observable signal for that.
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
    // A directory-create follow-up lives inside the coordinator's debouncer
    // from the moment it is scheduled, so stop() clears it along with
    // everything else pending; nothing may reach the callback after stop().
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let repo = temp_repo();

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
    // window a follow-up would still be waiting out.
    let debounce = Duration::from_millis(400);
    let mut coordinator =
        WatchCoordinator::new(debounce, callback, None).expect("create coordinator");
    coordinator
        .watch_directory(repo.path())
        .expect("watch repo");
    // Let OS-level subscriptions establish before the warmup write below.
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
    // Let the mkdir event be ingested (scheduling the follow-up) while
    // staying well inside the 400ms window before stopping.
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
         follow-up"
    );

    // And nothing may have been *queued* after stop() either: a follow-up that
    // re-entered the debouncer behind stop()'s clear would sit there until the
    // next flush and be delivered then.
    coordinator.flush_pending_events();
    assert_eq!(
        fired_after_stop.load(Ordering::SeqCst),
        0,
        "a stopped coordinator must hold nothing to flush: a follow-up that landed after \
         stop() cleared the debouncer would surface here"
    );
}
