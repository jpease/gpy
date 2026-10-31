use super::*;

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

    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.clone()],
    );

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

    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.clone()],
    );

    assert!(
        has_language_event(&events),
        "a non-git project's marker must emit a Language event, got: {events:?}"
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
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.clone()],
    );

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
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        vec![file.clone()],
    );

    assert!(
        events.is_empty(),
        "High-churn build directories inside the repo should stay filtered"
    );
}

#[test]
fn high_churn_dirs_filtered_when_enabled() {
    let registry = test_registry(true, HashSet::new());
    let events = events_for(
        &registry,
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        vec![PathBuf::from("/repo/node_modules/pkg/index.js")],
    );

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

/// A force-added file matching an ignore glob is tracked and not suppressed;
/// a non-force-added match still is.
#[test]
fn force_added_tracked_file_is_not_suppressed() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

    git_init(&repo);
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

/// A main repository with one commit (`.gitignore` = `gitignore`) and a linked
/// worktree beside it; returns `(main, worktree)`.
fn main_with_linked_worktree(tmp: &Path, gitignore: &str) -> (PathBuf, PathBuf) {
    let base = std::fs::canonicalize(tmp).expect("canonicalize temp dir");
    let worktree = base.join("linked");
    let main = base.join("main");
    std::fs::create_dir_all(&main).expect("main dir");
    git_init(&main);
    std::fs::write(main.join(".gitignore"), gitignore).expect("write gitignore");
    git_in(&main, &["add", ".gitignore"]);
    git_in(&main, &["commit", "-q", "-m", "init"]);
    let worktree_arg = worktree.to_string_lossy().into_owned();
    git_in(
        &main,
        &["worktree", "add", "-q", &worktree_arg, "-b", "feature"],
    );
    (main, worktree)
}

/// In a linked worktree `.git` is a pointer file, so a file force-added after the
/// first lookup must still be noticed once the background refresh finishes.
#[test]
fn force_added_file_in_linked_worktree_is_not_suppressed() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let (_main, worktree) = main_with_linked_worktree(tmp.path(), "*.log\n");
    let first = worktree.join("first.log");
    std::fs::write(&first, "tracked later\n").expect("write first.log");

    assert!(
        path_is_gitignored(&worktree, &first, &registry),
        "an untracked file matching the ignore glob must be suppressed"
    );

    git_in(&worktree, &["add", "-f", "first.log"]);

    // The first call after the index change starts the refresh; later calls see it.
    let picked_up = wait_until(Duration::from_secs(5), || {
        !path_is_gitignored(&worktree, &first, &registry)
    });
    assert!(
        picked_up,
        "a force-added file in a linked worktree must stop being suppressed"
    );
}

/// The main repository's `info/exclude` applies in its linked worktrees.
#[test]
fn main_info_exclude_applies_in_linked_worktree() {
    let registry = test_registry(true, HashSet::new());
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let (main, worktree) = main_with_linked_worktree(tmp.path(), "");
    std::fs::create_dir_all(main.join(".git/info")).expect("info dir");
    std::fs::write(main.join(".git/info/exclude"), "local-only.txt\n").expect("write exclude");

    assert!(
        path_is_gitignored(&worktree, &worktree.join("local-only.txt"), &registry),
        "a main-repository info/exclude rule must suppress matching worktree paths"
    );
    assert!(
        !path_is_gitignored(&worktree, &worktree.join("other.txt"), &registry),
        "a non-matching path must not be suppressed"
    );
}

/// Feed a content edit of `file` through `process_event` with `root` watched.
fn events_for_edit(root: &Path, file: &Path) -> Vec<PendingEvent> {
    let registry = test_registry(true, HashSet::from([root.to_path_buf()]));
    events_for(
        &registry,
        EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        vec![file.to_path_buf()],
    )
}

/// Directory-name tokens are a hint: tracked files under `vendor/`, `dist/`,
/// etc. refresh; ignored `node_modules/` churn doesn't.
#[test]
fn tracked_file_under_high_churn_token_dir_triggers_git_refresh() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let repo = std::fs::canonicalize(tmp.path()).expect("canonicalize repo");

    git_init(&repo);
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
