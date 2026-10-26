#![allow(clippy::missing_panics_doc)]

use super::backend::poll_watch_targets;
use super::ignore::{force_added_set, is_force_added};
use super::probe::{
    DEFAULT_WATCH_POLL_MS, DEFAULT_WATCH_PROBE_MS, engage_poll_fallback,
    native_backend_delivers_events,
};
use super::*;
use crate::sync_ext::MutexExt;
use crate::watcher::{FileEvent, GitPaths, PendingCallback, PendingEvent};
use notify::event::{
    AccessKind, AccessMode, CreateKind, DataChange, EventAttributes, EventKind, Flag, MetadataKind,
    ModifyKind, RenameMode,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

impl FileSystemWatcher {
    /// The backend currently armed.
    fn backend_kind(&self) -> BackendKind {
        self.backend.lock_or_recover().kind
    }

    /// Swap to the polling backend on demand via the production swap path.
    fn force_poll_fallback(&self, poll_interval: Duration) {
        if let Some(event_tx) = self.event_tx.as_ref() {
            engage_poll_fallback(
                &self.backend,
                event_tx,
                &self.stop_flag,
                poll_interval,
                &self.registry,
            );
        }
    }

    /// Registrations with owner counts, and the armed OS watches.
    pub(crate) fn held_watches(
        &self,
    ) -> (
        BTreeMap<(PathBuf, WatchMode), u32>,
        BTreeMap<PathBuf, WatchMode>,
    ) {
        let guard = self.backend.lock_or_recover();
        let owners = guard
            .registrations
            .iter()
            .map(|(key, registration)| (key.clone(), registration.owners))
            .collect();
        (owners, guard.os.armed_paths())
    }
}

/// Follow-up delay used by `process_event` in tests: short, but long enough to
/// tell a non-directory event scheduled none.
const TEST_FOLLOWUP_DELAY: Duration = Duration::from_millis(50);

/// A private [`WatchRegistry`]; empty `roots` makes attribution fall through to
/// the ancestor `.git` walk.
fn test_registry(worktree_enabled: bool, roots: HashSet<PathBuf>) -> Arc<WatchRegistry> {
    configure_registry(WatchRegistry::new(), worktree_enabled, roots)
}

/// [`test_registry`] whose force-added sets come from `ls_files` instead of git.
fn test_registry_with_ls_files(
    worktree_enabled: bool,
    roots: HashSet<PathBuf>,
    ls_files: impl Fn(&Path) -> Vec<PathBuf> + Send + Sync + 'static,
) -> Arc<WatchRegistry> {
    let registry = WatchRegistry::with_ignore(IgnoreCaches::new(Arc::new(ls_files)));
    configure_registry(registry, worktree_enabled, roots)
}

fn configure_registry(
    registry: WatchRegistry,
    worktree_enabled: bool,
    roots: HashSet<PathBuf>,
) -> Arc<WatchRegistry> {
    registry.set_worktree_enabled(worktree_enabled);
    for root in roots {
        registry.insert_root(root);
    }
    Arc::new(registry)
}

/// Capture events emitted when running the provided callback.
fn run_with_capture<F: FnOnce(Arc<Mutex<Vec<PendingEvent>>>)>(call: F) -> Vec<PendingEvent> {
    let captured = Arc::new(Mutex::new(Vec::new()));
    call(Arc::clone(&captured));
    Arc::try_unwrap(captured).unwrap().into_inner().unwrap()
}

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
        let events = run_with_capture(|captured| {
            let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
                captured.lock().unwrap().push(event);
            }));

            let evt = Event {
                kind,
                paths: vec![PathBuf::from("/repo/.git/HEAD")],
                attrs: EventAttributes::default(),
            };

            FileSystemWatcher::process_event(
                &evt,
                &callback_arc,
                &registry,
                TEST_FOLLOWUP_DELAY,
                &no_op_scheduler(),
            );
        });

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
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            paths: vec![PathBuf::from("/repo/.git/HEAD")],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    assert_eq!(events.len(), 1, "the write half must still be delivered");
}

#[test]
fn gitdir_changes_trigger_without_aggressive_mode() {
    let registry = test_registry(true, HashSet::new());
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
            paths: vec![PathBuf::from("/repo/.git/HEAD")],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    assert_eq!(events.len(), 1, "Expected gitdir event to trigger");
    assert!(matches!(
        events.first().map(|e| &e.event),
        Some(FileEvent::Git { .. })
    ));
}

#[test]
fn worktree_events_skipped_when_disabled() {
    let registry = test_registry(false, HashSet::new());
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("/repo/new_file.txt")],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

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

    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

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

    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

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

/// A gitignored language manifest still emits Language but no Git refresh.
#[test]
fn gitignored_language_manifest_does_not_emit_git_refresh() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    // Canonicalize: a `/var` vs `/private/var` mismatch makes `ignore` panic.
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".gitignore"), "Cargo.toml\n").expect("write gitignore");
    let file = repo.join("Cargo.toml");
    std::fs::write(&file, "data").expect("write file");

    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    assert_eq!(
        events.len(),
        1,
        "a gitignored manifest should still emit the specialized event \
         but no Git refresh, got: {events:?}"
    );
    assert!(matches!(
        events.first().map(|e| &e.event),
        Some(FileEvent::Language { .. })
    ));
}

/// A canonicalized repo with `.git`, `gitignore` and the marker at `marker`
/// (relative), plus the edit events the marker produces.
fn language_marker_events(gitignore: &str, marker: &str) -> Vec<PendingEvent> {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    // Canonicalize: a `/var` vs `/private/var` mismatch makes `ignore` panic.
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".gitignore"), gitignore).expect("write gitignore");
    let file = repo.join(marker);
    std::fs::create_dir_all(file.parent().expect("parent")).expect("marker dir");
    std::fs::write(&file, "{}").expect("write marker");
    events_for_edit(&repo, &file)
}

fn has_language_event(events: &[PendingEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e.event, FileEvent::Language { .. }))
}

#[test]
fn ignored_language_marker_emits_no_language_event() {
    let events = language_marker_events("node_modules/\n", "node_modules/x/package.json");
    assert!(
        !has_language_event(&events),
        "a marker under an ignored directory must not trigger re-detection, got: {events:?}"
    );
}

#[test]
fn root_language_marker_still_emits() {
    let events = language_marker_events("node_modules/\n", "package.json");
    assert!(
        has_language_event(&events),
        "a root marker must emit a Language event, got: {events:?}"
    );
}

