use super::*;

/// Backend whose pathspec scans always return `scan`.
struct FixedScanBackend {
    scan: crate::git::PathspecScan,
}

impl GitBackend for FixedScanBackend {
    fn complete_status(
        &self,
        path: &Path,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>> {
        NativeGitBackend.complete_status(path, max_ahead_behind, stash_enabled, timeout)
    }

    fn pathspec_status(
        &self,
        _repo_root: &Path,
        _paths: &[PathBuf],
        _timeout: Duration,
    ) -> Result<crate::git::PathspecScan> {
        Ok(self.scan.clone())
    }
}

#[test]
/// A `.git/index` write costs one full scan, not a failed incremental round first.
fn git_index_write_triggers_exactly_one_capture_round() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();
    let backend = CountingGitBackend::new();

    let index_path = repo.join(".git/index");
    let paths_hint = git_paths_hint(&GitPaths::single(index_path), &repo);
    assert!(
        paths_hint.is_none(),
        "precondition: a .git/index write must hint a full scan"
    );

    refresh_repo_status_with(&backend, &ctx, &repo, &config, paths_hint.as_deref());

    assert_eq!(
        (backend.full_count(), backend.pathspec_count()),
        (1, 0),
        "a .git/index write must cost exactly one full scan and no pathspec scan"
    );
}

#[test]
fn ordinary_file_write_takes_the_incremental_route_in_one_capture_round() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();
    let backend = CountingGitBackend::new();

    let source = repo.join("a.txt");
    fs::write(&source, "one\n").expect("write a.txt");
    commit_all(&repo);
    fs::write(&source, "one changed\n").expect("modify a.txt");

    // The incremental route needs a warm complete-detail cache entry.
    refresh_repo_status_with(&backend, &ctx, &repo, &config, None);
    assert_eq!(backend.full_count(), 1, "warm-up scan should be one call");

    let paths_hint = git_paths_hint(&GitPaths::single(source.clone()), &repo);
    assert_eq!(
        paths_hint,
        Some(vec![source]),
        "precondition: an ordinary file write must hint a single-path incremental scan"
    );

    refresh_repo_status_with(&backend, &ctx, &repo, &config, paths_hint.as_deref());

    assert_eq!(
        (backend.full_count(), backend.pathspec_count()),
        (1, 1),
        "the warm-up full scan plus exactly one pathspec scan and no second full scan"
    );
    assert_eq!(
        backend.pathspec_paths(0),
        Some(vec![PathBuf::from("a.txt")]),
        "the incremental scan must be pathspec-limited to the changed file"
    );
}

#[test]
fn incremental_and_full_scan_share_one_backend() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();
    let backend = CountingGitBackend::new();

    let index_path = repo.join(".git/index");
    let forced_hint = vec![index_path];

    refresh_repo_status_with(&backend, &ctx, &repo, &config, Some(&forced_hint));

    assert_eq!(
        backend.pathspec_paths(0),
        Some(vec![PathBuf::from(".git/index")]),
        "the incremental call site must share the caller's backend, or a \
         scan counter is blind to it"
    );
    assert_eq!(
        backend.full_count(),
        1,
        "the incremental attempt fell through to the full scan the event always needed"
    );
}

#[test]
fn incremental_refresh_reuses_the_ignore_case_of_its_scan() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();
    let cached = FileStatus {
        unstaged: true,
        ..FileStatus::default()
    };
    let seeded = crate::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 1,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Dirty,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };
    ctx.cache.set_canonical(
        &repo,
        seeded,
        HashMap::from([(PathBuf::from("Readme.md"), cached)]),
    );
    let backend = FixedScanBackend {
        scan: crate::git::PathspecScan {
            files: HashMap::new(),
            ignore_case: true,
        },
    };

    let refreshed =
        try_incremental_update_with(&backend, &ctx, &repo, &config, &[repo.join("README.md")]);

    let status = refreshed.status.expect("incremental update applied");
    assert_eq!(
        status.unstaged, 0,
        "the scan's case rule must replace the differently-cased cached entry"
    );
}

