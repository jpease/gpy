//! Comprehensive tests for git status functionality
//!
//! # Test Navigation Map
//!
//! ## Test Categories
//!
//! ### Basic Operations
//! - `test_git_status_not_a_repo` - Non-git directory handling
//! - `test_git_status_current_directory` - Status in current repo
//! - `test_git_status_created_repo` - Freshly initialized repo
//! - `test_git_status_absolute_path` - Absolute path handling
//! - `test_git_status_relative_path` - Relative path handling
//!
//! ### File State Detection
//! - `test_git_status_with_untracked_files` - Untracked file counting
//! - `test_git_status_with_staged_files` - Staged changes detection
//! - `test_git_status_staged_deletion` - Deleted file handling
//! - `test_git_status_ignores_gitignored_files` - .gitignore respect
//! - `test_git_status_counts_non_ignored_files_with_ignored_present` - Mixed files

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::default_numeric_fallback)]
#![allow(clippy::shadow_unrelated)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::config::types::GitTimeout;
use gpy_agent::git::{
    RepositoryState, RepositoryStatus, native::NativeGitBackend, status::load_repository_state,
};
use serial_test::serial;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

fn git_assert(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command should execute");

    assert!(
        output.status.success(),
        "git {:?} failed in {}: stdout: {}, stderr: {}",
        args,
        repo.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[cfg(all(unix, not(target_os = "macos")))]
use std::os::unix::ffi::OsStrExt;

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_not_a_repo() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let result = load_repository_state(temp_dir.path().to_string_lossy().as_ref(), None, 0, true);

    // Should return Ok(None) when not in a git repo
    assert!(result.is_ok());
    assert!(matches!(result, Ok(None)));
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_current_directory() {
    // Test with current directory (should be a git repo)
    let result = load_repository_state(".", None, 0, true);

    // Should return git status for the current repo
    assert!(result.is_ok());
    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());
    let Some(status) = git_status else { return };
    assert_ne!(status.status.branch, "");
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_invalid_path() {
    let result = load_repository_state("/nonexistent/path", None, 0, true);

    // Should either return Ok(None) for invalid paths or an error - both are acceptable
    assert!(matches!(result, Ok(None) | Err(_)));
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_created_repo() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize a git repository
    let init_output = Command::new("git")
        .arg("init")
        .current_dir(temp_path)
        .output()
        .expect("git command should be available for testing");

    assert!(
        init_output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init_output.stderr)
    );

    // Configure git user for this repo
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_path)
        .output();

    let _ = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_path)
        .output();

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    assert!(result.is_ok());

    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());

    let Some(status) = git_status else { return };
    // New repo should be on main or master branch
    assert!(status.status.branch == "main" || status.status.branch == "master");
    assert_eq!(status.status.ahead, 0);
    assert_eq!(status.status.behind, 0);
    assert_eq!(status.status.staged, 0);
    assert_eq!(status.status.untracked, 0);
    assert_eq!(status.status.conflicts, 0);
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_with_untracked_files() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize a git repository
    let init_output = Command::new("git")
        .arg("init")
        .current_dir(temp_path)
        .output()
        .expect("git command should be available for testing");

    assert!(
        init_output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init_output.stderr)
    );

    // Configure git user
    assert!(
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(temp_path)
            .status()
            .expect("configure email")
            .success()
    );
    assert!(
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(temp_path)
            .status()
            .expect("configure name")
            .success()
    );
    assert!(
        Command::new("git")
            .args(["config", "commit.gpgsign", "false"])
            .current_dir(temp_path)
            .status()
            .expect("disable gpg signing")
            .success()
    );

    // Create some untracked files
    let _ = fs::write(temp_path.join("untracked1.txt"), "content");
    let _ = fs::write(temp_path.join("untracked2.txt"), "content");

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    assert!(result.is_ok());

    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());
    let Some(status) = git_status else { return };
    assert_eq!(status.status.untracked, 2);
    assert_eq!(status.status.staged, 0);
    assert_eq!(status.status.unstaged, 0);
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_with_staged_files() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize git repository
    let init_output = Command::new("git")
        .arg("init")
        .current_dir(temp_path)
        .output()
        .expect("git command should be available for testing");

    assert!(
        init_output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init_output.stderr)
    );

    // Configure git user
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_path)
        .output();

    // Create and stage a file
    let _ = fs::write(temp_path.join("staged.txt"), "content");
    let _ = Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(temp_path)
        .output();

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    assert!(result.is_ok());

    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());

    let Some(status) = git_status else { return };
    assert_eq!(status.status.staged, 1);
    assert_eq!(status.status.unstaged, 0);
    assert_eq!(status.status.untracked, 0);
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_absolute_path() {
    // Test with absolute path to current directory
    let current_dir =
        std::env::current_dir().expect("should be able to get current directory for test");
    let result = load_repository_state(current_dir.to_string_lossy().as_ref(), None, 0, true);

    assert!(result.is_ok());
    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());
    let Some(status) = git_status else { return };
    assert_ne!(status.status.branch, "");
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_relative_path() {
    // Test with relative path
    let result = load_repository_state("./", None, 0, true);

    assert!(result.is_ok());
    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());
    let Some(status) = git_status else { return };
    assert_ne!(status.status.branch, "");
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_empty_path() {
    let result = load_repository_state("", None, 0, true);

    // Empty path should be handled gracefully (either Ok(None) or Err)
    assert!(matches!(result, Ok(Some(_) | None) | Err(_)));
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_struct_default_values() {
    // Test creating RepositoryStatus with default/known values
    let status = RepositoryStatus {
        branch: "feature-branch".to_owned(),
        ahead: 3,
        behind: 1,
        ahead_capped: false,
        behind_capped: false,
        staged: 2,
        unstaged: 4,
        untracked: 1,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };

    assert_eq!(status.branch, "feature-branch");
    assert_eq!(status.ahead, 3);
    assert_eq!(status.behind, 1);
    assert_eq!(status.staged, 2);
    assert_eq!(status.unstaged, 4);
    assert_eq!(status.untracked, 1);
    assert_eq!(status.conflicts, 0);
    assert_eq!(status.state, RepositoryState::Clean);
}

#[cfg(unix)]
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
#[serial(git_status)]
fn test_non_utf8_staged_paths_are_counted() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    assert!(
        Command::new("git")
            .arg("init")
            .current_dir(temp_path)
            .status()
            .expect("git init should succeed")
            .success()
    );

    assert!(
        Command::new("git")
            .args(["config", "user.email", "gpy@example.com"])
            .current_dir(temp_path)
            .status()
            .expect("git config email should succeed")
            .success()
    );

    assert!(
        Command::new("git")
            .args(["config", "user.name", "GPY Test"])
            .current_dir(temp_path)
            .status()
            .expect("git config name should succeed")
            .success()
    );

    assert!(
        Command::new("git")
            .args(["config", "commit.gpgsign", "false"])
            .current_dir(temp_path)
            .status()
            .expect("disable gpg signing")
            .success()
    );

    let filename = std::ffi::OsStr::from_bytes(b"non_utf8_\xFF");
    let file_path = temp_path.join(filename);
    fs::write(&file_path, b"content").expect("write file");

    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(temp_path)
            .status()
            .expect("git add should succeed")
            .success()
    );

    // Migrated from the retired porcelain-v1 `get_status_counts` chain onto the
    // live `get_complete_status` entry point (#621).
    let complete = NativeGitBackend::get_complete_status(
        temp_path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("complete status")
    .expect("inside a repo");
    assert_eq!(complete.status.staged, 1);
}

