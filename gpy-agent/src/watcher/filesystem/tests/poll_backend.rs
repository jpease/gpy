use super::super::backend::{SplitPollWatcher, routes_to_gitdir};
use super::*;
use notify::Watcher;

/// A real worktree write is delivered well before reconcile whatever the
/// native backend's health (falling back to `PollWatcher` if `FSEvents` is silent).
#[test]
fn real_worktree_write_is_delivered_even_when_native_backend_is_silent() {
    let poll = PollRepo::with_policy(FallbackPolicy::from_env());
    poll.watcher.watch(&poll.root).expect("watch repo");

    // Let the recursive watch arm before racing it.
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(poll.root.join("untracked.txt"), b"content").expect("write worktree file");

    let delivered = wait_for_any_event(&poll.events, Duration::from_secs(8));

    assert!(
        delivered,
        "a real worktree write must be delivered without waiting for the \
         periodic reconcile, even when the native backend is silent"
    );
}

/// The `PollWatcher` fallback observes real worktree changes.
#[test]
fn poll_fallback_backend_delivers_real_worktree_writes() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));
    assert_eq!(
        poll.watcher.backend_kind(),
        BackendKind::Poll,
        "ForcePoll must arm the polling backend without any probe"
    );

    poll.watcher.watch(&poll.root).expect("watch repo");
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(poll.root.join("untracked.txt"), b"content").expect("write worktree file");

    let delivered = wait_for_any_event(&poll.events, Duration::from_secs(5));

    assert!(
        delivered,
        "the poll fallback backend must deliver real worktree writes"
    );
}

/// `stop` wakes the blocked event thread even though the poll backend keeps a
/// sender clone alive, so a long poll interval cannot hold the join.
#[test]
fn stop_returns_promptly_with_poll_backend() {
    // Moving only the watcher out leaves the directory owned by `poll` until the end of the test.
    let poll = PollRepo::forced_poll(Duration::from_secs(60));
    let PollRepo {
        mut watcher,
        ref root,
        ..
    } = poll;
    watcher.watch(root).expect("watch repo");

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        watcher.stop();
        let _ = done_tx.send(());
    });
    let stopped = done_rx.recv_timeout(Duration::from_secs(2)).is_ok();

    assert!(stopped, "stop must return within 2s with the poll backend");
}

/// The poll fallback delivers edits to an existing file, which `PollWatcher`
/// reports only as `Modify(Metadata(WriteTime))`.
#[test]
fn poll_fallback_backend_delivers_edits_to_existing_files() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));

    // Seeded before arming, so only a later append can produce an event.
    let tracked = poll.root.join("tracked.txt");
    std::fs::write(&tracked, b"hello\n").expect("seed tracked file");
    poll.watcher.watch(&poll.root).expect("watch repo");

    let delivered = poll.append_until_delivered(&tracked, Duration::from_secs(10));

    assert!(
        delivered,
        "the poll fallback must deliver edits to an existing worktree file, \
         not only file creations"
    );
}

/// The poll fallback reports a same-size rewrite in the same whole second
/// (e.g. a moved ref); the mtime is pinned to make that deterministic.
#[test]
fn poll_fallback_detects_same_second_same_size_rewrite() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));
    let pinned = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let pin = |path: &Path, contents: &[u8]| {
        std::fs::write(path, contents).expect("write ref file");
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open ref file")
            .set_modified(pinned)
            .expect("pin mtime");
    };

    let ref_file = poll.root.join(".git/refs/heads/main");
    std::fs::create_dir_all(ref_file.parent().expect("refs dir")).expect("create refs dir");
    pin(&ref_file, b"1111111111111111111111111111111111111111\n");
    poll.watcher.watch(&poll.root).expect("watch repo");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut delivered = false;
    let mut flip = false;
    while Instant::now() < deadline {
        flip = !flip;
        let digit: &[u8] = if flip {
            b"2222222222222222222222222222222222222222\n"
        } else {
            b"1111111111111111111111111111111111111111\n"
        };
        pin(&ref_file, digit);
        if wait_for_any_event(&poll.events, Duration::from_millis(500)) {
            delivered = true;
            break;
        }
    }

    assert!(
        delivered,
        "a same-size rewrite within one mtime second must be delivered"
    );
}

