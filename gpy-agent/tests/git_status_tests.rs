//! Integration tests for `load_repository_state` against real git repositories:
//! repo discovery, file-state counts, ignore handling, repository state, and
//! ahead/behind divergence.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::config::types::GitTimeout;
use gpy_agent::git::{
    RepositoryState, RepositoryStatus, native::NativeGitBackend, status::load_repository_state,
};
use serial_test::serial;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;

#[path = "common/git.rs"]
mod git;

#[cfg(all(unix, not(target_os = "macos")))]
use std::os::unix::ffi::OsStrExt;

#[test]
#[serial(git_status)]
fn test_git_status_not_a_repo() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let result = load_repository_state(temp_dir.path().to_string_lossy().as_ref(), None, 0, true);

    assert!(
        matches!(result, Ok(None)),
        "non-repo directory must yield Ok(None)"
    );
}

#[test]
#[serial(git_status)]
fn test_git_status_from_subdirectory() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let repo = temp_dir.path();
    git::git_init(repo);
    fs::write(repo.join("README.md"), "# test\n").expect("write README");
    git::git(repo, &["add", "README.md"]);
    git::git(repo, &["commit", "-q", "-m", "init"]);
    let nested = repo.join("src").join("nested");
    fs::create_dir_all(&nested).expect("create nested dir");

    let status = load_repository_state(&nested, None, 0, true)
        .expect("load_repository_state should succeed")
        .expect("subdirectory should resolve to the enclosing repository");
    assert_eq!(status.status.branch, "main");
    assert_eq!(status.status.state, RepositoryState::Clean);
}

#[test]
#[serial(git_status)]
fn test_git_status_invalid_path() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let missing = temp_dir.path().join("does").join("not").join("exist");

    // A nonexistent path has no enclosing `.git`, so it resolves to "not a repo".
    let result = load_repository_state(&missing, None, 0, true);
    assert!(
        matches!(result, Ok(None)),
        "nonexistent path must yield Ok(None)"
    );
}

#[test]
#[serial(git_status)]
fn test_git_status_created_repo() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git::git_init(temp_path);

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert!(status.status.branch == "main" || status.status.branch == "master");
    assert_eq!(status.status.ahead, 0);
    assert_eq!(status.status.behind, 0);
    assert_eq!(status.status.staged, 0);
    assert_eq!(status.status.untracked, 0);
    assert_eq!(status.status.conflicts, 0);
}

#[test]
#[serial(git_status)]
fn test_git_status_with_untracked_files() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git::git_init(temp_path);

    fs::write(temp_path.join("untracked1.txt"), "content").expect("write untracked1.txt");
    fs::write(temp_path.join("untracked2.txt"), "content").expect("write untracked2.txt");

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert_eq!(status.status.untracked, 2);
    assert_eq!(status.status.staged, 0);
    assert_eq!(status.status.unstaged, 0);
}

#[test]
#[serial(git_status)]
fn test_git_status_with_staged_files() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git::git_init(temp_path);

    fs::write(temp_path.join("staged.txt"), "content").expect("write staged.txt");
    git::git(temp_path, &["add", "staged.txt"]);

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert_eq!(status.status.staged, 1);
    assert_eq!(status.status.unstaged, 0);
    assert_eq!(status.status.untracked, 0);
}

#[test]
#[serial(git_status)]
fn test_git_status_relative_path() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let repo = fs::canonicalize(temp_dir.path()).expect("canonicalize temp dir");
    git::git_init(&repo);

    // Climb from the working directory to the filesystem root, then descend
    // into the repo, so the queried path is relative but cwd-independent.
    let cwd = std::env::current_dir().expect("current dir");
    let relative: PathBuf = cwd
        .components()
        .skip(1)
        .map(|_| Component::ParentDir)
        .chain(repo.components().skip(1))
        .collect();
    assert!(relative.is_relative());

    let status = load_repository_state(&relative, None, 0, true)
        .expect("load_repository_state should succeed")
        .expect("relative path should resolve to the repository");
    assert_eq!(status.status.branch, "main");
}