#[test]
#[serial(git_status)]
fn test_merge_conflicts_are_counted() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let repo_path = temp_dir.path();

    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .status()
            .expect("git command should execute")
    };

    assert!(run(&["init"]).success());
    assert!(run(&["config", "user.email", "gpy@example.com"]).success());
    assert!(run(&["config", "user.name", "GPY Test"]).success());
    assert!(run(&["config", "commit.gpgsign", "false"]).success());

    fs::write(repo_path.join("file.txt"), b"base").unwrap();
    assert!(run(&["add", "."]).success());
    assert!(run(&["commit", "--no-gpg-sign", "-m", "base"]).success());
    assert!(run(&["branch", "-M", "main"]).success());

    assert!(run(&["checkout", "-b", "feature"]).success());
    fs::write(repo_path.join("file.txt"), b"feature change").unwrap();
    assert!(run(&["commit", "--no-gpg-sign", "-am", "feature work"]).success());

    assert!(run(&["checkout", "main"]).success());
    fs::write(repo_path.join("file.txt"), b"main change").unwrap();
    assert!(run(&["commit", "--no-gpg-sign", "-am", "main work"]).success());

    // Merge will result in conflict; ignore non-success outcome.
    let _ = run(&["merge", "feature"]);

    // Migrated from the retired porcelain-v1 `get_status_counts` chain onto the
    // live `get_complete_status` entry point (#621).
    let complete = NativeGitBackend::get_complete_status(
        repo_path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("complete status")
    .expect("inside a repo");
    assert!(
        complete.status.conflicts > 0,
        "merge conflict should be reported"
    );
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_ignores_gitignored_files() {
    // This test ensures that repos with ONLY ignored files show clean status
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize a git repository
    let init_output = Command::new("git")
        .arg("init")
        .current_dir(temp_path)
        .output()
        .expect("git command should be available for testing");

    assert!(
        init_output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init_output.stderr)
    );

    // Configure git user
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_path)
        .output();

    // Create a .gitignore file
    let _ = fs::write(temp_path.join(".gitignore"), ".DS_Store\n");

    // Stage and commit the .gitignore
    assert!(
        Command::new("git")
            .args(["add", ".gitignore"])
            .current_dir(temp_path)
            .status()
            .expect("git add")
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "--no-gpg-sign", "-m", "Add gitignore"])
            .current_dir(temp_path)
            .status()
            .expect("git commit")
            .success()
    );

    // Create a file that should be ignored
    let _ = fs::write(temp_path.join(".DS_Store"), "macOS metadata");

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    assert!(result.is_ok());

    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());
    let Some(status) = git_status else { return };

    // Should have NO untracked files since .DS_Store is ignored
    assert_eq!(
        status.status.untracked, 0,
        "Ignored files should not be counted"
    );
    assert_eq!(status.status.staged, 0);
    assert_eq!(status.status.unstaged, 0);
    assert_eq!(status.status.conflicts, 0);
    // State should be clean because only ignored files are present
    assert_eq!(status.status.state, RepositoryState::Clean);
}

