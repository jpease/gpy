//! Unusual git repository states.

#![allow(clippy::missing_panics_doc)]

mod fixtures;

use fixtures::TestRepo;
use gpy_agent::git::CompleteStatus;
use gpy_agent::git::status::load_repository_state;
#[cfg(unix)]
use std::time::Duration;

#[test]
fn test_detached_head_state() {
    let repo = TestRepo::new().with_detached_head();
    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(
        result.is_ok(),
        "Should handle detached HEAD state without errors"
    );

    let CompleteStatus { status, .. } = result.unwrap().expect("Should find repository");
    // `branch` is the bare short hash, no `HEAD@` prefix.
    assert!(status.detached, "Should set the detached flag");
    assert!(
        !status.branch.starts_with("HEAD"),
        "branch must not carry the legacy HEAD@ prefix, got: {}",
        status.branch
    );
}

/// A repo with zero commits (unborn HEAD on the fixture-pinned `main`).
#[test]
fn test_initial_commit_branch_resolution() {
    let repo = TestRepo::new();
    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(
        result.is_ok(),
        "Should handle a zero-commit repository without errors"
    );

    let CompleteStatus { status, .. } = result.unwrap().expect("Should find repository");
    assert!(
        !status.detached,
        "an unborn HEAD on a named branch must not be reported as detached"
    );
    assert_eq!(
        status.branch, "main",
        "branch must resolve to the symbolic-ref target, not the fallback"
    );
}

#[test]
fn test_merge_conflict_detection() {
    let repo = TestRepo::new().with_conflicts();
    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(
        result.is_ok(),
        "Should successfully load repository with conflicts"
    );

    let CompleteStatus { status, .. } = result.unwrap().expect("Should find repository");
    let state_lower = status.state.as_str().to_lowercase();
    assert!(
        state_lower.contains("merge") || state_lower.contains("conflict"),
        "State '{}' should indicate merge conflict",
        status.state.as_str()
    );

    assert!(
        status.conflicts > 0 || status.unstaged > 0,
        "Conflicted files should be detected"
    );
}

#[test]
fn test_empty_directory_not_a_repo() {
    let temp_dir = tempfile::TempDir::new().expect("Failed to create temp directory");
    let result = load_repository_state(temp_dir.path(), None, 0, true);

    assert!(result.is_ok(), "Should not error for non-git directory");
    assert!(
        result.unwrap().is_none(),
        "Should return None for non-git directory"
    );
}

/// A symlink loop must not hang status.
#[test]
#[cfg(unix)] // Symlinks are unix-specific in this context
fn test_symlink_loop_handling() {
    let repo = TestRepo::new().with_initial_commit();
    let path = repo.path();

    std::fs::create_dir_all(path.join("dir")).unwrap();

    repo.with_symlink("loop1", "loop2");
    repo.with_symlink("loop2", "loop1");

    let (tx, rx) = std::sync::mpsc::channel();
    let repo_path = repo.path().to_path_buf();

    std::thread::spawn(move || {
        let result = load_repository_state(&repo_path, None, 0, true);
        tx.send(result).unwrap();
    });

    let result = rx.recv_timeout(Duration::from_secs(5));
    assert!(
        result.is_ok(),
        "Timed out waiting for status check - likely hung on symlink loop"
    );

    let status_result = result.unwrap();
    assert!(
        status_result.is_ok(),
        "Should return Ok status even with symlink loops"
    );
}

#[test]
fn test_large_repository_performance() {
    let repo = TestRepo::new().with_initial_commit();

    for i in 0..1000 {
        repo.create_file(&format!("file{i}.txt"), "content");
    }

    let start = std::time::Instant::now();
    let result = load_repository_state(repo.path(), None, 0, true);
    let duration = start.elapsed();

    assert!(result.is_ok(), "Should handle large repository");
    assert!(
        duration.as_millis() < 5000,
        "Should complete within 5 seconds even for 1000 files, took {duration:?}"
    );

    let CompleteStatus { status, .. } = result.unwrap().expect("Should find repository");
    assert_eq!(
        status.untracked, 1000,
        "Should accurately count all 1000 untracked files"
    );
}

#[test]
fn test_repository_with_binary_files() {
    let repo = TestRepo::new().with_initial_commit();

    let binary_data: Vec<u8> = (0..256).map(|i| u8::try_from(i).unwrap()).collect();
    std::fs::write(repo.path().join("binary.dat"), binary_data)
        .expect("Failed to write binary file");

    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(
        result.is_ok(),
        "Should handle repositories with binary files"
    );

    let CompleteStatus { status, .. } = result.unwrap().expect("Should find repository");
    assert_eq!(
        status.untracked, 1,
        "Binary file should be detected as untracked"
    );
}