/// The poll backend's directory-mtime report is noise; file mtimes and other kinds aren't.
#[test]
fn poll_directory_mtime_event_is_noise_but_file_edit_is_not() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let file = tmp.path().join("f");
    std::fs::write(&file, b"x").expect("write file");
    let mtime = EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime));

    assert!(is_poll_directory_mtime_noise(
        &Event::new(mtime).add_path(tmp.path().to_path_buf())
    ));
    assert!(!is_poll_directory_mtime_noise(
        &Event::new(mtime).add_path(file)
    ));
    assert!(!is_poll_directory_mtime_noise(&Event::new(mtime)));
    assert!(!is_poll_directory_mtime_noise(
        &Event::new(EventKind::Create(CreateKind::Folder)).add_path(tmp.path().to_path_buf())
    ));
}

/// Unwatching an outer poll root keeps a nested root's directories armed.
#[test]
fn poll_unwatch_of_outer_keeps_inner_expansion() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));
    let inner = poll.root.join("inner");
    let sub = inner.join("sub");
    std::fs::create_dir_all(&sub).expect("create inner/sub");
    let tracked = sub.join("f.txt");
    std::fs::write(&tracked, b"seed\n").expect("seed inner file");

    poll.watcher.watch(&poll.root).expect("watch outer");
    poll.watcher.watch(&inner).expect("watch inner");
    poll.watcher.unwatch(&poll.root).expect("unwatch outer");
    let still_covered = poll.watcher.covers(&sub);
    poll.events.lock().unwrap().clear();

    let delivered = poll.append_until_delivered(&tracked, Duration::from_secs(10));

    assert!(still_covered, "the inner root still covers its subtree");
    assert!(
        delivered,
        "unwatching the outer root must not remove the inner root's poll targets"
    );
}

/// Under the poll backend, an edit inside a directory created after arming is delivered.
#[test]
fn poll_backend_watches_directories_created_after_arming() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));
    poll.watcher.watch(&poll.root).expect("watch repo");

    let new_dir = poll.root.join("newdir");
    std::fs::create_dir_all(&new_dir).expect("create newdir");
    let tracked = new_dir.join("a.txt");
    std::fs::write(&tracked, b"seed\n").expect("seed newdir/a.txt");

    // Let the creation and its stragglers land so only the edits below count.
    let created = wait_for_any_event(&poll.events, Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(400));
    poll.events.lock().unwrap().clear();

    // Re-append until one lands after the new directory's baseline, in a later second.
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);
    let mut delivered = false;
    while Instant::now() < deadline {
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&tracked)
            .expect("open newdir/a.txt for append");
        std::io::Write::write_all(&mut handle, b"modified\n").expect("append");
        drop(handle);

        if wait_for_git_event_for(&poll.events, &tracked, Duration::from_millis(500)) {
            delivered = true;
            break;
        }
    }

    assert!(created, "creating newdir must itself be reported");
    assert!(
        delivered,
        "the poll backend must deliver edits inside a directory created after arming"
    );
}

/// A commit on a branch whose `refs/heads/<prefix>/` dir was created after
/// arming is detected under the poll backend.
#[test]
fn poll_backend_detects_commit_on_branch_dir_created_after_arming() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));
    std::fs::remove_dir_all(poll.root.join(".git")).expect("drop the placeholder .git");
    git_init(&poll.root);
    git_in(&poll.root, &["commit", "-q", "--allow-empty", "-m", "init"]);
    poll.watcher.watch(&poll.root).expect("watch repo");

    // Creates `.git/refs/heads/feature/` after arming.
    git_in(&poll.root, &["checkout", "-q", "-b", "feature/x"]);
    let feature_ref = poll.root.join(".git/refs/heads/feature/x");

    // Commit until one is reported, as above.
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);
    let mut delivered = false;
    while Instant::now() < deadline {
        git_in(
            &poll.root,
            &["commit", "-q", "--allow-empty", "-m", "on feature"],
        );
        if wait_for_git_event_for(&poll.events, &feature_ref, Duration::from_millis(500)) {
            delivered = true;
            break;
        }
    }

    assert!(
        delivered,
        "a commit moving refs/heads/feature/x, whose directory was created \
         after arming, must be delivered under the poll backend"
    );
}