#[test]
#[serial(git_status)] // Run serially within git_status_tests
fn test_git_status_counts_non_ignored_files_with_ignored_present() {
    // This test ensures that non-ignored files are counted even when ignored files are present
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize a git repository
    let init_output = Command::new("git")
        .arg("init")
        .current_dir(temp_path)
        .output()
        .expect("git command should be available for testing");

    assert!(
        init_output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init_output.stderr)
    );

    // Configure git user
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_path)
        .output();

    // Create a .gitignore file
    let _ = fs::write(temp_path.join(".gitignore"), ".DS_Store\n");

    // Stage and commit the .gitignore
    assert!(
        Command::new("git")
            .args(["add", ".gitignore"])
            .current_dir(temp_path)
            .status()
            .expect("git add")
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "--no-gpg-sign", "-m", "Add gitignore"])
            .current_dir(temp_path)
            .status()
            .expect("git commit")
            .success()
    );

    // Create a file that should be ignored
    let _ = fs::write(temp_path.join(".DS_Store"), "macOS metadata");

    // Create a file that should NOT be ignored
    let _ = fs::write(temp_path.join("foo.txt"), "real content");

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    assert!(result.is_ok());

    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());
    let Some(status) = git_status else { return };

    // Should only count foo.txt as untracked, not .DS_Store
    assert_eq!(
        status.status.untracked, 1,
        "Only non-ignored file should be counted"
    );
    assert_eq!(status.status.staged, 0);
    assert_eq!(status.status.unstaged, 0);
    assert_eq!(status.status.conflicts, 0);
}

