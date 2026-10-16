//! Multi-repository watcher integration tests.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

#[path = "common/git.rs"]
mod git_helpers;
#[path = "common/wait.rs"]
mod wait;

use git_helpers::{git, git_init};
use gpy_agent::ipc::ClientDirectory;
use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
use gpy_agent::watcher::{DebouncedEvent, FileEvent, WatcherConfig};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;
use wait::wait_until;

/// Create `<parent>/<name>` as a git repo on `main` with one commit.
///
/// Uses [`git`]'s `-c` identity so the fixture ignores global git config
/// without writing `.git/config` entries that perturb `FSEvents` timing.
fn create_git_repo(parent: &Path, name: &str) -> PathBuf {
    let repo_path = parent.join(name);
    fs::create_dir_all(&repo_path).expect("create repo dir");
    git_init(&repo_path);

    // A global `core.fsmonitor` daemon competes for the same FSEvents and starves
    // gpy's stream of its first event under load.
    git(&repo_path, &["config", "core.fsmonitor", "false"]);

    fs::write(repo_path.join("README.md"), b"# Test repo").expect("write README");
    git(&repo_path, &["add", "README.md"]);
    git(&repo_path, &["commit", "-q", "-m", "Initial commit"]);

    repo_path
}

fn create_and_stage_file(repo_path: &Path, filename: &str, content: &str) {
    fs::write(repo_path.join(filename), content).expect("write file");
    git(repo_path, &["add", filename]);
}

/// Shared buffer of the `repo` roots of every debounced event delivered.
type RepoEvents = Arc<Mutex<Vec<PathBuf>>>;

/// Assert `working_dir/.git` is an external-gitdir pointer file, so a
/// mis-laid-out fixture fails loudly instead of passing vacuously.
fn assert_external_gitdir(working_dir: &Path) {
    let dot_git = working_dir.join(".git");
    assert!(
        dot_git.is_file(),
        "expected {} to be a gitdir pointer file (external gitdir layout)",
        dot_git.display()
    );
    let contents = fs::read_to_string(&dot_git).expect("read .git pointer");
    assert!(
        contents.trim_start().starts_with("gitdir:"),
        "expected a `gitdir:` pointer in {}, got: {contents:?}",
        dot_git.display()
    );
}

/// Poll `events` until it contains `expected` or the deadline passes.
fn wait_for_repo(events: &RepoEvents, expected: &Path, budget: Duration) -> bool {
    wait_until(budget, Duration::from_millis(100), || {
        events.lock().unwrap().iter().any(|repo| repo == expected)
    })
}

/// Build a watcher that records the `repo` of every debounced event.
fn recording_watcher() -> (MultiRepoWatcher, RepoEvents) {
    let events: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(move |event: DebouncedEvent| {
            events_clone.lock().unwrap().push(event.repo);
        }))
        .build()
        .expect("create watcher");
    (watcher, events)
}

#[test]
fn test_multi_repo_concurrent_registration() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "repo1");
    let repo2 = create_git_repo(temp_path, "repo2");
    let repo3 = create_git_repo(temp_path, "repo3");

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let callback = Box::new(move |event: DebouncedEvent| {
        events_clone.lock().unwrap().push(event.event);
    });

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("create watcher");

    watcher
        .register_client(1001, &repo1)
        .expect("register repo1");
    watcher
        .register_client(1002, &repo2)
        .expect("register repo2");
    watcher
        .register_client(1003, &repo3)
        .expect("register repo3");

    assert_eq!(watcher.watched_repo_count(), 3);
}

#[test]
fn test_multi_repo_event_isolation() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "isolated_repo1");
    let repo2 = create_git_repo(temp_path, "isolated_repo2");

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let callback = Box::new(move |event: DebouncedEvent| {
        events_clone.lock().unwrap().push(event.event);
    });

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(callback)
        .build()
        .expect("create watcher");

    watcher
        .register_client(2001, &repo1)
        .expect("register repo1");
    watcher
        .register_client(2002, &repo2)
        .expect("register repo2");

    // Arming produces legitimate events for both repos (Linux delivers `git init`
    // directory events late); drain them. Only repo1 writes may not leak into
    // repo2. Fixed pause: an OS watch has no observable "armed" signal.
    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    create_and_stage_file(&repo1, "file_in_repo1.txt", "content");
    assert!(
        wait_until(Duration::from_secs(10), Duration::from_millis(50), || {
            !events.lock().unwrap().is_empty()
        }),
        "staging a file in repo1 should produce an event"
    );

    let captured_events = events.lock().unwrap();

    // Canonicalize: macOS resolves /var -> /private/var.
    let canonical_repo1 = std::fs::canonicalize(&repo1).unwrap_or_else(|_| repo1.clone());
    let canonical_repo2 = std::fs::canonicalize(&repo2).unwrap_or_else(|_| repo2.clone());

    for event in captured_events.iter() {
        let FileEvent::Git { paths } = event else {
            continue;
        };
        for path in paths.as_slice().into_iter().flatten() {
            let canonical_event_path = std::fs::canonicalize(path)
                .or_else(|_| std::fs::canonicalize(path.parent().unwrap_or(path)))
                .unwrap_or_else(|_| path.clone());

            assert!(
                canonical_event_path.starts_with(&canonical_repo1),
                "Event path should be in repo1: {path:?} (canonical: {canonical_event_path:?}, repo1: {canonical_repo1:?})"
            );
            assert!(
                !canonical_event_path.starts_with(&canonical_repo2),
                "Event should not be in repo2: {path:?}"
            );
        }
    }
}

#[test]
fn test_multi_repo_same_relative_path() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "project_a");
    let repo2 = create_git_repo(temp_path, "project_b");

    let src1 = repo1.join("src");
    let src2 = repo2.join("src");
    fs::create_dir_all(&src1).expect("create src1");
    fs::create_dir_all(&src2).expect("create src2");
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .expect("create watcher");

    watcher.register_client(3001, &src1).expect("register src1");
    watcher.register_client(3002, &src2).expect("register src2");

    assert_eq!(watcher.watched_repo_count(), 2);
}

#[test]
fn test_multi_repo_unregistration() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "temp_repo1");
    let repo2 = create_git_repo(temp_path, "temp_repo2");
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .expect("create watcher");

    watcher
        .register_client(4001, &repo1)
        .expect("register repo1");
    watcher
        .register_client(4002, &repo2)
        .expect("register repo2");

    assert_eq!(watcher.watched_repo_count(), 2);

    watcher.unregister_client(4001);
    assert_eq!(watcher.watched_repo_count(), 1);
}

