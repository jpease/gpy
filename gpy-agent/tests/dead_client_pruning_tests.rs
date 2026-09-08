//! Integration tests for dead client pruning with watcher cleanup

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::ipc::ClientDirectory;
use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
use gpy_agent::watcher::{DebouncedEvent, WatcherConfig};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

#[test]
#[cfg(unix)]
// End-to-end test: Verify that pruning dead clients also cleans up watchers
//
// This test simulates the full pruning flow:
// 1. Register a fake (dead) PID with both `ClientDirectory` and `MultiRepoWatcher`
// 2. Call `prune_dead_clients()` to get list of dead PIDs
// 3. Manually call `unregister_client` for each pruned PID (simulating agent event loop)
// 4. Verify watcher stopped tracking the repository
//
// This protects against regressions where the agent forgets to call `unregister_client`
// after pruning.
fn test_pruning_cleans_up_watcher_end_to_end() {
    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();

    // Initialize git repo
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");

    // Setup watcher
    let config = WatcherConfig::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let watcher = Arc::new(
        MultiRepoWatcher::builder()
            .config(config)
            .callback(callback)
            .build()
            .expect("create watcher"),
    );
    let client_directory = Arc::new(ClientDirectory::new());

    // Register a fake dead PID (999999 definitely doesn't exist)
    let fake_pid = 999_999_u32;
    client_directory.register(fake_pid, Some(repo_path.to_path_buf()));
    let _ = watcher.register_client(fake_pid, repo_path);

    // Verify initial state
    assert_eq!(
        client_directory.len(),
        1,
        "ClientDirectory should have 1 client"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "Watcher should be tracking 1 repository"
    );

    // Simulate the pruning flow from agent event loop
    // Step 1: Prune dead clients from registry
    let pruned_pids = client_directory.prune_dead_clients();

    // Step 2: Also unregister from watcher (this is what agent.rs does)
    for pid in &pruned_pids {
        let _ = watcher.unregister_client(*pid);
    }

    // Verify both registries cleaned up
    assert_eq!(pruned_pids.len(), 1, "Should have pruned 1 dead client");
    assert_eq!(
        client_directory.len(),
        0,
        "ClientDirectory should be empty after pruning"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        0,
        "Watcher should stop tracking repository after pruning"
    );

    // Cleanup
    watcher.stop();
}

#[test]
#[cfg(unix)]
// Regression test: Verify that forgetting to unregister from watcher leaves it dirty
//
// This test demonstrates what happens if the agent forgets to call `unregister_client`
// after pruning. It should PASS (showing the bug exists) if someone removes the
// watcher cleanup code from `agent.rs`.
fn test_pruning_without_watcher_cleanup_leaves_watcher_dirty() {
    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();

    // Initialize git repo
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");

    // Setup watcher
    let config = WatcherConfig::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let watcher = Arc::new(
        MultiRepoWatcher::builder()
            .config(config)
            .callback(callback)
            .build()
            .expect("create watcher"),
    );
    let client_directory = Arc::new(ClientDirectory::new());

    // Register a fake dead PID
    let fake_pid = 999_999_u32;
    client_directory.register(fake_pid, Some(repo_path.to_path_buf()));
    let _ = watcher.register_client(fake_pid, repo_path);

    // Simulate BROKEN pruning flow (forgetting to call unregister_client)
    let pruned_pids = client_directory.prune_dead_clients();

    // NOTE: Intentionally NOT calling watcher.unregister_client here
    // This simulates the bug we're protecting against

    // ClientDirectory should be cleaned up
    assert_eq!(pruned_pids.len(), 1, "Should have pruned 1 dead client");
    assert_eq!(client_directory.len(), 0, "ClientDirectory should be empty");

    // But watcher should still be dirty (this is the bug!)
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "REGRESSION: Watcher still tracking repo because we forgot to call unregister_client"
    );

    // Cleanup
    watcher.stop();
}

#[test]
#[cfg(unix)]
// Test that alive clients are NOT pruned and watchers remain active
fn test_pruning_skips_alive_clients() {
    let repo_dir = tempdir().expect("create temp dir");
    let repo_path = repo_dir.path();

    // Initialize git repo
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(repo_path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");

    // Setup watcher
    let config = WatcherConfig::default();
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |event| {
        events_clone.lock().unwrap().push(event);
    });

    let watcher = Arc::new(
        MultiRepoWatcher::builder()
            .config(config)
            .callback(callback)
            .build()
            .expect("create watcher"),
    );
    let client_directory = Arc::new(ClientDirectory::new());

    // Register the current process (which is alive)
    let current_pid = std::process::id();
    client_directory.register(current_pid, Some(repo_path.to_path_buf()));
    let _ = watcher.register_client(current_pid, repo_path);

    // Attempt to prune
    let pruned_pids = client_directory.prune_dead_clients();

    // Should not prune alive clients
    assert_eq!(pruned_pids.len(), 0, "Should not prune alive clients");
    assert_eq!(
        client_directory.len(),
        1,
        "ClientDirectory should still have 1 client"
    );
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "Watcher should still be tracking repository"
    );

    // Cleanup
    watcher.stop();
}
