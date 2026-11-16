use super::tests::init_test_repo;
use super::*;

/// An expired deadline makes `run_git_until` fail before it spawns anything.
#[test]
fn detached_head_falls_back_to_branch_oid_when_rev_parse_cannot_run() {
    let (_dir, repo) = init_test_repo();

    let resolved = NativeGitBackend::resolve_branch_head(
        BranchHead::Detached,
        Some("8fb1ddef85e4672c6150364dcb5df6711fd16b65"),
        &repo,
        Instant::now(),
    )
    .expect("the oid names the commit");

    assert_eq!(resolved, ("8fb1dde".to_owned(), true));
}

#[test]
fn detached_head_without_oid_or_git_is_an_error() {
    let (_dir, repo) = init_test_repo();

    let resolved =
        NativeGitBackend::resolve_branch_head(BranchHead::Detached, None, &repo, Instant::now());

    assert!(resolved.is_err());
}

/// Git's short hash is reused per commit: once resolved, the same commit
/// names itself again without running git (here, past its deadline).
#[test]
fn detached_head_short_hash_is_reused_per_commit() {
    let (_dir, repo) = init_test_repo();
    let deadline = NativeGitBackend::default_deadline();
    let oid = run_git_until(&repo, &["rev-parse", "HEAD"], deadline).expect("rev-parse HEAD");

    let first =
        NativeGitBackend::resolve_branch_head(BranchHead::Detached, Some(&oid), &repo, deadline)
            .expect("git names the commit");
    assert_eq!(
        SHORT_HEADS.get(&(repo.clone(), oid.clone())),
        Some(first.0.clone())
    );

    let again = NativeGitBackend::resolve_branch_head(
        BranchHead::Detached,
        Some(&oid),
        &repo,
        Instant::now(),
    )
    .expect("the memo names the commit");
    assert_eq!(again, first);
}