#[test]
fn root_marker_with_negated_wildcard_gitignore_still_emits() {
    let events = language_marker_events("*\n!package.json\n", "package.json");
    assert!(
        has_language_event(&events),
        "a root marker must emit a Language event, got: {events:?}"
    );
}

#[test]
fn non_git_marker_under_token_dir_still_emits() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let project = std::fs::canonicalize(tmp.path()).expect("canonicalize project");
    let file = project.join("node_modules/x/package.json");
    std::fs::create_dir_all(file.parent().expect("parent")).expect("marker dir");
    std::fs::write(&file, "{}").expect("write marker");

    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));
        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    assert!(
        has_language_event(&events),
        "a non-git project's marker must emit a Language event, got: {events:?}"
    );
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
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

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

/// An event outside every watched root falls back to `find_git_root`, then the parent.
#[test]
fn event_outside_watched_roots_falls_back_to_ancestor_walk() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let outside_dir = tmp.path().to_path_buf();
    let file = outside_dir.join("tracked.txt");
    std::fs::write(&file, "data").expect("write file");

    // An unrelated watched root that is NOT a prefix of `file`.
    let other_tmp = tempfile::TempDir::new().expect("other temp dir");
    let registry = test_registry(true, HashSet::from([other_tmp.path().to_path_buf()]));
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    let expected_repo = MultiRepoWatcher::find_git_root(&file)
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

/// A high-churn token in an ancestor of the repo root (`/work/build/repo`)
/// must not suppress events.
#[test]
fn ignored_ancestor_directory_does_not_suppress_worktree_events() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let watched_root = tmp.path().join("build").join("my-repo");
    let src_dir = watched_root.join("src");
    std::fs::create_dir_all(&src_dir).expect("nested dirs");
    let file = src_dir.join("main.rs");
    std::fs::write(&file, "fn main() {}").expect("write file");

    let registry = test_registry(true, HashSet::from([watched_root.clone()]));
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    assert_eq!(
        events.len(),
        1,
        "an in-repo edit must not be suppressed just because a `build` \
         ancestor sits above the watched repo root"
    );
    assert_eq!(
        events.first().map(|e| &e.repo),
        Some(&watched_root),
        "event should still attribute to the watched root"
    );
}

/// A high-churn token inside the repo still suppresses events.
#[test]
fn ignored_directory_within_repo_still_suppressed() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let watched_root = tmp.path().to_path_buf();
    let node_modules_dir = watched_root.join("node_modules").join("pkg");
    std::fs::create_dir_all(&node_modules_dir).expect("nested dirs");
    let file = node_modules_dir.join("index.js");
    std::fs::write(&file, "module.exports = {};").expect("write file");
    // The token is only a hint; the repo's own ignore rule suppresses.
    std::fs::write(watched_root.join(".gitignore"), "node_modules/\n").expect("gitignore");

    let registry = test_registry(true, HashSet::from([watched_root]));
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    assert!(
        events.is_empty(),
        "High-churn build directories inside the repo should stay filtered"
    );
}

#[test]
fn high_churn_dirs_filtered_when_enabled() {
    let registry = test_registry(true, HashSet::new());
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            paths: vec![PathBuf::from("/repo/node_modules/pkg/index.js")],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    assert!(
        events.is_empty(),
        "High-churn build directories should stay filtered"
    );
}

#[test]
fn gitignored_glob_paths_are_detected() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = tmp.path();
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".gitignore"), "*.log\nbuild_output\n").expect("gitignore");

    assert!(
        path_is_gitignored(repo, &repo.join("debug.log"), &registry),
        "files matching an ignore glob should be detected"
    );
    assert!(
        path_is_gitignored(repo, &repo.join("build_output"), &registry),
        "an ignored name should be detected"
    );
    assert!(
        !path_is_gitignored(repo, &repo.join("src.rs"), &registry),
        "tracked files should not be reported as ignored"
    );
}

/// Un-ignoring a path in `.gitignore` invalidates the cached matcher.
#[test]
fn unignored_path_is_no_longer_suppressed_after_gitignore_edit() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = tmp.path();
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    let gitignore = repo.join(".gitignore");
    let target = repo.join("artifact.txt");

    std::fs::write(&gitignore, "artifact.txt\n").expect("write gitignore");
    assert!(
        path_is_gitignored(repo, &target, &registry),
        "path should be ignored while the rule is present"
    );

    // Fingerprint changes, so the matcher must be rebuilt.
    std::fs::write(&gitignore, "# nothing ignored\n").expect("rewrite gitignore");
    assert!(
        !path_is_gitignored(repo, &target, &registry),
        "path should stop being suppressed once the ignore rule is removed"
    );
}

/// A new `.git/info/exclude` rule is picked up without a restart.
#[test]
fn new_info_exclude_rule_invalidates_cached_matcher() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = tmp.path();
    let info_dir = repo.join(".git/info");
    std::fs::create_dir_all(&info_dir).expect("git info dir");
    let target = repo.join("local-only.txt");

    assert!(
        !path_is_gitignored(repo, &target, &registry),
        "path should not be ignored before any exclude rule exists"
    );

    std::fs::write(info_dir.join("exclude"), "local-only.txt\n").expect("write exclude");
    assert!(
        path_is_gitignored(repo, &target, &registry),
        "a newly added .git/info/exclude rule should take effect"
    );
}

/// Run a git command in `dir`, panicking on failure.
fn git_in(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A force-added file matching an ignore glob is tracked and not suppressed;
/// a non-force-added match still is.
#[test]
fn force_added_tracked_file_is_not_suppressed() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

    git_in(&repo, &["init"]);
    git_in(&repo, &["config", "user.email", "test@example.com"]);
    git_in(&repo, &["config", "user.name", "Test User"]);
    git_in(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join(".gitignore"), "*.log\n").expect("write gitignore");
    std::fs::write(repo.join("ignored.log"), "tracked\n").expect("write ignored.log");
    std::fs::write(repo.join("other.log"), "untracked\n").expect("write other.log");
    git_in(&repo, &["add", "-f", "ignored.log"]);
    git_in(&repo, &["add", ".gitignore"]);
    git_in(&repo, &["commit", "-m", "init"]);

    assert!(
        !path_is_gitignored(&repo, &repo.join("ignored.log"), &registry),
        "a force-added (tracked) file must not be suppressed"
    );
    assert!(
        path_is_gitignored(&repo, &repo.join("other.log"), &registry),
        "an untracked file matching the ignore glob must still be suppressed"
    );
}