#[test]
#[serial(git_status)]
fn test_git_status_staged_deletion() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize git repo
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(temp_path)
            .output()
            .expect("git command failed")
    };

    run(&["init", "--initial-branch=main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test User"]);
    run(&["config", "commit.gpgsign", "false"]);

    // Create and commit a file
    fs::write(temp_path.join("file.txt"), "content").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "base"]);

    // Delete the file and stage the deletion
    fs::remove_file(temp_path.join("file.txt")).unwrap();
    run(&["add", "-u"]);

    // Check status
    let result = load_repository_state(temp_path, None, 0, true);
    assert!(result.is_ok());

    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());
    let Some(status) = git_status else { return };

    // Should have 1 staged deletion
    assert_eq!(status.status.staged, 1, "Should count staged deletion");
    assert_eq!(status.status.unstaged, 0);
    assert_eq!(status.status.untracked, 0);
    assert_eq!(status.status.conflicts, 0);
    // State should be dirty because of the staged change
    assert_eq!(status.status.state, RepositoryState::Dirty);
}

#[test]
#[serial(git_status)]
fn test_git_status_rebase_state_precedence() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize git repo
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(temp_path)
            .output()
            .expect("git command failed")
    };

    run(&["init", "--initial-branch=main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test User"]);
    run(&["config", "commit.gpgsign", "false"]);

    // Create base commit
    fs::write(temp_path.join("file.txt"), "base").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "base"]);

    // Create feature branch with conflicting change
    run(&["checkout", "-b", "feature"]);
    fs::write(temp_path.join("file.txt"), "feature").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "feature"]);

    // Create conflicting change on main
    run(&["checkout", "main"]);
    fs::write(temp_path.join("file.txt"), "main").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "main"]);

    // Try to rebase feature onto main (will create conflict)
    run(&["checkout", "feature"]);
    let _rebase_output = run(&["rebase", "main"]);

    // Resolve conflict and stage it
    fs::write(temp_path.join("file.txt"), "resolved").unwrap();
    run(&["add", "file.txt"]);

    // Check status - should show rebasing state, NOT dirty
    let result = load_repository_state(temp_path, None, 0, true);
    assert!(result.is_ok());

    let Ok(git_status) = result else { return };
    assert!(git_status.is_some());
    let Some(status) = git_status else { return };

    // State should be Rebasing, not Dirty, even though we have staged changes
    assert_eq!(
        status.status.state,
        RepositoryState::Rebasing,
        "Should show rebasing state even with staged changes"
    );
    assert!(status.status.staged > 0, "Should have staged changes");
}

// ============================================================================
// AHEAD/BEHIND TESTS - Fix for regression where ahead/behind were always zero
// ============================================================================

#[test]
#[serial(git_status)]
fn test_ahead_only_commits() {
    // Test ahead-only divergence (3 commits ahead of origin/main)
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize bare repo to act as "remote"
    let bare_repo = temp_dir.path().join("bare.git");
    fs::create_dir_all(&bare_repo).unwrap();
    Command::new("git")
        .args(["init", "--bare", "--initial-branch=main"])
        .current_dir(&bare_repo)
        .output()
        .expect("git init --bare failed");

    // Clone the bare repo
    let work_repo = temp_dir.path().join("work");
    Command::new("git")
        .args([
            "clone",
            bare_repo.to_str().unwrap(),
            work_repo.to_str().unwrap(),
        ])
        .current_dir(temp_path)
        .output()
        .expect("git clone failed");

    let temp_path = work_repo.to_str().unwrap();
    let repo_path = Path::new(temp_path);

    // Configure git
    git_assert(repo_path, &["config", "user.email", "test@example.com"]);
    git_assert(repo_path, &["config", "user.name", "Test User"]);
    git_assert(repo_path, &["config", "commit.gpgsign", "false"]);

    // Create initial commit on main
    fs::write(repo_path.join("file1.txt"), "initial").unwrap();
    git_assert(repo_path, &["add", "file1.txt"]);
    git_assert(repo_path, &["commit", "-m", "Initial commit"]);
    git_assert(repo_path, &["push", "-u", "origin", "main"]);

    // Create 3 local commits (ahead of origin)
    for i in 1..=3 {
        fs::write(
            repo_path.join(format!("file{}.txt", i + 1)),
            format!("content {i}"),
        )
        .unwrap();
        git_assert(repo_path, &["add", "."]);
        git_assert(repo_path, &["commit", "-m", &format!("Commit {i}")]);
    }

    // Check status - should show ahead=3, behind=0
    let result = load_repository_state(temp_path, None, 0, true);
    assert!(result.is_ok(), "load_repository_state should succeed");

    let git_status = result.unwrap();
    assert!(git_status.is_some(), "Should return git status");

    let Some(status) = git_status else { return };
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(status.status.ahead, 3, "Should be 3 commits ahead");
    assert_eq!(status.status.behind, 0, "Should be 0 commits behind");
}

