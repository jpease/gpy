use super::*;

#[test]
fn git_event_refreshes_cache_immediately() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();

    let head_path = repo.join(".git/HEAD");
    let event = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(head_path),
        },
        repo: repo.clone(),
    };

    handle_file_event(&ctx, &event, &config);
    assert!(ctx.cache.get(&repo).is_some());

    let untracked_path = repo.join("new-file.txt");
    fs::write(&untracked_path, "hello world").expect("create untracked file");

    handle_file_event(&ctx, &event, &config);

    let updated_status = ctx
        .cache
        .get(&repo)
        .expect("updated status should be cached after event");
    assert!(
        updated_status.untracked >= 1,
        "expected untracked file to be reflected immediately"
    );
}

/// A repo under `git.skip_paths` must refresh nothing.
#[test]
fn git_event_for_skipped_repo_writes_no_cache() {
    let (tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let mut config = Config::default();
    config.git.skip_paths = vec![
        fs::canonicalize(tmp.path())
            .expect("canonical tmp")
            .to_string_lossy()
            .into_owned(),
    ];

    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert!(
        ctx.cache.get(&repo).is_none(),
        "a skipped repo must not gain a git cache entry from a watcher event"
    );

    handle_file_event(&ctx, &whole_repo_event(&repo), &Config::default());
    assert!(
        ctx.cache.get(&repo).is_some(),
        "control: the same event refreshes the cache without the skip entry"
    );
}

#[test]
fn git_event_clears_cache_when_repository_unavailable() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();

    let head_path = repo.join(".git/HEAD");
    let event = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(head_path),
        },
        repo: repo.clone(),
    };

    handle_file_event(&ctx, &event, &config);
    assert!(ctx.cache.get(&repo).is_some());

    let git_dir = repo.join(".git");
    let backup = repo.join(".git.bak");
    fs::rename(&git_dir, &backup).expect("rename git dir");

    handle_file_event(&ctx, &event, &config);

    assert!(
        ctx.cache.get(&repo).is_none(),
        "cache should be cleared when repository cannot be read"
    );
    let _ = fs::rename(&backup, &git_dir);
}

#[test]
/// `git pack-refs` can move any ref into `packed-refs`, so it needs a full scan.
fn packed_refs_event_refreshes_cache_immediately() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();

    let packed_refs_path = repo.join(".git/packed-refs");
    let event = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(packed_refs_path),
        },
        repo: repo.clone(),
    };

    handle_file_event(&ctx, &event, &config);
    assert!(
        ctx.cache.get(&repo).is_some(),
        "packed-refs event should refresh the git status cache"
    );
}

#[test]
/// The repo root as a path would become an empty pathspec git rejects.
fn repo_root_path_yields_no_incremental_hint() {
    let repo = PathBuf::from("/tmp/example-repo");
    assert!(
        git_paths_hint(&GitPaths::single(repo.clone()), &repo).is_none(),
        "an event whose path IS the repo root must force a full scan"
    );
}

#[test]
fn global_git_files_yield_no_incremental_hint() {
    let repo = PathBuf::from("/tmp/example-repo");
    for global in [
        ".gitignore",
        ".git/config",
        ".git/info/exclude",
        ".git/HEAD",
        ".git/packed-refs",
    ] {
        let path = repo.join(global);
        assert!(
            git_paths_hint(&GitPaths::single(path), &repo).is_none(),
            "{global} affects other files, so it must force a full scan"
        );
    }
}

#[test]
/// Worktree files named like `.git` internals (`config`, `HEAD`) still hint.
fn bare_named_worktree_files_still_hint_incrementally() {
    let repo = PathBuf::from("/tmp/example-repo");
    for worktree_relative in [
        "config",
        "src/config",
        "HEAD",
        "docs/HEAD",
        "exclude",
        "packed-refs",
    ] {
        let path = repo.join(worktree_relative);
        assert_eq!(
            git_paths_hint(&GitPaths::single(path.clone()), &repo),
            Some(vec![path]),
            "{worktree_relative} is a worktree file, not a .git internal: \
             it must not force a whole-repo scan"
        );
    }
}

#[test]
fn real_git_global_files_and_gitignore_escalate() {
    let repo = PathBuf::from("/tmp/example-repo");
    for global in [
        ".git/config",
        ".git/info/exclude",
        ".git/HEAD",
        ".git/packed-refs",
        ".gitignore",
        "src/.gitignore",
    ] {
        let path = repo.join(global);
        assert!(
            git_paths_hint(&GitPaths::single(path), &repo).is_none(),
            "{global} affects other files, so it must still force a full scan"
        );
    }
}

#[test]
fn tracked_file_yields_single_path_hint() {
    let repo = PathBuf::from("/tmp/example-repo");
    let path = repo.join("src/main.rs");
    assert_eq!(
        git_paths_hint(&GitPaths::single(path.clone()), &repo),
        Some(vec![path]),
        "an ordinary file change should hint an incremental single-file scan"
    );
}