#[test]
fn test_client_directory_multi_repo_tracking() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "tracking_repo1");
    let repo2 = create_git_repo(temp_path, "tracking_repo2");
    let repo3 = create_git_repo(temp_path, "tracking_repo3");

    let client_dir = ClientDirectory::new();

    client_dir.register(5001, Some(repo1.clone()));
    client_dir.register(5002, Some(repo1)); // Two clients in repo1
    client_dir.register(5003, Some(repo2));
    client_dir.register(5004, Some(repo3));

    assert_eq!(client_dir.len(), 4);

    assert!(!client_dir.is_empty());
}

#[test]
fn test_client_directory_repo_boundary() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "boundary_repo1");
    let repo2 = create_git_repo(temp_path, "boundary_repo2");

    let client_dir = ClientDirectory::new();

    client_dir.register(6001, Some(repo1.clone()));
    client_dir.register(6002, Some(repo2.clone()));

    assert_eq!(client_dir.len(), 2);

    client_dir.notify_repaint(Some(&repo1));
    client_dir.notify_repaint(Some(&repo2));
}

#[test]
fn test_multi_repo_scalability() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .expect("create watcher");

    #[allow(clippy::used_underscore_binding)]
    let mut _repos = Vec::new();
    for i in 0..10 {
        let repo = create_git_repo(temp_path, &format!("scalability_repo{i}"));
        watcher
            .register_client(7000 + i, &repo)
            .expect("register repo");
        _repos.push(repo);
    }

    assert_eq!(watcher.watched_repo_count(), 10);
}

#[test]
fn test_multi_repo_rapid_registration() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo = create_git_repo(temp_path, "rapid_reg_repo");

    let watcher = Arc::new(
        MultiRepoWatcher::builder()
            .config(WatcherConfig::default())
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .expect("create watcher"),
    );

    let handles: Vec<_> = (0..20)
        .map(|i| {
            let watcher_clone = Arc::clone(&watcher);
            let repo_clone = repo.clone();
            thread::spawn(move || {
                watcher_clone
                    .register_client(8000 + i, &repo_clone)
                    .expect("register client");
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("thread should complete");
    }

    assert_eq!(watcher.watched_repo_count(), 1);
}

#[test]
fn test_multi_repo_nested_repos() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let outer_repo = create_git_repo(temp_path, "outer_repo");

    let nested_repo = outer_repo.join("nested");
    fs::create_dir_all(&nested_repo).expect("create nested dir");

    git_init(&nested_repo);

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .expect("create watcher");

    watcher
        .register_client(9001, &outer_repo)
        .expect("register outer");
    watcher
        .register_client(9002, &nested_repo)
        .expect("register nested");

    assert_eq!(watcher.watched_repo_count(), 2);
}

#[test]
fn test_multi_repo_symlink_handling() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("create temp dir");
        let temp_path = temp.path();

        let real_repo = create_git_repo(temp_path, "real_repo");
        let symlink_path = temp_path.join("symlink_to_repo");

        symlink(&real_repo, &symlink_path).expect("create symlink");

        let watcher = MultiRepoWatcher::builder()
            .config(WatcherConfig::default())
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .expect("create watcher");

        watcher
            .register_client(10001, &real_repo)
            .expect("register real");
        watcher
            .register_client(10002, &symlink_path)
            .expect("register symlink");

        // Symlink and real path resolve to one watch.
        assert_eq!(watcher.watched_repo_count(), 1);
    }
}

// One shared notify watcher serves all repos; on macOS adding repo2 restarts
// the FSEventStream armed for repo1, and a fresh stream can miss its first
// event under load. The file_serial group keeps bootstraps from overlapping.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn test_multi_repo_concurrent_events() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "concurrent_repo1");
    let repo2 = create_git_repo(temp_path, "concurrent_repo2");

    // `DebouncedEvent::repo` is canonical; compare like with like.
    let repo1_canonical = fs::canonicalize(&repo1).expect("canonicalize repo1");
    let repo2_canonical = fs::canonicalize(&repo2).expect("canonicalize repo2");

    let events: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let callback = Box::new(move |event: DebouncedEvent| {
        events_clone.lock().unwrap().push(event.repo);
    });

    let watcher = Arc::new(
        MultiRepoWatcher::builder()
            .config(WatcherConfig::default())
            .callback(callback)
            .build()
            .expect("create watcher"),
    );

    watcher
        .register_client(11001, &repo1)
        .expect("register repo1");
    watcher
        .register_client(11002, &repo2)
        .expect("register repo2");

    let handle1 = thread::spawn(move || {
        create_and_stage_file(&repo1, "file1.txt", "data1");
    });

    let handle2 = thread::spawn(move || {
        create_and_stage_file(&repo2, "file2.txt", "data2");
    });

    handle1.join().expect("thread1 complete");
    handle2.join().expect("thread2 complete");

    let saw_both = wait_until(Duration::from_secs(10), Duration::from_millis(100), || {
        let seen = events.lock().unwrap();
        seen.contains(&repo1_canonical) && seen.contains(&repo2_canonical)
    });

    assert!(
        saw_both,
        "Should receive events from both repos, got: {:?}",
        events.lock().unwrap()
    );
}

// Submodule / linked-worktree gitdirs live outside the working tree
// (`.git/modules/<name>`, `.git/worktrees/<name>`), so `classify_event` must
// attribute them itself or the prompt stays stale until reconcile.

/// A submodule branch switch must surface a git event attributed to the
/// submodule's *working root* (not the superproject).
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn submodule_branch_change_attributes_to_submodule_working_root() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let sub_source = create_git_repo(temp_path, "sub_source");
    let super_repo = create_git_repo(temp_path, "super_repo");

    // Local file protocol: no network.
    let sub_source_url = sub_source.to_str().expect("utf-8 path");
    git(
        &super_repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            sub_source_url,
            "sub",
        ],
    );
    let sub_wd = super_repo.join("sub");
    git(&sub_wd, &["config", "core.fsmonitor", "false"]);
    assert_external_gitdir(&sub_wd);

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &sub_wd)
        .expect("register submodule client");

    let canonical_sub = fs::canonicalize(&sub_wd).expect("canonicalize submodule root");

    let_watch_arm();

    git(&sub_wd, &["checkout", "-b", "other"]);

    assert!(
        wait_for_repo(&events, &canonical_sub, Duration::from_secs(10)),
        "submodule branch switch must trigger a refresh attributed to the submodule working root"
    );
}