/// Repeated mkdir/rmdir cycles return the poll expansion to its baseline.
#[test]
fn poll_backend_drops_expansion_for_removed_directory() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));
    poll.watcher.watch(&poll.root).expect("watch repo");
    let baseline = poll_targets_of(&poll.watcher, &poll.root);
    let new_dir = poll.root.join("newdir");
    let nested = new_dir.join("nested");

    let mut cycles = Vec::new();
    for _ in 0_u8..3_u8 {
        std::fs::create_dir_all(&nested).expect("create newdir/nested");
        let expanded = wait_until(Duration::from_secs(5), || {
            let targets = poll_targets_of(&poll.watcher, &poll.root);
            targets.contains(&new_dir) && targets.contains(&nested)
        });
        let consistent_expanded = armed_dirs(&poll.watcher) == all_targets(&poll.watcher);

        std::fs::remove_dir_all(&new_dir).expect("remove newdir");
        let dropped = wait_until(Duration::from_secs(5), || {
            poll_targets_of(&poll.watcher, &poll.root) == baseline
        });
        let consistent_dropped = armed_dirs(&poll.watcher) == all_targets(&poll.watcher);
        cycles.push((expanded, consistent_expanded, dropped, consistent_dropped));
    }
    let armed_after = armed_dirs(&poll.watcher);

    for (cycle, (expanded, consistent_expanded, dropped, consistent_dropped)) in
        cycles.into_iter().enumerate()
    {
        assert!(
            expanded,
            "cycle {cycle}: newdir and newdir/nested must join the expansion"
        );
        assert!(
            consistent_expanded,
            "cycle {cycle}: the armed set must equal the registrations' targets"
        );
        assert!(
            dropped,
            "cycle {cycle}: removing newdir must return the expansion to its baseline"
        );
        assert!(
            consistent_dropped,
            "cycle {cycle}: removing newdir must unwatch what it dropped"
        );
    }
    assert_eq!(
        armed_after, baseline,
        "repeated mkdir/rmdir cycles must leave the armed set at its baseline"
    );
}

/// A gitignored directory created after arming is not armed.
#[test]
fn poll_backend_does_not_arm_gitignored_directory_created_after_arming() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));
    std::fs::write(poll.root.join(".gitignore"), b"node_modules/\n").expect("write .gitignore");
    poll.watcher.watch(&poll.root).expect("watch repo");

    let ignored = poll.root.join("node_modules");
    let control = poll.root.join("src");
    std::fs::create_dir_all(ignored.join("pkg")).expect("create node_modules/pkg");
    std::fs::create_dir_all(&control).expect("create src");

    // `src` proves the create path ran; the sleep lets `node_modules` land too.
    let control_armed = wait_until(Duration::from_secs(5), || {
        armed_dirs(&poll.watcher).contains(&control)
    });
    std::thread::sleep(Duration::from_millis(400));
    let armed = armed_dirs(&poll.watcher);

    assert!(control_armed, "a new non-ignored directory must be armed");
    assert!(
        !armed.iter().any(|path| path.starts_with(&ignored)),
        "a new gitignored directory must not be armed, got: {armed:?}"
    );
}