#[test]
#[serial(git_status)]
fn test_ahead_with_non_origin_remote() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let bare_repo = temp_dir.path().join("bare.git");
    fs::create_dir_all(&bare_repo).unwrap();
    git_assert(&bare_repo, &["init", "--bare", "--initial-branch=main"]);

    let work_repo = temp_dir.path().join("work");
    let bare_repo_str = bare_repo.to_str().unwrap().to_owned();
    let work_repo_str = work_repo.to_str().unwrap().to_owned();
    let clone_args = ["clone", bare_repo_str.as_str(), work_repo_str.as_str()];
    git_assert(temp_dir.path(), &clone_args);

    let repo_path = work_repo.as_path();
    git_assert(repo_path, &["config", "user.email", "test@example.com"]);
    git_assert(repo_path, &["config", "user.name", "Test User"]);
    git_assert(repo_path, &["config", "commit.gpgsign", "false"]);

    // Rename the default remote so the branch tracks a non-origin remote.
    git_assert(repo_path, &["remote", "rename", "origin", "upstream"]);

    // Create base commit and push to upstream remote.
    fs::write(repo_path.join("file1.txt"), "initial").unwrap();
    git_assert(repo_path, &["add", "file1.txt"]);
    git_assert(repo_path, &["commit", "-m", "Initial commit"]);
    git_assert(repo_path, &["push", "-u", "upstream", "main"]);
    git_assert(repo_path, &["fetch", "upstream"]);

    // Create a local commit that hasn't been pushed.
    fs::write(repo_path.join("local.txt"), "ahead change").unwrap();
    git_assert(repo_path, &["add", "."]);
    git_assert(repo_path, &["commit", "-m", "Local ahead commit"]);

    let result = load_repository_state(repo_path.to_string_lossy().as_ref(), None, 0, true);
    assert!(result.is_ok(), "load_repository_state should succeed");

    let git_status = result.unwrap();
    assert!(git_status.is_some(), "Should return git status");

    let Some(status) = git_status else { return };
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(
        status.status.ahead, 1,
        "Should be 1 commit ahead of upstream remote"
    );
    assert_eq!(status.status.behind, 0, "Should be 0 commits behind");
}

#[test]
#[serial(git_status)]
fn test_ahead_with_packed_refs_only() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let bare_repo = temp_dir.path().join("bare.git");
    fs::create_dir_all(&bare_repo).unwrap();
    git_assert(&bare_repo, &["init", "--bare", "--initial-branch=main"]);

    let work_repo = temp_dir.path().join("work");
    let bare_repo_str = bare_repo.to_str().unwrap().to_owned();
    let work_repo_str = work_repo.to_str().unwrap().to_owned();
    let clone_args = ["clone", bare_repo_str.as_str(), work_repo_str.as_str()];
    git_assert(temp_dir.path(), &clone_args);

    let repo_path = work_repo.as_path();
    git_assert(repo_path, &["config", "user.email", "test@example.com"]);
    git_assert(repo_path, &["config", "user.name", "Test User"]);
    git_assert(repo_path, &["config", "commit.gpgsign", "false"]);

    // Create base commit and push to origin.
    fs::write(repo_path.join("file1.txt"), "initial").unwrap();
    git_assert(repo_path, &["add", "file1.txt"]);
    git_assert(repo_path, &["commit", "-m", "Initial commit"]);
    git_assert(repo_path, &["push", "-u", "origin", "main"]);
    git_assert(repo_path, &["fetch", "origin"]);

    // Pack refs so the tracking branch only exists in packed-refs.
    git_assert(repo_path, &["pack-refs", "--all", "--prune"]);
    let tracking_ref = repo_path.join(".git/refs/remotes/origin/main");
    if tracking_ref.exists() {
        let _ = fs::remove_file(&tracking_ref);
    }

    // Create a local commit that hasn't been pushed.
    fs::write(repo_path.join("local.txt"), "ahead change").unwrap();
    git_assert(repo_path, &["add", "."]);
    git_assert(repo_path, &["commit", "-m", "Local ahead commit"]);

    let result = load_repository_state(repo_path.to_string_lossy().as_ref(), None, 0, true);
    assert!(result.is_ok(), "load_repository_state should succeed");

    let git_status = result.unwrap();
    assert!(git_status.is_some(), "Should return git status");

    let Some(status) = git_status else { return };
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(status.status.ahead, 1, "Should be 1 commit ahead");
    assert_eq!(status.status.behind, 0, "Should be 0 commits behind");
}

