//! Client attach/detach without `ops_lock`, the teardown grace, retrying a
//! degraded worktree watch, the catch-up refresh once a watch is live, and
//! registering git directories before their watch is armed.

use super::tests::{RecordingTarget, no_fault, recording_watcher, test_registry, tracked_worktree};
use super::{MultiRepoWatcher, WatchTarget};
use crate::watcher::WatchRegistry;
use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tempfile::TempDir;

/// Calls a [`FakeTarget`] received, in order.
type CallLog = Arc<Mutex<Vec<String>>>;

/// A target that arms nothing and records every call, so ordering between a
/// watch going live and its catch-up refresh is deterministic.
struct FakeTarget {
    log: CallLog,
}

impl FakeTarget {
    fn record(&self, call: &str, path: &Path) {
        self.log
            .lock()
            .expect("call log")
            .push(format!("{call} {}", path.display()));
    }
}

impl WatchTarget for FakeTarget {
    fn watch(&mut self, path: &Path) -> Result<()> {
        self.record("watch", path);
        Ok(())
    }

    fn watch_shallow(&mut self, path: &Path) -> Result<()> {
        self.record("watch_shallow", path);
        Ok(())
    }

    fn covers(&self, _path: &Path) -> bool {
        false
    }

    fn unwatch(&mut self, path: &Path) -> Result<()> {
        self.record("unwatch", path);
        Ok(())
    }

    fn unwatch_shallow(&mut self, path: &Path) -> Result<()> {
        self.record("unwatch_shallow", path);
        Ok(())
    }

    fn stop_watching(&mut self) {}

    fn request_refresh(&self, git_root: &Path) {
        self.record("refresh", git_root);
    }
}

/// A watcher over a [`FakeTarget`] with `grace`, and the target's call log.
fn fake_watcher(grace: Duration) -> (MultiRepoWatcher<FakeTarget>, CallLog) {
    let log = CallLog::default();
    let watcher = MultiRepoWatcher {
        watcher: Arc::new(Mutex::new(Some(FakeTarget {
            log: Arc::clone(&log),
        }))),
        watched_repos: Arc::default(),
        registry: test_registry(true),
        common_watches: Mutex::default(),
        ops_lock: Mutex::new(()),
        teardown_grace: grace,
    };
    (watcher, log)
}

/// How many catch-up refreshes `root` has been sent.
fn refreshes(log: &CallLog, root: &Path) -> usize {
    let expected = format!("refresh {}", root.display());
    log.lock()
        .expect("call log")
        .iter()
        .filter(|call| **call == expected)
        .count()
}

/// Generous ceiling for a call that must not block; only a hang reaches it.
const PROMPT_BUDGET: Duration = Duration::from_secs(5);

/// A temp dir holding `names` as plain repos (bare `.git` dirs), canonical.
fn plain_repos(names: &[&str]) -> (TempDir, Vec<PathBuf>) {
    let temp_dir = TempDir::new().expect("temp dir");
    let base = std::fs::canonicalize(temp_dir.path()).expect("canonical base");
    let roots = names
        .iter()
        .map(|name| {
            let root = base.join(name);
            std::fs::create_dir_all(root.join(".git")).expect("git dir");
            root
        })
        .collect();
    (temp_dir, roots)
}

/// Paths the watcher's target has unwatched so far.
fn unwatched(watcher: &MultiRepoWatcher<RecordingTarget>) -> Vec<PathBuf> {
    watcher
        .watcher
        .lock()
        .expect("watcher lock")
        .as_ref()
        .expect("coordinator present")
        .unwatched
        .clone()
}

/// Whether `root` has a tracked entry, client-free or not.
fn is_tracked(watcher: &MultiRepoWatcher<RecordingTarget>, root: &Path) -> bool {
    watcher
        .watched_repos
        .lock()
        .expect("repos lock")
        .contains_key(root)
}

/// Run `op` on another thread while `ops_lock` is held; whether it finished.
fn finishes_while_ops_lock_held(
    watcher: &Arc<MultiRepoWatcher<RecordingTarget>>,
    op: impl FnOnce(&MultiRepoWatcher<RecordingTarget>) + Send + 'static,
) -> bool {
    let guard = watcher.ops_lock.lock().expect("ops lock");
    let (done_tx, done_rx) = mpsc::channel();
    let worker_watcher = Arc::clone(watcher);
    let worker = std::thread::spawn(move || {
        op(&worker_watcher);
        let _ = done_tx.send(());
    });
    let finished = done_rx.recv_timeout(PROMPT_BUDGET).is_ok();
    drop(guard);
    worker.join().expect("worker thread");
    finished
}

/// A second shell joining a watched repository needs no arming, so it must
/// not queue behind another repository's arming walk on `ops_lock`.
#[test]
fn joining_a_watched_repo_does_not_wait_for_ops_lock() {
    let (_temp, roots) = plain_repos(&["a"]);
    let root = roots.first().expect("repo a").clone();
    let watcher = Arc::new(recording_watcher(test_registry(true), no_fault()));
    watcher.attach_client(1, &root).expect("first attach");

    let join_root = root.clone();
    assert!(finishes_while_ops_lock_held(&watcher, move |w| {
        w.attach_client(2, &join_root).expect("second attach");
    }));
    assert_eq!(watcher.client_pids(), vec![1, 2]);
}