/// A linked-worktree branch switch must surface a git event attributed to the
/// worktree's *working root* (not the main working tree).
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn linked_worktree_branch_change_attributes_to_worktree_working_root() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let super_repo = create_git_repo(temp_path, "wt_super");
    let wt_wd = temp_path.join("wt_linked");

    git(
        &super_repo,
        &[
            "worktree",
            "add",
            "-b",
            "wtbranch",
            wt_wd.to_str().expect("utf-8 path"),
        ],
    );
    git(&wt_wd, &["config", "core.fsmonitor", "false"]);
    assert_external_gitdir(&wt_wd);

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &wt_wd)
        .expect("register worktree client");

    let canonical_wt = fs::canonicalize(&wt_wd).expect("canonicalize worktree root");

    let_watch_arm();

    git(&wt_wd, &["checkout", "-b", "other2"]);

    assert!(
        wait_for_repo(&events, &canonical_wt, Duration::from_secs(10)),
        "worktree branch switch must trigger a refresh attributed to the worktree working root"
    );
}

/// An ordinary repo's `.git` change is attributed to its own root.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn ordinary_repo_head_change_still_triggers() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo = create_git_repo(temp_path, "ordinary_repo");
    assert!(repo.join(".git").is_dir(), "ordinary repo has a .git dir");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &repo)
        .expect("register ordinary client");

    let canonical_repo = fs::canonicalize(&repo).expect("canonicalize repo root");

    let_watch_arm();

    git(&repo, &["checkout", "-b", "feature"]);

    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(10)),
        "an ordinary repo's HEAD change must still trigger a refresh for its own root"
    );
}

/// Run `git rev-parse <rev>` in `dir` and return the resolved object id.
fn rev_parse(dir: &Path, rev: &str) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(dir)
        .output()
        .expect("git rev-parse");
    assert!(
        output.status.success(),
        "git rev-parse {rev} should succeed in {}",
        dir.display()
    );
    String::from_utf8(output.stdout)
        .expect("utf-8 object id")
        .trim()
        .to_owned()
}

/// Commit the staged index in `dir`.
fn git_commit(dir: &Path, message: &str) {
    git(dir, &["commit", "-q", "-m", message]);
}

/// A main checkout plus linked worktrees sharing its common git directory.
struct WorktreeFixture {
    /// The main checkout's working root.
    main: PathBuf,
    /// Canonical common git directory (`<main>/.git`) shared by every worktree.
    common: PathBuf,
    /// Canonical working roots of the linked worktrees, in creation order.
    linked: Vec<PathBuf>,
    /// Object id distinct from the seeded remote-tracking ref.
    advanced_sha: String,
}

/// Create `<parent>/<name>` with `branches.len()` linked worktrees.
///
/// Seeds `refs/remotes/origin/main` before any client registers so tests
/// measure fan-out, not poll-backend watch arming.
fn create_worktree_fixture(parent: &Path, name: &str, branches: &[&str]) -> WorktreeFixture {
    let main = create_git_repo(parent, name);

    let seed_sha = rev_parse(&main, "HEAD");
    git(
        &main,
        &["update-ref", "refs/remotes/origin/main", &seed_sha],
    );

    fs::write(main.join("second.txt"), b"second").expect("write second.txt");
    git(&main, &["add", "second.txt"]);
    git_commit(&main, "Second commit");
    let advanced_sha = rev_parse(&main, "HEAD");

    let mut linked = Vec::new();
    for branch in branches {
        let working_dir = parent.join(format!("{name}_{branch}"));
        git(
            &main,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                working_dir.to_str().expect("utf-8 path"),
            ],
        );
        git(&working_dir, &["config", "core.fsmonitor", "false"]);
        assert_external_gitdir(&working_dir);
        linked.push(fs::canonicalize(&working_dir).expect("canonicalize worktree root"));
    }

    WorktreeFixture {
        common: fs::canonicalize(main.join(".git")).expect("canonicalize common dir"),
        main,
        linked,
        advanced_sha,
    }
}

/// Number of debounced events recorded for `root`.
fn wake_count(events: &RepoEvents, root: &Path) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|repo| repo.as_path() == root)
        .count()
}

/// A remote-tracking ref rewritten via the main checkout refreshes the linked
/// worktree, whose ahead/behind reads that ref.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn common_remote_ref_change_outside_worktree_refreshes_linked_worktree() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_remote", &["wtbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &linked)
        .expect("register worktree client");

    let_watch_arm();

    fs::write(
        fixture.common.join("refs/remotes/origin/main"),
        format!("{}\n", fixture.advanced_sha),
    )
    .expect("advance remote-tracking ref");

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "a common-dir remote-ref change must refresh the linked worktree"
    );
}

/// An upstream config change in the main checkout refreshes the linked worktree.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn common_config_upstream_change_outside_worktree_refreshes_linked_worktree() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_config", &["cfgbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &linked)
        .expect("register worktree client");

    let_watch_arm();

    // From the main checkout: rewrites the shared `<common>/config`.
    git(
        &fixture.main,
        &["config", "branch.cfgbranch.remote", "origin"],
    );

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "a common-dir upstream config change must refresh the linked worktree"
    );
}

/// A main-checkout `index` write is per-checkout state and must wake zero
/// linked worktrees. Counted, not rendered: a spurious refresh looks identical
/// but costs a capture round.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn main_checkout_index_write_wakes_zero_linked_worktrees() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_index", &["idxbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();

    let canonical_main = fs::canonicalize(&fixture.main).expect("canonicalize main checkout");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &linked)
        .expect("register worktree client");

    drain_fixture_events(&events);

    create_and_stage_file(&fixture.main, "staged.txt", "staged");

    // Well past debounce, poll interval, and native->poll fallback.
    assert!(
        !wait_for_repo(&events, &linked, Duration::from_secs(8)),
        "a main-checkout index write must not wake the linked worktree"
    );
    assert_eq!(
        wake_count(&events, &linked),
        0,
        "a main-checkout index write must cost zero capture rounds in the linked worktree"
    );
    // No shell is registered at `<main>`, so any wake there is wasted work.
    assert_eq!(
        wake_count(&events, &canonical_main),
        0,
        "a main-checkout index write must not wake the UNREGISTERED main checkout either"
    );

    // Positive control: shared changes still arrive, so the zero above is real.
    fs::write(
        fixture.common.join("refs/remotes/origin/main"),
        format!("{}\n", fixture.advanced_sha),
    )
    .expect("advance remote-tracking ref");
    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "control: a shared remote-ref change must still refresh the linked worktree"
    );
}

