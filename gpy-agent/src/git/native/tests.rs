use super::*;
use crate::git::RebaseProgress;
use std::time::SystemTime;
use tempfile::TempDir;

/// `git config --local --get-all core.untrackedCache` as seen from `path`.
fn untracked_cache_values(path: &Path) -> String {
    let output = Command::new("git")
        .args(["config", "--local", "--get-all", "core.untrackedCache"])
        .current_dir(path)
        .output()
        .expect("run git config");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn does_not_override_explicit_untracked_cache_false() {
    let (_dir, path) = init_test_repo();
    let status = Command::new("git")
        .args(["config", "core.untrackedCache", "false"])
        .current_dir(&path)
        .status()
        .expect("run git config");
    assert!(status.success());

    NativeGitBackend::get_complete_status(&path, None, 100, false, Duration::from_secs(5))
        .expect("status");

    assert_eq!(untracked_cache_values(&path), "false");
}

#[test]
fn enables_untracked_cache_when_unset() {
    let (_dir, path) = init_test_repo();
    // A user/system-level setting would (correctly) stop gpy enabling it.
    if NativeGitBackend::is_untracked_cache_configured(&path, NativeGitBackend::default_deadline())
    {
        return;
    }

    NativeGitBackend::get_complete_status(&path, None, 100, false, Duration::from_secs(5))
        .expect("status");

    assert_eq!(untracked_cache_values(&path), "true");
}

#[test]
fn parse_msgnum_table() {
    let cases: &[(&str, Option<u32>)] = &[
        ("3", Some(3)),
        ("3\n", Some(3)),
        ("  3  \n", Some(3)),
        ("", None),
        ("not-a-number", None),
        ("-1", None),
    ];
    for (input, expected) in cases {
        assert_eq!(parse_msgnum(input), *expected, "input: {input:?}");
    }
}

#[test]
fn count_todo_lines_table() {
    let cases: &[(&str, Option<u32>)] = &[
        ("pick abc123 msg\npick def456 msg\n", Some(2)),
        ("pick abc123 msg\n# comment\n\nexec make test\n", Some(2)),
        ("# only comments\n\n", Some(0)),
        ("", Some(0)),
    ];
    for (input, expected) in cases {
        assert_eq!(count_todo_lines(input), *expected, "input: {input:?}");
    }
}

#[test]
fn parse_sequencer_command_table() {
    let cases: &[(&str, Option<RepositoryState>)] = &[
        ("pick abc x\n", Some(RepositoryState::CherryPicking)),
        ("# c\nrevert abc x\n", Some(RepositoryState::Reverting)),
        ("p abc\n", Some(RepositoryState::CherryPicking)),
        ("\n  \nrevert abc\n", Some(RepositoryState::Reverting)),
        ("exec true\n", None),
        ("# only\n", None),
        ("", None),
    ];
    for (input, expected) in cases {
        assert_eq!(
            parse_sequencer_command(input),
            *expected,
            "input: {input:?}"
        );
    }
}

/// Minimal git repo fixture (the `tests/fixtures` one isn't reachable from unit tests).
///
/// # Panics
///
/// Panics if git is not available or repository setup fails.
fn init_test_repo() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("create temp dir");
    let path = fs::canonicalize(dir.path()).expect("canonicalize temp dir");

    let run = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(&path)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run(&["init"]);
    run(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    run(&["config", "user.name", "Test User"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "commit.gpgsign", "false"]);
    fs::write(path.join("file.txt"), "content\n").expect("write file.txt");
    run(&["add", "."]);
    run(&["commit", "-m", "init"]);

    (dir, path)
}

/// Write an executable POSIX-`sh` script to `dir/name` for `GIT_SEQUENCE_EDITOR`.
/// Avoids `sed -i`, whose argument differs between BSD and GNU.
#[cfg(unix)]
fn write_sequence_editor_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let script = dir.join(name);
    fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write sequence-editor script");
    let mut perms = fs::metadata(&script)
        .expect("stat sequence-editor script")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).expect("chmod sequence-editor script executable");
    script
}