/// Leaving a repository that keeps other clients tears nothing down.
#[test]
fn leaving_a_shared_repo_does_not_wait_for_ops_lock() {
    let (_temp, roots) = plain_repos(&["a"]);
    let root = roots.first().expect("repo a").clone();
    let watcher = Arc::new(recording_watcher(test_registry(true), no_fault()));
    watcher.attach_client(1, &root).expect("attach 1");
    watcher.attach_client(2, &root).expect("attach 2");

    assert!(finishes_while_ops_lock_held(&watcher, |w| {
        w.unregister_client(2);
    }));
    assert_eq!(watcher.client_pids(), vec![1]);
    assert_eq!(watcher.watched_repo_count(), 1);
}

/// Within the grace a client-free repo stays armed and a returning client
/// re-arms nothing; after it, `release_idle_repos` tears it down.
#[test]
fn idle_repo_stays_armed_for_the_grace_then_is_released() {
    let grace = Duration::from_millis(200);
    let (_temp, roots) = plain_repos(&["a"]);
    let root = roots.first().expect("repo a").clone();
    let mut watcher = recording_watcher(test_registry(true), no_fault());
    watcher.teardown_grace = grace;

    watcher.attach_client(1, &root).expect("attach");
    watcher.unregister_client(1);
    assert_eq!(
        watcher.watched_repo_count(),
        0,
        "an idle repo is not counted"
    );
    assert!(watcher.watched_roots().is_empty(), "nor reconciled");
    assert!(is_tracked(&watcher, &root), "but it stays armed");
    assert_eq!(unwatched(&watcher), Vec::<PathBuf>::new());

    watcher
        .attach_client(1, &root)
        .expect("re-attach within the grace");
    assert!(unwatched(&watcher).is_empty(), "re-attach must not re-arm");
    assert_eq!(watcher.watched_repo_count(), 1);

    watcher.unregister_client(1);
    watcher.release_idle_repos();
    assert!(is_tracked(&watcher, &root), "still within the grace");

    std::thread::sleep(grace.saturating_mul(2));
    watcher.release_idle_repos();
    assert!(!is_tracked(&watcher, &root), "released after the grace");
    assert!(unwatched(&watcher).contains(&root));
}

/// A repo whose worktree watch failed (watch budget) is re-armed once
/// another repository releases its watches.
#[test]
fn degraded_repo_is_rearmed_when_another_repo_releases_its_watches() {
    let (_temp, roots) = plain_repos(&["a", "b"]);
    let degraded = roots.first().expect("repo a").clone();
    let other = roots.get(1).expect("repo b").clone();
    let failing = Arc::new(AtomicBool::new(true));
    let failing_for_fault = Arc::clone(&failing);
    let faulty_root = degraded.clone();
    let watcher = recording_watcher(
        test_registry(true),
        Arc::new(move |path: &Path| {
            (path == faulty_root && failing_for_fault.load(Ordering::SeqCst))
                .then(|| Error::watcher("injected watch-limit failure (test)"))
        }),
    );

    watcher
        .attach_client(1, &degraded)
        .expect("attach degraded");
    watcher.attach_client(2, &other).expect("attach other");
    assert!(
        !tracked_worktree(&watcher, &degraded),
        "degraded to .git-only"
    );

    failing.store(false, Ordering::SeqCst);
    watcher.unregister_client(2);

    assert!(
        tracked_worktree(&watcher, &degraded),
        "released watches must let the degraded repo re-arm its worktree"
    );
}

/// A change made before a repo's watch is live (e.g. a checkout racing the
/// shell's registration) has no event; arming queues one refresh after the
/// watch call, and joining the live repo queues none.
#[test]
fn arming_queues_one_catch_up_refresh_after_the_watch_is_live() {
    let (_temp, roots) = plain_repos(&["a"]);
    let root = roots.first().expect("repo a").clone();
    let (watcher, log) = fake_watcher(Duration::ZERO);

    watcher.attach_client(1, &root).expect("attach");
    let calls = log.lock().expect("call log").clone();
    let watch_at = calls
        .iter()
        .position(|call| *call == format!("watch {}", root.display()))
        .expect("the repo was watched");
    let refresh_at = calls
        .iter()
        .position(|call| *call == format!("refresh {}", root.display()))
        .expect("a catch-up refresh was queued");
    assert!(watch_at < refresh_at, "refresh before the watch: {calls:?}");
    assert_eq!(refreshes(&log, &root), 1);

    watcher.attach_client(2, &root).expect("join the live repo");
    assert_eq!(refreshes(&log, &root), 1, "a live repo needs no catch-up");
}