/// Whether the watcher callback skips a git event for `repo`.
fn git_event_skipped(ctx: &AgentContext, repo: &Path) -> bool {
    git_event_lacks_subscriber(ctx, &whole_repo_event(repo))
}

#[test]
fn unregistered_repo_git_event_is_skipped() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();

    assert!(
        git_event_skipped(&ctx, &repo),
        "a repository with no registered client must not be refreshed"
    );
}

#[test]
fn client_registered_at_repo_root_keeps_its_git_events() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    ctx.registry.register(4242, Some(repo.clone()));

    assert!(
        !git_event_skipped(&ctx, &repo),
        "a repository with a registered client must still refresh"
    );
}

#[test]
fn client_registered_inside_repo_subdirectory_keeps_its_git_events() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    ctx.registry.register(4242, Some(repo.join("src")));

    assert!(
        !git_event_skipped(&ctx, &repo),
        "a client cwd'd inside the repo must still be a subscriber"
    );
}

#[test]
fn client_registered_in_ancestor_directory_keeps_its_git_events() {
    let parent = tempdir().expect("create parent temp dir");
    let repo_dir = parent.path().join("proj");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    std::process::Command::new("git")
        .arg("init")
        .current_dir(&repo_dir)
        .output()
        .expect("git init");
    let repo = fs::canonicalize(&repo_dir).expect("canonical repo");
    let ancestor = fs::canonicalize(parent.path()).expect("canonical ancestor");

    let ctx = make_agent_context();
    ctx.registry.register(4242, Some(ancestor));

    assert!(
        !git_event_skipped(&ctx, &repo),
        "a client registered in an ancestor directory must still be a subscriber"
    );
}

#[test]
/// `cwd: None` subscribes to every repo, matching `notify_with_signal`.
fn client_with_no_workspace_keeps_every_git_event() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    ctx.registry.register(4242, None);

    assert!(
        !git_event_skipped(&ctx, &repo),
        "a client with no workspace must be treated as a subscriber for every repo"
    );
}

#[test]
fn client_registered_for_a_different_repo_does_not_count_as_a_subscriber() {
    let (_tmp_a, repo_a) = create_temp_repo();
    let (_tmp_b, repo_b) = create_temp_repo();
    let ctx = make_agent_context();
    ctx.registry.register(4242, Some(repo_a));

    assert!(
        git_event_skipped(&ctx, &repo_b),
        "a client registered for a different, unrelated repo must not count"
    );
}