#[test]
fn detached_head_sets_flag_not_branch_prefix() {
    let (_dir, path) = init_test_repo();

    let rev_parse = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&path)
        .output()
        .expect("rev-parse HEAD");
    let sha = String::from_utf8_lossy(&rev_parse.stdout).trim().to_owned();

    let checkout = Command::new("git")
        .args(["checkout", &sha])
        .current_dir(&path)
        .output()
        .expect("checkout detached commit");
    assert!(checkout.status.success());

    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");

    assert!(status.detached, "expected detached flag to be set");
    assert!(
        !status.branch.starts_with("HEAD@"),
        "branch must not carry the legacy HEAD@ prefix, got {:?}",
        status.branch
    );
}

/// `git.timeout_seconds` bounds the git subprocess: a tiny timeout fails, a generous one succeeds.
#[test]
fn get_complete_status_respects_configured_timeout() {
    let (_dir, path) = init_test_repo();

    let short_start = std::time::Instant::now();
    let short_timeout_result =
        NativeGitBackend::get_complete_status(&path, None, 0, true, Duration::from_nanos(1));
    let short_elapsed = short_start.elapsed();
    assert!(
        short_timeout_result.is_err(),
        "a 1ns timeout must bound the real git subprocess and time out, got {short_timeout_result:?} after {short_elapsed:?}"
    );

    let generous_start = std::time::Instant::now();
    let generous_timeout_result =
        NativeGitBackend::get_complete_status(&path, None, 0, true, Duration::from_secs(5));
    let generous_elapsed = generous_start.elapsed();
    assert!(
        generous_timeout_result.is_ok(),
        "a 5s timeout on the same repo must still succeed, got {generous_timeout_result:?} after {generous_elapsed:?}"
    );
}

#[test]
fn stash_count_reflects_stash_list() {
    let (_dir, path) = init_test_repo();

    for i in 0_u32..2 {
        fs::write(path.join("file.txt"), format!("content {i}\n")).expect("modify tracked file");
        let stash = Command::new("git")
            .args(["stash", "push", "-m", &format!("stash {i}")])
            .current_dir(&path)
            .output()
            .expect("stash push");
        assert!(stash.status.success());
    }

    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");

    assert_eq!(status.stash_count, 2);
}

#[test]
fn stash_count_is_zero_when_stash_disabled() {
    let (_dir, path) = init_test_repo();

    fs::write(path.join("file.txt"), "modified\n").expect("modify tracked file");
    let stash = Command::new("git")
        .args(["stash", "push", "-m", "stash"])
        .current_dir(&path)
        .output()
        .expect("stash push");
    assert!(stash.status.success());

    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        None,
        0,
        false,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");

    assert_eq!(
        status.stash_count, 0,
        "stash capture must be skipped when stash_enabled is false"
    );
}

#[test]
fn rebase_progress_present_for_interactive_rebase() {
    let (_dir, path) = init_test_repo();

    let rebase_merge = path.join(".git").join("rebase-merge");
    fs::create_dir_all(&rebase_merge).expect("create rebase-merge dir");
    fs::write(rebase_merge.join("msgnum"), "3\n").expect("write msgnum");
    fs::write(rebase_merge.join("end"), "5\n").expect("write end");
    // `total` = `msgnum` + remaining todo lines: 3 + 2 = 5.
    fs::write(
        rebase_merge.join("git-rebase-todo"),
        "pick aaaaaaa commit four\npick bbbbbbb commit five\n",
    )
    .expect("write git-rebase-todo");

    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");

    assert_eq!(
        status.rebase_progress,
        Some(RebaseProgress { step: 3, total: 5 })
    );
}