/// A directory under two overlapping roots stays armed until both release it.
#[test]
fn poll_expansion_of_new_directory_is_held_by_every_covering_root() {
    let poll = PollRepo::forced_poll(Duration::from_millis(200));
    let inner = poll.root.join("inner");
    std::fs::create_dir_all(&inner).expect("create inner");
    poll.watcher.watch(&poll.root).expect("watch outer");
    poll.watcher.watch(&inner).expect("watch inner");

    let new_dir = inner.join("newdir");
    std::fs::create_dir_all(&new_dir).expect("create inner/newdir");
    let held_by_both = wait_until(Duration::from_secs(5), || {
        poll_targets_of(&poll.watcher, &poll.root).contains(&new_dir)
            && poll_targets_of(&poll.watcher, &inner).contains(&new_dir)
    });
    let consistent = armed_dirs(&poll.watcher) == all_targets(&poll.watcher);

    poll.watcher.unwatch(&poll.root).expect("unwatch outer");
    let kept = armed_dirs(&poll.watcher).contains(&new_dir);
    poll.watcher.unwatch(&inner).expect("unwatch inner");
    let left = armed_dirs(&poll.watcher);

    assert!(
        held_by_both,
        "the new directory must join both the outer and the inner expansion"
    );
    assert!(
        consistent,
        "the armed set must equal the registrations' targets"
    );
    assert!(
        kept,
        "unwatching the outer root must keep the inner root's new directory armed"
    );
    assert!(
        left.is_empty(),
        "releasing both roots must unarm the new directory too, got: {left:?}"
    );
}

/// A backend swap re-arms every already-registered directory; the file is
/// written only after the swap.
#[test]
fn backend_swap_rearms_already_watched_paths() {
    // Probe disabled so the outcome doesn't depend on this machine's backend.
    let poll = PollRepo::with_policy(FallbackPolicy::NativeOnly);

    poll.watcher.watch(&poll.root).expect("watch repo");
    assert_eq!(poll.watcher.backend_kind(), BackendKind::Native);

    poll.watcher.force_poll_fallback(Duration::from_millis(200));
    assert_eq!(
        poll.watcher.backend_kind(),
        BackendKind::Poll,
        "the swap must leave the polling backend armed"
    );

    poll.events.lock().unwrap().clear();
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(poll.root.join("after_swap.txt"), b"content").expect("write worktree file");

    let delivered = wait_for_any_event(&poll.events, Duration::from_secs(5));

    assert!(
        delivered,
        "a repo watched before the backend swap must still be watched after it"
    );
}

/// A backend swap emits one rescan per re-armed root, since `PollWatcher`
/// misses changes made before arming.
#[test]
fn backend_swap_emits_rescan_for_each_rearmed_root() {
    let poll = PollRepo::with_policy(FallbackPolicy::NativeOnly);
    poll.watcher.watch(&poll.root).expect("watch repo");
    poll.events.lock().unwrap().clear();

    poll.watcher.force_poll_fallback(Duration::from_secs(1));

    let delivered = wait_for_any_event(&poll.events, Duration::from_secs(2));
    let repos: Vec<PathBuf> = poll
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|event| event.repo.clone())
        .collect();

    assert!(
        delivered,
        "the swap must emit a rescan for the watched root"
    );
    assert!(
        repos.contains(&poll.root),
        "the rescan must be attributed to the re-armed root, got: {repos:?}"
    );
}

/// A second swap must not rebuild a working poll backend.
#[test]
fn repeated_swap_is_a_no_op() {
    let poll = PollRepo::with_policy(FallbackPolicy::NativeOnly);
    poll.watcher.watch(&poll.root).expect("watch repo");

    poll.watcher.force_poll_fallback(Duration::from_secs(1));
    let after_first = wait_for_any_event(&poll.events, Duration::from_secs(2));
    // Let the first swap's rescan burst land, or slow runners miscount it.
    std::thread::sleep(Duration::from_millis(400));
    poll.events.lock().unwrap().clear();
    poll.watcher.force_poll_fallback(Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(200));
    let rescans_after_second = poll.events.lock().unwrap().len();

    assert!(after_first, "the first swap should emit a rescan");
    assert_eq!(
        rescans_after_second, 0,
        "a redundant swap must not re-arm or re-rescan"
    );
}

