use super::*;

/// Reading a watched file must not look like writing it: inotify's `OPEN` mask
/// would otherwise make every reload retrigger itself.
#[test]
fn pure_access_events_are_not_changes() {
    let registry = test_registry(true, HashSet::new());
    for kind in [
        EventKind::Access(AccessKind::Open(AccessMode::Any)),
        EventKind::Access(AccessKind::Close(AccessMode::Read)),
        EventKind::Access(AccessKind::Close(AccessMode::Write)),
        EventKind::Access(AccessKind::Read),
    ] {
        let events = events_for(&registry, kind, vec![PathBuf::from("/repo/.git/HEAD")]);

        assert!(
            events.is_empty(),
            "{kind:?} is a read, not a change, and must not trigger an update"
        );
    }
}

/// inotify raises one event per mask bit, so the `Modify(Data)` beside a
/// `Close(Write)` must still classify.
#[test]
fn write_alongside_a_close_still_triggers() {
    let registry = test_registry(true, HashSet::new());
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Any)),
        vec![PathBuf::from("/repo/.git/HEAD")],
    );

    assert_eq!(events.len(), 1, "the write half must still be delivered");
}

#[test]
fn gitdir_changes_trigger_without_aggressive_mode() {
    let registry = test_registry(true, HashSet::new());
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
        vec![PathBuf::from("/repo/.git/HEAD")],
    );

    assert_eq!(events.len(), 1, "Expected gitdir event to trigger");
    assert!(matches!(
        events.first().map(|e| &e.event),
        Some(FileEvent::Git { .. })
    ));
}

#[test]
fn worktree_events_skipped_when_disabled() {
    let registry = test_registry(false, HashSet::new());
    let events = events_for(
        &registry,
        EventKind::Create(CreateKind::File),
        vec![PathBuf::from("/repo/new_file.txt")],
    );

    assert!(
        events.is_empty(),
        "Worktree events should be skipped when watching is disabled"
    );
}

#[test]
fn worktree_content_edit_triggers_when_enabled() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = tmp.path();
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    let file = repo.join("tracked.txt");
    std::fs::write(&file, "data").expect("write file");

    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.clone()],
    );

    assert_eq!(
        events.len(),
        1,
        "a tracked-file content edit should emit one git event"
    );
    assert!(matches!(
        events.first().map(|e| &e.event),
        Some(FileEvent::Git { .. })
    ));
}

/// A file with both language/config and git-status significance emits a Git
/// refresh in addition to the specialized event.
fn assert_worktree_edit_emits_both_specialized_and_git(
    filename: &str,
    is_expected_specialized: impl Fn(&FileEvent) -> bool,
) {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = tmp.path();
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    let file = repo.join(filename);
    std::fs::create_dir_all(file.parent().expect("file parent")).expect("parent dirs");
    std::fs::write(&file, "data").expect("write file");

    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.clone()],
    );

    assert_eq!(
        events.len(),
        2,
        "editing {filename} should emit both the specialized event and a \
         Git refresh, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| is_expected_specialized(&e.event)),
        "expected the specialized event for {filename}, got: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, FileEvent::Git { .. })),
        "expected an additional Git refresh event for {filename} so the \
         git instant cache doesn't go stale, got: {events:?}"
    );
}

#[test]
fn worktree_language_manifest_edit_also_emits_git_refresh() {
    assert_worktree_edit_emits_both_specialized_and_git("Cargo.toml", |event| {
        matches!(event, FileEvent::Language { .. })
    });
}

#[test]
fn worktree_lockfile_edit_also_emits_git_refresh() {
    assert_worktree_edit_emits_both_specialized_and_git("Cargo.lock", |event| {
        matches!(event, FileEvent::Language { .. })
    });
}

#[test]
fn worktree_config_like_filename_edit_also_emits_git_refresh() {
    assert_worktree_edit_emits_both_specialized_and_git("config.toml", |event| {
        matches!(event, FileEvent::Config { .. })
    });
}

/// A tracked gpy theme in a dotfiles repo emits a Git refresh and a `Theme` signal.
#[test]
fn tracked_theme_file_in_repo_also_emits_git() {
    assert_worktree_edit_emits_both_specialized_and_git(".config/gpy/themes/a.toml", |event| {
        matches!(event, FileEvent::Theme { .. })
    });
}

/// A nested event attributes to its watched root by longest prefix; no `.git`
/// exists, so only the prefix match can succeed.
#[test]
fn nested_event_attributes_to_watched_root_via_prefix_match() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let watched_root = tmp.path().to_path_buf();
    let nested_dir = watched_root.join("nested").join("deep");
    std::fs::create_dir_all(&nested_dir).expect("nested dirs");
    let file = nested_dir.join("tracked.txt");
    std::fs::write(&file, "data").expect("write file");

    let registry = test_registry(true, HashSet::from([watched_root.clone()]));
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.clone()],
    );

    assert_eq!(
        events.len(),
        1,
        "a nested tracked-file edit should emit one git event"
    );
    assert_eq!(
        events.first().map(|e| &e.repo),
        Some(&watched_root),
        "event should attribute to the watched root via longest-prefix match"
    );
    assert!(matches!(
        events.first().map(|e| &e.event),
        Some(FileEvent::Git { .. })
    ));
}