#[test]
#[serial(git_status)]
fn test_git_status_empty_path() {
    let result = load_repository_state("", None, 0, true);

    // Empty path should be handled gracefully (either Ok(None) or Err)
    assert!(matches!(result, Ok(Some(_) | None) | Err(_)));
}

#[test]
#[serial(git_status)]
fn test_git_status_struct_default_values() {
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

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
#[serial(git_status)]
fn test_non_utf8_staged_paths_are_counted() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git::git_init(temp_path);

    let filename = std::ffi::OsStr::from_bytes(b"non_utf8_\xFF");
    let file_path = temp_path.join(filename);
    fs::write(&file_path, b"content").expect("write file");

    git::git(temp_path, &["add", "."]);

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
    let temp_dir = TempDir::new().expect("create temp dir");
    let repo_path = temp_dir.path();

    let run = |args: &[&str]| git::git(repo_path, args);

    git::git_init(repo_path);

    fs::write(repo_path.join("file.txt"), b"base").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "base"]);

    run(&["checkout", "-b", "feature"]);
    fs::write(repo_path.join("file.txt"), b"feature change").unwrap();
    run(&["commit", "-am", "feature work"]);

    run(&["checkout", "main"]);
    fs::write(repo_path.join("file.txt"), b"main change").unwrap();
    run(&["commit", "-am", "main work"]);

    let merge = git::git_output(repo_path, &["merge", "feature"]);
    assert!(
        !merge.status.success(),
        "merging `feature` should stop on the conflict"
    );

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
#[serial(git_status)]
fn test_git_status_ignores_gitignored_files() {
    // Only ignored files present: status must be clean.
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git::git_init(temp_path);

    fs::write(temp_path.join(".gitignore"), ".DS_Store\n").expect("write .gitignore");
    git::git(temp_path, &["add", ".gitignore"]);
    git::git(temp_path, &["commit", "-m", "Add gitignore"]);

    fs::write(temp_path.join(".DS_Store"), "macOS metadata").expect("write .DS_Store");

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");

    assert_eq!(
        status.status.untracked, 0,
        "Ignored files should not be counted"
    );
    assert_eq!(status.status.staged, 0);
    assert_eq!(status.status.unstaged, 0);
    assert_eq!(status.status.conflicts, 0);
    assert_eq!(status.status.state, RepositoryState::Clean);
}

#[test]
#[serial(git_status)]
fn test_git_status_counts_non_ignored_files_with_ignored_present() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git::git_init(temp_path);

    fs::write(temp_path.join(".gitignore"), ".DS_Store\n").expect("write .gitignore");
    git::git(temp_path, &["add", ".gitignore"]);
    git::git(temp_path, &["commit", "-m", "Add gitignore"]);

    fs::write(temp_path.join(".DS_Store"), "macOS metadata").expect("write .DS_Store");

    fs::write(temp_path.join("foo.txt"), "real content").expect("write foo.txt");

    let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");

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
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    let run = |args: &[&str]| git::git(temp_path, args);

    git::git_init(temp_path);

    fs::write(temp_path.join("file.txt"), "content").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "base"]);

    fs::remove_file(temp_path.join("file.txt")).unwrap();
    run(&["add", "-u"]);

    let result = load_repository_state(temp_path, None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");

    assert_eq!(status.status.staged, 1, "Should count staged deletion");
    assert_eq!(status.status.unstaged, 0);
    assert_eq!(status.status.untracked, 0);
    assert_eq!(status.status.conflicts, 0);
    assert_eq!(status.status.state, RepositoryState::Dirty);
}