#[test]
fn incremental_relative_path_rejects_repo_root_and_outsiders() {
    let repo = PathBuf::from("/tmp/example-repo");
    assert_eq!(
        incremental_relative_path(&repo, &repo),
        None,
        "the repo root has no relative path to scan incrementally"
    );
    assert_eq!(
        incremental_relative_path(Path::new("/tmp/other/file.rs"), &repo),
        None,
        "a path outside the repo has no relative path to scan incrementally"
    );
    let inside = repo.join("src/main.rs");
    assert_eq!(
        incremental_relative_path(&inside, &repo),
        Some(Path::new("src/main.rs")),
        "a path inside the repo scans relative to the repo root"
    );
}

#[test]
fn rescan_event_at_repo_root_refreshes_cache() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();

    fs::write(repo.join("new-file.txt"), "hello world").expect("create untracked file");

    let event = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(repo.clone()),
        },
        repo: repo.clone(),
    };
    handle_file_event(&ctx, &event, &config);

    let status = ctx
        .cache
        .get(&repo)
        .expect("rescan event should refresh the git status cache");
    assert!(
        status.untracked >= 1,
        "rescan should full-scan the repo and see the untracked file"
    );
}

#[test]
fn several_ordinary_files_yield_a_multi_path_hint() {
    let repo = PathBuf::from("/tmp/example-repo");
    let first = repo.join("a.txt");
    let second = repo.join("src/b.rs");

    assert_eq!(
        git_paths_hint(&GitPaths::Paths(vec![first.clone(), second.clone()]), &repo),
        Some(vec![first, second]),
        "a coalesced burst of ordinary files should scan every one of them"
    );
}

#[test]
/// One global file changes status of unnamed files, so the whole burst escalates.
fn one_global_path_escalates_the_whole_burst() {
    let repo = PathBuf::from("/tmp/example-repo");
    let paths = GitPaths::Paths(vec![repo.join("a.txt"), repo.join(".git/HEAD")]);

    assert!(
        git_paths_hint(&paths, &repo).is_none(),
        "a HEAD write alongside worktree edits must still refresh branch state"
    );
}

#[test]
/// `git status` omits `.git` internals, so a pathspec scan would read them as clean.
fn git_internal_paths_escalate_to_a_full_scan() {
    let repo = PathBuf::from("/tmp/example-repo");
    for internal in [
        ".git/index",
        ".git/refs/heads/main",
        ".git/refs/remotes/origin/main",
        ".git/refs/stash",
        ".git/ORIG_HEAD",
        ".git/MERGE_HEAD",
        ".git/REBASE_HEAD",
        ".git/CHERRY_PICK_HEAD",
        ".git/REVERT_HEAD",
        ".git/BISECT_LOG",
        ".git/FETCH_HEAD",
    ] {
        let paths = GitPaths::single(repo.join(internal));
        assert!(
            git_paths_hint(&paths, &repo).is_none(),
            "{internal} is invisible to git status and must force a full scan"
        );
    }
}

#[test]
/// A nested repo's `.git` file seen via the parent repo still escalates.
fn nested_git_internal_paths_still_escalate_to_a_full_scan() {
    let repo = PathBuf::from("/tmp/example-repo");
    for nested in [
        "nested-repo/.git/HEAD",
        "vendor/nested-repo/.git/index",
        "a/b/.git/config",
    ] {
        let paths = GitPaths::single(repo.join(nested));
        assert!(
            git_paths_hint(&paths, &repo).is_none(),
            "{nested} has `.git` as a non-first component and must still force a full scan"
        );
    }
}

#[test]
fn external_git_dir_paths_yield_no_incremental_hint() {
    // The submodule git dir is not under the attributed working root.
    let submodule_root = PathBuf::from("/tmp/superproject/sub");
    let submodule_git_dir_index = PathBuf::from("/tmp/superproject/.git/modules/sub/index");
    assert!(
        git_paths_hint(&GitPaths::single(submodule_git_dir_index), &submodule_root).is_none(),
        "a submodule's git-dir path must force a full scan of the submodule's working root"
    );

    let worktree_root = PathBuf::from("/tmp/other-worktree");
    let worktree_git_dir_index = PathBuf::from("/tmp/main-repo/.git/worktrees/feature/index");
    assert!(
        git_paths_hint(&GitPaths::single(worktree_git_dir_index), &worktree_root).is_none(),
        "a linked worktree's git-dir path must force a full scan of the worktree's working root"
    );
}

#[test]
fn whole_repo_paths_yield_no_incremental_hint() {
    let repo = PathBuf::from("/tmp/example-repo");
    assert!(git_paths_hint(&GitPaths::WholeRepo, &repo).is_none());
}
