//! Unit tests for `GitStatusCache`

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::git::{RepositoryState, RepositoryStatus};
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn test_cache_new() {
    let cache = GitStatusCache::new();
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
}

#[test]
fn test_cache_set_and_get() {
    let cache = GitStatusCache::new();
    let repo_path = PathBuf::from("/tmp/test-repo");

    let status = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    // Cache miss initially
    assert!(cache.get(&repo_path).is_none());
    assert_eq!(cache.len(), 0);

    // Set cache entry
    cache.set(&repo_path, status, HashMap::new());
    assert_eq!(cache.len(), 1);
    assert!(!cache.is_empty());

    // Cache hit
    let cached = cache.get(&repo_path).expect("Should have cached entry");
    assert_eq!(cached.branch, "main");
    assert_eq!(cached.state, RepositoryState::Clean);
}

#[test]
fn test_cache_invalidate() {
    let cache = GitStatusCache::new();
    let repo_path = PathBuf::from("/tmp/test-repo");

    let status = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 1,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 2,
        unstaged: 3,
        untracked: 4,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    // Set and verify
    cache.set(&repo_path, status, HashMap::new());
    assert_eq!(cache.len(), 1);
    assert!(cache.get(&repo_path).is_some());

    // Invalidate
    cache.invalidate(&repo_path);
    assert_eq!(cache.len(), 0);
    assert!(cache.get(&repo_path).is_none());
}

#[test]
fn test_cache_multiple_repos() {
    let cache = GitStatusCache::new();

    let repo1 = PathBuf::from("/tmp/repo1");
    let repo2 = PathBuf::from("/tmp/repo2");
    let repo3 = PathBuf::from("/tmp/repo3");

    let status1 = dummy_status();

    let status2 = RepositoryStatus {
        branch: "develop".to_owned(),
        ahead: 2,
        behind: 1,
        unstaged: 5,
        ..dummy_status()
    };

    let status3 = RepositoryStatus {
        branch: "feature".to_owned(),
        staged: 1,
        untracked: 3,
        conflicts: 2,
        state: RepositoryState::Merging,
        ..dummy_status()
    };

    // Cache all three
    cache.set(&repo1, status1, HashMap::new());
    cache.set(&repo2, status2, HashMap::new());
    cache.set(&repo3, status3, HashMap::new());

    assert_eq!(cache.len(), 3);

    // Verify each can be retrieved independently
    assert_eq!(cache.get(&repo1).unwrap().branch, "main");
    assert_eq!(cache.get(&repo2).unwrap().branch, "develop");
    assert_eq!(cache.get(&repo3).unwrap().branch, "feature");

    // Invalidate one
    cache.invalidate(&repo2);
    assert_eq!(cache.len(), 2);
    assert!(cache.get(&repo1).is_some());
    assert!(cache.get(&repo2).is_none());
    assert!(cache.get(&repo3).is_some());
}

#[test]
fn test_cache_clear() {
    let cache = GitStatusCache::new();

    let status = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    // Add multiple entries
    cache.set(&PathBuf::from("/tmp/repo1"), status.clone(), HashMap::new());
    cache.set(&PathBuf::from("/tmp/repo2"), status.clone(), HashMap::new());
    cache.set(&PathBuf::from("/tmp/repo3"), status, HashMap::new());

    assert_eq!(cache.len(), 3);

    // Clear all
    cache.clear();
    assert_eq!(cache.len(), 0);
    assert!(cache.is_empty());
}

#[test]
fn test_cache_update_existing() {
    let cache = GitStatusCache::new();
    let repo_path = PathBuf::from("/tmp/test-repo");

    let status_v1 = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    let status_v2 = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 1,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 2,
        unstaged: 3,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Dirty,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    // Set initial
    cache.set(&repo_path, status_v1, HashMap::new());
    assert_eq!(cache.get(&repo_path).unwrap().ahead, 0);
    assert_eq!(cache.len(), 1);

    // Update with new status
    cache.set(&repo_path, status_v2, HashMap::new());
    assert_eq!(cache.get(&repo_path).unwrap().ahead, 1);
    assert_eq!(cache.get(&repo_path).unwrap().staged, 2);
    assert_eq!(cache.len(), 1); // Still only one entry
}

