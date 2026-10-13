//! `git` subprocess helpers for building fixture repositories.

#![allow(dead_code)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use std::path::Path;
use std::process::Command;

/// Run `git args` in `dir` with a fixed identity and commit signing disabled,
/// panicking with git's stderr if the command fails.
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

/// Initialize an empty repository at `dir` on branch `main`.
pub fn git_init(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
}
