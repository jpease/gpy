use super::tests::init_test_repo;
use super::*;

/// Run `git` in `path` with a deterministic, non-interactive environment.
fn git(path: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_EDITOR", "true")
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"))
}

fn git_ok(path: &Path, args: &[&str]) {
    let output = git(path, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Commit `contents` to `file` on the current branch.
fn commit_file(path: &Path, file: &str, contents: &str, message: &str) {
    fs::write(path.join(file), contents).expect("write file");
    git_ok(path, &["add", file]);
    git_ok(path, &["commit", "-m", message]);
}

#[test]
fn rebase_merges_conflict_is_rebasing_with_progress() {
    let (_dir, path) = init_test_repo();
    commit_file(&path, "g", "b\n", "add g");
    let base = String::from_utf8(git(&path, &["rev-parse", "HEAD"]).stdout)
        .expect("utf8 sha")
        .trim()
        .to_owned();

    git_ok(&path, &["checkout", "-b", "feat"]);
    commit_file(&path, "file.txt", "feat\n", "feat");
    git_ok(&path, &["checkout", "main"]);
    commit_file(&path, "file.txt", "main\n", "main");
    assert!(
        !git(&path, &["merge", "feat"]).status.success(),
        "merge must conflict"
    );
    fs::write(path.join("file.txt"), "merged\n").expect("resolve merge");
    git_ok(&path, &["add", "file.txt"]);
    git_ok(&path, &["commit", "-m", "M"]);

    git_ok(&path, &["checkout", "-b", "newbase", &base]);
    commit_file(&path, "g", "g2\n", "newbase");
    git_ok(&path, &["checkout", "main"]);

    let rebase = git(
        &path,
        &[
            "-c",
            "core.editor=true",
            "rebase",
            "-i",
            "--rebase-merges",
            "--onto",
            "newbase",
            &base,
            "main",
        ],
    );
    assert!(!rebase.status.success(), "rebase must stop on the conflict");
    assert!(path.join(".git/MERGE_HEAD").exists());

    fs::write(path.join("file.txt"), "merged\n").expect("resolve rebase conflict");
    git_ok(&path, &["add", "file.txt"]);

    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::Rebasing)
    );
    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");
    assert_eq!(status.state, RepositoryState::Rebasing);
    assert!(status.rebase_progress.is_some());
}

#[test]
fn merge_conflict_without_rebase_is_merging() {
    let (_dir, path) = init_test_repo();
    git_ok(&path, &["checkout", "-b", "other"]);
    commit_file(&path, "file.txt", "other\n", "other");
    git_ok(&path, &["checkout", "main"]);
    commit_file(&path, "file.txt", "main\n", "main");
    assert!(
        !git(&path, &["merge", "other"]).status.success(),
        "merge must conflict"
    );
    fs::write(path.join("file.txt"), "merged\n").expect("resolve merge");
    git_ok(&path, &["add", "file.txt"]);

    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::Merging)
    );
    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");
    assert_eq!(status.rebase_progress, None);
}