/// Feed a content edit of `file` through `process_event` with `root` watched.
fn events_for_edit(root: &Path, file: &Path) -> Vec<PendingEvent> {
    let registry = test_registry(true, HashSet::from([root.to_path_buf()]));
    run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));
        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.to_path_buf()],
            attrs: EventAttributes::default(),
        };
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    })
}

/// Directory-name tokens are a hint: tracked files under `vendor/`, `dist/`,
/// etc. refresh; ignored `node_modules/` churn doesn't.
#[test]
fn tracked_file_under_high_churn_token_dir_triggers_git_refresh() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

    git_in(&repo, &["init"]);
    git_in(&repo, &["config", "user.email", "test@example.com"]);
    git_in(&repo, &["config", "user.name", "Test User"]);
    git_in(&repo, &["config", "commit.gpgsign", "false"]);
    let tracked = [
        "vendor/lib.go",
        "dist/index.js",
        "build/script.sh",
        "target/keep.txt",
    ];
    for rel in tracked {
        let file = repo.join(rel);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dirs");
        std::fs::write(&file, "committed\n").expect("write tracked file");
    }
    git_in(&repo, &["add", "-A"]);
    git_in(&repo, &["commit", "-m", "init"]);

    for rel in tracked {
        let events = events_for_edit(&repo, &repo.join(rel));
        assert_eq!(events.len(), 1, "tracked `{rel}` edit must emit one event");
        assert_eq!(events.first().map(|e| &e.repo), Some(&repo), "`{rel}`");
        assert!(
            matches!(
                events.first().map(|e| &e.event),
                Some(FileEvent::Git { .. })
            ),
            "`{rel}` edit must be a Git event"
        );
    }

    // Ignored churn stays suppressed.
    std::fs::write(repo.join(".gitignore"), "node_modules/\ndist/\n").expect("gitignore");
    let ignored = repo.join("node_modules").join("pkg").join("index.js");
    std::fs::create_dir_all(ignored.parent().expect("parent")).expect("dirs");
    std::fs::write(&ignored, "x\n").expect("write ignored");
    assert!(
        events_for_edit(&repo, &ignored).is_empty(),
        "an ignored node_modules edit must stay suppressed"
    );

    // `dist/` is ignored, but `dist/index.js` is force-added.
    assert_eq!(
        events_for_edit(&repo, &repo.join("dist/index.js")).len(),
        1,
        "a tracked file inside an ignored dist/ must still emit an event"
    );
    let untracked = repo.join("dist").join("new.js");
    std::fs::write(&untracked, "x\n").expect("write untracked");
    assert!(
        events_for_edit(&repo, &untracked).is_empty(),
        "an untracked file in an ignored dist/ must stay suppressed"
    );
}

/// How long the stand-in `git ls-files` takes.
const SLOW_BACKEND_RUN: Duration = Duration::from_millis(300);

/// Budget for a second repo's lookup; well under [`SLOW_BACKEND_RUN`].
const FAST_LOOKUP_BUDGET: Duration = Duration::from_millis(100);

/// Generous ceiling for gates and async refreshes; only a hang reaches it.
const GATE_BUDGET: Duration = Duration::from_secs(10);

/// A one-shot gate: the stand-in backend opens it, the test waits on it.
type TestGate = Arc<(Mutex<bool>, std::sync::Condvar)>;

/// Create a closed [`TestGate`].
fn new_gate() -> TestGate {
    Arc::new((Mutex::new(false), std::sync::Condvar::new()))
}

/// Open `gate`, waking every waiter.
fn open_gate(gate: &TestGate) {
    let (flag, condvar) = &**gate;
    *flag.lock().unwrap() = true;
    condvar.notify_all();
}

/// Block until `gate` opens; `false` means `budget` elapsed first.
fn wait_for_gate(gate: &TestGate, budget: Duration) -> bool {
    let (flag, condvar) = &**gate;
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    let mut open = flag.lock().unwrap();
    while !*open {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let (next, _) = condvar.wait_timeout(open, remaining).unwrap();
        open = next;
    }
    drop(open);
    true
}

/// Give `repo` the `.git/index` [`force_added_set`] fingerprints, without `git init`.
fn fake_repo_with_index(tmp: &tempfile::TempDir) -> PathBuf {
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".git").join("index"), b"index").expect("write index");
    repo
}

/// As [`fake_repo_with_index`], but leaks `tmp` so the path can outlive it.
fn fake_repo_with_index_leaked(tmp: tempfile::TempDir) -> PathBuf {
    let repo = std::fs::canonicalize(tmp.keep()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".git").join("index"), b"index").expect("write index");
    repo
}

/// `force_added_set` must not hold the cache mutex while `git ls-files` runs.
#[test]
fn force_added_refresh_never_holds_the_cache_mutex() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = fake_repo_with_index(&tmp);

    let entered = new_gate();
    let release = new_gate();
    let entered_for_ls_files = Arc::clone(&entered);
    let release_for_ls_files = Arc::clone(&release);
    let registry = test_registry_with_ls_files(true, HashSet::new(), move |_| {
        open_gate(&entered_for_ls_files);
        wait_for_gate(&release_for_ls_files, GATE_BUDGET);
        Vec::new()
    });

    let repo_for_worker = repo.clone();
    let registry_for_worker = Arc::clone(&registry);
    let worker = std::thread::spawn(move || {
        drop(force_added_set(&repo_for_worker, &registry_for_worker));
    });

    assert!(
        wait_for_gate(&entered, GATE_BUDGET),
        "the stand-in backend should have been entered"
    );
    let cache_free_during_git = registry.ignore().force_added.try_lock().is_ok();
    open_gate(&release);
    worker.join().expect("worker thread");

    assert!(
        cache_free_during_git,
        "the force-added cache mutex must be free while `git ls-files` runs"
    );
}