/// A per-worktree `HEAD` under the common dir reaches its own worktree only.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn per_worktree_head_write_wakes_zero_sibling_worktrees() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_sib", &["sibone", "sibtwo"]);
    let (first, second) = (
        fixture
            .linked
            .first()
            .expect("fixture has a first linked worktree")
            .clone(),
        fixture
            .linked
            .get(1)
            .expect("fixture has a second linked worktree")
            .clone(),
    );

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &first)
        .expect("register first worktree");
    watcher
        .register_client(std::process::id() + 1, &second)
        .expect("register second worktree");

    drain_fixture_events(&events);

    // Detach in place: rewriting the same ref is byte-identical and invisible
    // to the poll backend.
    let admin_dir = fixture.common.join("worktrees").join("wt_sib_sibone");
    assert!(
        admin_dir.is_dir(),
        "expected per-worktree admin dir at {}",
        admin_dir.display()
    );
    let detached_head = format!("{}\n", rev_parse(&first, "HEAD"));
    fs::write(admin_dir.join("HEAD"), detached_head).expect("rewrite worktree HEAD");

    assert!(
        wait_for_repo(&events, &first, Duration::from_secs(20)),
        "a per-worktree HEAD write must refresh its own worktree"
    );
    assert_eq!(
        wake_count(&events, &second),
        0,
        "a per-worktree HEAD write must not fan out to a sibling worktree"
    );
}

/// The shared common-dir watch is refcounted: dropping one worktree keeps the
/// other receiving shared events.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn common_dir_watch_survives_until_last_dependent_worktree_unregisters() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_rc", &["rcone", "rctwo"]);
    let (first, second) = (
        fixture
            .linked
            .first()
            .expect("fixture has a first linked worktree")
            .clone(),
        fixture
            .linked
            .get(1)
            .expect("fixture has a second linked worktree")
            .clone(),
    );

    let (watcher, events) = recording_watcher();
    let first_pid = std::process::id();
    let second_pid = first_pid + 1;
    watcher
        .register_client(first_pid, &first)
        .expect("register first worktree");
    watcher
        .register_client(second_pid, &second)
        .expect("register second worktree");

    watcher.unregister_client(first_pid);

    let_watch_arm();

    fs::write(
        fixture.common.join("refs/remotes/origin/main"),
        format!("{}\n", fixture.advanced_sha),
    )
    .expect("advance remote-tracking ref");

    assert!(
        wait_for_repo(&events, &second, Duration::from_secs(20)),
        "the remaining dependent worktree must still receive shared metadata events"
    );
    assert_eq!(
        wake_count(&events, &first),
        0,
        "the unregistered worktree must receive nothing"
    );
}

/// A registered main checkout's recursive watch covers `<common>`, so the
/// worktree arms nothing extra; unregistering main must not drop its coverage.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn linked_worktree_keeps_common_coverage_after_main_checkout_unregisters() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_cover", &["covbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();

    let (watcher, events) = recording_watcher();
    let main_pid = std::process::id();
    let linked_pid = main_pid + 1;
    watcher
        .register_client(main_pid, &fixture.main)
        .expect("register main checkout");
    watcher
        .register_client(linked_pid, &linked)
        .expect("register worktree client");

    watcher.unregister_client(main_pid);

    let_watch_arm();

    fs::write(
        fixture.common.join("refs/remotes/origin/main"),
        format!("{}\n", fixture.advanced_sha),
    )
    .expect("advance remote-tracking ref");

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "the linked worktree must keep receiving shared metadata after the covering watch is dropped"
    );
}

/// Fixed pause for a new watch to arm; OS watches expose no armed signal.
fn let_watch_arm() {
    thread::sleep(Duration::from_millis(150));
}

/// Let a new watch arm, then discard events arming itself produced.
fn let_watch_arm_then_drain(events: &RepoEvents) {
    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();
}

/// Fixed pause for watches to settle after arm/release.
fn settle() {
    thread::sleep(Duration::from_millis(500));
}

/// Settle, then discard events. `FSEvents` may deliver a pre-arm fixture write
/// (usually `<common>/config`) after arming, which would break zero-wake checks.
fn drain_fixture_events(events: &RepoEvents) {
    settle();
    events.lock().unwrap().clear();
}

/// An outer repository with a nested repository inside it, both canonical.
fn nested_repos(parent: &Path) -> (PathBuf, PathBuf) {
    let outer = fs::canonicalize(create_git_repo(parent, "outer")).expect("canonical outer");
    let inner = fs::canonicalize(create_git_repo(&outer, "inner")).expect("canonical inner");
    (outer, inner)
}

/// Unregistering the outer repo keeps the nested repo's OS watches.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn nested_repo_keeps_events_after_outer_unregisters() {
    let temp = tempdir().expect("create temp dir");
    let (outer, inner) = nested_repos(temp.path());

    let (watcher, events) = recording_watcher();
    let outer_pid = std::process::id();
    let inner_pid = outer_pid + 1;
    watcher
        .register_client(outer_pid, &outer)
        .expect("register outer");
    watcher
        .register_client(inner_pid, &inner)
        .expect("register inner");
    settle();
    watcher.unregister_client(outer_pid);
    settle();
    events.lock().unwrap().clear();

    fs::write(inner.join("README.md"), b"changed content here\n").expect("edit inner file");

    assert!(
        wait_for_repo(&events, &inner, Duration::from_secs(20)),
        "the nested repository must keep receiving events after the outer one unregisters, \
         got: {:?}",
        events.lock().unwrap()
    );
}

