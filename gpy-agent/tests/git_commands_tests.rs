//! Tests for git/commands.rs
//!
//! Tests git command execution and status detection using test fixtures.

#![allow(clippy::missing_panics_doc)]

mod fixtures;

use fixtures::TestRepo;
use gpy_agent::git::status::load_repository_state;

#[test]
fn test_load_repository_state_with_clean_repo() {
    let repo = TestRepo::new().with_initial_commit();
    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(result.is_ok(), "Should successfully load clean repository");
    let status = result.unwrap().expect("Should find repository");
    assert_eq!(status.status.untracked, 0);
    assert_eq!(status.status.staged, 0);
    assert_eq!(status.status.unstaged, 0);
}

#[test]
fn test_load_repository_state_with_untracked_file() {
    let repo = TestRepo::new().with_initial_commit().with_untracked_file();

    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(result.is_ok(), "Should successfully load repository");
    let status = result.unwrap().expect("Should find repository");
    assert_eq!(status.status.untracked, 1, "Should detect 1 untracked file");
}

#[test]
fn test_load_repository_state_with_staged_file() {
    let repo = TestRepo::new()
        .with_initial_commit()
        .with_staged_file("new.txt", "content");

    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(result.is_ok(), "Should successfully load repository");
    let status = result.unwrap().expect("Should find repository");
    assert_eq!(status.status.staged, 1, "Should detect 1 staged file");
}

#[test]
fn test_load_repository_state_with_unstaged_changes() {
    let repo = TestRepo::new()
        .with_initial_commit()
        .with_unstaged_changes();

    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(result.is_ok(), "Should successfully load repository");
    let status = result.unwrap().expect("Should find repository");
    assert_eq!(
        status.status.unstaged, 1,
        "Should detect 1 file with unstaged changes"
    );
}

#[test]
fn test_load_repository_state_with_conflicts() {
    let repo = TestRepo::new().with_initial_commit().with_conflicts();

    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(
        result.is_ok(),
        "Should successfully load repository with conflicts"
    );
    let status = result.unwrap().expect("Should find repository");
    assert!(
        status.status.state.as_str().contains("merge")
            || status.status.state.as_str().contains("conflict"),
        "Should detect merge/conflict state"
    );
}

#[test]
fn test_load_repository_state_with_detached_head() {
    let repo = TestRepo::new().with_detached_head();

    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(
        result.is_ok(),
        "Should successfully load repository in detached HEAD state"
    );
    let status = result.unwrap().expect("Should find repository");
    // Detached HEAD is reported via the explicit `detached` flag; `branch` is
    // the bare short commit hash with no `HEAD@` prefix (#244).
    assert!(status.status.detached, "Should set the detached flag");
    assert!(
        !status.status.branch.starts_with("HEAD"),
        "branch must not carry the legacy HEAD@ prefix, got: {}",
        status.status.branch
    );
}

#[test]
fn test_load_repository_state_invalid_path() {
    let result = load_repository_state(std::path::Path::new("/nonexistent/path"), None, 0, true);

    // Should return Ok(None) for invalid paths or Err depending on implementation details of canonicalize
    // But load_repository_state calls find_repo_root which returns Ok(None) if discover fails.
    // However, if path doesn't exist, discover might error.
    // Let's accept either Ok(None) or Err, but repo discovery usually returns an error if the path is invalid.
    // Actually find_repo_root returns Ok(None) if discover fails.
    assert!(matches!(result, Ok(None) | Err(_)));
}

#[test]
fn test_load_repository_state_not_a_repo() {
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp directory");
    let result = load_repository_state(temp_dir.path(), None, 0, true);

    assert!(matches!(result, Ok(None)));
}

/// Test that git status correctly identifies mixed state (staged + unstaged)
#[test]
fn test_load_repository_state_mixed_changes() {
    let repo = TestRepo::new().with_initial_commit();

    // Add a staged file
    repo.create_file("staged.txt", "staged content")
        .stage_file("staged.txt");

    // Add an unstaged file (modify tracked file without staging)
    repo.create_file("README.md", "Modified content");

    // Add an untracked file
    repo.create_file("untracked.txt", "untracked content");

    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(result.is_ok(), "Should successfully load repository");
    let status = result.unwrap().expect("Should find repository");
    assert_eq!(status.status.staged, 1, "Should detect 1 staged file");
    assert_eq!(status.status.unstaged, 1, "Should detect 1 unstaged file");
    assert_eq!(status.status.untracked, 1, "Should detect 1 untracked file");
}
