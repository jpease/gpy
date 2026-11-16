//! How repository config reaches the agent's git commands: a repository's
//! `core.fsmonitor` command never runs, and `core.excludesFile` is resolved
//! the way git resolves it.

use super::tests::init_test_repo;
use super::*;
use std::os::unix::fs::PermissionsExt;

fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?} failed: {output:?}");
}

/// Install a repository-local `core.fsmonitor` hook that creates `marker`
/// whenever git runs it. Both live under `.git`, so the worktree stays clean.
fn install_fsmonitor_hook(repo: &Path) -> PathBuf {
    let git_dir = repo.join(".git");
    let marker = git_dir.join("fsmonitor-ran");
    let hook = git_dir.join("fsmonitor-hook.sh");
    fs::write(
        &hook,
        format!("#!/bin/sh\n: > '{}'\nexit 1\n", marker.display()),
    )
    .expect("write hook");
    let mut perms = fs::metadata(&hook).expect("stat hook").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&hook, perms).expect("chmod hook");
    git(
        repo,
        &["config", "core.fsmonitor", hook.to_str().expect("utf8")],
    );
    marker
}

/// Entering a repository must not run a command its own config names.
#[test]
fn agent_git_commands_never_run_the_repository_fsmonitor_hook() {
    let (_dir, repo) = init_test_repo();
    let marker = install_fsmonitor_hook(&repo);

    NativeGitBackend::get_complete_status(&repo, 100, true, Duration::from_secs(10))
        .expect("status")
        .expect("a repository");
    let _ = NativeGitBackend::tracked_ignored_files(&repo);
    assert!(!marker.exists(), "the agent ran the fsmonitor hook");

    // Control: plain git runs the hook, so the check above is meaningful.
    git(&repo, &["status", "--porcelain"]);
    assert!(marker.exists(), "plain git status should run the hook");
}

/// A relative `core.excludesFile` is taken against the repository root, an
/// absolute one as written.
#[test]
fn global_excludes_file_follows_core_excludes_file() {
    let (dir, repo) = init_test_repo();

    git(&repo, &["config", "core.excludesFile", "rules/ignore"]);
    assert_eq!(
        NativeGitBackend::global_excludes_file(&repo),
        Some(repo.join("rules").join("ignore"))
    );

    let absolute = dir.path().join("elsewhere").join("ignore");
    git(
        &repo,
        &[
            "config",
            "core.excludesFile",
            absolute.to_str().expect("utf8"),
        ],
    );
    assert_eq!(
        NativeGitBackend::global_excludes_file(&repo),
        Some(absolute)
    );
}