/// Unregistering the nested repo keeps the outer repo's shared descriptors (inotify).
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn nested_repo_outer_keeps_inner_subtree_after_inner_unregisters() {
    let temp = tempdir().expect("create temp dir");
    let (outer, inner) = nested_repos(temp.path());

    let (watcher, events) = recording_watcher();
    let outer_pid = std::process::id();
    let inner_pid = outer_pid + 1;
    watcher
        .register_client(outer_pid, &outer)
        .expect("register outer");
    watcher
        .register_client(inner_pid, &inner)
        .expect("register inner");
    settle();
    watcher.unregister_client(inner_pid);
    settle();
    events.lock().unwrap().clear();

    fs::write(inner.join("newfile.txt"), b"x\n").expect("create file in inner subtree");

    assert!(
        wait_for_repo(&events, &outer, Duration::from_secs(20)),
        "the outer repository must keep its watch on the nested subtree after the nested \
         repository unregisters, got: {:?}",
        events.lock().unwrap()
    );
}

/// Worktree registered before its main checkout: unregistering main must keep
/// the overlapping admin-dir watch (inotify, poll).
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn linked_worktree_registered_first_keeps_admin_events_after_main_unregisters() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_first", &["firstbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();
    git(&fixture.main, &["branch", "other"]);

    let (watcher, events) = recording_watcher();
    let linked_pid = std::process::id();
    let main_pid = linked_pid + 1;
    watcher
        .register_client(linked_pid, &linked)
        .expect("register worktree client");
    watcher
        .register_client(main_pid, &fixture.main)
        .expect("register main checkout");
    settle();
    watcher.unregister_client(main_pid);
    settle();
    events.lock().unwrap().clear();

    // Writes `<main>/.git/worktrees/<name>/HEAD`.
    git(&linked, &["symbolic-ref", "HEAD", "refs/heads/other"]);

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "the linked worktree must keep its admin-dir watch after the main checkout \
         unregisters, got: {:?}",
        events.lock().unwrap()
    );
}

/// `watch_worktree = false`: worktree arms `<common>` shallow, main arms it
/// recursively. Unregistering main must keep the shallow watch (`FSEvents`
/// removes every stream entry equal to the path).
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn gitdir_only_main_unregister_keeps_worktree_common_config_fanout() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_gitdir", &["gdbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();

    let (watcher, events) = recording_watcher();
    watcher.set_watch_worktree(false);
    let linked_pid = std::process::id();
    let main_pid = linked_pid + 1;
    watcher
        .register_client(linked_pid, &linked)
        .expect("register worktree client");
    settle();

    git(
        &fixture.main,
        &["config", "branch.gdbranch.remote", "origin"],
    );
    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "control: a common config write must reach the worktree before the main checkout \
         registers"
    );

    watcher
        .register_client(main_pid, &fixture.main)
        .expect("register main checkout");
    settle();
    watcher.unregister_client(main_pid);
    // Releasing main's watch rebuilds the FSEvents stream; nothing to poll for.
    thread::sleep(Duration::from_millis(1500));
    events.lock().unwrap().clear();

    // Writes `<common>/config`.
    git(
        &fixture.main,
        &["config", "branch.gdbranch.merge", "refs/heads/gdbranch"],
    );

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "a common config write must still reach the worktree after the main checkout \
         registers and unregisters, got: {:?}",
        events.lock().unwrap()
    );
}

/// A commit inside the worktree writes shared `<common>/refs/heads/<branch>`,
/// attributed to the unregistered main checkout: must not wake it.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn worktree_commit_wakes_only_the_linked_worktree() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_commit", &["cmtbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();
    let canonical_main = fs::canonicalize(&fixture.main).expect("canonicalize main checkout");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &linked)
        .expect("register worktree client");

    let_watch_arm();

    fs::write(linked.join("wt.txt"), b"worktree file").expect("write worktree file");
    git(&linked, &["add", "wt.txt"]);
    git_commit(&linked, "Worktree commit");

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "a commit in the linked worktree must refresh the linked worktree"
    );
    thread::sleep(Duration::from_secs(3));
    assert_eq!(
        wake_count(&events, &canonical_main),
        0,
        "a commit in the linked worktree must not wake the unregistered main checkout"
    );
}

/// When the main checkout is registered, its own gitdir writes still wake it.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn registered_main_checkout_still_wakes_on_its_own_gitdir_write() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_both", &["bothbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();
    let canonical_main = fs::canonicalize(&fixture.main).expect("canonicalize main checkout");

    let (watcher, events) = recording_watcher();
    let main_pid = std::process::id();
    watcher
        .register_client(main_pid, &fixture.main)
        .expect("register main checkout");
    watcher
        .register_client(main_pid + 1, &linked)
        .expect("register worktree client");

    drain_fixture_events(&events);

    // Direct write: `git add` would also touch the worktree and wake main
    // regardless of `.git` handling.
    fs::write(fixture.common.join("index"), b"gpy test index write").expect("rewrite index");

    assert!(
        wait_for_repo(&events, &canonical_main, Duration::from_secs(20)),
        "a registered main checkout must still wake on its own index write"
    );
    assert_eq!(
        wake_count(&events, &linked),
        0,
        "index is per-checkout state and must never fan out to the linked worktree"
    );
}

/// Linked worktree of a bare repo: the common dir has no `.git` component, so
/// events reach `process_event`'s third arm, not `classify_event`.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn bare_superproject_common_dir_fans_out_without_waking_the_bare_repo() {
    let temp = tempdir().expect("create temp dir");
    let root = temp.path();

    let bare = root.join("bare_super.git");
    fs::create_dir_all(&bare).expect("create bare dir");
    git(&bare, &["-c", "init.defaultBranch=main", "init", "--bare"]);
    git(&bare, &["config", "core.fsmonitor", "false"]);

    let seed = root.join("seed");
    git(
        root,
        &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()],
    );
    git(&seed, &["config", "core.fsmonitor", "false"]);
    fs::write(seed.join("a.txt"), b"a").expect("write a.txt");
    git(&seed, &["add", "a.txt"]);
    git_commit(&seed, "Seed");
    git(&seed, &["push", "origin", "HEAD:refs/heads/main"]);
    let seed_sha = rev_parse(&seed, "HEAD");

    let canonical_bare = fs::canonicalize(&bare).expect("canonicalize bare repo");
    git(
        &bare,
        &["update-ref", "refs/remotes/origin/main", &seed_sha],
    );

    let wt = root.join("bare_wt");
    git(&bare, &["worktree", "add", wt.to_str().unwrap(), "main"]);
    git(&wt, &["config", "core.fsmonitor", "false"]);
    let canonical_wt = fs::canonicalize(&wt).expect("canonicalize worktree");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &canonical_wt)
        .expect("register worktree client");
    let_watch_arm();

    // Per-checkout state in the unregistered bare repo: must wake nobody.
    fs::write(canonical_bare.join("ORIG_HEAD"), format!("{seed_sha}\n")).expect("write ORIG_HEAD");
    // Absence: wait past debounce and a poll cycle.
    thread::sleep(Duration::from_secs(3));
    assert_eq!(
        wake_count(&events, &canonical_bare),
        0,
        "per-checkout state in a bare common dir must not wake the unregistered bare repo"
    );

    // Control: shared state still reaches the worktree.
    fs::write(
        canonical_bare.join("refs/remotes/origin/main"),
        format!("{seed_sha}\n"),
    )
    .expect("advance remote-tracking ref");
    assert!(
        wait_for_repo(&events, &canonical_wt, Duration::from_secs(20)),
        "control: shared metadata in a bare common dir must refresh the linked worktree"
    );
}