/// An event outside every watched root falls back to `find_worktree_root`, then the parent.
#[test]
fn event_outside_watched_roots_falls_back_to_ancestor_walk() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let outside_dir = tmp.path().to_path_buf();
    let file = outside_dir.join("tracked.txt");
    std::fs::write(&file, "data").expect("write file");

    // An unrelated watched root that is NOT a prefix of `file`.
    let other_tmp = tempfile::TempDir::new().expect("other temp dir");
    let registry = test_registry(true, HashSet::from([other_tmp.path().to_path_buf()]));
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.clone()],
    );

    let expected_repo = crate::git::find_worktree_root(&file)
        .or_else(|| file.parent().map(Path::to_path_buf))
        .expect("fallback always yields a parent for an absolute path");

    assert_eq!(
        events.len(),
        1,
        "a tracked-file edit outside any watched root should still emit one git event"
    );
    assert_eq!(
        events.first().map(|e| &e.repo),
        Some(&expected_repo),
        "no watched root matches, so attribution must match today's ancestor-walk fallback"
    );
}

/// A directory create emits its own git event and schedules exactly one
/// follow-up, rescanning files whose creates raced the not-yet-armed watch.
#[test]
fn directory_create_schedules_one_followup_git_event() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let new_dir = tmp.path().join("newmod");
    std::fs::create_dir_all(&new_dir).expect("create new dir");

    let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
    let (callback, events) = recording_callback();
    let callback_arc = Arc::new(callback);
    let (scheduler, captured_schedules) = recording_scheduler();

    let evt = Event {
        kind: EventKind::Create(CreateKind::Folder),
        paths: vec![new_dir.clone()],
        attrs: EventAttributes::default(),
    };
    let called_at = Instant::now();
    FileSystemWatcher::process_event(
        &evt,
        &callback_arc,
        &registry,
        TEST_FOLLOWUP_DELAY,
        &scheduler,
    );

    let recorded = events.lock().unwrap();
    assert_eq!(
        recorded.len(),
        1,
        "directory-create should emit its own git event immediately"
    );
    let immediate = recorded.first().expect("the immediate event");
    assert!(
        matches!(immediate.event, FileEvent::Git { .. }),
        "the immediate event must be a git event"
    );
    assert_eq!(immediate.repo, tmp.path().to_path_buf());
    drop(recorded);

    // ...and exactly one follow-up, same repo, no earlier than the delay.
    let scheduled_guard = captured_schedules.lock().unwrap();
    assert_eq!(
        scheduled_guard.len(),
        1,
        "exactly one follow-up git event should be scheduled for a directory create"
    );
    let (pending, due) = scheduled_guard
        .first()
        .expect("the scheduled follow-up")
        .clone();
    drop(scheduled_guard);
    assert_eq!(
        pending.repo,
        tmp.path().to_path_buf(),
        "follow-up must target the same repo the mkdir attributed to"
    );
    let FileEvent::Git { paths } = &pending.event else {
        panic!("the follow-up must be a git event");
    };
    assert_eq!(
        paths.as_slice(),
        Some([new_dir].as_slice()),
        "the follow-up must name the created directory"
    );
    assert!(
        due >= called_at + TEST_FOLLOWUP_DELAY,
        "the follow-up must not come due before the follow-up delay has passed"
    );
}

/// A plain file-`Create` schedules no follow-up.
#[test]
fn plain_file_create_schedules_no_followup() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let file = tmp.path().join("new_file.txt");
    std::fs::write(&file, "data").expect("write file");

    let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
    let (callback, events) = recording_callback();
    let callback_arc = Arc::new(callback);
    let (scheduler, captured_schedules) = recording_scheduler();

    let evt = Event {
        kind: EventKind::Create(CreateKind::File),
        paths: vec![file],
        attrs: EventAttributes::default(),
    };
    FileSystemWatcher::process_event(
        &evt,
        &callback_arc,
        &registry,
        TEST_FOLLOWUP_DELAY,
        &scheduler,
    );

    assert_eq!(
        events.lock().unwrap().len(),
        1,
        "a file create should emit exactly one git event"
    );
    assert!(
        captured_schedules.lock().unwrap().is_empty(),
        "a plain file create must not schedule a follow-up rescan"
    );
}