/// Add commits 2..=5 on top of `init_test_repo()`'s existing "init" commit
/// so the repo has a 5-step rebase plan.
#[cfg(unix)]
fn add_numbered_commits(path: &Path, range: std::ops::RangeInclusive<u32>) {
    for i in range {
        fs::write(path.join("file.txt"), format!("content {i}\n")).expect("modify tracked file");
        let commit = Command::new("git")
            .args(["commit", "-am", &format!("commit {i}")])
            .current_dir(path)
            .output()
            .expect("commit");
        assert!(
            commit.status.success(),
            "commit {i} failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
    }
}

/// Start `git rebase -i --root`, changing the first `pick` to `edit` so the rebase stops
/// after step 1.
///
/// Asserts `msgnum` = 1, `end` = 5 and returns the `.git/rebase-merge` dir.
#[cfg(unix)]
fn start_rebase_paused_at_first_edit(dir: &TempDir, path: &Path) -> PathBuf {
    let edit_first = write_sequence_editor_script(
        dir.path(),
        "edit_first_pick.sh",
        r#"todo="$1"
tmp="$todo.tmp"
: > "$tmp"
changed=0
while IFS= read -r line || [ -n "$line" ]; do
case "$line" in
    pick\ *)
        if [ "$changed" -eq 0 ]; then
            echo "edit ${line#pick }" >> "$tmp"
            changed=1
        else
            echo "$line" >> "$tmp"
        fi
        ;;
    *)
        echo "$line" >> "$tmp"
        ;;
esac
done < "$todo"
mv "$tmp" "$todo""#,
    );

    let status = Command::new("git")
        .args(["rebase", "-i", "--root"])
        .env("GIT_SEQUENCE_EDITOR", &edit_first)
        .current_dir(path)
        .status()
        .expect("start interactive rebase, paused at the first edit");
    assert!(
        status.success(),
        "rebase -i --root should stop cleanly at the first edit"
    );

    let rebase_merge = path.join(".git").join("rebase-merge");
    let msgnum_before = fs::read_to_string(rebase_merge.join("msgnum"))
        .expect("read msgnum")
        .trim()
        .to_owned();
    assert_eq!(
        msgnum_before, "1",
        "setup: rebase should have stopped after step 1"
    );
    let end_before = fs::read_to_string(rebase_merge.join("end"))
        .expect("read end")
        .trim()
        .to_owned();
    assert_eq!(
        end_before, "5",
        "setup: end should reflect the original 5-commit plan"
    );

    rebase_merge
}

/// Run `git rebase --edit-todo`, dropping 2 of the remaining 4 `pick`s;
/// `end` stays at "5".
#[cfg(unix)]
fn edit_todo_dropping_two_picks(dir: &TempDir, path: &Path) {
    let drop_two = write_sequence_editor_script(
        dir.path(),
        "drop_two_picks.sh",
        r#"todo="$1"
tmp="$todo.tmp"
: > "$tmp"
kept=0
while IFS= read -r line || [ -n "$line" ]; do
case "$line" in
    pick\ *)
        kept=$((kept + 1))
        if [ "$kept" -le 2 ]; then
            echo "$line" >> "$tmp"
        fi
        ;;
    *)
        echo "$line" >> "$tmp"
        ;;
esac
done < "$todo"
mv "$tmp" "$todo""#,
    );

    let status = Command::new("git")
        .args(["rebase", "--edit-todo"])
        .env("GIT_SEQUENCE_EDITOR", &drop_two)
        .current_dir(path)
        .status()
        .expect("edit the rebase todo");
    assert!(status.success(), "rebase --edit-todo should succeed");
}

/// `total` reflects `git-rebase-todo` after `--edit-todo`, not stale
/// `rebase-merge/end`: `git status` reports 1 done + 2 remaining = 3.
#[test]
#[cfg(unix)]
fn rebase_progress_total_reflects_edited_todo_after_edit_todo() {
    let (dir, path) = init_test_repo();
    add_numbered_commits(&path, 2_u32..=5);
    let rebase_merge = start_rebase_paused_at_first_edit(&dir, &path);
    edit_todo_dropping_two_picks(&dir, &path);

    let end_after = fs::read_to_string(rebase_merge.join("end"))
        .expect("read end")
        .trim()
        .to_owned();
    assert_eq!(
        end_after, "5",
        "self-check: git must not rewrite `end` on --edit-todo, or this test proves nothing"
    );

    let progress = NativeGitBackend::get_rebase_progress(&path)
        .expect("get_rebase_progress should report progress for the paused rebase");

    assert_ne!(
        progress.total, 5,
        "total must not equal the stale pre-edit end value"
    );
    assert_eq!(
        progress,
        RebaseProgress { step: 1, total: 3 },
        "total must reflect the edited plan: 1 done + 2 remaining picks"
    );

    let abort = Command::new("git")
        .args(["rebase", "--abort"])
        .current_dir(&path)
        .output()
        .expect("abort rebase");
    assert!(
        abort.status.success(),
        "rebase --abort should succeed: {}",
        String::from_utf8_lossy(&abort.stderr)
    );
}

