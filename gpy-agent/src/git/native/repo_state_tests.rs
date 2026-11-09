use super::tests::init_test_repo;
use super::*;
use crate::git::RebaseProgress;
use std::fmt::Write as _;

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

/// Write a synthetic interactive-rebase state into `path` and return the status.
fn status_with_todo(path: &Path, todo: &str) -> RepositoryStatus {
    let rebase_merge = path.join(".git").join("rebase-merge");
    fs::create_dir_all(&rebase_merge).expect("create rebase-merge dir");
    fs::write(rebase_merge.join("msgnum"), "1\n").expect("write msgnum");
    fs::write(rebase_merge.join("git-rebase-todo"), todo).expect("write git-rebase-todo");
    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        path,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");
    status
}

#[test]
fn rebase_progress_total_counts_every_step_of_a_long_todo() {
    let (_dir, path) = init_test_repo();
    let mut todo = String::new();
    for i in 1_u32..=2000_u32 {
        writeln!(
            todo,
            "pick {i:07x} Add handling for case number {i} in the parser"
        )
        .expect("write to String");
    }

    let status = status_with_todo(&path, &todo);

    assert_eq!(
        status.rebase_progress,
        Some(RebaseProgress {
            step: 1,
            total: 2001
        })
    );
}

#[test]
fn rebase_progress_is_unknown_past_the_byte_cap() {
    let (_dir, path) = init_test_repo();
    let line = "pick 0000001 Add handling for the parser\n";
    let todo = line.repeat((REBASE_TODO_READ_CAP / line.len()) + 2);
    assert!(todo.len() > REBASE_TODO_READ_CAP);

    let status = status_with_todo(&path, &todo);

    assert_eq!(status.rebase_progress, None);
    assert_eq!(status.state, RepositoryState::Rebasing);
}

#[test]
fn count_todo_steps_counts_non_utf8_subjects() {
    assert_eq!(count_todo_steps(&b"pick abc caf\xe9\n# c\n"[..]), Some(1));
}

#[test]
fn count_todo_steps_gives_up_on_an_oversized_newline_free_line() {
    let todo = vec![b'x'; REBASE_TODO_READ_CAP + 10];
    assert_eq!(count_todo_steps(&todo[..]), None);
}
