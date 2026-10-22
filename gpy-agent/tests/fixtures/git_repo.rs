//! Git repository test fixtures.

#![allow(dead_code)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// A temporary git repository, removed on drop.
pub struct TestRepo {
    dir: TempDir,
    path: PathBuf,
}

impl TestRepo {
    /// Create a git-initialized repository.
    ///
    /// # Panics
    /// Panics if git is unavailable or init fails.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory");
        // Canonicalize: macOS /var -> /private/var.
        let path = fs::canonicalize(dir.path()).expect("Failed to canonicalize temp path");

        let output = Command::new("git")
            .args(["init"])
            .current_dir(&path)
            .output()
            .expect("Failed to run git init");

        assert!(
            output.status.success(),
            "Failed to initialize git repository: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Pin the branch name: `with_conflicts` checks out `main` regardless of
        // the host's `init.defaultBranch`.
        let branch_output = Command::new("git")
            .args(["symbolic-ref", "HEAD", "refs/heads/main"])
            .current_dir(&path)
            .output()
            .expect("Failed to pin initial branch name");

        assert!(
            branch_output.status.success(),
            "Failed to set initial branch to main: {}",
            String::from_utf8_lossy(&branch_output.stderr)
        );

        Self::configure_git(&path);

        Self { dir, path }
    }

    fn configure_git(path: &Path) {
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("Failed to configure git user.name");

        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .expect("Failed to configure git user.email");

        Command::new("git")
            .args(["config", "commit.gpgsign", "false"])
            .current_dir(path)
            .output()
            .expect("Failed to disable gpg signing");

        Command::new("git")
            .args(["config", "protocol.file.allow", "always"])
            .current_dir(path)
            .output()
            .expect("Failed to allow file protocol");

        // A global `core.fsmonitor` daemon competes for FSEvents and starves gpy's
        // stream of its first event under load.
        Command::new("git")
            .args(["config", "core.fsmonitor", "false"])
            .current_dir(path)
            .output()
            .expect("Failed to disable core.fsmonitor");
    }

    /// Create a shallow clone of another repository.
    pub fn as_shallow_clone(source: &TestRepo, depth: u32) -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory");
        let path = dir.path().to_path_buf();

        // file:// forces a real clone that honours `--depth`.
        let source_url = format!("file://{}", source.path().to_string_lossy());

        let output = Command::new("git")
            .args(["clone", "--depth", &depth.to_string(), &source_url, "."])
            .current_dir(&path)
            .output()
            .expect("Failed to shallow clone");

        assert!(
            output.status.success(),
            "Failed to shallow clone: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        Self::configure_git(&path);
        Self { dir, path }
    }