#[test]
fn rebase_progress_none_for_non_interactive_rebase() {
    let (_dir, path) = init_test_repo();

    let rebase_apply = path.join(".git").join("rebase-apply");
    fs::create_dir_all(&rebase_apply).expect("create rebase-apply dir");
    fs::write(rebase_apply.join("next"), "1\n").expect("write next");
    fs::write(rebase_apply.join("last"), "3\n").expect("write last");

    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");

    assert!(
        status.rebase_progress.is_none(),
        "am-style rebase-apply must not populate step/total"
    );
}

#[test]
fn get_repo_state_returns_applying_for_conflicted_git_am() {
    let (_dir, path) = init_test_repo();

    // Realistic conflicted `git am -3` layout: rebase-apply has the same
    // `next`/`last` as `rebase --apply`, so detection keys off `applying`.
    let rebase_apply = path.join(".git").join("rebase-apply");
    fs::create_dir_all(&rebase_apply).expect("create rebase-apply dir");
    for name in [
        "0001",
        "abort-safety",
        "apply-opt",
        "applying",
        "author-script",
        "final-commit",
        "info",
        "keep",
        "last",
        "messageid",
        "msg",
        "next",
        "patch",
        "quiet",
        "quoted-cr",
        "scissors",
        "sign",
        "threeway",
        "utf8",
    ] {
        fs::write(rebase_apply.join(name), "").expect("write am marker file");
    }

    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::Applying)
    );

    // am detection must not affect interactive-rebase progress.
    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");
    assert!(
        status.rebase_progress.is_none(),
        "am-style rebase-apply must not populate step/total"
    );
}

/// Run git in `path`, returning whether it succeeded (conflicting
/// cherry-picks/reverts exit non-zero by design).
fn git_status_ok(path: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "core.editor=true"])
        .args(args)
        .current_dir(path)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"))
        .status
        .success()
}

/// Build `main` (conflicting edit of `file.txt`) and `other` (three
/// commits, the first conflicting), leaving `main` checked out.
fn setup_multi_commit_conflict(path: &Path) {
    for (args, file, body) in [
        (&["checkout", "-q", "-b", "other"][..], None, ""),
        (&["commit", "-q", "-am", "o1"][..], Some("file.txt"), "o1\n"),
        (&["add", "g"][..], Some("g"), "o2\n"),
        (&["commit", "-q", "-m", "o2"][..], None, ""),
        (&["add", "h"][..], Some("h"), "o3\n"),
        (&["commit", "-q", "-m", "o3"][..], None, ""),
        (&["checkout", "-q", "main"][..], None, ""),
        (&["commit", "-q", "-am", "m"][..], Some("file.txt"), "m\n"),
    ] {
        if let Some(name) = file {
            fs::write(path.join(name), body).expect("write fixture file");
        }
        assert!(git_status_ok(path, args), "git {args:?} failed");
    }
}