/// One repo's slow `git ls-files` must not stall another repo's
/// lookup through the shared force-added cache mutex.
#[test]
fn a_slow_repo_does_not_delay_another_repos_lookup() {
    let slow_tmp = tempfile::TempDir::new().expect("temp dir");
    let fast_tmp = tempfile::TempDir::new().expect("temp dir");
    let slow_repo = fake_repo_with_index(&slow_tmp);
    let fast_repo = fake_repo_with_index(&fast_tmp);

    let entered = new_gate();
    let entered_for_ls_files = Arc::clone(&entered);
    let slow_for_ls_files = slow_repo.clone();
    let registry = test_registry_with_ls_files(true, HashSet::new(), move |root| {
        if root == slow_for_ls_files {
            open_gate(&entered_for_ls_files);
            std::thread::sleep(SLOW_BACKEND_RUN);
        }
        Vec::new()
    });

    let slow_for_worker = slow_repo.clone();
    let registry_for_worker = Arc::clone(&registry);
    let worker = std::thread::spawn(move || {
        drop(force_added_set(&slow_for_worker, &registry_for_worker));
    });

    assert!(
        wait_for_gate(&entered, GATE_BUDGET),
        "the slow stand-in backend should have been entered"
    );
    let started = Instant::now();
    drop(force_added_set(&fast_repo, &registry));
    let elapsed = started.elapsed();

    worker.join().expect("worker thread");

    assert!(
        elapsed < FAST_LOOKUP_BUDGET,
        "a lookup for an unrelated repo took {elapsed:?}, so it queued behind \
         the slow repo's `git ls-files`"
    );
}

/// One repo's cold-start poll walk (which may run `git ls-files`) must not
/// block another repo's `watch()` on the shared backend mutex.
#[test]
fn cold_start_for_one_repo_does_not_block_another_repos_watch() {
    let primary_tmp = tempfile::TempDir::new().expect("temp dir");
    let secondary_tmp = tempfile::TempDir::new().expect("temp dir");
    let repo_a = fake_repo_with_index(&primary_tmp);
    let repo_b = fake_repo_with_index(&secondary_tmp);

    // Ignored, so the descent reaches `force_added_set`.
    std::fs::write(repo_a.join(".gitignore"), b"ignored/\n").expect("write .gitignore");
    std::fs::create_dir_all(repo_a.join("ignored")).expect("ignored dir");

    let entered = new_gate();
    let release = new_gate();
    let entered_for_ls_files = Arc::clone(&entered);
    let release_for_ls_files = Arc::clone(&release);
    let gated_repo = repo_a.clone();
    let registry = test_registry_with_ls_files(
        true,
        HashSet::from([repo_a.clone(), repo_b.clone()]),
        move |root| {
            if root == gated_repo {
                open_gate(&entered_for_ls_files);
                wait_for_gate(&release_for_ls_files, GATE_BUDGET);
            }
            Vec::new()
        },
    );
    let (callback, _events) = recording_callback_boxed();
    let watcher = FileSystemWatcher::with_fallback_policy(
        callback,
        Some(Arc::clone(&registry)),
        Duration::from_millis(50),
        no_op_scheduler(),
        FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_secs(3_600),
        },
    )
    .expect("create watcher");

    std::thread::scope(|scope| {
        let watcher_ref = &watcher;
        let repo_a_for_worker = repo_a.clone();
        let worker = scope.spawn(move || {
            watcher_ref.watch(&repo_a_for_worker).expect("watch repo a");
        });

        assert!(
            wait_for_gate(&entered, GATE_BUDGET),
            "the stand-in backend for repo A's cold start should have been entered"
        );

        let started = Instant::now();
        watcher.watch(&repo_b).expect("watch repo b");
        let elapsed = started.elapsed();

        open_gate(&release);
        worker.join().expect("worker thread");

        assert!(
            elapsed < FAST_LOOKUP_BUDGET,
            "watch() for an unrelated repo took {elapsed:?} while repo A's cold \
             start was still running, so it queued behind the shared backend \
             mutex"
        );
    });
}

/// Fixture parking `force_poll_fallback` between its snapshot and the swap,
/// so repo B arms natively and must be re-armed by the race fallback.
fn swap_race_fixture() -> (FileSystemWatcher, PathBuf, PathBuf, TestGate, TestGate) {
    let repo_a = fake_repo_with_index_leaked(tempfile::TempDir::new().expect("temp dir"));
    let repo_b = fake_repo_with_index_leaked(tempfile::TempDir::new().expect("temp dir"));
    std::fs::write(repo_a.join(".gitignore"), b"ignored/\n").expect("write .gitignore");
    std::fs::create_dir_all(repo_a.join("ignored")).expect("ignored dir");

    // Gates repo A's `git ls-files` only once armed, after its native watch.
    let armed = Arc::new(AtomicBool::new(false));
    let entered = new_gate();
    let release = new_gate();
    let armed_for_ls_files = Arc::clone(&armed);
    let entered_for_ls_files = Arc::clone(&entered);
    let release_for_ls_files = Arc::clone(&release);
    let gated_repo = repo_a.clone();
    let registry = test_registry_with_ls_files(
        true,
        HashSet::from([repo_a.clone(), repo_b.clone()]),
        move |root| {
            if root == gated_repo && armed_for_ls_files.load(Ordering::SeqCst) {
                open_gate(&entered_for_ls_files);
                wait_for_gate(&release_for_ls_files, GATE_BUDGET);
            }
            Vec::new()
        },
    );
    let (callback, _events) = recording_callback_boxed();
    let watcher = FileSystemWatcher::with_fallback_policy(
        callback,
        Some(Arc::clone(&registry)),
        Duration::from_millis(50),
        no_op_scheduler(),
        FallbackPolicy::NativeOnly,
    )
    .expect("create watcher");
    watcher.watch(&repo_a).expect("watch repo a natively");
    armed.store(true, Ordering::SeqCst);

    (watcher, repo_a, repo_b, entered, release)
}

#[test]
fn watch_racing_the_native_to_poll_swap_still_ends_up_armed_correctly() {
    let (watcher, _repo_a, repo_b, entered, release) = swap_race_fixture();

    std::thread::scope(|scope| {
        let watcher_ref = &watcher;
        let swap = scope.spawn(move || {
            watcher_ref.force_poll_fallback(Duration::from_secs(3_600));
        });

        assert!(
            wait_for_gate(&entered, GATE_BUDGET),
            "the swap should have parked on repo A's cold start"
        );
        assert_eq!(
            watcher.backend_kind(),
            BackendKind::Native,
            "the swap must not have applied yet -- it is parked before its own \
             final lock"
        );

        watcher
            .watch(&repo_b)
            .expect("watch repo b while the swap is parked");

        open_gate(&release);
        swap.join().expect("swap thread");
    });

    assert_eq!(
        watcher.backend_kind(),
        BackendKind::Poll,
        "the swap must have completed"
    );
    let guard = watcher.backend.lock().expect("backend lock");
    let registration = guard
        .registrations
        .get(&(repo_b.clone(), WatchMode::Recursive))
        .expect("repo B must still be a watched path after the swap");
    assert!(
        registration.targets.contains(&(repo_b, WatchMode::Shallow)),
        "repo B, added while the swap was mid-flight and so absent from its \
         pre-swap snapshot, must still end up re-armed on the poll backend \
         with its expansion, got: {:?}",
        registration.targets
    );
    drop(guard);
}