/// A `CreateKind::Any` (e.g. `FSEvents`) on a real directory still schedules a
/// follow-up via the `is_dir()` fallback.
#[test]
fn ambiguous_create_of_directory_schedules_followup_via_is_dir() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let new_dir = tmp.path().join("ambiguous_dir");
    std::fs::create_dir_all(&new_dir).expect("create new dir");

    let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
    let (callback, events) = recording_callback();
    let callback_arc = Arc::new(callback);
    let (scheduler, captured_schedules) = recording_scheduler();

    let evt = Event {
        kind: EventKind::Create(CreateKind::Any),
        paths: vec![new_dir],
        attrs: EventAttributes::default(),
    };
    FileSystemWatcher::process_event(
        &evt,
        &callback_arc,
        &registry,
        TEST_FOLLOWUP_DELAY,
        &scheduler,
    );

    assert_eq!(
        events.lock().unwrap().len(),
        1,
        "the ambiguous create should still emit its own git event immediately"
    );
    assert_eq!(
        captured_schedules.lock().unwrap().len(),
        1,
        "an ambiguous create resolving to a real directory should schedule a follow-up"
    );
}

/// A `CreateKind::Any` on a plain file schedules no follow-up.
#[test]
fn ambiguous_create_of_file_schedules_no_followup() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let file = tmp.path().join("ambiguous.txt");
    std::fs::write(&file, "data").expect("write file");

    let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
    let (callback, events) = recording_callback();
    let callback_arc = Arc::new(callback);
    let (scheduler, captured_schedules) = recording_scheduler();

    let evt = Event {
        kind: EventKind::Create(CreateKind::Any),
        paths: vec![file],
        attrs: EventAttributes::default(),
    };
    FileSystemWatcher::process_event(
        &evt,
        &callback_arc,
        &registry,
        TEST_FOLLOWUP_DELAY,
        &scheduler,
    );

    assert_eq!(
        events.lock().unwrap().len(),
        1,
        "the ambiguous create should still emit its own git event immediately"
    );
    assert!(
        captured_schedules.lock().unwrap().is_empty(),
        "an ambiguous create of a plain file must not schedule a follow-up"
    );
}

/// A Language/Config path with no `.git` above it (e.g. `~/.config/gpy/config.toml`)
/// emits its specialized event but no Git refresh against a non-repo.
fn assert_path_outside_any_repo_emits_no_git_refresh(
    filename: &str,
    is_expected_specialized: impl Fn(&FileEvent) -> bool,
) {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    // Canonicalize for the macOS `/var` symlink.
    let dir = std::fs::canonicalize(tmp.path()).expect("canonicalize dir");
    // No `.git` anywhere in this tree.
    let file = dir.join(filename);
    std::fs::create_dir_all(file.parent().expect("file parent")).expect("parent dirs");
    std::fs::write(&file, "data").expect("write file");

    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.clone()],
    );

    assert_eq!(
        events.len(),
        1,
        "{filename} outside any repo should still emit the specialized \
         event but no Git refresh, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| is_expected_specialized(&e.event)),
        "expected the specialized event for {filename}, got: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.event, FileEvent::Git { .. })),
        "{filename} outside any repo must not emit a Git event, got: {events:?}"
    );
}

#[test]
fn config_file_outside_any_repo_does_not_emit_git_refresh() {
    assert_path_outside_any_repo_emits_no_git_refresh("config.toml", |event| {
        matches!(event, FileEvent::Config { .. })
    });
}

/// A theme path with no enclosing repository emits no Git refresh.
#[test]
fn theme_file_outside_any_repo_does_not_emit_git_refresh() {
    assert_path_outside_any_repo_emits_no_git_refresh("gpy/themes/x.toml", |event| {
        matches!(event, FileEvent::Theme { .. })
    });
}

#[test]
fn language_manifest_outside_any_repo_does_not_emit_git_refresh() {
    assert_path_outside_any_repo_emits_no_git_refresh("package.json", |event| {
        matches!(event, FileEvent::Language { .. })
    });
}

/// A metadata-only worktree event reaches the Git path.
#[test]
fn worktree_metadata_write_time_event_triggers_git_refresh() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    let file = repo.join("tracked.txt");
    std::fs::write(&file, b"hello").expect("write file");

    let registry = test_registry(true, HashSet::from([repo]));
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)),
        vec![file.clone()],
    );

    assert_eq!(
        events.len(),
        1,
        "an mtime-only change to a worktree file must trigger a git refresh \
         (this is all PollWatcher can report), got: {events:?}"
    );
}

/// A pure access-time event must not trigger a scan.
#[test]
fn worktree_access_time_event_does_not_trigger_git_refresh() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    let file = repo.join("tracked.txt");
    std::fs::write(&file, b"hello").expect("write file");

    let registry = test_registry(true, HashSet::from([repo]));
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
        vec![file.clone()],
    );

    assert!(
        events.is_empty(),
        "a read-only access-time touch must not wake the git refresh \
         pipeline, got: {events:?}"
    );
}