/// The `PollWatcher` fallback does not watch a gitignored directory.
#[test]
fn poll_watch_targets_excludes_gitignored_directories() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::write(repo.join(".gitignore"), b"vendor/\n").expect("write .gitignore");
    std::fs::create_dir_all(repo.join("vendor").join("deep")).expect("vendor dir");
    std::fs::write(repo.join("vendor").join("deep").join("bundled.txt"), b"x")
        .expect("write vendored file");
    std::fs::create_dir_all(repo.join("src")).expect("src dir");

    let targets = poll_watch_targets(&repo, &registry);

    assert!(
        targets.contains(&repo.join("src")),
        "a non-ignored directory must still be a watch target, got: {targets:?}"
    );
    assert!(
        !targets.iter().any(|p| p.starts_with(repo.join("vendor"))),
        "a gitignored directory (and everything under it) must not be a \
         watch target, got: {targets:?}"
    );
}

/// A `.gitignore` rule (e.g. `pack/`) never prunes anything under `.git`.
#[test]
fn poll_watch_targets_never_prunes_inside_git_dir() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::write(repo.join(".gitignore"), b"pack/\n").expect("write .gitignore");
    std::fs::create_dir_all(repo.join(".git").join("objects").join("pack"))
        .expect(".git/objects/pack dir");

    let targets = poll_watch_targets(&repo, &registry);

    assert!(
        targets.contains(&repo.join(".git").join("objects").join("pack")),
        "a .gitignore rule must never prune a directory under .git, got: {targets:?}"
    );
}

/// Git metadata outside the `objects` store is polled with content comparison;
/// the worktree and object packs are not.
#[test]
fn split_poll_routes_gitdir_paths_to_content_comparison() {
    let registry = test_registry(true, HashSet::new());
    let root = Path::new("/work/repo");
    let separate_gitdir = Path::new("/gitdirs/repo-store");
    let common_dir = Path::new("/mirrors/foo.git");
    registry.register_external_gitdir(separate_gitdir, root);
    registry.register_common_gitdir(common_dir, root);

    for path in [
        root.join(".git/HEAD"),
        root.join(".git/refs/heads/main"),
        separate_gitdir.to_path_buf(),
        separate_gitdir.join("refs"),
        common_dir.to_path_buf(),
        common_dir.join("refs"),
    ] {
        assert!(
            routes_to_gitdir(&path, &registry),
            "{} must be compared by content",
            path.display()
        );
    }
    for path in [
        root.join("src/main.rs"),
        root.to_path_buf(),
        root.join(".git/objects/pack"),
        root.join(".git/modules/sub/objects/pack"),
        common_dir.join("objects/ab"),
        separate_gitdir.join("objects"),
    ] {
        assert!(
            !routes_to_gitdir(&path, &registry),
            "{} must not be compared by content",
            path.display()
        );
    }
}

/// After the registry forgets a common dir not named `.git`, `unwatch` still
/// releases its path from whichever poller took it.
#[test]
fn split_poll_unwatch_releases_a_common_dir_after_unregister() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let root = std::fs::canonicalize(tmp.path()).expect("canonicalize root");
    let common_dir = root.join("foo.git");
    let refs = common_dir.join("refs");
    std::fs::create_dir_all(&refs).expect("refs dir");
    registry.register_common_gitdir(&common_dir, &root);

    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let mut watcher =
        SplitPollWatcher::new_split(&event_tx, Duration::from_millis(100), Arc::clone(&registry))
            .expect("split poll watcher");
    watcher
        .watch(&refs, notify::RecursiveMode::Recursive)
        .expect("watch refs");

    registry.unregister_common_gitdir(&common_dir, &root);
    watcher.unwatch(&refs).expect("unwatch after unregister");

    std::fs::write(refs.join("main"), b"new\n").expect("write ref");
    assert!(
        event_rx.recv_timeout(Duration::from_millis(600)).is_err(),
        "a released path must produce no events"
    );
}