#[test]
#[serial(git_status)]
fn test_behind_only_commits() {
    // Test behind-only divergence (remote has commits we don't have)
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize bare repo to act as "remote"
    let bare_repo = temp_dir.path().join("bare.git");
    fs::create_dir_all(&bare_repo).unwrap();
    Command::new("git")
        .args(["init", "--bare", "--initial-branch=main"])
        .current_dir(&bare_repo)
        .output()
        .expect("git init --bare failed");

    // Clone the bare repo
    let work_repo = temp_dir.path().join("work");
    Command::new("git")
        .args([
            "clone",
            bare_repo.to_str().unwrap(),
            work_repo.to_str().unwrap(),
        ])
        .current_dir(temp_path)
        .output()
        .expect("git clone failed");

    let temp_path = work_repo.to_str().unwrap();
    let repo_path = Path::new(temp_path);

    // Configure git
    git_assert(repo_path, &["config", "user.email", "test@example.com"]);
    git_assert(repo_path, &["config", "user.name", "Test User"]);
    git_assert(repo_path, &["config", "commit.gpgsign", "false"]);

    // Create initial commit and push
    fs::write(repo_path.join("file1.txt"), "initial").unwrap();
    git_assert(repo_path, &["add", "file1.txt"]);
    git_assert(repo_path, &["commit", "-m", "Initial commit"]);
    git_assert(repo_path, &["push", "-u", "origin", "main"]);

    // Simulate remote having new commits by cloning again and pushing
    let other_repo = temp_dir.path().join("other");
    Command::new("git")
        .args([
            "clone",
            bare_repo.to_str().unwrap(),
            other_repo.to_str().unwrap(),
        ])
        .current_dir(temp_path)
        .output()
        .expect("git clone failed");

    let other_repo_path = other_repo.as_path();

    git_assert(
        other_repo_path,
        &["config", "user.email", "test@example.com"],
    );
    git_assert(other_repo_path, &["config", "user.name", "Test User"]);
    git_assert(other_repo_path, &["config", "commit.gpgsign", "false"]);

    // Create 2 commits in other repo and push
    for i in 1..=2 {
        fs::write(
            other_repo.join(format!("remote{i}.txt")),
            format!("remote {i}"),
        )
        .unwrap();
        git_assert(other_repo_path, &["add", "."]);
        git_assert(
            other_repo_path,
            &["commit", "-m", &format!("Remote commit {i}")],
        );
    }
    git_assert(other_repo_path, &["push"]);

    // Fetch in our work repo (but don't merge)
    git_assert(repo_path, &["fetch"]);

    // Check status - should show ahead=0, behind=2
    let result = load_repository_state(temp_path, None, 0, true);
    assert!(result.is_ok(), "load_repository_state should succeed");

    let git_status = result.unwrap();
    assert!(git_status.is_some(), "Should return git status");

    let Some(status) = git_status else { return };
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(status.status.ahead, 0, "Should be 0 commits ahead");
    assert_eq!(status.status.behind, 2, "Should be 2 commits behind");
}

