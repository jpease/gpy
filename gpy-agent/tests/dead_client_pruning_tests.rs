//! `Agent::prune_dead_clients_and_cleanup` removes a dead client from both the
//! `ClientDirectory` and the `MultiRepoWatcher`, and leaves live clients alone.
//!
//! The tests call the agent's own pruning method (the one its timer runs), so
//! dropping the watcher cleanup from it fails them.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

#[path = "common/git.rs"]
mod git;

use gpy_agent::agent::Agent;
use tempfile::tempdir;

#[test]
fn pruning_dead_client_unregisters_it_from_the_watcher() {
    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();
    git::git_init(repo_path);

    let agent =
        Agent::new_for_testing_with_watcher(repo_path).expect("Failed to create test agent");

    // A child that has exited and been reaped: its PID is no longer a live process.
    let mut child = std::process::Command::new("true")
        .spawn()
        .expect("spawn `true`");
    let dead_pid = child.id();
    child.wait().expect("reap `true`");
    agent.register_test_client(dead_pid, repo_path);
    assert_eq!(agent.client_count(), 1, "Should have 1 client registered");
    assert_eq!(
        agent.watcher_repo_count(),
        1,
        "Watcher should be tracking 1 repository"
    );

    agent.prune_dead_clients_and_cleanup();

    assert_eq!(
        agent.client_count(),
        0,
        "ClientDirectory should be empty after pruning"
    );
    assert_eq!(
        agent.watcher_repo_count(),
        0,
        "pruning must also unregister the dead client from the watcher"
    );
}

#[test]
fn pruning_keeps_live_clients_and_their_watches() {
    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();
    git::git_init(repo_path);

    let agent =
        Agent::new_for_testing_with_watcher(repo_path).expect("Failed to create test agent");

    agent.register_test_client(std::process::id(), repo_path);
    assert_eq!(agent.client_count(), 1, "Should have 1 client");
    assert_eq!(agent.watcher_repo_count(), 1, "Should watch 1 repo");

    agent.prune_dead_clients_and_cleanup();

    assert_eq!(agent.client_count(), 1, "live client must not be pruned");
    assert_eq!(
        agent.watcher_repo_count(),
        1,
        "live client's repository must stay watched"
    );
}