/// After the off-thread refresh, a newly force-added file is picked up and the
/// earlier one stays exempt.
#[test]
fn force_added_set_picks_up_an_index_change_after_the_refresh() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

    git_in(&repo, &["init"]);
    git_in(&repo, &["config", "user.email", "test@example.com"]);
    git_in(&repo, &["config", "user.name", "Test User"]);
    git_in(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join(".gitignore"), "*.log\n").expect("write gitignore");
    std::fs::write(repo.join("first.log"), "tracked\n").expect("write first.log");
    std::fs::write(repo.join("other.log"), "untracked\n").expect("write other.log");
    git_in(&repo, &["add", "-f", "first.log"]);
    git_in(&repo, &["add", ".gitignore"]);
    git_in(&repo, &["commit", "-m", "init"]);

    assert!(
        !path_is_gitignored(&repo, &repo.join("first.log"), &registry),
        "the force-added file must not be suppressed"
    );

    // A second force-add rewrites `.git/index`, staling the cached set.
    let second = repo.join("second.log");
    std::fs::write(&second, "also tracked\n").expect("write second.log");
    git_in(&repo, &["add", "-f", "second.log"]);

    let deadline = Instant::now()
        .checked_add(GATE_BUDGET)
        .unwrap_or_else(Instant::now);
    let mut picked_up = false;
    while Instant::now() < deadline {
        if is_force_added(&repo, &second, &registry) {
            picked_up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        picked_up,
        "the newly force-added file must appear once the refresh completes"
    );
    assert!(
        !path_is_gitignored(&repo, &repo.join("first.log"), &registry),
        "the original force-add must survive the refresh"
    );
    assert!(
        path_is_gitignored(&repo, &repo.join("other.log"), &registry),
        "an untracked file matching the ignore glob must still be suppressed"
    );
}

/// A nested `.gitignore` un-ignoring a root-ignored path is honored and stays live.
#[test]
fn nested_unignore_is_honored_and_invalidates_on_edit() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = tmp.path();
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    std::fs::write(repo.join(".gitignore"), "build/\n").expect("root gitignore");

    let subdir = repo.join("subdir");
    std::fs::create_dir_all(subdir.join("build")).expect("nested build dir");
    let target = subdir.join("build").join("foo");
    std::fs::write(&target, "x").expect("write target");

    assert!(
        path_is_gitignored(repo, &target, &registry),
        "root build/ rule should ignore the nested path with no override"
    );

    std::fs::write(subdir.join(".gitignore"), "!build/\n").expect("nested un-ignore");
    assert!(
        !path_is_gitignored(repo, &target, &registry),
        "a nested !build/ rule must re-include the path"
    );

    // Different length, so the fingerprint flips deterministically.
    std::fs::write(subdir.join(".gitignore"), "# no override here now\n")
        .expect("rewrite nested gitignore");
    assert!(
        path_is_gitignored(repo, &target, &registry),
        "dropping the nested negation must re-ignore the path without a restart"
    );
}

/// Collect events into a shared buffer that outlives `process_event`; never
/// `try_unwrap`s, so the watcher's own `Arc` clone can't fail the read.
fn recording_callback() -> (Arc<PendingCallback>, CapturedEvents) {
    let captured: Arc<Mutex<Vec<PendingEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_for_cb = Arc::clone(&captured);
    let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
        captured_for_cb.lock().unwrap().push(event);
    }));
    (callback_arc, captured)
}

/// Shared buffer of events captured by a recording callback.
type CapturedEvents = Arc<Mutex<Vec<PendingEvent>>>;

/// A scheduler that does nothing, for paths that never schedule a follow-up.
fn no_op_scheduler() -> DelayedEventScheduler {
    Arc::new(|_, _| {})
}

/// Every `(PendingEvent, Instant)` a scheduler was invoked with.
type CapturedSchedules = Arc<Mutex<Vec<(PendingEvent, Instant)>>>;

/// A scheduler that records requests, so tests assert on the scheduling
/// decision; delivery is `DebounceEngine`'s contract.
fn recording_scheduler() -> (DelayedEventScheduler, CapturedSchedules) {
    let captured: CapturedSchedules = Arc::new(Mutex::new(Vec::new()));
    let captured_for_hook = Arc::clone(&captured);
    let scheduler: DelayedEventScheduler = Arc::new(move |pending, due| {
        captured_for_hook.lock().unwrap().push((pending, due));
    });
    (scheduler, captured)
}

/// Like [`recording_callback`], but yields the bare [`PendingCallback`].
fn recording_callback_boxed() -> (PendingCallback, CapturedEvents) {
    let captured: Arc<Mutex<Vec<PendingEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_for_cb = Arc::clone(&captured);
    let callback: PendingCallback = Box::new(move |event| {
        captured_for_cb.lock().unwrap().push(event);
    });
    (callback, captured)
}

/// A directory create emits its own git event and schedules exactly one
/// follow-up, rescanning files whose creates raced the not-yet-armed watch.
#[test]
fn directory_create_schedules_one_followup_git_event() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let new_dir = tmp.path().join("newmod");
    std::fs::create_dir_all(&new_dir).expect("create new dir");

    let registry = test_registry(true, HashSet::from([tmp.path().to_path_buf()]));
    let (callback_arc, events) = recording_callback();
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
    let (callback_arc, events) = recording_callback();
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
    let (callback_arc, events) = recording_callback();
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
    let (callback_arc, events) = recording_callback();
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

/// A no-paths overflow (e.g. inotify `IN_Q_OVERFLOW`, surfaced as `Ok` with
/// `need_rescan()`) rescans every watched root.
#[test]
fn rescan_overflow_with_no_paths_triggers_all_watched_roots() {
    let tmp1 = tempfile::TempDir::new().expect("temp dir 1");
    let tmp2 = tempfile::TempDir::new().expect("temp dir 2");
    let root1 = tmp1.path().to_path_buf();
    let root2 = tmp2.path().to_path_buf();

    let registry = test_registry(true, HashSet::from([root1.clone(), root2.clone()]));
    let (callback_arc, events) = recording_callback();

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
    let (callback_arc, events) = recording_callback();

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
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
            paths: vec![PathBuf::from("/repo/.git/HEAD")],
            attrs: EventAttributes::default(),
        };
        assert!(
            !evt.need_rescan(),
            "test fixture must not be a rescan event"
        );

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

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
    let (callback_arc, events) = recording_callback();

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
    let (callback_arc, events) = recording_callback();

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

    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));
        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };
        // Matches the real theme/config watcher.
        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

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

