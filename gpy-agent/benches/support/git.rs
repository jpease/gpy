//! Shared benchmark fixture: git repositories for benchmarks.

#![allow(
    clippy::expect_used,
    reason = "benchmark fixture setup panics on error"
)]
#![allow(clippy::missing_panics_doc, reason = "benchmark fixture helper")]

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Run `git args` in `dir` with a fixed identity and commit signing disabled.
pub fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "user.name=T"])
        .args(["-c", "user.email=t@example.com"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Initialize an empty git repository at `dir` on branch `main`.
pub fn git_init(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
}

/// Create a temp git repository whose single commit tracks `num_files` files.
#[must_use]
pub fn committed_repo(num_files: usize) -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("gpy-bench-git")
        .tempdir()
        .expect("failed to create temp dir");
    let path = dir.path();

    git_init(path);
    for i in 0..num_files {
        std::fs::write(path.join(format!("file_{i}.txt")), "content").expect("write fixture file");
    }
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "init"]);

    dir
}