#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
/// Drives the real `create_watcher` gate, observed via `ctx.cache`.
/// `file_serial`: concurrent `FSEvents` bootstraps are flaky.
fn create_watcher_skips_refresh_for_unregistered_repo_and_refreshes_registered_one() {
    /// Absence window: long enough for a gated refresh that was coming to land.
    const REFRESH_ABSENCE_WINDOW: Duration = Duration::from_secs(3);

    let (_tmp, repo) = create_temp_repo();
    // A repo with a shell registered, whose refresh proves the watch live.
    let (_sentinel_tmp, sentinel) = create_temp_repo();
    for dir in [&repo, &sentinel] {
        std::process::Command::new("git")
            .args(["config", "core.fsmonitor", "false"])
            .current_dir(dir)
            .output()
            .expect("disable fsmonitor");
    }

    let ctx = make_agent_context();
    let watcher = create_watcher(WatcherConfig::default(), &ctx).expect("create real watcher");
    // The watch registry is independent of `ctx.registry`, as in production.
    watcher
        .attach_client(std::process::id(), &repo)
        .expect("register with watcher");
    watcher
        .attach_client(std::process::id() + 1, &sentinel)
        .expect("register sentinel with watcher");
    // Nonexistent PID: `notify_repaint_force` prunes it via ESRCH, so the
    // sentinel is rewritten until its first refresh lands.
    let sentinel_pid = 999_998_u32;
    let sentinel_deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut probed_at: Option<std::time::Instant> = None;
    while ctx.cache.get(&sentinel).is_none() && std::time::Instant::now() < sentinel_deadline {
        if probed_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
            ctx.registry.register(sentinel_pid, Some(sentinel.clone()));
            fs::write(sentinel.join("probe.txt"), "probe\n").expect("write sentinel probe");
            probed_at = Some(std::time::Instant::now());
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        ctx.cache.get(&sentinel).is_some(),
        "the watch never delivered the sentinel repo's write"
    );

    fs::write(repo.join("a.txt"), "one\n").expect("write a.txt");

    let settle_deadline = std::time::Instant::now() + REFRESH_ABSENCE_WINDOW;
    while std::time::Instant::now() < settle_deadline {
        assert!(
            ctx.cache.get(&repo).is_none(),
            "an unregistered repo must not gain a cache entry from a real watcher event"
        );
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        ctx.cache.get(&repo).is_none(),
        "an unregistered repo must not gain a cache entry from a real watcher event"
    );

    // Nonexistent PID: `notify_repaint_force` prunes it via ESRCH.
    let fake_pid = 999_999_u32;
    ctx.registry.register(fake_pid, Some(repo.clone()));
    fs::write(repo.join("b.txt"), "two\n").expect("write b.txt");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut populated = false;
    while std::time::Instant::now() < deadline {
        if ctx.cache.get(&repo).is_some() {
            populated = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        populated,
        "a registered repo must gain a cache entry through the real watcher within the budget"
    );

    watcher.stop();
}

#[test]
fn coalesced_multi_file_event_matches_a_full_scan() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();

    let first = repo.join("a.txt");
    let second = repo.join("b.txt");
    fs::write(&first, "one\n").expect("write a.txt");
    fs::write(&second, "two\n").expect("write b.txt");
    commit_all(&repo);

    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(ctx.cache.get(&repo).expect("warm cache").unstaged, 0);

    fs::write(&first, "one changed\n").expect("modify a.txt");
    fs::write(&second, "two changed\n").expect("modify b.txt");

    let event = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::Paths(vec![first, second]),
        },
        repo: repo.clone(),
    };
    handle_file_event(&ctx, &event, &config);
    let incremental = ctx.cache.get(&repo).expect("status after coalesced event");

    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    let full = ctx.cache.get(&repo).expect("status after full scan");

    assert_eq!(
        incremental.unstaged, 2,
        "both modified files must be counted"
    );
    assert_eq!(incremental, full, "coalesced event must match a full scan");
}

#[test]
/// `git status` keys renames under the destination; the source must be cleared.
fn renamed_source_path_is_dropped_from_the_cache() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();

    let source = repo.join("a.txt");
    fs::write(&source, "one\n").expect("write a.txt");
    commit_all(&repo);

    fs::write(&source, "one changed\n").expect("modify a.txt");
    let edit = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(source.clone()),
        },
        repo: repo.clone(),
    };
    handle_file_event(&ctx, &edit, &config);
    assert_eq!(ctx.cache.get(&repo).expect("edit cached").unstaged, 1);

    let destination = repo.join("b.txt");
    let output = std::process::Command::new("git")
        .args(["mv", "a.txt", "b.txt"])
        .current_dir(&repo)
        .output()
        .expect("git mv");
    assert!(output.status.success(), "git mv failed");

    let rename = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::Paths(vec![source, destination]),
        },
        repo: repo.clone(),
    };
    handle_file_event(&ctx, &rename, &config);
    let incremental = ctx.cache.get(&repo).expect("status after rename burst");

    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    let full = ctx.cache.get(&repo).expect("status after full scan");

    assert_eq!(
        incremental, full,
        "a rename burst must not leave the source path counted"
    );
}

/// Warms the cache, edits `other.txt`, returns `(incremental, full)`.
fn incremental_and_full_after_other_edit(
    ctx: &AgentContext,
    repo: &Path,
    config: &Config,
) -> (RepositoryStatus, RepositoryStatus) {
    handle_file_event(ctx, &whole_repo_event(repo), config);
    assert!(ctx.cache.get(repo).is_some(), "warm cache");

    let other = repo.join("other.txt");
    fs::write(&other, "edit\n").expect("modify other.txt");
    let edit = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(other),
        },
        repo: repo.to_path_buf(),
    };
    handle_file_event(ctx, &edit, config);
    let incremental = ctx.cache.get(repo).expect("status after edit");

    ctx.cache.invalidate(repo);
    handle_file_event(ctx, &whole_repo_event(repo), config);
    let full = ctx.cache.get(repo).expect("status after full scan");
    (incremental, full)
}

