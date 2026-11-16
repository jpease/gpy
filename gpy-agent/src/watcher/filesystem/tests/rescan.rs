use super::*;
use std::assert_matches;

/// A no-paths overflow (e.g. inotify `IN_Q_OVERFLOW`, surfaced as `Ok` with
/// `need_rescan()`) rescans every watched root.
#[test]
fn rescan_overflow_with_no_paths_triggers_all_watched_roots() {
    let tmp1 = tempfile::TempDir::new().expect("temp dir 1");
    let tmp2 = tempfile::TempDir::new().expect("temp dir 2");
    let root1 = tmp1.path().to_path_buf();
    let root2 = tmp2.path().to_path_buf();

    let registry = test_registry(true, HashSet::from([root1.clone(), root2.clone()]));
    let (callback, events) = recording_callback();
    let callback_arc = Arc::new(callback);

    let evt = Event::new(EventKind::Other).set_flag(Flag::Rescan);
    FileSystemWatcher::process_event(
        &evt,
        &callback_arc,
        &registry,
        TEST_FOLLOWUP_DELAY,
        &no_op_scheduler(),
    );

    let recorded = events.lock().unwrap();
    assert_eq!(
        recorded.len(),
        2,
        "an overflow with no paths should rescan every watched root exactly once"
    );
    let mut rescanned_repos: Vec<&PathBuf> = recorded.iter().map(|p| &p.repo).collect();
    rescanned_repos.sort();
    let mut expected_repos = vec![&root1, &root2];
    expected_repos.sort();
    assert_eq!(rescanned_repos, expected_repos);
    for pending in recorded.iter() {
        assert_matches!(
            pending.event,
            FileEvent::Git { .. },
            "a rescan should be delivered as a synthetic git event"
        );
    }
}

/// A path-naming overflow rescans only its owning root.
#[test]
fn rescan_overflow_with_path_attributes_to_owning_root_only() {
    let tmp1 = tempfile::TempDir::new().expect("temp dir 1");
    let tmp2 = tempfile::TempDir::new().expect("temp dir 2");
    let root1 = tmp1.path().to_path_buf();
    let root2 = tmp2.path().to_path_buf();
    let subdir_path = root1.join("some").join("subdir");

    let registry = test_registry(true, HashSet::from([root1.clone(), root2]));
    let (callback, events) = recording_callback();
    let callback_arc = Arc::new(callback);

    let evt = Event::new(EventKind::Other)
        .set_flag(Flag::Rescan)
        .add_path(subdir_path);
    FileSystemWatcher::process_event(
        &evt,
        &callback_arc,
        &registry,
        TEST_FOLLOWUP_DELAY,
        &no_op_scheduler(),
    );

    let recorded = events.lock().unwrap();
    let recorded_len = recorded.len();
    let first_repo = recorded.first().map(|p| p.repo.clone());
    let first_is_git_event = recorded
        .first()
        .is_some_and(|p| matches!(p.event, FileEvent::Git { .. }));
    drop(recorded);
    assert_eq!(
        recorded_len, 1,
        "an overflow naming a path under root1 should rescan only root1"
    );
    assert_eq!(
        first_repo,
        Some(root1),
        "the rescan should attribute to root1 via longest-prefix match"
    );
    assert!(first_is_git_event);
}

/// A normal event is unaffected by the overflow branch.
#[test]
fn non_rescan_event_is_unaffected_by_overflow_handling() {
    let registry = test_registry(true, HashSet::new());
    let kind = EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any));
    assert!(
        !Event::new(kind).need_rescan(),
        "test fixture must not be a rescan event"
    );
    let events = events_for(&registry, kind, vec![PathBuf::from("/repo/.git/HEAD")]);

    assert_eq!(
        events.len(),
        1,
        "a normal gitdir event should still trigger exactly one git event, unchanged"
    );
    assert_matches!(
        events.first().map(|e| &e.event),
        Some(FileEvent::Git { .. })
    );
}

