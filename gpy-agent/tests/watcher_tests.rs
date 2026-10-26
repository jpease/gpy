//! Unit tests for MultiRepoWatcher

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::doc_markdown)] // watcher API paths include socket filenames that aren't Markdown links

#[path = "common/git.rs"]
mod git_helpers;
#[path = "common/wait.rs"]
mod wait;

use git_helpers::git_init;
use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
use gpy_agent::watcher::{DebouncedEvent, FileEvent, WatchCoordinator, WatcherConfig};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
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

/// File [`prime_watch`] writes to prove a watch is live.
const PROBE_FILE: &str = "watch-probe.txt";
/// How long [`prime_watch`] waits on one probe write before writing it again.
const PROBE_REWRITE: Duration = Duration::from_secs(1);
/// Deadline for one write to reach a callback through a live watch.
const DELIVERY_DEADLINE: Duration = Duration::from_secs(10);
/// Longest a [`Gate`] holds a callback, so a regression fails instead of hanging.
const GATE_HOLD: Duration = Duration::from_secs(30);

/// A latch a callback blocks on until the test opens it.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Gate {
    /// Block until [`Gate::open`] or [`GATE_HOLD`] elapses.
    fn wait(&self) {
        let guard = self.open.lock().expect("lock");
        drop(
            self.opened
                .wait_timeout_while(guard, GATE_HOLD, |is_open| !*is_open)
                .expect("lock"),
        );
    }

    fn open(&self) {
        *self.open.lock().expect("lock") = true;
        self.opened.notify_all();
    }
}

/// What a gated callback shares with its test.
#[derive(Default)]
struct Dispatch {
    /// Set once [`prime_watch`] proved the watch live; earlier events are ignored.
    armed: AtomicBool,
    /// Repositories whose probe write was delivered.
    probed: Mutex<HashSet<PathBuf>>,
    /// A gated callback started.
    started: AtomicBool,
    /// A gated callback returned.
    finished: AtomicBool,
    /// An event the test does not gate was delivered.
    other_delivered: AtomicBool,
    gate: Gate,
}

impl Dispatch {
    /// Record a probe and report whether `event` is a real one to act on.
    fn admit(&self, event: &DebouncedEvent) -> bool {
        if is_probe(event) {
            self.probed.lock().expect("lock").insert(event.repo.clone());
            return false;
        }
        self.armed.load(Ordering::SeqCst)
    }

    fn probed_repos(&self) -> usize {
        self.probed.lock().expect("lock").len()
    }

    /// Run as a slow callback: block on the gate between `started` and `finished`.
    fn run_gated(&self) {
        self.started.store(true, Ordering::SeqCst);
        self.gate.wait();
        self.finished.store(true, Ordering::SeqCst);
    }
}

/// Whether `event` covers only [`prime_watch`]'s probe file.
fn is_probe(event: &DebouncedEvent) -> bool {
    matches!(&event.event, FileEvent::Git { paths } if paths.as_slice().is_some_and(|changed| {
        changed
            .iter()
            .all(|path| path.file_name() == Some(OsStr::new(PROBE_FILE)))
    }))
}

/// Write a probe file in each of `repos` until `delivered` reports the watch
/// live. OS watches expose no "armed" signal and a write that lands before the
/// subscription is live is never reported, so a delivered write is the proof;
/// the probe repeats every [`PROBE_REWRITE`] rather than sleeping a guess.
fn prime_watch(repos: &[&Path], mut delivered: impl FnMut() -> bool) {
    let mut written_at: Option<Instant> = None;
    let live = wait_until(Duration::from_secs(10), Duration::from_millis(10), || {
        if delivered() {
            return true;
        }
        if written_at.is_none_or(|at| at.elapsed() >= PROBE_REWRITE) {
            for repo in repos {
                std::fs::write(repo.join(PROBE_FILE), b"probe").expect("write probe file");
            }
            written_at = Some(Instant::now());
        }
        false
    });
    assert!(live, "the watch never delivered a probe write");
}

