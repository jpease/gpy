//! Git Repository Test Fixtures
//!
//! Utilities for creating and manipulating git repositories for testing.

#![allow(dead_code)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::str_to_string)]
#![allow(clippy::mem_forget)]
#![allow(clippy::use_self)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// A test git repository that will be cleaned up automatically
pub struct TestRepo {
    dir: TempDir,
    path: PathBuf,
}

impl TestRepo {
    /// Create a new test repository with git initialized
    ///
    /// # Panics
    /// Panics if git is not available or repository creation fails
    pub fn new() -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory");
        // Canonicalize path immediately to avoid /var vs /private/var issues on macOS
        let path = fs::canonicalize(dir.path()).expect("Failed to canonicalize temp path");

        // Initialize git repository
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

        // Pin the initial branch name so fixtures (e.g. `with_conflicts`,
        // which hardcodes a `git checkout main` step) are deterministic
        // regardless of the host's `init.defaultBranch` config. Works on the
        // unborn HEAD before any commit exists.
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
        // Configure git user
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

        // gpy-agent#386: git's own fsmonitor spawns a background daemon per
        // repo that independently subscribes to FSEvents for the same path
        // gpy's own watcher is watching, on a machine where `core.fsmonitor`
        // is enabled globally (a common perf setting -- see
        // `benchmarks/README.md`'s existing precedent for disabling it in
        // benchmark fixtures for the same reason). That competing daemon
        // measurably starves gpy's own FSEventStream of its first event under
        // load. Test fixtures never need git's own filesystem-watching
        // optimization, so disable it unconditionally here regardless of the
        // host's global config.
        Command::new("git")
            .args(["config", "core.fsmonitor", "false"])
            .current_dir(path)
            .output()
            .expect("Failed to disable core.fsmonitor");
    }

    /// Create a shallow clone of another repository
    pub fn as_shallow_clone(source: &TestRepo, depth: u32) -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory");
        let path = dir.path().to_path_buf();

        // Use file:// protocol to force local clone behavior that respects depth
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

    /// Create a branch
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

    /// Checkout a branch
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

    /// Add a worktree
    pub fn with_worktree(&self, branch: &str, _relative_path: &str) -> PathBuf {
        // Worktree path should be outside the main repo but inside temp dir usually,
        // but TestRepo owns its TempDir. We'll put it in a sibling directory if possible,
        // or just create a new TempDir for it? No, worktree add requires a path.
        // Ideally we return a PathBuf that the test can use.
        // We'll assume relative_path is relative to the repo parent or just a path.
        // For safety, let's use a sibling path in the same temp dir structure if possible,
        // but TempDir is the parent.
        // Actually, `self.dir` IS the temp dir, and `self.path` is `self.dir.path()`.
        // So we can't easily put it "outside" `self.path` but inside `self.dir` because `self.path` IS `self.dir`.
        // We'll create a new TempDir just to hold the worktree?
        // Or we can put it inside the repo (supported but weird)?
        // The prompt says "git worktree add ../worktree-dir".
        // This implies accessing the parent of the temp dir.
        // The parent of `TempDir` is usually `/tmp/`.
        // We should probably create a new TempDir for the worktree to ensure cleanup.
        // But `git worktree add` needs to control the directory creation.
        // Let's creating a directory inside the current repo but ignore it? No.
        // Let's use a nested directory for the main repo?
        // Too late for `TestRepo::new` structure.

        // We will create a separate TempDir for the worktree, and leak it?
        // Or return a struct that holds the TempDir?
        // The prompt signature is `with_worktree(...) -> PathBuf`.

        // Let's try to create it as a subdirectory of the repo's parent.
        // Warning: we don't own the parent.

        // Better: Create a directory INSIDE the repo `worktrees/` and git ignore it?
        // Or just use a new TempDir.
        let worktree_dir = TempDir::new().expect("Failed to create worktree dir");
        let worktree_path = worktree_dir.path().to_path_buf();
        // We must keep worktree_dir alive, otherwise it deletes on drop.
        // But we return PathBuf.
        // We will leak it for the test duration.
        std::mem::forget(worktree_dir);

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

    /// Add a submodule
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

    /// Create a symlink (Unix only)
    #[cfg(unix)]
    pub fn with_symlink(&self, name: &str, target: &str) -> &Self {
        let link_path = self.path.join(name);
        std::os::unix::fs::symlink(target, &link_path).expect("Failed to create symlink");
        self
    }

    /// Get the path to the repository
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the path as a string (for use with APIs expecting strings)
    pub fn path_string(&self) -> String {
        self.path
            .to_str()
            .expect("Repository path should be valid UTF-8")
            .to_string()
    }

    /// Create a file in the repository with the given content
    pub fn create_file(&self, name: &str, content: &str) -> &Self {
        let file_path = self.path.join(name);
        fs::write(&file_path, content)
            .unwrap_or_else(|e| panic!("Failed to create file {name}: {e}"));
        self
    }

    /// Stage a file (git add)
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

    /// Commit staged changes
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

    /// Add N commits to the repository
    pub fn with_commits(&self, count: u32) -> &Self {
        for i in 0..count {
            self.create_file(&format!("file{i}.txt"), &format!("content {i}"))
                .stage_file(&format!("file{i}.txt"))
                .commit(&format!("Commit {i}"));
        }
        self
    }

    /// Create an initial commit (empty repository to non-empty)
    pub fn with_initial_commit(self) -> Self {
        self.create_file("README.md", "# Test Repository\n")
            .stage_file("README.md")
            .commit("Initial commit");
        self
    }

    /// Add an untracked file
    pub fn with_untracked_file(self) -> Self {
        self.create_file("untracked.txt", "This file is not tracked\n");
        self
    }

    /// Add a staged file (ready to commit)
    pub fn with_staged_file(self, name: &str, content: &str) -> Self {
        self.create_file(name, content).stage_file(name);
        self
    }

    /// Modify an existing tracked file (creates unstaged changes)
    pub fn with_unstaged_changes(self) -> Self {
        // First commit a file
        self.create_file("modified.txt", "Original content\n")
            .stage_file("modified.txt")
            .commit("Add file to modify");

        // Then modify it without staging
        self.create_file("modified.txt", "Modified content\n");
        self
    }

    /// Create a repository with merge conflict state
    pub fn with_conflicts(self) -> Self {
        // Create main branch with a commit
        self.create_file("conflict.txt", "Content on main\n")
            .stage_file("conflict.txt")
            .commit("Commit on main");

        // Create feature branch
        Command::new("git")
            .args(["checkout", "-b", "feature"])
            .current_dir(&self.path)
            .output()
            .expect("Failed to create feature branch");

        // Modify file on feature branch
        self.create_file("conflict.txt", "Content on feature\n")
            .stage_file("conflict.txt")
            .commit("Commit on feature");

        // Go back to main and modify the same file
        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(&self.path)
            .output()
            .expect("Failed to checkout main");

        self.create_file("conflict.txt", "Different content on main\n")
            .stage_file("conflict.txt")
            .commit("Conflicting commit on main");

        // Try to merge feature (will create conflict)
        let _ = Command::new("git")
            .args(["merge", "feature"])
            .current_dir(&self.path)
            .output();

        self
    }

    /// Create a repository in detached HEAD state
    pub fn with_detached_head(self) -> Self {
        // Need at least one commit
        self.create_file("file.txt", "content\n")
            .stage_file("file.txt")
            .commit("Initial commit");

        // Get the commit hash
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.path)
            .output()
            .expect("Failed to get commit hash");

        let commit_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Checkout the commit directly (detached HEAD)
        Command::new("git")
            .args(["checkout", &commit_hash])
            .current_dir(&self.path)
            .output()
            .expect("Failed to create detached HEAD");

        self
    }

    /// Execute a git command in this repository
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