/// Several overflow paths under one root collapse to one rescan.
#[test]
fn rescan_overflow_dedupes_multiple_paths_under_same_root() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let root = tmp.path().to_path_buf();

    let registry = test_registry(true, HashSet::from([root.clone()]));
    let (callback, events) = recording_callback();
    let callback_arc = Arc::new(callback);

    let evt = Event::new(EventKind::Other)
        .set_flag(Flag::Rescan)
        .add_path(root.join("a"))
        .add_path(root.join("b").join("c"))
        .add_path(root.join("d"));
    FileSystemWatcher::process_event(
        &evt,
        &callback_arc,
        &registry,
        TEST_FOLLOWUP_DELAY,
        &no_op_scheduler(),
    );

    let recorded = events.lock().unwrap();
    assert_eq!(
        recorded.len(),
        1,
        "multiple overflow paths under one root should dedupe to a single rescan event"
    );
    assert_eq!(recorded.first().map(|p| &p.repo), Some(&root));
}

/// A rescan emits `GitPaths::WholeRepo`, not `GitPaths::single(root)`.
#[test]
fn rescan_emits_whole_repo_for_each_affected_root() {
    let tmp1 = tempfile::TempDir::new().expect("temp dir 1");
    let tmp2 = tempfile::TempDir::new().expect("temp dir 2");
    let root1 = tmp1.path().to_path_buf();
    let root2 = tmp2.path().to_path_buf();

    let registry = test_registry(true, HashSet::from([root1, root2]));
    let (callback, events) = recording_callback();
    let callback_arc = Arc::new(callback);

    let evt = Event::new(EventKind::Other).set_flag(Flag::Rescan);
    FileSystemWatcher::process_event(
        &evt,
        &callback_arc,
        &registry,
        TEST_FOLLOWUP_DELAY,
        &no_op_scheduler(),
    );

    let recorded = events.lock().unwrap();
    assert_eq!(recorded.len(), 2, "one rescan event per affected root");
    for pending in recorded.iter() {
        assert_matches!(
            &pending.event,
            FileEvent::Git {
                paths: GitPaths::WholeRepo
            },
            "a rescan must carry GitPaths::WholeRepo, got: {:?}",
            pending.event
        );
    }
}

/// A full event queue drops the event but flags the overflow, which the event
/// thread turns into a whole-repo rescan of every watched root.
#[test]
fn queue_overflow_triggers_whole_repo_rescan() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let root = tmp.path().to_path_buf();
    let registry = test_registry(true, HashSet::from([root.clone()]));
    let (callback, events) = recording_callback();

    // Two events into a one-slot queue before the thread runs: the second
    // overflows. Both are `Access`, which emits nothing, so any delivery is
    // the overflow's rescan.
    let (event_tx, event_rx) = event_queue(1);
    event_tx.send(Event::new(EventKind::Access(AccessKind::Any)));
    event_tx.send(Event::new(EventKind::Access(AccessKind::Any)));
    drop(event_tx);

    // The backend forwards to its own queue, so the thread ends once drained.
    let (backend_tx, _backend_rx) = event_queue(1);
    let backend_state = WatcherBackend::armed(
        build_native_watcher(&backend_tx).expect("native watcher"),
        BackendKind::Native,
    );
    let poll_active = Arc::clone(&backend_state.poll_active);
    let handle = FileSystemWatcher::spawn_event_thread(
        event_rx,
        callback,
        &Arc::new(AtomicBool::new(false)),
        EventThreadState {
            registry: Arc::clone(&registry),
            backend: Arc::new(Mutex::new(backend_state)),
            poll_active,
        },
        FollowupHandoff {
            delay: TEST_FOLLOWUP_DELAY,
            schedule: no_op_scheduler(),
        },
    );
    handle.join().expect("event thread");

    let recorded = events.lock().unwrap();
    assert_eq!(recorded.len(), 1, "one rescan for the one watched root");
    let pending = recorded.first().expect("rescan event");
    assert_eq!(pending.repo, root);
    assert_matches!(
        &pending.event,
        FileEvent::Git {
            paths: GitPaths::WholeRepo
        },
        "an overflow must rescan the whole repo, got: {:?}",
        pending.event
    );
}
