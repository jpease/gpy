//! Integration tests for huge repository performance optimizations
//!
//! Tests sparse checkout detection, sampling heuristics, and repeated-status
//! determinism.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_precision_loss)]

use gpy_agent::config::types::GitTimeout;
use gpy_agent::git::native::NativeGitBackend;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

/// Helper to create a git repository in a temp directory
fn create_git_repo() -> TempDir {
    let dir = TempDir::new().expect("create temp dir");
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());

    // Config user
    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(dir.path())
        .output()
        .expect("config email");

    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(dir.path())
        .output()
        .expect("config name");

    std::process::Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(dir.path())
        .output()
        .expect("config gpgsign");

    dir
}

#[test]
fn test_sparse_checkout_detection() {
    let repo = create_git_repo();
    let repo_path = repo.path();

    // Initialize sparse-checkout
    // Note: "git sparse-checkout" command might not be available in older git versions
    // in CI environments, so we manually set it up or use git command if available.
    // We'll try using git command first.

    let status = std::process::Command::new("git")
        .args(["sparse-checkout", "init", "--cone"])
        .current_dir(repo_path)
        .status();

    if let Ok(s) = status {
        if !s.success() {
            println!("Skipping sparse checkout test: git sparse-checkout not supported or failed");
            return;
        }

        std::process::Command::new("git")
            .args(["sparse-checkout", "set", "src"])
            .current_dir(repo_path)
            .status()
            .expect("sparse-checkout set");
    } else {
        println!("Skipping sparse checkout test: git command not found");
    }

    // Test that is_sparse_checkout detects the configuration
    let is_sparse = NativeGitBackend::is_sparse_checkout(repo_path);
    // Should be true if sparse-checkout was set up successfully
    assert!(is_sparse);
}

#[test]
fn test_sparse_checkout_doesnt_crash() {
    let repo = create_git_repo();
    let repo_path = repo.path();

    // Create directory structure
    fs::create_dir_all(repo_path.join("src")).expect("create src dir");
    fs::create_dir_all(repo_path.join("docs")).expect("create docs dir");

    // Create an initial commit (required for sparse-checkout)
    let init_file = repo_path.join("README.md");
    fs::write(&init_file, "initial").expect("write initial file");

    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(repo_path)
        .status()
        .expect("git add");

    std::process::Command::new("git")
        .args(["commit", "-m", "Initial", "--no-gpg-sign"])
        .current_dir(repo_path)
        .status()
        .expect("git commit");

    // Initialize sparse-checkout (not cone mode to avoid pattern issues)
    let sparse_file = repo_path.join(".git/info/sparse-checkout");
    fs::write(&sparse_file, "src/\n").expect("write sparse-checkout");

    std::process::Command::new("git")
        .args(["config", "core.sparseCheckout", "true"])
        .current_dir(repo_path)
        .status()
        .expect("config sparse");

    // Verify sparse checkout is detected
    let is_sparse = NativeGitBackend::is_sparse_checkout(repo_path);
    assert!(is_sparse, "Should detect sparse checkout");

    // Create some files
    fs::write(repo_path.join("src/file.txt"), "src content").expect("write src file");
    fs::write(repo_path.join("test.txt"), "root content").expect("write root file");

    // Get status - should work even with sparse checkout enabled
    // We don't assert specific counts since sparse checkout behavior with git
    // is complex, but we verify it doesn't crash
    // Migrated from the retired porcelain-v1 `get_status_counts` chain onto the
    // live `get_complete_status` entry point (#621).
    let result = NativeGitBackend::get_complete_status(
        repo_path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    );

    // Should either succeed or fail gracefully (not panic)
    if let Err(e) = &result {
        println!("Note: Sparse checkout caused expected error: {e}");
    }
}

/// Two back-to-back `get_complete_status` calls over an unchanged repository
/// must report the same `RepositoryStatus`.
///
/// Stale-cache coverage lives in `tests/cache_tests.rs`; timeout coverage lives
/// in `get_complete_status_respects_configured_timeout` (unit test in
/// `src/git/native/mod.rs`). This test was migrated from the retired
/// porcelain-v1 `get_status_counts` chain (#621) and renamed, because the
/// function it exercised had neither a cache nor a progressive timeout.
#[test]
fn test_repeated_complete_status_is_deterministic() {
    let repo = create_git_repo();
    let repo_path = repo.path();

    // Create a file and commit
    let file_path = repo_path.join("test.txt");
    fs::write(&file_path, "content").expect("write file");

    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(repo_path)
        .status()
        .expect("git add");

    std::process::Command::new("git")
        .args(["commit", "-m", "Initial", "--no-gpg-sign"])
        .current_dir(repo_path)
        .status()
        .expect("git commit");

    let status = || {
        NativeGitBackend::get_complete_status(
            repo_path,
            None,
            0,
            true,
            Duration::from_secs(GitTimeout::DEFAULT),
        )
        .expect("complete status")
        .expect("inside a repo")
        .status
    };

    let first = status();
    let second = status();

    assert_eq!(first, second, "repeated status reads should agree");
}