/// Git events for a client-free repo in its grace are dropped, so a client
/// returning to it queues one catch-up refresh.
#[test]
fn returning_to_an_idle_repo_queues_one_catch_up_refresh() {
    let (_temp, roots) = plain_repos(&["a"]);
    let root = roots.first().expect("repo a").clone();
    let (watcher, log) = fake_watcher(Duration::from_secs(3_600));

    watcher.attach_client(1, &root).expect("attach");
    watcher.unregister_client(1);
    watcher
        .attach_client(1, &root)
        .expect("return within the grace");

    assert_eq!(refreshes(&log, &root), 2, "arming plus the return");
}

/// A repo reports a watched period only while a client is attached, and a
/// client returning to it starts a new one: changes made while it was idle
/// were never reported, so a status scanned before must not be trusted.
#[test]
fn watched_since_covers_only_attached_periods() {
    let (_temp, roots) = plain_repos(&["a"]);
    let root = roots.first().expect("repo a").clone();
    let (watcher, _log) = fake_watcher(Duration::from_secs(3_600));
    assert_eq!(watcher.watched_since(&root), None);

    watcher.attach_client(1, &root).expect("attach");
    let first = watcher
        .watched_since(&root)
        .expect("watched while attached");
    watcher.attach_client(2, &root).expect("join the live repo");
    assert_eq!(
        watcher.watched_since(&root),
        Some(first),
        "joining keeps it"
    );

    watcher.unregister_client(1);
    watcher.unregister_client(2);
    assert_eq!(watcher.watched_since(&root), None, "idle: nothing reports");

    std::thread::sleep(Duration::from_millis(2));
    watcher
        .attach_client(1, &root)
        .expect("return within the grace");
    assert!(
        watcher
            .watched_since(&root)
            .is_some_and(|since| since > first),
        "returning starts a new watched period"
    );
}

/// Each armed path, and whether the registry knew it as git metadata then.
type ArmLog = Arc<Mutex<Vec<(PathBuf, bool)>>>;

/// A target that arms nothing and records, per watch, whether the registry
/// already held the path's git directory: the poll backend picks a path's
/// poller from the registry when the path is armed.
struct RoutingTarget {
    registry: Arc<WatchRegistry>,
    log: ArmLog,
}

impl RoutingTarget {
    fn record(&self, path: &Path) {
        let registered = self.registry.path_below_registered_gitdir(path).is_some();
        self.log
            .lock()
            .expect("arm log")
            .push((path.to_path_buf(), registered));
    }
}

impl WatchTarget for RoutingTarget {
    fn watch(&mut self, path: &Path) -> Result<()> {
        self.record(path);
        Ok(())
    }

    fn watch_shallow(&mut self, path: &Path) -> Result<()> {
        self.record(path);
        Ok(())
    }

    fn covers(&self, _path: &Path) -> bool {
        false
    }

    fn unwatch(&mut self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn unwatch_shallow(&mut self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn stop_watching(&mut self) {}
}

/// A linked worktree's gitdir and a bare common dir have no `.git` component,
/// so only the registry tells the poll backend they are git metadata. Both
/// must be registered before they are armed, on attach and on the re-arm
/// after the gitdir moved.
#[test]
fn external_and_common_gitdirs_are_registered_before_they_are_armed() {
    let temp_dir = TempDir::new().expect("temp dir");
    let base = std::fs::canonicalize(temp_dir.path()).expect("canonical base");
    let common = base.join("main.git");
    std::fs::create_dir_all(common.join("refs")).expect("common refs");
    let root = base.join("wt");
    std::fs::create_dir_all(&root).expect("worktree root");
    let link_worktree = |name: &str| {
        let admin = common.join("worktrees").join(name);
        std::fs::create_dir_all(&admin).expect("admin dir");
        std::fs::write(admin.join("commondir"), "../..\n").expect("commondir");
        std::fs::write(root.join(".git"), format!("gitdir: {}\n", admin.display()))
            .expect(".git pointer");
        admin
    };
    let registry = test_registry(true);
    let log = ArmLog::default();
    let watcher = MultiRepoWatcher {
        watcher: Arc::new(Mutex::new(Some(RoutingTarget {
            registry: Arc::clone(&registry),
            log: Arc::clone(&log),
        }))),
        watched_repos: Arc::default(),
        registry,
        common_watches: Mutex::default(),
        ops_lock: Mutex::new(()),
        teardown_grace: Duration::ZERO,
    };

    let first_gitdir = link_worktree("wt");
    watcher.attach_client(1, &root).expect("attach");
    let moved_gitdir = link_worktree("wt-moved");
    watcher
        .attach_client(1, &root)
        .expect("re-arm after the gitdir moved");

    let arms = log.lock().expect("arm log").clone();
    for gitdir_path in [
        first_gitdir,
        moved_gitdir,
        common.clone(),
        common.join("refs"),
    ] {
        let registered_when_armed: Vec<bool> = arms
            .iter()
            .filter(|(path, _)| *path == gitdir_path)
            .map(|(_, registered)| *registered)
            .collect();
        assert!(
            !registered_when_armed.is_empty() && registered_when_armed.iter().all(|known| *known),
            "{} was armed before it was registered: {arms:?}",
            gitdir_path.display()
        );
    }
}