/// A temp directory outside any git repository. Requires `TMPDIR` itself to
/// lie outside a repository, which every test here that needs a non-repo
/// path assumes.
fn non_repo_dir() -> TempDir {
    let dir = tempdir().expect("create temp dir");
    assert!(
        gpy_agent::git::find_git_dir_only(dir.path()).is_none(),
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

    let git_root = gpy_agent::git::find_git_dir_only(&nested).expect("git root from nested dir");

    assert_eq!(
        git_root,
        std::fs::canonicalize(repo.path()).expect("canonical repo path")
    );
}

#[test]
fn test_find_git_root_no_repo() {
    let dir = tempdir().expect("create temp dir");

    assert_eq!(
        gpy_agent::git::find_git_dir_only(dir.path()),
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

    let git_root = gpy_agent::git::find_git_dir_only(&symlink_dir).expect("git root from symlink");
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
fn test_attach_client_non_git_directory() {
    let watcher = idle_watcher();
    let dir = non_repo_dir();

    watcher
        .attach_client(12345, dir.path())
        .expect("registering outside a repo should succeed");

    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn test_attach_client_in_git_repo() {
    let watcher = idle_watcher();
    let repo = temp_repo();

    watcher
        .attach_client(12345, repo.path())
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
            .attach_client(pid, repo.path())
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
        .attach_client(12345, repo.path())
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
            .attach_client(pid, repo.path())
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
fn test_attach_client_moves_repo() {
    let watcher = idle_watcher();
    let repo = temp_repo();
    let elsewhere = non_repo_dir();

    watcher
        .attach_client(12345, repo.path())
        .expect("register client");
    assert_eq!(watcher.watched_repo_count(), 1);

    watcher
        .attach_client(12345, elsewhere.path())
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
        .attach_client(12345, repo.path())
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
        .attach_client(fake_pid, repo_path)
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
        gpy_agent::git::find_git_dir_only(slow_repo.path()).expect("slow repo should resolve");

    let state = Arc::new(Dispatch::default());
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = {
        let state_cb = Arc::clone(&state);
        Box::new(move |event| {
            if !state_cb.admit(&event) {
                return;
            }
            if event.repo == slow_root {
                state_cb.run_gated();
            } else {
                state_cb.other_delivered.store(true, Ordering::SeqCst);
            }
        })
    };

    let mut coordinator = WatchCoordinator::new(Duration::from_millis(20), callback, None)
        .expect("create coordinator");
    coordinator
        .watch_directory(slow_repo.path())
        .expect("watch slow repo");
    coordinator
        .watch_directory(fast_repo.path())
        .expect("watch fast repo");
    prime_watch(&[slow_repo.path(), fast_repo.path()], || {
        state.probed_repos() == 2
    });
    state.armed.store(true, Ordering::SeqCst);

    std::fs::write(slow_repo.path().join(".git/HEAD"), "ref: refs/heads/main\n")
        .expect("write slow HEAD");
    assert!(
        wait_until(DELIVERY_DEADLINE, Duration::from_millis(10), || {
            state.started.load(Ordering::SeqCst)
        }),
        "the slow repo's callback should have started"
    );
    std::fs::write(fast_repo.path().join(".git/HEAD"), "ref: refs/heads/main\n")
        .expect("write fast HEAD");
    let delivered = wait_until(DELIVERY_DEADLINE, Duration::from_millis(10), || {
        state.other_delivered.load(Ordering::SeqCst)
    });
    let slow_still_running = !state.finished.load(Ordering::SeqCst);

    state.gate.open();
    coordinator.stop();

    assert!(
        delivered,
        "fast repo's event should be delivered without waiting on the slow repo's callback"
    );
    assert!(
        slow_still_running,
        "the fast repo's event must arrive while the slow repo's callback is still running"
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
    // callbacks itself, so stop() returns while a callback is still running.
    let repo = temp_repo();

    let state = Arc::new(Dispatch::default());
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = {
        let state_cb = Arc::clone(&state);
        Box::new(move |event| {
            if state_cb.admit(&event) {
                state_cb.run_gated();
            }
        })
    };

    let mut coordinator = WatchCoordinator::new(Duration::from_millis(20), callback, None)
        .expect("create coordinator");
    coordinator
        .watch_directory(repo.path())
        .expect("watch repo");
    prime_watch(&[repo.path()], || state.probed_repos() == 1);
    state.armed.store(true, Ordering::SeqCst);

    std::fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("write HEAD");
    assert!(
        wait_until(DELIVERY_DEADLINE, Duration::from_millis(10), || {
            state.started.load(Ordering::SeqCst)
        }),
        "callback should have started dispatching"
    );

    // A stop() that waited on the callback could only return once the gate
    // timed out and the callback finished.
    coordinator.stop();
    let finished_before_stop_returned = state.finished.load(Ordering::SeqCst);
    state.gate.open();

    assert!(
        !finished_before_stop_returned,
        "stop() must return while the in-flight callback is still running"
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
    // Prove the watch is live and delivering worktree events through this
    // coordinator before relying on it below -- otherwise a mkdir whose event
    // never arrived would make the assertions vacuous.
    prime_watch(&[repo.path()], || delivered.load(Ordering::SeqCst) > 0);

    // The scenario: create a directory (scheduling a follow-up due one
    // debounce window out), then stop well inside that window.
    std::fs::create_dir_all(repo.path().join("newdir")).expect("mkdir");
    // Let the mkdir event be ingested (scheduling the follow-up) while
    // staying well inside the 400ms window before stopping. Nothing reports
    // ingestion; stopping too early only makes the test vacuous, not flaky.
    std::thread::sleep(Duration::from_millis(120));
    coordinator.stop();
    stopped.store(true, Ordering::SeqCst);

    // Absence window: past the follow-up's due time and several flush
    // periods, giving any leftover mechanism every chance to misfire.
    std::thread::sleep(debounce * 2 + Duration::from_millis(100));
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