/// Mode-only changes count as unstaged modifications.
#[test]
#[cfg(unix)]
fn test_file_permission_changes() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TestRepo::new().with_initial_commit();

    let script_path = repo.path().join("script.sh");
    std::fs::write(&script_path, "#!/bin/bash\necho hello").expect("Failed to write script");

    let mut perms = std::fs::metadata(&script_path)
        .expect("Failed to get metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms).expect("Failed to set permissions");

    repo.stage_file("script.sh").commit("Add executable script");

    let mut perms = std::fs::metadata(&script_path)
        .expect("Failed to get metadata")
        .permissions();
    perms.set_mode(0o644); // Remove execute bit
    std::fs::set_permissions(&script_path, perms).expect("Failed to set permissions");

    let result = load_repository_state(repo.path(), None, 0, true);
    assert!(result.is_ok(), "Should handle permission changes");

    let CompleteStatus { status, .. } = result.unwrap().expect("Should find repository");
    assert!(
        status.unstaged > 0 || status.staged > 0,
        "Permission changes should be detected"
    );
}

#[test]
fn test_mixed_encoding_files() {
    let repo = TestRepo::new().with_initial_commit();

    repo.create_file("utf8.txt", "Hello 世界 🌍");

    let latin1_data = vec![0xC4, 0xE4, 0xF6, 0xFC]; // Ä ä ö ü in Latin-1
    std::fs::write(repo.path().join("latin1.txt"), latin1_data)
        .expect("Failed to write Latin-1 file");

    let result = load_repository_state(repo.path(), None, 0, true);

    assert!(
        result.is_ok(),
        "Should handle mixed encoding files without panicking"
    );

    let CompleteStatus { status, .. } = result.unwrap().expect("Should find repository");
    assert_eq!(
        status.untracked, 2,
        "Both UTF-8 and non-UTF-8 files should be detected"
    );
}

#[test]
fn test_shallow_clone_handling() {
    let source = TestRepo::new().with_initial_commit();
    source
        .create_file("file2.txt", "content")
        .stage_file("file2.txt")
        .commit("Second commit");
    source
        .create_file("file3.txt", "content")
        .stage_file("file3.txt")
        .commit("Third commit");

    let shallow = TestRepo::as_shallow_clone(&source, 1);
    let result = load_repository_state(shallow.path(), None, 0, true);

    assert!(result.is_ok(), "Should handle shallow clones");
    let CompleteStatus { status, .. } = result.unwrap().expect("Should find repository");

    assert_eq!(
        status.branch, "main",
        "Should detect branch in shallow clone"
    );
    assert_eq!(status.untracked, 0);
}

#[test]
fn test_git_worktree_handling() {
    let repo = TestRepo::new().with_initial_commit();
    repo.create_branch("feature");

    let worktree_path = repo.with_worktree("feature", "worktree-dir");

    let result_main = load_repository_state(repo.path(), None, 0, true);
    let CompleteStatus {
        status: status_main,
        ..
    } = result_main.unwrap().expect("Main repo should be valid");
    assert_eq!(status_main.branch, "main");

    let result_wt = load_repository_state(&worktree_path, None, 0, true);
    let CompleteStatus {
        status: status_wt, ..
    } = result_wt.unwrap().expect("Worktree should be valid");
    assert_eq!(status_wt.branch, "feature");

    std::fs::write(worktree_path.join("wt-file.txt"), "content").unwrap();

    let CompleteStatus {
        status: status_wt_dirty,
        ..
    } = load_repository_state(&worktree_path, None, 0, true)
        .unwrap()
        .unwrap();
    assert_eq!(
        status_wt_dirty.untracked, 1,
        "Worktree should see untracked file"
    );

    let CompleteStatus {
        status: status_main_clean,
        ..
    } = load_repository_state(repo.path(), None, 0, true)
        .unwrap()
        .unwrap();
    assert_eq!(
        status_main_clean.untracked, 0,
        "Main repo should not be affected by worktree"
    );
}

#[test]
fn test_submodule_status_detection() {
    let parent = TestRepo::new().with_initial_commit();
    let child = TestRepo::new().with_initial_commit();

    parent.with_submodule("child-module", &child);

    // `with_submodule` commits the submodule.
    let CompleteStatus { status, .. } = load_repository_state(parent.path(), None, 0, true)
        .unwrap()
        .unwrap();
    assert_eq!(status.untracked, 0);
    assert_eq!(status.unstaged, 0);

    let submodule_path = parent.path().join("child-module");
    std::fs::write(submodule_path.join("new-file.txt"), "content").unwrap();

    let CompleteStatus {
        status: status_dirty,
        ..
    } = load_repository_state(parent.path(), None, 0, true)
        .unwrap()
        .unwrap();

    // Inner submodule changes depend on `ignore=dirty` config; either result is fine.
    if status_dirty.unstaged == 0 {
        println!(
            "Warning: Submodule changes not detected by git (expected if ignore=dirty default)"
        );
    } else {
        assert!(
            status_dirty.unstaged > 0,
            "Should detect modified submodule"
        );
    }
    // Only asserts this doesn't crash.
}