#[test]
fn incremental_refresh_matches_full_scan_during_merge_conflict() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();

    fs::write(repo.join("a"), "base\n").expect("write a");
    fs::write(repo.join("b"), "base\n").expect("write b");
    fs::write(repo.join("other.txt"), "o\n").expect("write other.txt");
    commit_all(&repo);
    assert!(git_in(&repo, &["checkout", "-qb", "side"]));
    fs::write(repo.join("a"), "side\n").expect("write a");
    fs::write(repo.join("b"), "side\n").expect("write b");
    assert!(git_in(&repo, &["commit", "-qam", "side"]));
    assert!(git_in(&repo, &["checkout", "-q", "-"]));
    fs::write(repo.join("a"), "main\n").expect("write a");
    fs::write(repo.join("b"), "main\n").expect("write b");
    assert!(git_in(&repo, &["commit", "-qam", "main"]));
    // The merge conflicts by design.
    let _conflicted = git_in(&repo, &["merge", "side"]);

    let (incremental, full) = incremental_and_full_after_other_edit(&ctx, &repo, &config);

    assert_eq!(
        incremental, full,
        "incremental refresh must match a full scan"
    );
    assert_eq!(
        (
            incremental.staged,
            incremental.unstaged,
            incremental.conflicts
        ),
        (0, 1, 2)
    );
}

#[test]
/// `git rm --cached` yields a staged delete and untracked record for one path.
fn incremental_refresh_keeps_staged_delete_with_untracked_same_path() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let config = Config::default();

    fs::write(repo.join("secret.env"), "s\n").expect("write secret.env");
    fs::write(repo.join("other.txt"), "o\n").expect("write other.txt");
    commit_all(&repo);
    assert!(git_in(&repo, &["rm", "-q", "--cached", "secret.env"]));

    let (incremental, full) = incremental_and_full_after_other_edit(&ctx, &repo, &config);

    assert_eq!(
        incremental, full,
        "incremental refresh must match a full scan"
    );
    assert_eq!(
        (
            incremental.staged,
            incremental.unstaged,
            incremental.untracked
        ),
        (1, 1, 1)
    );
}

#[test]
fn git_events_are_skipped_when_git_disabled() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let mut config = Config::default();
    config.git.enabled = false;

    let head_path = repo.join(".git/HEAD");
    let event = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(head_path),
        },
        repo,
    };

    handle_file_event(&ctx, &event, &config);
    assert!(
        ctx.cache.is_empty(),
        "cache should remain empty when git is disabled"
    );
}

#[test]
fn status_changed_detects_transitions() {
    use crate::git::RepositoryState;
    let clean = status(0, RepositoryState::Clean);
    let dirty = status(1, RepositoryState::Dirty);

    assert!(!status_changed(Some(&clean), Some(&clean)));
    assert!(!status_changed(None, None));

    assert!(status_changed(None, Some(&clean)));
    assert!(status_changed(Some(&clean), None));
    assert!(status_changed(Some(&clean), Some(&dirty)));
}

#[cfg(unix)]
#[test]
fn reconcile_full_scan_picks_up_untracked_changes() {
    let (_tmp, repo) = create_temp_repo();
    let mut ctx = make_agent_context();
    let rings = count_rings(&mut ctx);
    let config = Config::default();

    refresh_and_notify_if_changed(&ctx, &repo, &config, None);
    let baseline = ctx
        .cache
        .get(&repo)
        .expect("status cached after first scan");
    assert_eq!(baseline.untracked, 0, "fresh repo starts clean");

    fs::write(repo.join("dropped.txt"), "x").expect("write file");

    let before = rung(&rings);
    refresh_and_notify_if_changed(&ctx, &repo, &config, None);
    let after = rung(&rings);

    let updated = ctx.cache.get(&repo).expect("status cached");
    assert!(
        updated.untracked >= 1,
        "reconcile full scan should reflect the untracked file"
    );
    assert!(
        after > before,
        "reconcile should notify on the detected change"
    );
}