    /// Create a branch.
    pub fn create_branch(&self, branch: &str) -> &Self {
        let output = Command::new("git")
            .args(["branch", branch])
            .current_dir(&self.path)
            .output()
            .expect("Failed to create branch");

        assert!(
            output.status.success(),
            "Failed to create branch {branch}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self
    }

    /// Check out a branch.
    pub fn checkout(&self, branch: &str) -> &Self {
        let output = Command::new("git")
            .args(["checkout", branch])
            .current_dir(&self.path)
            .output()
            .expect("Failed to checkout branch");

        assert!(
            output.status.success(),
            "Failed to checkout branch {branch}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self
    }

    /// Add a worktree for `branch` in a fresh temp dir and return its path.
    pub fn with_worktree(&self, branch: &str, _relative_path: &str) -> PathBuf {
        let worktree_dir = TempDir::new().expect("Failed to create worktree dir");
        // Kept so the worktree outlives this call.
        let worktree_path = worktree_dir.keep();

        let output = Command::new("git")
            .args(["worktree", "add", worktree_path.to_str().unwrap(), branch])
            .current_dir(&self.path)
            .output()
            .expect("Failed to add worktree");

        assert!(
            output.status.success(),
            "Failed to add worktree: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        worktree_path
    }

    /// Add a submodule.
    pub fn with_submodule(&self, name: &str, source: &TestRepo) -> &Self {
        let output = Command::new("git")
            .args([
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                source.path().to_str().unwrap(),
                name,
            ])
            .current_dir(&self.path)
            .output()
            .expect("Failed to add submodule");

        assert!(
            output.status.success(),
            "Failed to add submodule: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        self.commit(&format!("Add submodule {name}"));
        self
    }

    /// Create a symlink (Unix only).
    #[cfg(unix)]
    pub fn with_symlink(&self, name: &str, target: &str) -> &Self {
        let link_path = self.path.join(name);
        std::os::unix::fs::symlink(target, &link_path).expect("Failed to create symlink");
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
            .expect("Repository path should be valid UTF-8")
            .to_owned()
    }

    /// Create a file with the given content.
    pub fn create_file(&self, name: &str, content: &str) -> &Self {
        let file_path = self.path.join(name);
        fs::write(&file_path, content)
            .unwrap_or_else(|e| panic!("Failed to create file {name}: {e}"));
        self
    }

    /// Stage a file.
    pub fn stage_file(&self, name: &str) -> &Self {
        let output = Command::new("git")
            .args(["add", name])
            .current_dir(&self.path)
            .output()
            .expect("Failed to run git add");

        assert!(
            output.status.success(),
            "Failed to stage file {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self
    }

    /// Commit staged changes.
    pub fn commit(&self, message: &str) -> &Self {
        let output = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.path)
            .output()
            .expect("Failed to run git commit");

        assert!(
            output.status.success(),
            "Failed to commit: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self
    }

    /// Add `count` commits.
    pub fn with_commits(&self, count: u32) -> &Self {
        for i in 0..count {
            self.create_file(&format!("file{i}.txt"), &format!("content {i}"))
                .stage_file(&format!("file{i}.txt"))
                .commit(&format!("Commit {i}"));
        }
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

        Command::new("git")
            .args(["checkout", "-b", "feature"])
            .current_dir(&self.path)
            .output()
            .expect("Failed to create feature branch");

        self.create_file("conflict.txt", "Content on feature\n")
            .stage_file("conflict.txt")
            .commit("Commit on feature");

        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&self.path)
            .output()
            .expect("Failed to checkout main");

        self.create_file("conflict.txt", "Different content on main\n")
            .stage_file("conflict.txt")
            .commit("Conflicting commit on main");

        let _ = Command::new("git")
            .args(["merge", "feature"])
            .current_dir(&self.path)
            .output();

        self
    }

    /// Leave the repository in detached HEAD state.
    pub fn with_detached_head(self) -> Self {
        self.create_file("file.txt", "content\n")
            .stage_file("file.txt")
            .commit("Initial commit");

        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.path)
            .output()
            .expect("Failed to get commit hash");

        let commit_hash = String::from_utf8_lossy(&output.stdout).trim().to_owned();

        Command::new("git")
            .args(["checkout", &commit_hash])
            .current_dir(&self.path)
            .output()
            .expect("Failed to create detached HEAD");

        self
    }

    /// Run a git command in this repository.
    pub fn git_command(&self, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.path)
            .output()
            .expect("Failed to execute git command")
    }
}

impl Default for TestRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_repo() {
        let repo = TestRepo::new();
        assert!(repo.path().exists());
        assert!(repo.path().join(".git").exists());
    }

    #[test]
    fn test_with_initial_commit() {
        let repo = TestRepo::new().with_initial_commit();
        assert!(repo.path().join("README.md").exists());

        let output = repo.git_command(&["log", "--oneline"]);
        assert!(output.status.success());
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(log.contains("Initial commit"));
    }

    #[test]
    fn test_with_untracked_file() {
        let repo = TestRepo::new().with_initial_commit().with_untracked_file();

        assert!(repo.path().join("untracked.txt").exists());

        let output = repo.git_command(&["status", "--porcelain"]);
        let status = String::from_utf8_lossy(&output.stdout);
        assert!(status.contains("?? untracked.txt"));
    }

    #[test]
    fn test_with_staged_file() {
        let repo = TestRepo::new()
            .with_initial_commit()
            .with_staged_file("new.txt", "content");

        let output = repo.git_command(&["status", "--porcelain"]);
        let status = String::from_utf8_lossy(&output.stdout);
        assert!(status.contains("A  new.txt"));
    }
}