/// A real worktree write is delivered well before reconcile whatever the
/// native backend's health (falling back to `PollWatcher` if `FSEvents` is silent).
#[test]
fn real_worktree_write_is_delivered_even_when_native_backend_is_silent() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");

    let registry = test_registry(true, HashSet::from([repo.clone()]));
    let (callback, events) = recording_callback_boxed();

    let mut watcher = FileSystemWatcher::new(
        callback,
        Some(Arc::clone(&registry)),
        Duration::from_millis(50),
        no_op_scheduler(),
    )
    .expect("create watcher");
    watcher.watch(&repo).expect("watch repo");

    // Let the recursive watch arm before racing it.
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(repo.join("untracked.txt"), b"content").expect("write worktree file");

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut delivered = false;
    while Instant::now() < deadline {
        if !events.lock().unwrap().is_empty() {
            delivered = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    watcher.stop();

    assert!(
        delivered,
        "a real worktree write must be delivered without waiting for the \
         periodic reconcile, even when the native backend is silent"
    );
}

/// A real repo plus a worktree-enabled `FileSystemWatcher` under `policy`.
fn real_repo_watcher(policy: FallbackPolicy) -> (FileSystemWatcher, PathBuf, CapturedEvents) {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    // Leaked: the watcher must outlive the `TempDir`; tests stop it explicitly.
    let repo = std::fs::canonicalize(tmp.keep()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");

    let registry = test_registry(true, HashSet::from([repo.clone()]));
    let (callback, events) = recording_callback_boxed();
    let watcher = FileSystemWatcher::with_fallback_policy(
        callback,
        Some(Arc::clone(&registry)),
        Duration::from_millis(50),
        no_op_scheduler(),
        policy,
    )
    .expect("create watcher");
    (watcher, repo, events)
}

/// Poll `events` until non-empty or `budget` elapses.
fn wait_for_any_event(events: &CapturedEvents, budget: Duration) -> bool {
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    while Instant::now() < deadline {
        if !events.lock().unwrap().is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// The `PollWatcher` fallback observes real worktree changes.
#[test]
fn poll_fallback_backend_delivers_real_worktree_writes() {
    let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });
    assert_eq!(
        watcher.backend_kind(),
        BackendKind::Poll,
        "ForcePoll must arm the polling backend without any probe"
    );

    watcher.watch(&repo).expect("watch repo");
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(repo.join("untracked.txt"), b"content").expect("write worktree file");

    let delivered = wait_for_any_event(&events, Duration::from_secs(5));
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

    assert!(
        delivered,
        "the poll fallback backend must deliver real worktree writes"
    );
}

/// `stop` wakes the blocked event thread even though the poll backend keeps a
/// sender clone alive, so a long poll interval cannot hold the join.
#[test]
fn stop_returns_promptly_with_poll_backend() {
    let (mut watcher, repo, _events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_secs(60),
    });
    watcher.watch(&repo).expect("watch repo");

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        watcher.stop();
        let _ = done_tx.send(());
    });
    let stopped = done_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    let _ = std::fs::remove_dir_all(&repo);

    assert!(stopped, "stop must return within 2s with the poll backend");
}

/// The poll fallback delivers edits to an existing file, which `PollWatcher`
/// reports only as `Modify(Metadata(WriteTime))`.
#[test]
fn poll_fallback_backend_delivers_edits_to_existing_files() {
    let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });

    // Seeded before arming, so only a later append can produce an event.
    let tracked = repo.join("tracked.txt");
    std::fs::write(&tracked, b"hello\n").expect("seed tracked file");
    watcher.watch(&repo).expect("watch repo");

    // Re-append on an interval: the baseline is taken on a background thread,
    // and an append landing before it would never be seen.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut delivered = false;
    while std::time::Instant::now() < deadline {
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&tracked)
            .expect("open tracked file for append");
        std::io::Write::write_all(&mut handle, b"modified\n").expect("append");
        drop(handle);

        if wait_for_any_event(&events, Duration::from_millis(500)) {
            delivered = true;
            break;
        }
    }
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

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
    let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });
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

    let ref_file = repo.join("branch-ref");
    pin(&ref_file, b"1111111111111111111111111111111111111111\n");
    watcher.watch(&repo).expect("watch repo");

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
        if wait_for_any_event(&events, Duration::from_millis(500)) {
            delivered = true;
            break;
        }
    }
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

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
    let (mut watcher, outer, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });
    let inner = outer.join("inner");
    let sub = inner.join("sub");
    std::fs::create_dir_all(&sub).expect("create inner/sub");
    let tracked = sub.join("f.txt");
    std::fs::write(&tracked, b"seed\n").expect("seed inner file");

    watcher.watch(&outer).expect("watch outer");
    watcher.watch(&inner).expect("watch inner");
    watcher.unwatch(&outer).expect("unwatch outer");
    let still_covered = watcher.covers(&sub);
    events.lock().unwrap().clear();

    // Re-append until delivered (baseline race).
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut delivered = false;
    while Instant::now() < deadline {
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&tracked)
            .expect("open inner file for append");
        std::io::Write::write_all(&mut handle, b"modified\n").expect("append");
        drop(handle);

        if wait_for_any_event(&events, Duration::from_millis(500)) {
            delivered = true;
            break;
        }
    }
    watcher.stop();
    let _ = std::fs::remove_dir_all(&outer);

    assert!(still_covered, "the inner root still covers its subtree");
    assert!(
        delivered,
        "unwatching the outer root must not remove the inner root's poll targets"
    );
}

/// Whether `events` holds a Git event naming `path`.
fn has_git_event_for(events: &CapturedEvents, path: &Path) -> bool {
    events
        .lock()
        .unwrap()
        .iter()
        .any(|pending| match &pending.event {
            FileEvent::Git { paths } => paths
                .as_slice()
                .is_some_and(|named| named.iter().any(|named_path| named_path == path)),
            _ => false,
        })
}