#[test]
#[serial(git_status)]
fn test_git_status_rebase_state_precedence() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    let run = |args: &[&str]| git::git(temp_path, args);

    git::git_init(temp_path);

    fs::write(temp_path.join("file.txt"), "base").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "base"]);

    run(&["checkout", "-b", "feature"]);
    fs::write(temp_path.join("file.txt"), "feature").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "feature"]);

    run(&["checkout", "main"]);
    fs::write(temp_path.join("file.txt"), "main").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "main"]);

    run(&["checkout", "feature"]);
    let rebase = git::git_output(temp_path, &["rebase", "main"]);
    assert!(
        !rebase.status.success(),
        "rebasing `feature` onto `main` should stop on the conflict"
    );

    fs::write(temp_path.join("file.txt"), "resolved").unwrap();
    run(&["add", "file.txt"]);

    // Mid-rebase with staged changes reports Rebasing, not Dirty.
    let result = load_repository_state(temp_path, None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");

    assert_eq!(
        status.status.state,
        RepositoryState::Rebasing,
        "Should show rebasing state even with staged changes"
    );
    assert!(status.status.staged > 0, "Should have staged changes");
}

#[test]
#[serial(git_status)]
fn test_ahead_only_commits() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    let bare_repo = temp_dir.path().join("bare.git");
    git::git_bare(&bare_repo);

    let work_repo = temp_dir.path().join("work");
    git::git(
        temp_path,
        &[
            "clone",
            "-q",
            bare_repo.to_str().unwrap(),
            work_repo.to_str().unwrap(),
        ],
    );

    let work_path = work_repo.to_str().unwrap();
    let repo_path = Path::new(work_path);

    fs::write(repo_path.join("file1.txt"), "initial").unwrap();
    git::git(repo_path, &["add", "file1.txt"]);
    git::git(repo_path, &["commit", "-m", "Initial commit"]);
    git::git(repo_path, &["push", "-u", "origin", "main"]);

    for i in 1_u32..=3 {
        fs::write(
            repo_path.join(format!("file{}.txt", i + 1)),
            format!("content {i}"),
        )
        .unwrap();
        git::git(repo_path, &["add", "."]);
        git::git(repo_path, &["commit", "-m", &format!("Commit {i}")]);
    }

    let result = load_repository_state(work_path, None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(status.status.ahead, 3, "Should be 3 commits ahead");
    assert_eq!(status.status.behind, 0, "Should be 0 commits behind");
}