#[test]
fn get_repo_state_detects_cherry_pick_sequencer_after_manual_commit() {
    let (_dir, path) = init_test_repo();
    setup_multi_commit_conflict(&path);

    assert!(
        !git_status_ok(&path, &["cherry-pick", "other~2", "other~1", "other"]),
        "first pick must conflict"
    );
    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::CherryPicking)
    );
    fs::write(path.join("file.txt"), "resolved\n").expect("resolve conflict");
    assert!(git_status_ok(&path, &["add", "file.txt"]));
    assert!(git_status_ok(&path, &["commit", "-q", "--no-edit"]));
    assert!(
        !path.join(".git").join("CHERRY_PICK_HEAD").exists(),
        "manual commit removes CHERRY_PICK_HEAD"
    );
    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::CherryPicking)
    );
    assert!(git_status_ok(&path, &["cherry-pick", "--quit"]));
    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        None
    );
}

#[test]
fn get_repo_state_detects_revert_sequencer_after_manual_commit() {
    let (_dir, path) = init_test_repo();
    for (name, body, msg) in [
        ("file.txt", "a1\n", "A"),
        ("g", "b\n", "B"),
        ("file.txt", "c\n", "C"),
    ] {
        fs::write(path.join(name), body).expect("write fixture file");
        assert!(git_status_ok(&path, &["add", name]));
        assert!(git_status_ok(&path, &["commit", "-q", "-m", msg]));
    }
    // Reverting A conflicts with C; B remains in the sequence.
    assert!(
        !git_status_ok(&path, &["revert", "HEAD~2", "HEAD~1"]),
        "first revert must conflict"
    );
    fs::write(path.join("file.txt"), "resolved\n").expect("resolve conflict");
    assert!(git_status_ok(&path, &["add", "file.txt"]));
    assert!(git_status_ok(&path, &["commit", "-q", "--no-edit"]));
    assert!(
        !path.join(".git").join("REVERT_HEAD").exists(),
        "manual commit removes REVERT_HEAD"
    );
    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::Reverting)
    );
}

#[test]
fn get_repo_state_detects_reftable_cherry_pick_and_revert() {
    let dir = TempDir::new().expect("create temp dir");
    let path = fs::canonicalize(dir.path()).expect("canonicalize temp dir");
    if !git_status_ok(&path, &["init", "-q", "--ref-format=reftable"]) {
        return; // git too old for reftable
    }
    for args in [
        &["config", "user.name", "T"][..],
        &["config", "user.email", "t@e"][..],
        &["symbolic-ref", "HEAD", "refs/heads/main"][..],
    ] {
        assert!(git_status_ok(&path, args));
    }
    for (args, body) in [
        (&["commit", "-q", "-am", "base"][..], "base\n"),
        (&["checkout", "-q", "-b", "other"][..], ""),
        (&["commit", "-q", "-am", "o"][..], "o\n"),
        (&["checkout", "-q", "main"][..], ""),
        (&["commit", "-q", "-am", "m"][..], "m\n"),
    ] {
        if !body.is_empty() {
            fs::write(path.join("f"), body).expect("write f");
            if args.contains(&"base") {
                assert!(git_status_ok(&path, &["add", "f"]));
            }
        }
        assert!(git_status_ok(&path, args), "git {args:?} failed");
    }
    assert!(!git_status_ok(&path, &["cherry-pick", "other"]));
    fs::write(path.join("f"), "r\n").expect("resolve");
    assert!(git_status_ok(&path, &["add", "f"]));
    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::CherryPicking)
    );
    assert!(git_status_ok(&path, &["cherry-pick", "--abort"]));
    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        None
    );
    assert!(git_status_ok(&path, &["revert", "--no-commit", "HEAD"]));
    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::Reverting)
    );
}

#[test]
fn get_repo_state_returns_rebasing_for_apply_backend_rebase() {
    let (_dir, path) = init_test_repo();

    // `git rebase --apply` shares rebase-apply with `git am` but never
    // writes the `applying` marker; it must keep resolving to Rebasing
    // rather than being misread as an am in progress.
    let rebase_apply = path.join(".git").join("rebase-apply");
    fs::create_dir_all(&rebase_apply).expect("create rebase-apply dir");
    fs::write(rebase_apply.join("next"), "1\n").expect("write next");
    fs::write(rebase_apply.join("last"), "3\n").expect("write last");

    assert_eq!(
        NativeGitBackend::get_repo_state(&path, NativeGitBackend::default_deadline()),
        Some(RepositoryState::Rebasing)
    );
}

