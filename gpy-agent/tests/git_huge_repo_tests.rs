//! Huge-repo handling: sparse checkout detection and repeated-status determinism.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::cast_precision_loss)]

use gpy_agent::config::types::GitTimeout;
use gpy_agent::git::native::NativeGitBackend;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

fn create_git_repo() -> TempDir {
    let dir = TempDir::new().expect("create temp dir");
    let status = std::process::Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());

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

    // Older git may lack `git sparse-checkout`.
    // Note: "git sparse-checkout" command might not be available in older git versions

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

    let is_sparse = NativeGitBackend::is_sparse_checkout(repo_path);
    // True if sparse-checkout setup succeeded.
    assert!(is_sparse);
}

#[test]
fn test_sparse_checkout_doesnt_crash() {
    let repo = create_git_repo();
    let repo_path = repo.path();

    fs::create_dir_all(repo_path.join("src")).expect("create src dir");
    fs::create_dir_all(repo_path.join("docs")).expect("create docs dir");

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

    let sparse_file = repo_path.join(".git/info/sparse-checkout");
    fs::write(&sparse_file, "src/\n").expect("write sparse-checkout");

    std::process::Command::new("git")
        .args(["config", "core.sparseCheckout", "true"])
        .current_dir(repo_path)
        .status()
        .expect("config sparse");

    let is_sparse = NativeGitBackend::is_sparse_checkout(repo_path);
    assert!(is_sparse, "Should detect sparse checkout");

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
