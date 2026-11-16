//! Client attach/detach without `ops_lock`, the teardown grace, and retrying a
//! degraded worktree watch.

use super::MultiRepoWatcher;
use super::tests::{RecordingTarget, no_fault, recording_watcher, test_registry, tracked_worktree};
use crate::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use tempfile::TempDir;

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