#[test]
fn detached_stashed_and_mid_rebase_are_captured_simultaneously() {
    let (_dir, path) = init_test_repo();

    fs::write(path.join("file.txt"), "modified\n").expect("modify tracked file");
    let stash = Command::new("git")
        .args(["stash", "push", "-m", "stash"])
        .current_dir(&path)
        .output()
        .expect("stash push");
    assert!(stash.status.success());

    let rev_parse = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&path)
        .output()
        .expect("rev-parse HEAD");
    let sha = String::from_utf8_lossy(&rev_parse.stdout).trim().to_owned();
    let checkout = Command::new("git")
        .args(["checkout", &sha])
        .current_dir(&path)
        .output()
        .expect("checkout detached commit");
    assert!(checkout.status.success());

    let rebase_merge = path.join(".git").join("rebase-merge");
    fs::create_dir_all(&rebase_merge).expect("create rebase-merge dir");
    fs::write(rebase_merge.join("msgnum"), "1\n").expect("write msgnum");
    fs::write(rebase_merge.join("end"), "2\n").expect("write end");
    // 1 done + 1 remaining pick = total 2.
    fs::write(
        rebase_merge.join("git-rebase-todo"),
        "pick aaaaaaa commit two\n",
    )
    .expect("write git-rebase-todo");

    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        None,
        0,
        true,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");

    assert!(status.detached, "expected detached flag to be set");
    assert_eq!(status.stash_count, 1);
    assert_eq!(
        status.rebase_progress,
        Some(RebaseProgress { step: 1, total: 2 })
    );
}

#[test]
fn test_find_repo_root_nonexistent() {
    // A bare `/...` path resolves to the current drive on Windows.
    let nonexistent = std::env::temp_dir().join("gpy-nonexistent-test-dir-abc123xyz");
    let result = crate::git::find_repo_root(&nonexistent);
    assert!(result.is_none());
}

#[test]
fn resolve_overall_state_conflicts_win() {
    let state = resolve_overall_state(1, Some(RepositoryState::Merging), 5, 3, 2);
    assert_eq!(state, RepositoryState::Conflicts);
}

#[test]
fn resolve_overall_state_special_state_wins_over_dirty() {
    let state = resolve_overall_state(0, Some(RepositoryState::Merging), 5, 3, 2);
    assert_eq!(state, RepositoryState::Merging);
}

#[test]
fn resolve_overall_state_dirty() {
    let staged_dirty = resolve_overall_state(0, None, 1, 0, 0);
    assert_eq!(staged_dirty, RepositoryState::Dirty);
    let unstaged_dirty = resolve_overall_state(0, None, 0, 1, 0);
    assert_eq!(unstaged_dirty, RepositoryState::Dirty);
    let untracked_dirty = resolve_overall_state(0, None, 0, 0, 1);
    assert_eq!(untracked_dirty, RepositoryState::Dirty);
}

#[test]
fn resolve_overall_state_clean() {
    let state = resolve_overall_state(0, None, 0, 0, 0);
    assert_eq!(state, RepositoryState::Clean);
}

/// `-z` status parsing keys renames by destination and clamps ahead/behind.
/// The rename record's trailing original path must be skipped.
#[test]
fn parse_v2_output_handles_rename_and_clamps_counts() {
    let output = concat!(
        "# branch.head main\0",
        "# branch.ab +3 -1\0",
        "2 R. N... 100644 100644 100644 abc abc R100 new name.txt\0",
        "old name.txt\0",
        "? untracked.txt\0",
    );

    let result = parser::parse_v2_records(output, 2);

    assert_eq!(result.branch, BranchHead::Named("main".to_owned()));
    // Ahead 3 > max 2 -> clamped and capped; behind 1 <= max -> verbatim.
    assert_eq!(result.ahead, 2);
    assert!(result.ahead_capped);
    assert_eq!(result.behind, 1);
    assert!(!result.behind_capped);

    // The rename is one staged change, keyed by its destination.
    assert_eq!(result.staged, 1);
    assert_eq!(result.untracked, 1);
    assert!(result.files.contains_key(Path::new("new name.txt")));
    assert!(
        !result.files.contains_key(Path::new("old name.txt")),
        "the original-path record must be consumed, not stored as an entry"
    );
    assert!(
        !result
            .files
            .contains_key(Path::new("new name.txt old name.txt")),
        "destination and original paths must not be concatenated"
    );
    let renamed = result
        .files
        .get(Path::new("new name.txt"))
        .expect("renamed file present");
    assert!(renamed.staged);
}