/// A superproject with one submodule checked out at `sub`.
struct SubmoduleFixture {
    /// Canonical superproject working root.
    super_root: PathBuf,
    /// Canonical submodule working root (`<super>/sub`).
    sub_root: PathBuf,
    /// Uncanonicalized submodule working directory, for running git in.
    sub_dir: PathBuf,
    /// Submodule commit with a tree identical to the tip: switching changes only
    /// the gitlink.
    identical_tree_sha: String,
}

/// Build `<parent>/<name>` with a submodule at `sub` plus an identical-tree
/// commit, so a parent refresh provably can't come from a worktree event.
fn create_submodule_fixture(parent: &Path, name: &str) -> SubmoduleFixture {
    let sub_source = create_git_repo(parent, &format!("{name}_src"));
    let identical_tree_sha = rev_parse(&sub_source, "HEAD");

    // Add a file, then remove it again: the third commit's tree is the first's.
    fs::write(sub_source.join("scratch.txt"), b"scratch").expect("write scratch.txt");
    git(&sub_source, &["add", "scratch.txt"]);
    git_commit(&sub_source, "Add scratch");
    fs::remove_file(sub_source.join("scratch.txt")).expect("remove scratch.txt");
    git(&sub_source, &["add", "-A"]);
    git_commit(&sub_source, "Remove scratch");
    assert_eq!(
        rev_parse(&sub_source, "HEAD^{tree}"),
        rev_parse(&sub_source, &format!("{identical_tree_sha}^{{tree}}")),
        "fixture must offer two submodule commits with the same tree object"
    );

    let super_repo = create_git_repo(parent, name);
    git(
        &super_repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            sub_source.to_str().expect("utf-8 path"),
            "sub",
        ],
    );
    let sub_dir = super_repo.join("sub");
    git(&sub_dir, &["config", "core.fsmonitor", "false"]);
    assert_external_gitdir(&sub_dir);
    git_commit(&super_repo, "Add submodule");

    SubmoduleFixture {
        super_root: fs::canonicalize(&super_repo).expect("canonicalize superproject root"),
        sub_root: fs::canonicalize(&sub_dir).expect("canonicalize submodule root"),
        sub_dir,
        identical_tree_sha,
    }
}

/// A ref-only submodule checkout refreshes the superproject even with no shell
/// registered in the submodule; no existing attribution route covers it.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn ref_only_submodule_checkout_refreshes_unregistered_parent() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_submodule_fixture(temp.path(), "sm_parent_only");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &fixture.super_root)
        .expect("register superproject client");

    let_watch_arm_then_drain(&events);

    // Rewrites `<super>/.git/modules/sub/HEAD` only; no working-tree file.
    git(&fixture.sub_dir, &["checkout", &fixture.identical_tree_sha]);

    assert!(
        wait_for_repo(&events, &fixture.super_root, Duration::from_secs(20)),
        "a ref-only submodule checkout must refresh the superproject whose gitlink moved"
    );
}

/// With both registered, a submodule branch switch refreshes both.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn submodule_branch_switch_refreshes_both_registered_parent_and_child() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_submodule_fixture(temp.path(), "sm_both");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &fixture.super_root)
        .expect("register superproject client");
    watcher
        .register_client(std::process::id() + 1, &fixture.sub_root)
        .expect("register submodule client");

    let_watch_arm_then_drain(&events);

    git(&fixture.sub_dir, &["checkout", "-b", "other"]);

    assert!(
        wait_for_repo(&events, &fixture.sub_root, Duration::from_secs(20)),
        "a submodule branch switch must still refresh the submodule itself"
    );
    assert!(
        wait_for_repo(&events, &fixture.super_root, Duration::from_secs(20)),
        "a submodule branch switch must also refresh the registered superproject"
    );
}

/// With only the submodule registered, the unsubscribed superproject must
/// wake zero times.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn submodule_branch_switch_wakes_zero_unregistered_parents() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_submodule_fixture(temp.path(), "sm_child_only");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &fixture.sub_root)
        .expect("register submodule client");

    let_watch_arm_then_drain(&events);

    git(&fixture.sub_dir, &["checkout", "-b", "other"]);

    // Positive control first, so the zero below isn't a dead watcher.
    assert!(
        wait_for_repo(&events, &fixture.sub_root, Duration::from_secs(20)),
        "control: a submodule branch switch must refresh the submodule itself"
    );
    assert_eq!(
        wake_count(&events, &fixture.super_root),
        0,
        "an unregistered superproject must not buy a capture round"
    );
}

/// An ordinary nested repo is not a submodule: its `.git`-only changes must
/// wake the enclosing registered repo zero times.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn ordinary_nested_repo_metadata_change_wakes_zero_parents() {
    let temp = tempdir().expect("create temp dir");
    let outer = create_git_repo(temp.path(), "nest_outer");
    let inner = create_git_repo(&outer, "nested");
    assert!(
        inner.join(".git").is_dir(),
        "an ordinary nested repo keeps an in-tree .git directory"
    );
    let canonical_outer = fs::canonicalize(&outer).expect("canonicalize outer root");
    let canonical_inner = fs::canonicalize(&inner).expect("canonicalize inner root");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &outer)
        .expect("register outer client");
    watcher
        .register_client(std::process::id() + 1, &inner)
        .expect("register inner client");

    let_watch_arm_then_drain(&events);

    git(&inner, &["checkout", "-b", "innerbranch"]);

    assert!(
        wait_for_repo(&events, &canonical_inner, Duration::from_secs(20)),
        "control: the nested repository must refresh itself"
    );
    assert_eq!(
        wake_count(&events, &canonical_outer),
        0,
        "an ordinary nested repository must not refresh the repository containing it"
    );
}

