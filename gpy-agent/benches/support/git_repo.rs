//! Shared benchmark fixture: a git repository with one commit of N files.

#[path = "../../tests/common/git.rs"]
mod git;

use tempfile::TempDir;

/// Create a temp git repository whose single commit tracks `num_files` files.
#[must_use]
pub fn committed_repo(num_files: usize) -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix("gpy-bench-git")
        .tempdir()
        .expect("failed to create temp dir");
    let path = dir.path();

    git::git_init(path);
    for i in 0..num_files {
        std::fs::write(path.join(format!("file_{i}.txt")), "content").expect("write fixture file");
    }
    git::git(path, &["add", "."]);
    git::git(path, &["commit", "-q", "-m", "init"]);

    dir
}