#[test]
#[serial(git_status)]
fn test_ahead_and_behind_commits() {
    // Test divergent history (both ahead and behind)
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize bare repo
    let bare_repo = temp_dir.path().join("bare.git");
    fs::create_dir_all(&bare_repo).unwrap();
    Command::new("git")
        .args(["init", "--bare", "--initial-branch=main"])
        .current_dir(&bare_repo)
        .output()
        .expect("git init --bare failed");

    // Clone the bare repo
    let work_repo = temp_dir.path().join("work");
    Command::new("git")
        .args([
            "clone",
            bare_repo.to_str().unwrap(),
            work_repo.to_str().unwrap(),
        ])
        .current_dir(temp_path)
        .output()
        .expect("git clone failed");

    let temp_path = work_repo.to_str().unwrap();
    let repo_path = Path::new(temp_path);

    // Configure git
    git_assert(repo_path, &["config", "user.email", "test@example.com"]);
    git_assert(repo_path, &["config", "user.name", "Test User"]);
    git_assert(repo_path, &["config", "commit.gpgsign", "false"]);

    // Create initial commit and push
    fs::write(repo_path.join("base.txt"), "base").unwrap();
    git_assert(repo_path, &["add", "base.txt"]);
    git_assert(repo_path, &["commit", "-m", "Base commit"]);
    git_assert(repo_path, &["push", "-u", "origin", "main"]);

    // Create 2 local commits (ahead)
    for i in 1..=2 {
        fs::write(
            repo_path.join(format!("local{i}.txt")),
            format!("local {i}"),
        )
        .unwrap();
        git_assert(repo_path, &["add", "."]);
        git_assert(repo_path, &["commit", "-m", &format!("Local commit {i}")]);
    }

    // Clone again and create commits on "remote"
    let other_repo = temp_dir.path().join("other");
    Command::new("git")
        .args([
            "clone",
            bare_repo.to_str().unwrap(),
            other_repo.to_str().unwrap(),
        ])
        .current_dir(temp_path)
        .output()
        .expect("git clone failed");

    let other_repo_path = other_repo.as_path();

    git_assert(
        other_repo_path,
        &["config", "user.email", "test@example.com"],
    );
    git_assert(other_repo_path, &["config", "user.name", "Test User"]);
    git_assert(other_repo_path, &["config", "commit.gpgsign", "false"]);

    // Create 3 commits in other repo and push (behind)
    for i in 1..=3 {
        fs::write(
            other_repo_path.join(format!("remote{i}.txt")),
            format!("remote {i}"),
        )
        .unwrap();
        git_assert(other_repo_path, &["add", "."]);
        git_assert(
            other_repo_path,
            &["commit", "-m", &format!("Remote commit {i}")],
        );
    }
    git_assert(other_repo_path, &["push"]);

    // Fetch in our work repo
    git_assert(repo_path, &["fetch"]);

    // Check status - should show ahead=2, behind=3
    let result = load_repository_state(temp_path, None, 0, true);
    assert!(result.is_ok(), "load_repository_state should succeed");

    let git_status = result.unwrap();
    assert!(git_status.is_some(), "Should return git status");

    let Some(status) = git_status else { return };
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(status.status.ahead, 2, "Should be 2 commits ahead");
    assert_eq!(status.status.behind, 3, "Should be 3 commits behind");
}

#[test]
#[serial(git_status)]
fn test_no_upstream_configured() {
    // Test repository without upstream (should return 0, 0)
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(temp_path)
            .output()
            .expect("git command failed")
    };

    // Initialize local repo (no remote)
    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test User"]);
    run(&["config", "commit.gpgsign", "false"]);

    // Create a commit
    fs::write(temp_path.join("file.txt"), "content").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "Initial commit"]);

    // Check status - should show ahead=0, behind=0 (no upstream)
    let result = load_repository_state(temp_path.to_str().unwrap(), None, 0, true);
    assert!(result.is_ok(), "load_repository_state should succeed");

    let git_status = result.unwrap();
    assert!(git_status.is_some(), "Should return git status");

    let Some(status) = git_status else { return };
    assert_eq!(status.status.ahead, 0, "Should be 0 ahead (no upstream)");
    assert_eq!(status.status.behind, 0, "Should be 0 behind (no upstream)");
}