/// A worktree admin dir sits under `.git` like a submodule's but has no
/// gitlink, so its `HEAD` write must not wake the main checkout. Guards keying
/// parent fan-out on `/.git/modules/` rather than any external gitdir.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn linked_worktree_admin_write_wakes_zero_parent_repos() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_worktree_fixture(temp.path(), "wt_notparent", &["npbranch"]);
    let linked = fixture
        .linked
        .first()
        .expect("fixture has a linked worktree")
        .clone();
    let canonical_main = fs::canonicalize(&fixture.main).expect("canonicalize main checkout");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &fixture.main)
        .expect("register main checkout client");
    watcher
        .register_client(std::process::id() + 1, &linked)
        .expect("register worktree client");

    let_watch_arm_then_drain(&events);

    let admin_dir = fixture
        .common
        .join("worktrees")
        .join("wt_notparent_npbranch");
    assert!(
        admin_dir.is_dir(),
        "expected per-worktree admin dir at {}",
        admin_dir.display()
    );
    // Detach: a byte-identical ref rewrite is invisible to the poll backend.
    let detached_head = format!("{}\n", rev_parse(&linked, "HEAD"));
    fs::write(admin_dir.join("HEAD"), detached_head).expect("rewrite worktree HEAD");

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "control: a per-worktree HEAD write must refresh its own worktree"
    );
    assert_eq!(
        wake_count(&events, &canonical_main),
        0,
        "a linked worktree is not a submodule: its admin dir must not refresh the main checkout"
    );
}

/// Write a `GIT_SEQUENCE_EDITOR` that prepends two `break` lines, so an
/// interactive rebase stops twice. Advancing between breaks moves only
/// `rebase-merge/msgnum`; the first stop still writes `HEAD`.
#[cfg(unix)]
fn double_break_sequence_editor(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let script = dir.join("insert_two_breaks.sh");
    fs::write(
        &script,
        "#!/bin/sh\n{ echo break; echo break; cat \"$1\"; } > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"\n",
    )
    .expect("write sequence-editor script");
    let mut perms = fs::metadata(&script).expect("stat script").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).expect("chmod script executable");
    script
}

/// An interactive-rebase step advance (msgnum only, no `HEAD`) is significant.
/// Uses real `git rebase` so an incidental `HEAD` write can't satisfy it.
#[test]
#[cfg(unix)]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn rebase_msgnum_progress_is_metadata_only_and_refreshes_repository() {
    let temp = tempdir().expect("create temp dir");
    let repo = create_git_repo(temp.path(), "rebase_progress");
    let canonical_repo = fs::canonicalize(&repo).expect("canonicalize repo root");
    let editor = double_break_sequence_editor(temp.path());

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &repo)
        .expect("register client");

    let_watch_arm_then_drain(&events);

    let rebase_start_status = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "rebase", "-i", "--root"])
        .env("GIT_SEQUENCE_EDITOR", &editor)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .current_dir(&repo)
        .status()
        .expect("start interactive rebase, paused at the first break");
    assert!(
        rebase_start_status.success(),
        "rebase -i --root should stop cleanly at the first break"
    );

    // Setup wake: the initial detached checkout writes HEAD.
    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "setup: starting the rebase must refresh the repository"
    );
    events.lock().unwrap().clear();

    let head_before = fs::read_to_string(repo.join(".git").join("HEAD")).expect("read HEAD");

    // Under test: advances `rebase-merge/msgnum` only.
    git(&repo, &["rebase", "--continue"]);

    let head_after = fs::read_to_string(repo.join(".git").join("HEAD")).expect("read HEAD");
    assert_eq!(
        head_before, head_after,
        "self-check: the continue step must be metadata-only, or this test proves nothing"
    );

    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "a metadata-only rebase-merge/msgnum advance must refresh the repository"
    );
}

/// Creating `rebase-apply/applying` and removing `rebase-apply` each refresh
/// the repo on their own.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn rebase_operation_state_transitions_refresh_repository() {
    let temp = tempdir().expect("create temp dir");
    let repo = create_git_repo(temp.path(), "rebase_state");
    let canonical_repo = fs::canonicalize(&repo).expect("canonicalize repo root");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &repo)
        .expect("register client");

    let_watch_arm_then_drain(&events);

    let rebase_apply = repo.join(".git").join("rebase-apply");
    fs::create_dir(&rebase_apply).expect("create rebase-apply dir");
    fs::write(rebase_apply.join("applying"), "").expect("write applying marker");

    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "creating rebase-apply/applying must refresh the repository (Applying state)"
    );
    events.lock().unwrap().clear();

    fs::remove_dir_all(&rebase_apply).expect("remove rebase-apply dir");

    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "removing rebase-apply must refresh the repository (state clears)"
    );
}

/// High-churn `rebase-merge/patch` writes must not refresh; only
/// `msgnum`/`end` are read.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn rebase_high_churn_file_wakes_zero_times() {
    let temp = tempdir().expect("create temp dir");
    let repo = create_git_repo(temp.path(), "rebase_churn");
    let canonical_repo = fs::canonicalize(&repo).expect("canonicalize repo root");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &repo)
        .expect("register client");

    let_watch_arm_then_drain(&events);

    let rebase_merge = repo.join(".git").join("rebase-merge");
    fs::create_dir(&rebase_merge).expect("create rebase-merge dir");
    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "setup: creating rebase-merge must refresh the repository"
    );
    events.lock().unwrap().clear();

    fs::write(rebase_merge.join("patch"), "diff --git a/f b/f\n").expect("write patch");

    assert!(
        !wait_for_repo(&events, &canonical_repo, Duration::from_secs(8)),
        "a rebase-merge/patch write is high-churn and must not refresh the repository"
    );
    assert_eq!(
        wake_count(&events, &canonical_repo),
        0,
        "a rebase-merge/patch write must cost zero capture rounds"
    );

    // Positive control: `msgnum` does refresh.
    fs::write(rebase_merge.join("msgnum"), "1\n").expect("write msgnum");
    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "control: a rebase-merge/msgnum write must still refresh the repository"
    );
}