/// Poll `events` until it holds a Git event naming `path`, or `budget` elapses.
fn wait_for_git_event_for(events: &CapturedEvents, path: &Path, budget: Duration) -> bool {
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    while Instant::now() < deadline {
        if has_git_event_for(events, path) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// The OS watches `watcher` holds armed, by path.
fn armed_dirs(watcher: &FileSystemWatcher) -> BTreeSet<PathBuf> {
    watcher.held_watches().1.into_keys().collect()
}

/// The poll targets the recursive registration of `root` holds.
fn poll_targets_of(watcher: &FileSystemWatcher, root: &Path) -> BTreeSet<PathBuf> {
    let guard = watcher.backend.lock().expect("backend lock");
    guard
        .registrations
        .get(&(root.to_path_buf(), WatchMode::Recursive))
        .map(|registration| {
            registration
                .targets
                .iter()
                .map(|(target, _)| target.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The union of every registration's targets: what must be armed.
fn all_targets(watcher: &FileSystemWatcher) -> BTreeSet<PathBuf> {
    let guard = watcher.backend.lock().expect("backend lock");
    guard
        .registrations
        .values()
        .flat_map(|registration| registration.targets.iter())
        .map(|(target, _)| target.clone())
        .collect()
}

/// Poll until `done` holds, or `budget` elapses.
fn wait_until(budget: Duration, done: impl Fn() -> bool) -> bool {
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// Under the poll backend, an edit inside a directory created after arming is delivered.
#[test]
fn poll_backend_watches_directories_created_after_arming() {
    let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });
    watcher.watch(&repo).expect("watch repo");

    let new_dir = repo.join("newdir");
    std::fs::create_dir_all(&new_dir).expect("create newdir");
    let tracked = new_dir.join("a.txt");
    std::fs::write(&tracked, b"seed\n").expect("seed newdir/a.txt");

    // Let the creation and its stragglers land so only the edits below count.
    let created = wait_for_any_event(&events, Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(400));
    events.lock().unwrap().clear();

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

        if wait_for_git_event_for(&events, &tracked, Duration::from_millis(500)) {
            delivered = true;
            break;
        }
    }
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

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
    let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });
    std::fs::remove_dir_all(repo.join(".git")).expect("drop the placeholder .git");
    git_in(&repo, &["init", "-q"]);
    git_in(&repo, &["config", "user.email", "test@example.com"]);
    git_in(&repo, &["config", "user.name", "Test User"]);
    git_in(&repo, &["config", "commit.gpgsign", "false"]);
    git_in(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    watcher.watch(&repo).expect("watch repo");

    // Creates `.git/refs/heads/feature/` after arming.
    git_in(&repo, &["checkout", "-q", "-b", "feature/x"]);
    let feature_ref = repo.join(".git/refs/heads/feature/x");

    // Commit until one is reported, as above.
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);
    let mut delivered = false;
    while Instant::now() < deadline {
        git_in(
            &repo,
            &["commit", "-q", "--allow-empty", "-m", "on feature"],
        );
        if wait_for_git_event_for(&events, &feature_ref, Duration::from_millis(500)) {
            delivered = true;
            break;
        }
    }
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

    assert!(
        delivered,
        "a commit moving refs/heads/feature/x, whose directory was created \
         after arming, must be delivered under the poll backend"
    );
}

/// Repeated mkdir/rmdir cycles return the poll expansion to its baseline.
#[test]
fn poll_backend_drops_expansion_for_removed_directory() {
    let (mut watcher, repo, _events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });
    watcher.watch(&repo).expect("watch repo");
    let baseline = poll_targets_of(&watcher, &repo);
    let new_dir = repo.join("newdir");
    let nested = new_dir.join("nested");

    let mut cycles = Vec::new();
    for _ in 0_u8..3_u8 {
        std::fs::create_dir_all(&nested).expect("create newdir/nested");
        let expanded = wait_until(Duration::from_secs(5), || {
            let targets = poll_targets_of(&watcher, &repo);
            targets.contains(&new_dir) && targets.contains(&nested)
        });
        let consistent_expanded = armed_dirs(&watcher) == all_targets(&watcher);

        std::fs::remove_dir_all(&new_dir).expect("remove newdir");
        let dropped = wait_until(Duration::from_secs(5), || {
            poll_targets_of(&watcher, &repo) == baseline
        });
        let consistent_dropped = armed_dirs(&watcher) == all_targets(&watcher);
        cycles.push((expanded, consistent_expanded, dropped, consistent_dropped));
    }
    let armed_after = armed_dirs(&watcher);
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

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
    let (mut watcher, repo, _events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });
    std::fs::write(repo.join(".gitignore"), b"node_modules/\n").expect("write .gitignore");
    watcher.watch(&repo).expect("watch repo");

    let ignored = repo.join("node_modules");
    let control = repo.join("src");
    std::fs::create_dir_all(ignored.join("pkg")).expect("create node_modules/pkg");
    std::fs::create_dir_all(&control).expect("create src");

    // `src` proves the create path ran; the sleep lets `node_modules` land too.
    let control_armed = wait_until(Duration::from_secs(5), || {
        armed_dirs(&watcher).contains(&control)
    });
    std::thread::sleep(Duration::from_millis(400));
    let armed = armed_dirs(&watcher);
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

    assert!(control_armed, "a new non-ignored directory must be armed");
    assert!(
        !armed.iter().any(|path| path.starts_with(&ignored)),
        "a new gitignored directory must not be armed, got: {armed:?}"
    );
}

/// A directory under two overlapping roots stays armed until both release it.
#[test]
fn poll_expansion_of_new_directory_is_held_by_every_covering_root() {
    let (mut watcher, outer, _events) = real_repo_watcher(FallbackPolicy::ForcePoll {
        poll_interval: Duration::from_millis(200),
    });
    let inner = outer.join("inner");
    std::fs::create_dir_all(&inner).expect("create inner");
    watcher.watch(&outer).expect("watch outer");
    watcher.watch(&inner).expect("watch inner");

    let new_dir = inner.join("newdir");
    std::fs::create_dir_all(&new_dir).expect("create inner/newdir");
    let held_by_both = wait_until(Duration::from_secs(5), || {
        poll_targets_of(&watcher, &outer).contains(&new_dir)
            && poll_targets_of(&watcher, &inner).contains(&new_dir)
    });
    let consistent = armed_dirs(&watcher) == all_targets(&watcher);

    watcher.unwatch(&outer).expect("unwatch outer");
    let kept = armed_dirs(&watcher).contains(&new_dir);
    watcher.unwatch(&inner).expect("unwatch inner");
    let left = armed_dirs(&watcher);
    watcher.stop();
    let _ = std::fs::remove_dir_all(&outer);

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

/// A metadata-only worktree event reaches the Git path.
#[test]
fn worktree_metadata_write_time_event_triggers_git_refresh() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    let file = repo.join("tracked.txt");
    std::fs::write(&file, b"hello").expect("write file");

    let registry = test_registry(true, HashSet::from([repo]));
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

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
    let events = run_with_capture(|captured| {
        let callback_arc: Arc<PendingCallback> = Arc::new(Box::new(move |event| {
            captured.lock().unwrap().push(event);
        }));

        let evt = Event {
            kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
            paths: vec![file.clone()],
            attrs: EventAttributes::default(),
        };

        FileSystemWatcher::process_event(
            &evt,
            &callback_arc,
            &registry,
            TEST_FOLLOWUP_DELAY,
            &no_op_scheduler(),
        );
    });

    assert!(
        events.is_empty(),
        "a read-only access-time touch must not wake the git refresh \
         pipeline, got: {events:?}"
    );
}

