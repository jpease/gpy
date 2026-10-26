//! Huge-repo handling: sparse checkout detection and repeated-status determinism.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::cast_precision_loss)]

#[path = "common/git.rs"]
mod git;

use gpy_agent::config::types::GitTimeout;
use gpy_agent::git::native::NativeGitBackend;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

fn create_git_repo() -> TempDir {
    let dir = TempDir::new().expect("create temp dir");
    git::git_init(dir.path());
    dir
}

#[test]
fn test_sparse_checkout_doesnt_crash() {
    let repo = create_git_repo();
    let repo_path = repo.path();

    fs::create_dir_all(repo_path.join("src")).expect("create src dir");
    fs::create_dir_all(repo_path.join("docs")).expect("create docs dir");

    let init_file = repo_path.join("README.md");
    fs::write(&init_file, "initial").expect("write initial file");

    git::git_commit_all(repo_path, "Initial");

    let sparse_file = repo_path.join(".git/info/sparse-checkout");
    fs::write(&sparse_file, "src/\n").expect("write sparse-checkout");

    git::git(repo_path, &["config", "core.sparseCheckout", "true"]);

    fs::write(repo_path.join("src/file.txt"), "src content").expect("write src file");
    fs::write(repo_path.join("test.txt"), "root content").expect("write root file");

    // Must not crash under sparse checkout; exact counts are git-version dependent.
    let result = NativeGitBackend::get_complete_status(
        repo_path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    );

    if let Err(e) = &result {
        println!("Note: Sparse checkout caused expected error: {e}");
    }
}

/// Back-to-back `get_complete_status` calls on an unchanged repo must agree.
#[test]
fn test_repeated_complete_status_is_deterministic() {
    let repo = create_git_repo();
    let repo_path = repo.path();

    let file_path = repo_path.join("test.txt");
    fs::write(&file_path, "content").expect("write file");

    git::git_commit_all(repo_path, "Initial");

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