/// A rebase in a registered submodule refreshes the submodule and the
/// registered superproject (whose status renders the gitlink).
#[test]
#[cfg(unix)]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn submodule_rebase_msgnum_progress_refreshes_both_registered_parent_and_child() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_submodule_fixture(temp.path(), "sm_rebase");
    let editor = double_break_sequence_editor(temp.path());
    let submodule_gitdir = fixture.super_root.join(".git").join("modules").join("sub");
    // A registered unrelated repo must never wake.
    let unrelated_root = fs::canonicalize(create_git_repo(temp.path(), "sm_rebase_unrelated"))
        .expect("canonicalize unrelated root");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &fixture.super_root)
        .expect("register superproject client");
    watcher
        .register_client(std::process::id() + 1, &fixture.sub_root)
        .expect("register submodule client");
    watcher
        .register_client(std::process::id() + 2, &unrelated_root)
        .expect("register unrelated client");

    let_watch_arm_then_drain(&events);

    let rebase_start_status = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "rebase", "-i", "--root"])
        .env("GIT_SEQUENCE_EDITOR", &editor)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .current_dir(&fixture.sub_dir)
        .status()
        .expect("start interactive rebase inside the submodule, paused at the first break");
    assert!(
        rebase_start_status.success(),
        "rebase -i --root should stop cleanly at the first break"
    );

    assert!(
        wait_for_repo(&events, &fixture.sub_root, Duration::from_secs(20)),
        "setup: starting the rebase must refresh the submodule"
    );
    assert!(
        wait_for_repo(&events, &fixture.super_root, Duration::from_secs(20)),
        "setup: starting the rebase must also refresh the registered superproject"
    );
    events.lock().unwrap().clear();

    let head_before = fs::read_to_string(submodule_gitdir.join("HEAD")).expect("read HEAD");

    git(&fixture.sub_dir, &["rebase", "--continue"]);

    let head_after = fs::read_to_string(submodule_gitdir.join("HEAD")).expect("read HEAD");
    assert_eq!(
        head_before, head_after,
        "self-check: the continue step must be metadata-only, or this test proves nothing"
    );

    assert!(
        wait_for_repo(&events, &fixture.sub_root, Duration::from_secs(20)),
        "a metadata-only submodule rebase advance must refresh the submodule itself"
    );
    assert!(
        wait_for_repo(&events, &fixture.super_root, Duration::from_secs(20)),
        "a metadata-only submodule rebase advance must also refresh the registered superproject, \
whose gitlink is about to move"
    );
    assert_eq!(
        wake_count(&events, &unrelated_root),
        0,
        "a submodule rebase must wake the submodule and its registered superproject and nothing else"
    );
}

/// An unreadable directory, made readable again on drop so `TempDir` cleanup
/// works. inotify needs read permission to watch it.
#[cfg(target_os = "linux")]
struct UnreadableDir(PathBuf);

#[cfg(target_os = "linux")]
impl UnreadableDir {
    fn create(dir: PathBuf) -> Self {
        use std::os::unix::fs::PermissionsExt;

        fs::create_dir_all(dir.join("sub")).expect("create locked dir");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).expect("lock dir");
        Self(dir)
    }
}

#[cfg(target_os = "linux")]
impl Drop for UnreadableDir {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;

        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

/// inotify stops a recursive watch at the first unreadable directory without
/// undoing earlier ones; that partial tree must not keep delivering events.
#[cfg(target_os = "linux")]
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn failed_recursive_watch_leaves_no_live_watches_after_unregister() {
    if nix::unistd::geteuid().is_root() {
        eprintln!("skipped: root bypasses the EACCES that fails the recursive watch");
        return;
    }
    let temp = tempdir().expect("create temp dir");
    let repo = fs::canonicalize(create_git_repo(temp.path(), "partial")).expect("canonical repo");
    for index in 0_u8..5_u8 {
        fs::create_dir(repo.join(format!("dir{index}"))).expect("create subdirectory");
    }
    let _locked = UnreadableDir::create(repo.join("locked"));

    let (watcher, events) = recording_watcher();
    let pid = std::process::id();
    watcher
        .register_client(pid, &repo)
        .expect("register degraded repo");
    settle();

    git(&repo, &["checkout", "-q", "-b", "feature"]);
    assert!(
        wait_for_repo(&events, &repo, Duration::from_secs(10)),
        "the degraded repository must still hear its .git while registered"
    );

    watcher.unregister_client(pid);
    settle();
    events.lock().unwrap().clear();

    fs::write(repo.join("README.md"), b"after unregister\n").expect("edit root file");
    fs::write(repo.join("dir0").join("x.txt"), b"x\n").expect("create nested file");
    thread::sleep(Duration::from_secs(3));

    let delivered = events.lock().unwrap().clone();
    assert_eq!(
        delivered,
        Vec::<PathBuf>::new(),
        "no watch may outlive the repository's last client"
    );
}

/// Rolling back a partial recursive watch drops a nested repo's descriptors
/// too; they must be re-armed, and the outer partial tree stay gone.
#[cfg(target_os = "linux")]
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn failed_recursive_watch_rollback_keeps_nested_repo_events() {
    if nix::unistd::geteuid().is_root() {
        eprintln!("skipped: root bypasses the EACCES that fails the recursive watch");
        return;
    }
    let temp = tempdir().expect("create temp dir");
    let (outer, inner) = nested_repos(temp.path());
    let _locked = UnreadableDir::create(outer.join("locked"));

    let (watcher, events) = recording_watcher();
    let inner_pid = std::process::id();
    let outer_pid = inner_pid + 1;
    watcher
        .register_client(inner_pid, &inner)
        .expect("register inner");
    watcher
        .register_client(outer_pid, &outer)
        .expect("register degraded outer");
    settle();
    watcher.unregister_client(outer_pid);
    settle();
    events.lock().unwrap().clear();

    fs::write(outer.join("README.md"), b"after unregister\n").expect("edit outer file");
    fs::write(inner.join("README.md"), b"changed content here\n").expect("edit inner file");
    let inner_heard = wait_for_repo(&events, &inner, Duration::from_secs(20));
    thread::sleep(Duration::from_secs(3));

    let delivered = events.lock().unwrap().clone();
    assert!(
        inner_heard,
        "the nested repository registered earlier must keep its events after the outer \
         repository's failed watch is rolled back, got: {delivered:?}"
    );
    assert!(
        !delivered.contains(&outer),
        "the outer repository's partial watch must not outlive its last client, \
         got: {delivered:?}"
    );
}
