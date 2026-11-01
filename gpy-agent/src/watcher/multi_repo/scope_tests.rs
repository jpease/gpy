//! Repository filter and home/root worktree-watch exclusion.

use super::MultiRepoWatcher;
use super::tests::{RecordingTarget, no_fault, recording_watcher, test_registry, tracked_worktree};
use crate::watcher::WatchRegistry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tempfile::TempDir;

/// A temp dir holding a bare `.git` directory, and its canonical root.
fn repo_dir() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("temp dir");
    std::fs::create_dir_all(temp_dir.path().join(".git")).expect("git dir");
    let root = std::fs::canonicalize(temp_dir.path()).expect("canonical root");
    (temp_dir, root)
}

/// Paths armed for the repository at `root`.
fn armed_paths(watcher: &MultiRepoWatcher<RecordingTarget>, root: &Path) -> Vec<PathBuf> {
    watcher
        .watched_repos
        .lock()
        .expect("repos lock")
        .get(root)
        .expect("repo tracked")
        .armed
        .clone()
}

/// The canonical home directory with a `.git` directory in it.
fn home_repository() -> PathBuf {
    let home = crate::paths::home_path().expect("test home");
    std::fs::create_dir_all(home.join(".git")).expect("home git dir");
    std::fs::canonicalize(&home).expect("canonical home")
}

#[test]
fn home_repository_gets_gitdir_only_watch() {
    let home = home_repository();
    let watcher = recording_watcher(test_registry(true), no_fault());

    watcher.attach_client(1, &home).expect("attach");

    assert!(!tracked_worktree(&watcher, &home));
    assert_eq!(armed_paths(&watcher, &home), vec![home.join(".git")]);
}

#[test]
fn resync_keeps_home_repository_gitdir_only() {
    let home = home_repository();
    let watcher = recording_watcher(test_registry(true), no_fault());
    watcher.attach_client(1, &home).expect("attach");

    watcher.resync_watch_worktree();

    assert!(!tracked_worktree(&watcher, &home));
    assert_eq!(armed_paths(&watcher, &home), vec![home.join(".git")]);
    assert_eq!(watcher.watched_repo_count(), 1);
}

#[test]
fn rejected_repository_is_not_watched() {
    let (_temp, root) = repo_dir();
    let registry = test_registry(true);
    registry.set_repo_filter(Arc::new(|_root: &Path| false));
    let watcher = recording_watcher(registry, no_fault());

    watcher.attach_client(1, &root).expect("attach");

    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn unlisted_git_segment_rejects_every_repo() {
    let (_temp, root) = repo_dir();
    let mut config = crate::config::Config::default();
    config
        .ui
        .enabled_segments
        .retain(|segment| segment != "git");
    let registry = test_registry(true);
    registry.set_repo_filter(crate::agent::events::git_repo_filter(&config));
    let watcher = recording_watcher(registry, no_fault());

    watcher.attach_client(1, &root).expect("attach");

    assert_eq!(watcher.watched_repo_count(), 0);
}

#[test]
fn attaching_into_a_rejected_repository_detaches_the_previous_one() {
    let (_allowed_temp, allowed) = repo_dir();
    let (_rejected_temp, rejected) = repo_dir();
    let registry = test_registry(true);
    let rejected_root = rejected.clone();
    registry.set_repo_filter(Arc::new(move |root: &Path| root != rejected_root));
    let watcher = recording_watcher(registry, no_fault());

    watcher.attach_client(7, &allowed).expect("attach allowed");
    assert_eq!(watcher.watched_roots(), vec![allowed]);

    watcher
        .attach_client(7, &rejected)
        .expect("attach rejected");

    assert_eq!(watcher.watched_repo_count(), 0);
    assert_eq!(watcher.client_pids(), Vec::<u32>::new());
}

#[test]
fn resync_repo_filter_tears_down_rejected_and_arms_allowed() {
    let (_temp, root) = repo_dir();
    let registry: Arc<WatchRegistry> = test_registry(true);
    let skipped = Arc::new(AtomicBool::new(false));
    let skipped_for_filter = Arc::clone(&skipped);
    registry.set_repo_filter(Arc::new(move |_root: &Path| {
        !skipped_for_filter.load(Ordering::Relaxed)
    }));
    let watcher = recording_watcher(registry, no_fault());
    let clients = vec![(3_u32, root.clone())];
    watcher.attach_client(3, &root).expect("attach");
    assert_eq!(watcher.watched_repo_count(), 1);

    skipped.store(true, Ordering::Relaxed);
    watcher.resync_repo_filter(&clients);
    assert_eq!(watcher.watched_repo_count(), 0);
    assert!(!watcher.registry().root_is_registered(&root));

    skipped.store(false, Ordering::Relaxed);
    watcher.resync_repo_filter(&clients);
    assert_eq!(watcher.watched_repo_count(), 1);
    assert_eq!(
        watcher.client_pids().into_iter().collect::<HashSet<_>>(),
        HashSet::from([3])
    );
}
