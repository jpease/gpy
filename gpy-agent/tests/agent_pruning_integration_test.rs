//! Integration test for Agent's dead client pruning with watcher cleanup
//!
//! This test verifies that the Agent's `prune_dead_clients_and_cleanup` method
//! correctly cleans up both the `ClientDirectory` AND the `MultiRepoWatcher`.
//!
//! Unlike the unit tests in `dead_client_pruning_tests.rs`, this test actually
//! calls the Agent's pruning method rather than manually calling `unregister_client`.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::agent::Agent;
use tempfile::tempdir;

#[test]
#[cfg(unix)]
// Integration test: Verify `Agent::prune_dead_clients_and_cleanup()` cleans up watcher
//
// This is the TRUE regression test. If someone removes the watcher cleanup
// code from `Agent::prune_dead_clients_and_cleanup()`, this test will FAIL.
//
// The test does NOT manually call `watcher.unregister_client()` - it relies
// entirely on the Agent's implementation to do the right thing.
fn test_agent_pruning_cleans_up_watcher() {
    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();

    // Initialize git repo
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");

    // Create agent with watcher enabled
    let agent =
        Agent::new_for_testing_with_watcher(repo_path).expect("Failed to create test agent");

    // Register a fake dead PID (999999 definitely doesn't exist)
    let fake_pid = 999_999_u32;
    agent.register_test_client(fake_pid, repo_path);

    // Verify initial state
    assert_eq!(agent.client_count(), 1, "Should have 1 client registered");
    assert_eq!(
        agent.watcher_repo_count(),
        1,
        "Watcher should be tracking 1 repository"
    );

    // Call the Agent's pruning method (this is what the timer calls)
    // This should prune from ClientDirectory AND unregister from MultiRepoWatcher
    agent.prune_dead_clients_and_cleanup();

    // Verify BOTH registries were cleaned up by the Agent
    assert_eq!(
        agent.client_count(),
        0,
        "ClientDirectory should be empty after Agent pruning"
    );
    assert_eq!(
        agent.watcher_repo_count(),
        0,
        "CRITICAL: Watcher should be cleaned up by Agent.prune_dead_clients_and_cleanup()"
    );

    // If this assertion fails, it means the Agent forgot to call
    // watcher.unregister_client() in prune_dead_clients_and_cleanup()
}

#[test]
#[cfg(unix)]
// Verify that Agent pruning skips alive clients
fn test_agent_pruning_skips_alive_clients() {
    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();

    // Initialize git repo
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");

    // Create agent
    let agent =
        Agent::new_for_testing_with_watcher(repo_path).expect("Failed to create test agent");

    // Register the current process (which is alive)
    let current_pid = std::process::id();
    agent.register_test_client(current_pid, repo_path);

    // Verify initial state
    assert_eq!(agent.client_count(), 1, "Should have 1 client");
    assert_eq!(agent.watcher_repo_count(), 1, "Should watch 1 repo");

    // Attempt to prune
    agent.prune_dead_clients_and_cleanup();

    // Alive clients should NOT be pruned
    assert_eq!(
        agent.client_count(),
        1,
        "Should still have 1 client (alive)"
    );
    assert_eq!(
        agent.watcher_repo_count(),
        1,
        "Should still watch 1 repo (alive client)"
    );
}