/// `core.ignorecase` is read from the repository's config and
/// decides whether pathspecs fold case.
#[test]
fn ignores_case_reflects_the_repository_config() {
    for (setting, expected) in [("true", true), ("false", false)] {
        let (_dir, path) = init_test_repo();
        let status = Command::new("git")
            .args(["config", "core.ignorecase", setting])
            .current_dir(&path)
            .output()
            .expect("run git config")
            .status;
        assert!(status.success());
        assert_eq!(NativeGitBackend::ignores_case(&path), expected);
    }
}

fn set_ignore_case(path: &Path, setting: &str) {
    let status = Command::new("git")
        .args(["config", "core.ignorecase", setting])
        .current_dir(path)
        .status()
        .expect("run git config");
    assert!(status.success());
}

/// A changed `core.ignorecase` is served from cache until the entry expires,
/// then re-read.
#[test]
fn ignores_case_rereads_config_once_the_entry_expires() {
    let (_dir, path) = init_test_repo();
    set_ignore_case(&path, "true");
    assert!(NativeGitBackend::ignores_case(&path));

    set_ignore_case(&path, "false");
    assert!(
        NativeGitBackend::ignores_case(&path),
        "a fresh entry is served without re-reading"
    );

    IGNORE_CASE.insert_at(
        path.clone(),
        true,
        SystemTime::now() - IGNORE_CASE_TTL - Duration::from_secs(1),
    );
    assert!(
        !NativeGitBackend::ignores_case(&path),
        "an expired entry is re-read from the config"
    );
}

#[test]
fn config_caches_stay_within_capacity() {
    for index in 0..NATIVE_GIT_CONFIG_CACHE_CAPACITY + 16 {
        let repo = PathBuf::from(format!("/nonexistent/gpy-cache-bound/{index}"));
        IGNORE_CASE.insert(repo.clone(), false);
        UNTRACKED_CACHE_CHECKED.insert_if_absent(repo, ());
    }

    assert!(IGNORE_CASE.len() <= NATIVE_GIT_CONFIG_CACHE_CAPACITY);
    assert!(UNTRACKED_CACHE_CHECKED.len() <= NATIVE_GIT_CONFIG_CACHE_CAPACITY);
}

/// A pathspec is passed to git as the raw `OsStr`, so a file whose
/// name is not valid UTF-8 is still matched. Skipped where the filesystem
/// rejects such names (APFS).
#[cfg(unix)]
#[test]
fn pathspec_matches_a_non_utf8_file_name() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let (_dir, path) = init_test_repo();
    let name = PathBuf::from(OsStr::from_bytes(b"bad\xff.txt"));
    if fs::write(path.join(&name), "one\n").is_err() {
        return;
    }
    for args in [vec!["add", "-A"], vec!["commit", "-q", "-m", "non-utf8"]] {
        let output = Command::new("git")
            .args(&args)
            .current_dir(&path)
            .output()
            .expect("run git");
        assert!(output.status.success(), "git {args:?} failed");
    }
    fs::write(path.join(&name), "two\n").expect("modify non-UTF-8 file");

    let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
        &path,
        Some(&[name]),
        0,
        false,
        Duration::from_secs(GitTimeout::DEFAULT),
    )
    .expect("get_complete_status should succeed")
    .expect("repo should be detected");

    assert_eq!(
        status.unstaged, 1,
        "the edit must be seen through the pathspec"
    );
}