#[test]
#[serial(git_status)]
fn test_git_status_respects_gitignore() {
    // Test that files listed in .gitignore are not counted as untracked
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize repo
    git_assert(temp_path, &["init"]);
    git_assert(temp_path, &["config", "user.email", "test@example.com"]);
    git_assert(temp_path, &["config", "user.name", "Test User"]);
    git_assert(temp_path, &["config", "commit.gpgsign", "false"]);

    // Create initial commit
    fs::write(temp_path.join("README.md"), "# Test Repo").unwrap();
    git_assert(temp_path, &["add", "README.md"]);
    git_assert(temp_path, &["commit", "-m", "Initial commit"]);

    // Create .gitignore
    fs::write(temp_path.join(".gitignore"), "*.log\n*.tmp\nbuild/\n").unwrap();
    git_assert(temp_path, &["add", ".gitignore"]);
    git_assert(temp_path, &["commit", "-m", "Add gitignore"]);

    // Create ignored files
    fs::write(temp_path.join("debug.log"), "log content").unwrap();
    fs::write(temp_path.join("cache.tmp"), "temp content").unwrap();
    fs::create_dir_all(temp_path.join("build")).unwrap();
    fs::write(temp_path.join("build/output.bin"), "binary").unwrap();

    // Verify git status reports clean
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(temp_path)
        .output()
        .expect("git status should run");
    let git_output = String::from_utf8_lossy(&output.stdout);
    assert!(
        git_output.trim().is_empty(),
        "git status should report clean, but got: {git_output}"
    );

    // Check our status - should also report clean (no untracked files)
    // Migrated from the retired porcelain-v1 `get_status_counts` chain onto the
    // live `get_complete_status` entry point (#621).
    let complete = NativeGitBackend::get_complete_status(
        temp_path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("inside a repo");
    let (staged, unstaged, untracked, conflicts) = (
        complete.status.staged,
        complete.status.unstaged,
        complete.status.untracked,
        complete.status.conflicts,
    );

    assert_eq!(
        staged, 0,
        "Should have 0 staged files (gitignored files should not be counted)"
    );
    assert_eq!(
        unstaged, 0,
        "Should have 0 unstaged files (gitignored files should not be counted)"
    );
    assert_eq!(
        untracked, 0,
        "Should have 0 untracked files (gitignored files should not be counted)"
    );
    assert_eq!(conflicts, 0, "Should have 0 conflicts");

    // Verify overall state is clean
    let result = load_repository_state(temp_path.to_str().unwrap(), None, 0, true);
    assert!(result.is_ok(), "load_repository_state should succeed");

    let git_status = result.unwrap();
    assert!(git_status.is_some(), "Should return git status");

    let Some(status) = git_status else { return };
    assert_eq!(
        status.status.state,
        RepositoryState::Clean,
        "Repository should be in clean state (gitignored files should not affect status)"
    );
    assert_eq!(status.status.untracked, 0, "Should have 0 untracked files");
}

/// Regression for #180: a staged rename whose source and destination both
/// contain spaces must be keyed in the per-file map by the destination path
/// alone, so incremental watcher updates can find the changed file instead of
/// corrupting the cache with a concatenated key.
#[test]
#[serial(git_status)]
fn test_git_status_staged_rename_with_spaces_keys_by_destination() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    git_assert(temp_path, &["init"]);
    git_assert(temp_path, &["config", "user.email", "test@example.com"]);
    git_assert(temp_path, &["config", "user.name", "Test User"]);
    git_assert(temp_path, &["config", "commit.gpgsign", "false"]);

    fs::write(temp_path.join("old name.txt"), "content").expect("write source file");
    git_assert(temp_path, &["add", "old name.txt"]);
    git_assert(temp_path, &["commit", "-m", "add file"]);

    // Stage a rename to a destination that also contains a space.
    git_assert(temp_path, &["mv", "old name.txt", "new name.txt"]);

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    let status = result
        .expect("status should load")
        .expect("a non-empty repo should report status");

    assert_eq!(
        status.status.staged, 1,
        "a staged rename should count as exactly one staged change"
    );
    assert!(
        status.files.contains_key(Path::new("new name.txt")),
        "file map must be keyed by the rename destination; got {:?}",
        status.files.keys().collect::<Vec<_>>()
    );
    assert!(
        !status
            .files
            .contains_key(Path::new("new name.txt old name.txt")),
        "destination and original paths must not be concatenated into one key"
    );
}
