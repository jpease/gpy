use super::*;

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
        assert!(
            matches!(pending.event, FileEvent::Git { .. }),
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
    assert!(matches!(
        events.first().map(|e| &e.event),
        Some(FileEvent::Git { .. })
    ));
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
        assert!(
            matches!(
                &pending.event,
                FileEvent::Git {
                    paths: GitPaths::WholeRepo
                }
            ),
            "a rescan must carry GitPaths::WholeRepo, got: {:?}",
            pending.event
        );
    }
}