/// A backend swap re-arms every already-registered directory; the file is
/// written only after the swap.
#[test]
fn backend_swap_rearms_already_watched_paths() {
    // Probe disabled so the outcome doesn't depend on this machine's backend.
    let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::NativeOnly);

    watcher.watch(&repo).expect("watch repo");
    assert_eq!(watcher.backend_kind(), BackendKind::Native);

    watcher.force_poll_fallback(Duration::from_millis(200));
    assert_eq!(
        watcher.backend_kind(),
        BackendKind::Poll,
        "the swap must leave the polling backend armed"
    );

    events.lock().unwrap().clear();
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(repo.join("after_swap.txt"), b"content").expect("write worktree file");

    let delivered = wait_for_any_event(&events, Duration::from_secs(5));
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

    assert!(
        delivered,
        "a repo watched before the backend swap must still be watched after it"
    );
}

/// A backend swap emits one rescan per re-armed root, since `PollWatcher`
/// misses changes made before arming.
#[test]
fn backend_swap_emits_rescan_for_each_rearmed_root() {
    let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::NativeOnly);
    watcher.watch(&repo).expect("watch repo");
    events.lock().unwrap().clear();

    watcher.force_poll_fallback(Duration::from_secs(1));

    let delivered = wait_for_any_event(&events, Duration::from_secs(2));
    let repos: Vec<PathBuf> = events
        .lock()
        .unwrap()
        .iter()
        .map(|event| event.repo.clone())
        .collect();
    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

    assert!(
        delivered,
        "the swap must emit a rescan for the watched root"
    );
    assert!(
        repos.contains(&repo),
        "the rescan must be attributed to the re-armed root, got: {repos:?}"
    );
}

/// A second swap must not rebuild a working poll backend.
#[test]
fn repeated_swap_is_a_no_op() {
    let (mut watcher, repo, events) = real_repo_watcher(FallbackPolicy::NativeOnly);
    watcher.watch(&repo).expect("watch repo");

    watcher.force_poll_fallback(Duration::from_secs(1));
    let after_first = wait_for_any_event(&events, Duration::from_secs(2));
    // Let the first swap's rescan burst land, or slow runners miscount it.
    std::thread::sleep(Duration::from_millis(400));
    events.lock().unwrap().clear();
    watcher.force_poll_fallback(Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(200));
    let rescans_after_second = events.lock().unwrap().len();

    watcher.stop();
    let _ = std::fs::remove_dir_all(&repo);

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

/// The health probe is a self-contained round trip that terminates within its
/// bound; either verdict is valid for the machine.
#[test]
fn health_probe_terminates_within_its_timeout() {
    let stop_flag = AtomicBool::new(false);
    let started = Instant::now();
    let _verdict = native_backend_delivers_events(Duration::from_millis(600), &stop_flag);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the probe must respect its timeout, took {:?}",
        started.elapsed()
    );
}

/// A stopping watcher aborts the probe and reports "healthy".
#[test]
fn health_probe_aborts_and_reports_healthy_when_stopping() {
    let stop_flag = AtomicBool::new(true);
    assert!(
        native_backend_delivers_events(Duration::from_secs(30), &stop_flag),
        "an aborted probe must not report the native backend as silent"
    );
}

#[test]
fn fallback_policy_defaults_to_probing_for_repo_watchers() {
    assert_eq!(
        FallbackPolicy::resolve(None, None, None),
        FallbackPolicy::ProbeThenFallback {
            probe_timeout: Duration::from_millis(DEFAULT_WATCH_PROBE_MS),
            poll_interval: Duration::from_millis(DEFAULT_WATCH_POLL_MS),
        }
    );
}

/// Theme/config watchers get the same probe-and-swap as the repo watcher.
#[test]
fn fallback_policy_probes_for_watchers_without_a_repo_registry() {
    assert_eq!(
        FallbackPolicy::resolve(None, None, None),
        FallbackPolicy::ProbeThenFallback {
            probe_timeout: Duration::from_millis(DEFAULT_WATCH_PROBE_MS),
            poll_interval: Duration::from_millis(DEFAULT_WATCH_POLL_MS),
        },
        "config/theme watchers must auto-detect a silent backend too"
    );
}

#[test]
fn fallback_policy_probe_zero_disables_auto_detection() {
    assert_eq!(
        FallbackPolicy::resolve(None, Some("0"), None),
        FallbackPolicy::NativeOnly
    );
}

#[test]
fn fallback_policy_honors_forced_poll() {
    assert_eq!(
        FallbackPolicy::resolve(Some("1"), None, Some("750")),
        FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(750),
        }
    );
}

#[test]
fn fallback_policy_ignores_unparsable_and_zero_overrides() {
    assert_eq!(
        FallbackPolicy::resolve(Some("no"), Some("abc"), Some("0")),
        FallbackPolicy::ProbeThenFallback {
            probe_timeout: Duration::from_millis(DEFAULT_WATCH_PROBE_MS),
            poll_interval: Duration::from_millis(DEFAULT_WATCH_POLL_MS),
        }
    );
}

#[test]
fn fallback_policy_honors_custom_probe_and_poll_intervals() {
    assert_eq!(
        FallbackPolicy::resolve(None, Some(" 500 "), Some(" 1500 ")),
        FallbackPolicy::ProbeThenFallback {
            probe_timeout: Duration::from_millis(500),
            poll_interval: Duration::from_millis(1_500),
        }
    );
}