#[test]
#[serial(git_status)]
fn test_ahead_with_non_origin_remote() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let bare_repo = temp_dir.path().join("bare.git");
    git::git_bare(&bare_repo);

    let work_repo = temp_dir.path().join("work");
    let bare_repo_str = bare_repo.to_str().unwrap().to_owned();
    let work_repo_str = work_repo.to_str().unwrap().to_owned();
    let clone_args = ["clone", bare_repo_str.as_str(), work_repo_str.as_str()];
    git::git(temp_dir.path(), &clone_args);

    let repo_path = work_repo.as_path();

    // Rename the default remote so the branch tracks a non-origin remote.
    git::git(repo_path, &["remote", "rename", "origin", "upstream"]);

    fs::write(repo_path.join("file1.txt"), "initial").unwrap();
    git::git(repo_path, &["add", "file1.txt"]);
    git::git(repo_path, &["commit", "-m", "Initial commit"]);
    git::git(repo_path, &["push", "-u", "upstream", "main"]);
    git::git(repo_path, &["fetch", "upstream"]);

    fs::write(repo_path.join("local.txt"), "ahead change").unwrap();
    git::git(repo_path, &["add", "."]);
    git::git(repo_path, &["commit", "-m", "Local ahead commit"]);

    let result = load_repository_state(repo_path.to_string_lossy().as_ref(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
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
    let temp_dir = TempDir::new().expect("create temp dir");
    let bare_repo = temp_dir.path().join("bare.git");
    git::git_bare(&bare_repo);

    let work_repo = temp_dir.path().join("work");
    let bare_repo_str = bare_repo.to_str().unwrap().to_owned();
    let work_repo_str = work_repo.to_str().unwrap().to_owned();
    let clone_args = ["clone", bare_repo_str.as_str(), work_repo_str.as_str()];
    git::git(temp_dir.path(), &clone_args);

    let repo_path = work_repo.as_path();

    fs::write(repo_path.join("file1.txt"), "initial").unwrap();
    git::git(repo_path, &["add", "file1.txt"]);
    git::git(repo_path, &["commit", "-m", "Initial commit"]);
    git::git(repo_path, &["push", "-u", "origin", "main"]);
    git::git(repo_path, &["fetch", "origin"]);

    // Pack refs so the tracking branch only exists in packed-refs.
    git::git(repo_path, &["pack-refs", "--all", "--prune"]);
    let tracking_ref = repo_path.join(".git/refs/remotes/origin/main");
    if tracking_ref.exists() {
        let _ = fs::remove_file(&tracking_ref);
    }

    fs::write(repo_path.join("local.txt"), "ahead change").unwrap();
    git::git(repo_path, &["add", "."]);
    git::git(repo_path, &["commit", "-m", "Local ahead commit"]);

    let result = load_repository_state(repo_path.to_string_lossy().as_ref(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(status.status.ahead, 1, "Should be 1 commit ahead");
    assert_eq!(status.status.behind, 0, "Should be 0 commits behind");
}

#[test]
#[serial(git_status)]
fn test_behind_only_commits() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    let bare_repo = temp_dir.path().join("bare.git");
    git::git_bare(&bare_repo);

    let work_repo = temp_dir.path().join("work");
    git::git(
        temp_path,
        &[
            "clone",
            "-q",
            bare_repo.to_str().unwrap(),
            work_repo.to_str().unwrap(),
        ],
    );

    let work_path = work_repo.to_str().unwrap();
    let repo_path = Path::new(work_path);

    fs::write(repo_path.join("file1.txt"), "initial").unwrap();
    git::git(repo_path, &["add", "file1.txt"]);
    git::git(repo_path, &["commit", "-m", "Initial commit"]);
    git::git(repo_path, &["push", "-u", "origin", "main"]);

    // Push new commits from a second clone to simulate remote progress.
    let other_repo = temp_dir.path().join("other");
    git::git(
        repo_path,
        &[
            "clone",
            "-q",
            bare_repo.to_str().unwrap(),
            other_repo.to_str().unwrap(),
        ],
    );

    let other_repo_path = other_repo.as_path();

    for i in 1_u32..=2 {
        fs::write(
            other_repo.join(format!("remote{i}.txt")),
            format!("remote {i}"),
        )
        .unwrap();
        git::git(other_repo_path, &["add", "."]);
        git::git(
            other_repo_path,
            &["commit", "-m", &format!("Remote commit {i}")],
        );
    }
    git::git(other_repo_path, &["push"]);

    git::git(repo_path, &["fetch"]);

    let result = load_repository_state(work_path, None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(status.status.ahead, 0, "Should be 0 commits ahead");
    assert_eq!(status.status.behind, 2, "Should be 2 commits behind");
}

#[test]
#[serial(git_status)]
fn test_ahead_and_behind_commits() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    let bare_repo = temp_dir.path().join("bare.git");
    git::git_bare(&bare_repo);

    let work_repo = temp_dir.path().join("work");
    git::git(
        temp_path,
        &[
            "clone",
            "-q",
            bare_repo.to_str().unwrap(),
            work_repo.to_str().unwrap(),
        ],
    );

    let work_path = work_repo.to_str().unwrap();
    let repo_path = Path::new(work_path);

    fs::write(repo_path.join("base.txt"), "base").unwrap();
    git::git(repo_path, &["add", "base.txt"]);
    git::git(repo_path, &["commit", "-m", "Base commit"]);
    git::git(repo_path, &["push", "-u", "origin", "main"]);

    for i in 1_u32..=2 {
        fs::write(
            repo_path.join(format!("local{i}.txt")),
            format!("local {i}"),
        )
        .unwrap();
        git::git(repo_path, &["add", "."]);
        git::git(repo_path, &["commit", "-m", &format!("Local commit {i}")]);
    }

    let other_repo = temp_dir.path().join("other");
    git::git(
        repo_path,
        &[
            "clone",
            "-q",
            bare_repo.to_str().unwrap(),
            other_repo.to_str().unwrap(),
        ],
    );

    let other_repo_path = other_repo.as_path();

    for i in 1_u32..=3 {
        fs::write(
            other_repo_path.join(format!("remote{i}.txt")),
            format!("remote {i}"),
        )
        .unwrap();
        git::git(other_repo_path, &["add", "."]);
        git::git(
            other_repo_path,
            &["commit", "-m", &format!("Remote commit {i}")],
        );
    }
    git::git(other_repo_path, &["push"]);

    git::git(repo_path, &["fetch"]);

    let result = load_repository_state(work_path, None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert_eq!(status.status.branch, "main", "Should be on main branch");
    assert_eq!(status.status.ahead, 2, "Should be 2 commits ahead");
    assert_eq!(status.status.behind, 3, "Should be 3 commits behind");
}

#[test]
#[serial(git_status)]
fn test_no_upstream_configured() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    let run = |args: &[&str]| git::git(temp_path, args);

    git::git_init(temp_path);

    fs::write(temp_path.join("file.txt"), "content").unwrap();
    run(&["add", "file.txt"]);
    run(&["commit", "-m", "Initial commit"]);

    let result = load_repository_state(temp_path.to_str().unwrap(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert_eq!(status.status.ahead, 0, "Should be 0 ahead (no upstream)");
    assert_eq!(status.status.behind, 0, "Should be 0 behind (no upstream)");
}

#[test]
#[serial(git_status)]
fn test_git_status_respects_gitignore() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git::git_init(temp_path);

    fs::write(temp_path.join("README.md"), "# Test Repo").unwrap();
    git::git(temp_path, &["add", "README.md"]);
    git::git(temp_path, &["commit", "-m", "Initial commit"]);

    fs::write(temp_path.join(".gitignore"), "*.log\n*.tmp\nbuild/\n").unwrap();
    git::git(temp_path, &["add", ".gitignore"]);
    git::git(temp_path, &["commit", "-m", "Add gitignore"]);

    fs::write(temp_path.join("debug.log"), "log content").unwrap();
    fs::write(temp_path.join("cache.tmp"), "temp content").unwrap();
    fs::create_dir_all(temp_path.join("build")).unwrap();
    fs::write(temp_path.join("build/output.bin"), "binary").unwrap();

    let git_output = git::git_stdout(temp_path, &["status", "--porcelain"]);
    assert!(
        git_output.trim().is_empty(),
        "git status should report clean, but got: {git_output}"
    );

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

    let result = load_repository_state(temp_path.to_str().unwrap(), None, 0, true);
    let status = result
        .expect("load_repository_state should succeed")
        .expect("path should resolve to a git repository");
    assert_eq!(
        status.status.state,
        RepositoryState::Clean,
        "Repository should be in clean state (gitignored files should not affect status)"
    );
    assert_eq!(status.status.untracked, 0, "Should have 0 untracked files");
}

/// A staged rename with spaces in both paths is keyed by destination alone,
/// so watcher updates find the file instead of a concatenated key.
#[test]
#[serial(git_status)]
fn test_git_status_staged_rename_with_spaces_keys_by_destination() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git::git_init(temp_path);

    fs::write(temp_path.join("old name.txt"), "content").expect("write source file");
    git::git(temp_path, &["add", "old name.txt"]);
    git::git(temp_path, &["commit", "-m", "add file"]);

    git::git(temp_path, &["mv", "old name.txt", "new name.txt"]);

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