#[test]
fn test_cache_thread_safety() {
    use std::sync::Arc;
    use std::thread;

    let cache = Arc::new(GitStatusCache::new());
    let mut handles = vec![];

    // Spawn multiple threads that all try to access the cache
    for i in 0..10 {
        let cache_clone = Arc::clone(&cache);
        let handle = thread::spawn(move || {
            let repo_path = PathBuf::from(format!("/tmp/repo-{i}"));
            let status = RepositoryStatus {
                branch: format!("branch-{i}"),
                ahead: i,
                behind: 0,
                ahead_capped: false,
                behind_capped: false,
                staged: 0,
                unstaged: 0,
                untracked: 0,
                conflicts: 0,
                state: RepositoryState::Clean,
                stash_count: 0,
                detached: false,
                rebase_progress: None,
            };

            cache_clone.set(&repo_path, status, HashMap::new());
            assert!(cache_clone.get(&repo_path).is_some());
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }

    // All entries should be present
    assert_eq!(cache.len(), 10);
}

#[cfg(unix)]
#[test]
fn test_cache_symlink_keys_share_entry() {
    let cache = GitStatusCache::new();
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let repo_path = temp_dir.path();

    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .status()
            .expect("git command should execute")
    };

    assert!(run(&["init"]).success());
    assert!(run(&["config", "user.email", "gpy@example.com"]).success());
    assert!(run(&["config", "user.name", "GPY Test"]).success());

    let repo_root = gpy_agent::git::find_repo_root(repo_path).expect("repo root");
    let symlink_path = repo_path.join("repo-link");
    if symlink_path.exists() {
        let _ =
            std::fs::remove_file(&symlink_path).or_else(|_| std::fs::remove_dir_all(&symlink_path));
    }
    symlink(&repo_root, &symlink_path).expect("create symlink");

    let status = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    cache.set(&repo_root, status, HashMap::new());
    assert!(cache.get(&repo_root).is_some());
    assert!(cache.get(&symlink_path).is_some());
}

fn dummy_status() -> RepositoryStatus {
    RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    }
}

/// A pre-canonicalized key round-trips through the `*_canonical` fast paths.
///
/// No filesystem access is involved: the key here is a non-existent path, so any
/// hidden `fs::canonicalize` call would error and the entry could not be found
/// under the literal key — proving the fast path uses the key verbatim.
#[test]
fn canonical_fast_path_round_trips_without_filesystem() {
    let cache = GitStatusCache::new();
    let key = PathBuf::from("/nonexistent/canonical/repo");

    assert!(cache.get_canonical(&key).is_none());

    cache.set_canonical(&key, dummy_status(), HashMap::new());
    assert_eq!(cache.len(), 1);

    let hit = cache
        .get_canonical(&key)
        .expect("pre-canonical key should round-trip");
    assert_eq!(hit.branch, "main");
    assert!(cache.get_any_canonical(&key).is_some());
    assert!(cache.is_fresh_canonical(&key));

    cache.invalidate_canonical(&key);
    assert!(cache.get_canonical(&key).is_none());
    assert_eq!(cache.len(), 0);
}

/// The `*_canonical` fast paths must NOT resolve symlinks.
///
/// Unlike the canonicalizing public methods, an entry stored under the real repo
/// root is reachable via `get` through a symlink (which canonicalizes), but is a
/// miss via `get_canonical` on the symlink path (which does not).
#[cfg(unix)]
#[test]
fn canonical_fast_path_does_not_resolve_symlinks() {
    let cache = GitStatusCache::new();
    let Ok(temp_dir) = TempDir::new() else {
        return;
    };
    let repo_root = temp_dir
        .path()
        .canonicalize()
        .expect("temp dir should canonicalize");
    let symlink_path = repo_root.join("repo-link");
    symlink(&repo_root, &symlink_path).expect("create symlink");

    cache.set_canonical(&repo_root, dummy_status(), HashMap::new());

    // Canonicalizing path resolves the symlink to the stored key -> hit.
    assert!(cache.get(&symlink_path).is_some());
    // Fast path uses the symlink path verbatim -> miss (no second canonicalize).
    assert!(cache.get_canonical(&symlink_path).is_none());
    // Fast path with the actual canonical key -> hit.
    assert!(cache.get_canonical(&repo_root).is_some());
}
