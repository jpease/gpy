//! `git` subprocess helpers and the [`TestRepo`] fixture. Every command runs
//! with the developer's global and system git config switched off and a fixed
//! identity, so a fixture repository is the same on every machine; a command
//! a fixture depends on panics with git's stderr when it fails.

#![allow(
    dead_code,
    reason = "included by several test and bench binaries, each using a subset"
)]
#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// The null device, as `GIT_CONFIG_GLOBAL` accepts it.
#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

/// A `git` command in `dir` that reads no global or system config, commits
/// as a fixed identity, and never signs. Append the subcommand with `args`.
pub fn git_command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"]);
    command
}

/// Run `git args` in `dir` and return its output whatever the exit status,
/// for commands that are expected to fail or whose status is the result.
pub fn git_output(dir: &Path, args: &[&str]) -> Output {
    git_command(dir).args(args).output().expect("spawn git")
}

/// Run `git args` in `dir`, panicking with git's stderr if it fails, and
/// return its stdout.
pub fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = git_output(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout is UTF-8")
}

/// Run `git args` in `dir`, panicking with git's stderr if it fails.
pub fn git(dir: &Path, args: &[&str]) {
    git_stdout(dir, args);
}

/// Initialize an empty repository at `dir` on branch `main`.
pub fn git_init(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
}

/// Initialize an empty bare repository at `dir` (created if missing) whose
/// `HEAD` is `main`.
pub fn git_bare(dir: &Path) {
    fs::create_dir_all(dir).expect("create bare repository dir");
    git(dir, &["init", "-q", "--bare", "-b", "main"]);
}

/// Stage every change in the work tree of `dir` and commit it as `message`.
pub fn git_commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// A temporary git repository on branch `main`, removed on drop.
pub struct TestRepo {
    dir: TempDir,
    path: PathBuf,
}

impl TestRepo {
    /// Create an empty repository.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        // Canonicalize: macOS /var -> /private/var.
        let path = fs::canonicalize(dir.path()).expect("canonicalize temp path");
        git_init(&path);
        Self::configure(&path);
        Self { dir, path }
    }

    /// Repository-local settings for the agent under test, which reads the
    /// user's real git config.
    fn configure(path: &Path) {
        git(path, &["config", "protocol.file.allow", "always"]);
        // A global `core.fsmonitor` daemon competes for FSEvents and starves gpy's
        // stream of its first event under load.
        git(path, &["config", "core.fsmonitor", "false"]);
    }

    /// Create a shallow clone of another repository.
    pub fn as_shallow_clone(source: &Self, depth: u32) -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path().to_path_buf();
        // file:// forces a real clone that honours `--depth`.
        let source_url = format!("file://{}", source.path().display());
        git(
            &path,
            &[
                "clone",
                "-q",
                "--depth",
                &depth.to_string(),
                &source_url,
                ".",
            ],
        );
        Self::configure(&path);
        Self { dir, path }
    }

    /// Create a branch.
    pub fn create_branch(&self, branch: &str) -> &Self {
        git(&self.path, &["branch", branch]);
        self
    }

    /// Add a worktree for `branch` in a fresh temp dir and return its path.
    pub fn with_worktree(&self, branch: &str) -> PathBuf {
        // Kept so the worktree outlives this call.
        let worktree_path = TempDir::new().expect("create worktree dir").keep();
        let worktree = worktree_path.to_str().expect("UTF-8 worktree path");
        git(&self.path, &["worktree", "add", worktree, branch]);
        worktree_path
    }

    /// Add `source` as submodule `name` and commit it.
    pub fn with_submodule(&self, name: &str, source: &Self) -> &Self {
        let url = source.path().to_str().expect("UTF-8 submodule path");
        git(
            &self.path,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                url,
                name,
            ],
        );
        self.commit(&format!("Add submodule {name}"))
    }

    /// Create a symlink.
    #[cfg(unix)]
    pub fn with_symlink(&self, name: &str, target: &str) -> &Self {
        std::os::unix::fs::symlink(target, self.path.join(name)).expect("create symlink");
        self
    }

    /// Repository path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Repository path as a string.
    pub fn path_string(&self) -> String {
        self.path
            .to_str()
            .expect("repository path is UTF-8")
            .to_owned()
    }

    /// Create a file with the given content.
    pub fn create_file(&self, name: &str, content: &str) -> &Self {
        fs::write(self.path.join(name), content).expect("write repository file");
        self
    }

    /// Stage a file.
    pub fn stage_file(&self, name: &str) -> &Self {
        git(&self.path, &["add", name]);
        self
    }

    /// Commit staged changes.
    pub fn commit(&self, message: &str) -> &Self {
        git(&self.path, &["commit", "-q", "-m", message]);
        self
    }

    /// Create an initial commit.
    pub fn with_initial_commit(self) -> Self {
        self.create_file("README.md", "# Test Repository\n")
            .stage_file("README.md")
            .commit("Initial commit");
        self
    }

    /// Add an untracked file.
    pub fn with_untracked_file(self) -> Self {
        self.create_file("untracked.txt", "This file is not tracked\n");
        self
    }

    /// Add a staged file.
    pub fn with_staged_file(self, name: &str, content: &str) -> Self {
        self.create_file(name, content).stage_file(name);
        self
    }

    /// Modify a tracked file without staging it.
    pub fn with_unstaged_changes(self) -> Self {
        self.create_file("modified.txt", "Original content\n")
            .stage_file("modified.txt")
            .commit("Add file to modify");
        self.create_file("modified.txt", "Modified content\n");
        self
    }

    /// Leave the repository in a merge-conflict state.
    pub fn with_conflicts(self) -> Self {
        self.create_file("conflict.txt", "Content on main\n")
            .stage_file("conflict.txt")
            .commit("Commit on main");
        git(&self.path, &["checkout", "-q", "-b", "feature"]);
        self.create_file("conflict.txt", "Content on feature\n")
            .stage_file("conflict.txt")
            .commit("Commit on feature");
        git(&self.path, &["checkout", "-q", "main"]);
        self.create_file("conflict.txt", "Different content on main\n")
            .stage_file("conflict.txt")
            .commit("Conflicting commit on main");
        let merge = git_output(&self.path, &["merge", "feature"]);
        assert!(
            !merge.status.success(),
            "merging `feature` into `main` should stop on a conflict"
        );
        self
    }

    /// Leave the repository in detached HEAD state.
    pub fn with_detached_head(self) -> Self {
        self.create_file("file.txt", "content\n")
            .stage_file("file.txt")
            .commit("Initial commit");
        let head = git_stdout(&self.path, &["rev-parse", "HEAD"]);
        git(&self.path, &["checkout", "-q", head.trim()]);
        self
    }
}

impl Default for TestRepo {
    fn default() -> Self {
        Self::new()
    }
}
