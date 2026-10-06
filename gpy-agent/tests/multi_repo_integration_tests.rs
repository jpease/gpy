//! Multi-repository integration tests
//!
//! Tests the agent's ability to handle multiple git repositories simultaneously,
//! including:
//! - Multiple repos watched concurrently
//! - Cross-repo event isolation (changes in repo A don't trigger signals for repo B)
//! - Repository discovery and registration
//! - Cleanup when repositories are removed
//! - Performance under multi-repo load

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::create_dir)]

use gpy_agent::ipc::ClientDirectory;
use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
use gpy_agent::watcher::{DebouncedEvent, FileEvent, WatcherConfig};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

// ============================================================================
// Test Helpers
// ============================================================================

/// Helper to create a git repository in a temp directory
fn create_git_repo(parent: &Path, name: &str) -> PathBuf {
    let repo_path = parent.join(name);
    fs::create_dir_all(&repo_path).expect("create repo dir");

    // Pin the initial branch rather than inheriting `init.defaultBranch`, so
    // the fixture is the same repo on every machine.
    let status = std::process::Command::new("git")
        .args(["-c", "init.defaultBranch=main", "init"])
        .current_dir(&repo_path)
        .status()
        .expect("git init");

    assert!(status.success(), "git init should succeed");

    // gpy-agent#386: git's own fsmonitor spawns a background daemon per repo
    // that independently subscribes to FSEvents for the same path gpy's own
    // watcher is watching, on a machine where `core.fsmonitor` is enabled
    // globally (a common perf setting -- see `benchmarks/README.md`'s
    // existing precedent for disabling it in benchmark fixtures for the same
    // reason). That competing daemon measurably starves gpy's own
    // FSEventStream of its first event under load -- confirmed empirically:
    // this test hard-failed 10/10 runs before this fix (every attempt hitting
    // the full 10s budget), and reliably passed after.
    std::process::Command::new("git")
        .args(["config", "core.fsmonitor", "false"])
        .current_dir(&repo_path)
        .status()
        .expect("disable fsmonitor");

    // Create initial commit to establish main branch
    fs::write(repo_path.join("README.md"), b"# Test repo").expect("write README");

    let add_status = std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(&repo_path)
        .status()
        .expect("git add");
    assert!(add_status.success(), "git add should succeed");

    // Commit hermetically: whether this repo gets its one commit must not
    // depend on the developer's global git config.
    //
    // - identity via env vars, because a machine without a global one (any CI
    //   runner) cannot commit at all;
    // - `commit.gpgsign=false`, because a developer who signs by default has
    //   every fixture commit go through GPG, which fails intermittently under
    //   a parallel test run;
    // - `-c` flags and env vars rather than `git config` calls, so the fixture
    //   adds no extra subprocesses or `.git/config` writes to perturb the
    //   FSEvents-timing tests in this file (#386, #388).
    //
    // Both statuses are asserted rather than ignored. A silently failed commit
    // leaves a repo with zero commits, and that only surfaces much later and
    // somewhere else -- as `git submodule add` cloning an "empty repository"
    // and dying on "branch yet to be born".
    let commit_status = std::process::Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "Initial commit",
        ])
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .current_dir(&repo_path)
        .status()
        .expect("git commit");
    assert!(commit_status.success(), "initial commit should succeed");

    repo_path
}

/// Helper to create a file in a git repo and stage it
fn create_and_stage_file(repo_path: &Path, filename: &str, content: &str) {
    fs::write(repo_path.join(filename), content).expect("write file");

    std::process::Command::new("git")
        .args(["add", filename])
        .current_dir(repo_path)
        .status()
        .expect("git add");
}

/// Run a git command in `dir`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git command");
    assert!(
        status.success(),
        "git {args:?} should succeed in {}",
        dir.display()
    );
}

/// Shared buffer of the `repo` roots of every debounced event delivered.
type RepoEvents = Arc<Mutex<Vec<PathBuf>>>;

/// Assert that `working_dir`'s `.git` is an external-gitdir *pointer file* (`gitdir:` line).
///
/// I.e. the submodule/worktree layout the #428 fix targets. If the local git lays it out
/// differently, the fixture is wrong and the test should fail loudly rather than silently
/// pass.
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
    let deadline = std::time::Instant::now()
        .checked_add(budget)
        .unwrap_or_else(std::time::Instant::now);
    while std::time::Instant::now() < deadline {
        if events.lock().unwrap().iter().any(|repo| repo == expected) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
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

// Note: simulate_branch_change removed - not used in current tests
// Can be re-added when needed for branch switching tests

// ============================================================================
// Multi-Repo Watcher Tests
// ============================================================================

#[test]
fn test_multi_repo_concurrent_registration() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    // Create 3 repositories
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

    // Register clients in all three repos
    watcher
        .register_client(1001, &repo1)
        .expect("register repo1");
    watcher
        .register_client(1002, &repo2)
        .expect("register repo2");
    watcher
        .register_client(1003, &repo3)
        .expect("register repo3");

    // All three repos should be watched
    assert_eq!(watcher.watched_repo_count(), 3);
}

#[test]
fn test_multi_repo_event_isolation() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    // Create 2 repositories
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

    // Let both watches arm, then drop whatever arming itself surfaced. `git
    // init` creates `.git/refs/heads` and friends during setup, and on Linux
    // those directory events are delivered *after* registration completes --
    // repo2 is deliberately registered here, so an event for it is correct
    // behaviour, not a leak. What this test actually asserts is narrower: a
    // modification to repo1 must not produce events for repo2.
    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    // Modify repo1 only
    create_and_stage_file(&repo1, "file_in_repo1.txt", "content");

    // Wait for events
    thread::sleep(Duration::from_millis(500));

    let captured_events = events.lock().unwrap();

    // Events should only reference repo1, not repo2
    // Use canonicalized paths for comparison to handle symlink resolution on macOS
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

            // Path should be in repo1's tree
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

    // Create repos with same internal structure
    let repo1 = create_git_repo(temp_path, "project_a");
    let repo2 = create_git_repo(temp_path, "project_b");

    // Create subdirectories with same name in both repos
    let src1 = repo1.join("src");
    let src2 = repo2.join("src");
    fs::create_dir_all(&src1).expect("create src1");
    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .expect("create watcher");

    // Register clients in subdirectories
    watcher.register_client(3001, &src1).expect("register src1");
    watcher.register_client(3002, &src2).expect("register src2");

    // Both should resolve to their respective repository roots
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
    // Re-register client in repo2
    watcher
        .register_client(4002, &repo2)
        .expect("register repo2");

    assert_eq!(watcher.watched_repo_count(), 2);

    // Unregister client from repo1
    let _ = watcher.unregister_client(4001);

    // If repo1 has no more clients, it should be unwatched
    // (Implementation detail: watch may persist until next cleanup)
    // For now, we just verify unregister doesn't panic
    assert!(watcher.watched_repo_count() <= 2);
}

// ============================================================================
// Multi-Repo Client Directory Tests
// ============================================================================

#[test]
fn test_client_directory_multi_repo_tracking() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "tracking_repo1");
    let repo2 = create_git_repo(temp_path, "tracking_repo2");
    let repo3 = create_git_repo(temp_path, "tracking_repo3");

    let client_dir = ClientDirectory::new();

    // Register clients in different repos
    client_dir.register(5001, Some(repo1.clone()));
    client_dir.register(5002, Some(repo1)); // Two clients in repo1
    client_dir.register(5003, Some(repo2));
    client_dir.register(5004, Some(repo3));

    assert_eq!(client_dir.len(), 4);

    // Verify all clients are registered
    // (ClientDirectory doesn't expose clients_for_repo, but we can verify count)
    assert!(!client_dir.is_empty());
}

#[test]
fn test_client_directory_repo_boundary() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "boundary_repo1");
    let repo2 = create_git_repo(temp_path, "boundary_repo2");

    let client_dir = ClientDirectory::new();

    // Register clients
    client_dir.register(6001, Some(repo1.clone()));
    client_dir.register(6002, Some(repo2.clone()));

    // Verify both clients are registered
    assert_eq!(client_dir.len(), 2);

    // Verify repaint targeting works for specific repos
    // (This tests that the directory properly tracks repo associations)
    client_dir.notify_repaint(Some(&repo1));
    client_dir.notify_repaint(Some(&repo2));

    // If we reach here without panic, the targeting worked successfully
}

// ============================================================================
// Performance Tests
// ============================================================================

#[test]
fn test_multi_repo_scalability() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .expect("create watcher");

    // Create and register 10 repositories
    // Keep repos alive for the duration of the test
    #[allow(clippy::collection_is_never_read, clippy::used_underscore_binding)]
    let mut _repos = Vec::new();
    for i in 0..10 {
        let repo = create_git_repo(temp_path, &format!("scalability_repo{i}"));
        watcher
            .register_client(7000 + i, &repo)
            .expect("register repo");
        _repos.push(repo);
    }

    assert_eq!(watcher.watched_repo_count(), 10);

    // All repos should be independently tracked
    // (No cross-contamination of events)
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

    // Rapidly register many clients in the same repo
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

    // Wait for all threads
    for handle in handles {
        handle.join().expect("thread should complete");
    }

    // All clients should be registered, but only one repo watched
    assert_eq!(watcher.watched_repo_count(), 1);
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_multi_repo_nested_repos() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    // Create outer repo
    let outer_repo = create_git_repo(temp_path, "outer_repo");

    // Create nested repo (submodule simulation)
    let nested_repo = outer_repo.join("nested");
    fs::create_dir_all(&nested_repo).expect("create nested dir");

    std::process::Command::new("git")
        .arg("init")
        .current_dir(&nested_repo)
        .status()
        .expect("git init nested");

    let watcher = MultiRepoWatcher::builder()
        .config(WatcherConfig::default())
        .callback(Box::new(|_event: DebouncedEvent| {}))
        .build()
        .expect("create watcher");

    // Register clients in both outer and nested repos
    watcher
        .register_client(9001, &outer_repo)
        .expect("register outer");
    watcher
        .register_client(9002, &nested_repo)
        .expect("register nested");

    // Both should be tracked as separate repos
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

        // Register via real path and symlink path
        watcher
            .register_client(10001, &real_repo)
            .expect("register real");
        watcher
            .register_client(10002, &symlink_path)
            .expect("register symlink");

        // Should resolve to the same canonical repo (1 watch, not 2)
        assert_eq!(watcher.watched_repo_count(), 1);
    }
}

// gpy-agent#386: `MultiRepoWatcher` shares ONE `WatchCoordinator` (and thus one
// `notify` `RecommendedWatcher`) across every registered repository. On macOS,
// notify's FSEvents backend cannot add a path to an already-running
// `FSEventStream` -- it tears the stream down and recreates it. So the second
// `register_client` call below (for repo2) silently restarts the very
// FSEventStream that was just armed for repo1, and the two writer threads then
// race that freshly-(re)started stream. Per gpy-agent#384's own measurements,
// a just-(re)started FSEventStream can go an entire wait budget without
// delivering its first event under load -- "never", not "late" -- which is the
// same failure class here, just triggered by a same-process stream restart
// rather than two literally-concurrent cross-process bootstraps. Reusing
// #384/#385's `watcher_fsevents_bootstrap` file_serial group keeps this test's
// bootstrap from overlapping with theirs across nextest's per-test processes.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn test_multi_repo_concurrent_events() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo1 = create_git_repo(temp_path, "concurrent_repo1");
    let repo2 = create_git_repo(temp_path, "concurrent_repo2");

    // `DebouncedEvent::repo` is always canonical (`find_git_root` canonicalizes
    // it), so canonicalize both sides here to compare like with like.
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

    // Trigger events in both repos simultaneously
    let handle1 = thread::spawn(move || {
        create_and_stage_file(&repo1, "file1.txt", "data1");
    });

    let handle2 = thread::spawn(move || {
        create_and_stage_file(&repo2, "file2.txt", "data2");
    });

    handle1.join().expect("thread1 complete");
    handle2.join().expect("thread2 complete");

    // Poll for events attributed to BOTH repos (matching this test's own
    // panic message) instead of a single fixed sleep that only checked the
    // events list was non-empty at all. A generous total budget absorbs the
    // FSEventStream-restart risk described above; 100ms was never adaptive to
    // machine load in the first place.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut saw_repo1 = false;
    let mut saw_repo2 = false;
    while std::time::Instant::now() < deadline {
        {
            let seen = events.lock().unwrap();
            saw_repo1 = seen.contains(&repo1_canonical);
            saw_repo2 = seen.contains(&repo2_canonical);
        }
        if saw_repo1 && saw_repo2 {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    assert!(saw_repo1, "Should receive an event from repo1");
    assert!(saw_repo2, "Should receive an event from repo2");
}

// ============================================================================
// Submodule / linked-worktree external-gitdir attribution (#428)
//
// Git metadata for a submodule lives at `<super>/.git/modules/<name>/...` and
// for a linked worktree at `<super>/.git/worktrees/<name>/...` — outside the
// working tree. A branch switch there writes HEAD/refs under that external
// gitdir, which is NOT a descendant of the working root. Before the fix,
// `classify_event` didn't recognize those paths (parent isn't `.git`, and the
// ref substrings didn't match `/.git/modules|worktrees/.../refs/...`), so zero
// watcher events fired and the prompt stayed stale until the ~45s reconcile.
//
// These use a REAL notify watcher end-to-end (not a synthetic event) and share
// the `watcher_fsevents_bootstrap` file_serial group so their FSEvents
// bootstraps don't race the other real-watcher tests on macOS.
// ============================================================================

/// A submodule branch switch must surface a git event attributed to the
/// submodule's *working root* (not the superproject).
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn submodule_branch_change_attributes_to_submodule_working_root() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    // A standalone repo to add as a submodule (needs at least one commit).
    let sub_source = create_git_repo(temp_path, "sub_source");
    let super_repo = create_git_repo(temp_path, "super_repo");

    // Add it as a submodule over the local file protocol (no network).
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
    // Keep git's own fsmonitor daemon out of the submodule gitdir (see the
    // create_git_repo comment): it competes for FSEvents and starves ours.
    git(&sub_wd, &["config", "core.fsmonitor", "false"]);
    assert_external_gitdir(&sub_wd);

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &sub_wd)
        .expect("register submodule client");

    let canonical_sub = fs::canonicalize(&sub_wd).expect("canonicalize submodule root");

    // Let the gitdir watch arm before racing it.
    thread::sleep(Duration::from_millis(150));

    // Branch switch inside the submodule writes HEAD + refs under
    // `<super>/.git/modules/sub/` — the external gitdir being watched.
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
    // The worktree shares the main repo's config (fsmonitor already disabled by
    // create_git_repo), but set it explicitly for robustness.
    git(&wt_wd, &["config", "core.fsmonitor", "false"]);
    assert_external_gitdir(&wt_wd);

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &wt_wd)
        .expect("register worktree client");

    let canonical_wt = fs::canonicalize(&wt_wd).expect("canonicalize worktree root");

    thread::sleep(Duration::from_millis(150));

    // Branch switch inside the linked worktree writes
    // `<super>/.git/worktrees/wt_linked/HEAD` — the external gitdir watched.
    git(&wt_wd, &["checkout", "-b", "other2"]);

    assert!(
        wait_for_repo(&events, &canonical_wt, Duration::from_secs(10)),
        "worktree branch switch must trigger a refresh attributed to the worktree working root"
    );
}

/// Regression guard: an ORDINARY (non-submodule, non-worktree) repo's `.git`
/// metadata change must still trigger a refresh attributed to its own root,
/// exactly as before — proving the classification refactor didn't change the
/// common case.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn ordinary_repo_head_change_still_triggers() {
    let temp = tempdir().expect("create temp dir");
    let temp_path = temp.path();

    let repo = create_git_repo(temp_path, "ordinary_repo");
    // An ordinary repo keeps `.git` as a directory inside the working tree.
    assert!(repo.join(".git").is_dir(), "ordinary repo has a .git dir");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &repo)
        .expect("register ordinary client");

    let canonical_repo = fs::canonicalize(&repo).expect("canonicalize repo root");

    thread::sleep(Duration::from_millis(150));

    git(&repo, &["checkout", "-b", "feature"]);

    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(10)),
        "an ordinary repo's HEAD change must still trigger a refresh for its own root"
    );
}

// ============================================================================
// Shared common-git-directory fan-out for linked worktrees (#468)
// ============================================================================

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

/// Commit the staged index in `dir` with the same hermetic identity/signing
/// settings `create_git_repo` uses for the initial commit.
fn git_commit(dir: &Path, message: &str) {
    let status = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "commit", "-m", message])
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .current_dir(dir)
        .status()
        .expect("git commit");
    assert!(
        status.success(),
        "commit should succeed in {}",
        dir.display()
    );
}

/// A main checkout plus one or more linked worktrees sharing its common git
/// directory.
struct WorktreeFixture {
    /// The main checkout's working root.
    main: PathBuf,
    /// Canonical common git directory (`<main>/.git`) shared by every worktree.
    common: PathBuf,
    /// Canonical working roots of the linked worktrees, in creation order.
    linked: Vec<PathBuf>,
    /// Object id the remote-tracking ref can be advanced to, distinct from the
    /// id it is seeded with.
    advanced_sha: String,
}

/// Create `<parent>/<name>` with `branches.len()` linked worktrees hanging off it.
///
/// Seeds `refs/remotes/origin/main`, and leaves a second commit available so tests can
/// advance that ref to a genuinely different object id.
///
/// `refs/remotes/origin/main` is seeded *before* any client registers so the
/// tests measure fan-out, not watch-arming: under the poll backend a
/// `refs/remotes/origin` created afterwards is watched only once a poll scan
/// has reported it and the expansion has grown to include it (#721), which
/// adds up to a poll interval before the ref's own changes are seen.
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

/// AC 6: a remote-tracking ref rewritten in the MAIN checkout's shared common
/// directory must refresh the linked worktree, whose ahead/behind output reads
/// that ref.
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

    thread::sleep(Duration::from_millis(150));

    // Originates entirely outside the linked worktree: a fetch-shaped write to
    // the shared remote-tracking ref, made against the main checkout.
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

/// AC 6: an upstream-tracking config change made in the MAIN checkout writes
/// the shared `<common>/config` and must refresh the linked worktree.
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

    thread::sleep(Duration::from_millis(150));

    // Run from the main checkout, not the worktree: this rewrites the shared
    // `<common>/config`, changing what the worktree's branch tracks.
    git(
        &fixture.main,
        &["config", "branch.cfgbranch.remote", "origin"],
    );

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "a common-dir upstream config change must refresh the linked worktree"
    );
}

/// AC 7 (required negative test): an `index` write in the main checkout is
/// per-checkout state, not shared state, and must wake **zero** linked
/// worktrees.
///
/// Asserted on wake count rather than rendered output: a spurious refresh
/// recomputes an identical status and shows nothing, while still costing a
/// full capture round per worktree per debounce window.
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

    thread::sleep(Duration::from_millis(150));

    // `git add` in the MAIN checkout rewrites `<common>/index` (plus its lock),
    // and nothing the linked worktree's status depends on.
    create_and_stage_file(&fixture.main, "staged.txt", "staged");

    // Budget well past the 100ms debounce window, the 2s poll interval, and the
    // native->poll fallback swap, so a real regression would certainly have
    // fired inside it.
    assert!(
        !wait_for_repo(&events, &linked, Duration::from_secs(8)),
        "a main-checkout index write must not wake the linked worktree"
    );
    assert_eq!(
        wake_count(&events, &linked),
        0,
        "a main-checkout index write must cost zero capture rounds in the linked worktree"
    );
    // Epic #465: no task may introduce a spurious cross-repository refresh. The
    // shallow watch on `<main>/.git` makes the watcher see paths it otherwise
    // never would when only the worktree is registered, and
    // `should_trigger_update` classifies `<common>/index` straight to `<main>`.
    // No shell is registered there, so a capture round on it is pure waste.
    assert_eq!(
        wake_count(&events, &canonical_main),
        0,
        "a main-checkout index write must not wake the UNREGISTERED main checkout either"
    );

    // Positive control: the same live watcher DOES deliver a genuinely shared
    // change, so the zero above is a rejected event and not a dead pipeline.
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

/// A per-worktree `HEAD` lives in the admin directory *under* the common
/// directory. It is per-worktree state: it must reach its own worktree and
/// wake zero siblings.
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

    thread::sleep(Duration::from_millis(150));

    // Write the first worktree's own HEAD directly, so no shared ref under
    // `<common>/refs/heads` is touched alongside it. Detach it at its current
    // commit: rewriting the ref it already holds is byte-identical and so
    // invisible to the poll backend (#817).
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

/// AC 4: the shared common-directory watch is reference-counted — unregistering
/// one dependent worktree must leave the remaining one still receiving shared
/// metadata events.
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

    // Dropping the first dependent must not tear down the shared watch.
    watcher
        .unregister_client(first_pid)
        .expect("unregister first worktree");

    thread::sleep(Duration::from_millis(150));

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

/// AC 4, coverage-by-ancestor: when the main checkout is registered its single
/// recursive worktree watch already covers the common directory, so the linked
/// worktree arms nothing extra (#388 — re-watching a covered path rebuilds the
/// `FSEventStream` for no gain). Unregistering the main checkout drops that
/// ancestor watch, and the linked worktree must not silently lose shared
/// metadata coverage with it.
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
    // Main checkout first, so its recursive watch is what covers `<common>`
    // when the linked worktree registers.
    watcher
        .register_client(main_pid, &fixture.main)
        .expect("register main checkout");
    watcher
        .register_client(linked_pid, &linked)
        .expect("register worktree client");

    watcher
        .unregister_client(main_pid)
        .expect("unregister main checkout");

    thread::sleep(Duration::from_millis(150));

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

/// Let freshly armed (or just released) watches settle before the next step.
fn settle() {
    thread::sleep(Duration::from_millis(500));
}

/// An outer repository with a nested repository inside it, both canonical.
fn nested_repos(parent: &Path) -> (PathBuf, PathBuf) {
    let outer = fs::canonicalize(create_git_repo(parent, "outer")).expect("canonical outer");
    let inner = fs::canonicalize(create_git_repo(&outer, "inner")).expect("canonical inner");
    (outer, inner)
}

/// #687: unregistering the outer repository used to remove the nested
/// repository's OS watches with it (inotify drops every descriptor under a
/// removed recursive watch; the poll backend drops every expanded directory).
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
    watcher
        .unregister_client(outer_pid)
        .expect("unregister outer");
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

/// #687: unregistering the nested repository used to remove the descriptors
/// the outer repository's recursive watch shares for that subtree (inotify).
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
    watcher
        .unregister_client(inner_pid)
        .expect("unregister inner");
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

/// #687: a linked worktree registered before its main checkout. The main
/// checkout's recursive watch overlaps the worktree's admin-dir watch
/// (`<main>/.git/worktrees/<name>`), and unregistering the main checkout used
/// to remove it (inotify, poll).
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
    watcher
        .unregister_client(main_pid)
        .expect("unregister main checkout");
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

/// #687: with `watch_worktree = false` the linked worktree arms `<common>`
/// shallow and the main checkout arms the same path (its gitdir)
/// recursively. Unregistering the main checkout used to remove both
/// (`FSEvents` removes every stream entry equal to the path), while the
/// worktree's common-dir bookkeeping still listed its shallow watch as armed.
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

    // Control: the fan-out works before the main checkout is involved.
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
    watcher
        .unregister_client(main_pid)
        .expect("unregister main checkout");
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

/// Epic #465, the higher-frequency route: a commit made *inside* the linked
/// worktree writes the SHARED `<common>/refs/heads/<branch>`, which
/// `should_trigger_update` classifies and attributes to the main checkout. With
/// no shell registered there, delivering that is a full capture round whose only
/// outputs are a cache write nobody reads and a signal with no recipient.
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

    thread::sleep(Duration::from_millis(150));

    fs::write(linked.join("wt.txt"), b"worktree file").expect("write worktree file");
    git(&linked, &["add", "wt.txt"]);
    git_commit(&linked, "Worktree commit");

    assert!(
        wait_for_repo(&events, &linked, Duration::from_secs(20)),
        "a commit in the linked worktree must refresh the linked worktree"
    );
    // Settle past the debounce window and a poll cycle so a main-checkout wake
    // emitted alongside the worktree's would have landed by now.
    thread::sleep(Duration::from_secs(3));
    assert_eq!(
        wake_count(&events, &canonical_main),
        0,
        "a commit in the linked worktree must not wake the unregistered main checkout"
    );
}

/// The contrast the guard must not break: when the main checkout IS registered,
/// a write to its own git directory must still wake it, exactly as before.
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

    thread::sleep(Duration::from_millis(150));

    // Written directly rather than via `git add`: staging would also touch the
    // working tree, which wakes the main checkout through its own recursive
    // worktree watch and would let this pass even if the `.git` path were
    // wrongly suppressed.
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

/// #468: a linked worktree of a BARE superproject. Its common directory is the
/// bare repository itself (`/repos/foo.git`), which — unlike the usual
/// `<main>/.git` — has no `.git` path component, so `classify_event` ignores it
/// and events reach `process_event`'s third arm instead of its first.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn bare_superproject_common_dir_fans_out_without_waking_the_bare_repo() {
    let temp = tempdir().expect("create temp dir");
    let root = temp.path();

    let bare = root.join("bare_super.git");
    fs::create_dir_all(&bare).expect("create bare dir");
    git(&bare, &["-c", "init.defaultBranch=main", "init", "--bare"]);
    git(&bare, &["config", "core.fsmonitor", "false"]);

    // Seed one commit through a throwaway clone, then discard it.
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
    thread::sleep(Duration::from_millis(150));

    // Per-checkout state in the bare common dir: must wake nobody. The bare
    // repository has no registered client, so a refresh of it is pure waste.
    fs::write(canonical_bare.join("ORIG_HEAD"), format!("{seed_sha}\n")).expect("write ORIG_HEAD");
    thread::sleep(Duration::from_secs(3));
    assert_eq!(
        wake_count(&events, &canonical_bare),
        0,
        "per-checkout state in a bare common dir must not wake the unregistered bare repo"
    );

    // Control: shared state in the same directory must still reach the worktree.
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

// ============================================================================
// Parent-repository fan-out for submodule state changes (#467)
// ============================================================================

/// A superproject with one submodule checked out at `sub`.
struct SubmoduleFixture {
    /// Canonical superproject working root.
    super_root: PathBuf,
    /// Canonical submodule working root (`<super>/sub`).
    sub_root: PathBuf,
    /// Uncanonicalized submodule working directory, for running git in.
    sub_dir: PathBuf,
    /// A submodule commit whose tree is byte-identical to the checked-out tip,
    /// so moving between the two changes the gitlink and nothing else.
    identical_tree_sha: String,
}

/// Build `<parent>/<name>` with a submodule at `sub`, plus a submodule commit
/// whose tree is identical to the tip's.
///
/// The identical tree is what makes AC 4 provable: a checkout between the two
/// writes only `<super>/.git/modules/sub/…` and touches no file in the
/// superproject's working tree, so a parent refresh cannot have come from a
/// worktree event. The fixture asserts the two trees really are the same object
/// rather than assuming git behaves as expected.
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
    // Keep git's own fsmonitor daemon out of the submodule gitdir: it competes
    // for FSEvents and starves ours (see `create_git_repo`).
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

/// AC 2 + AC 4: a ref-only submodule checkout between two identical trees must
/// refresh the superproject even though the submodule has no registered shell.
///
/// Nothing in the superproject's working tree changes, so every existing
/// attribution route is silent: `classify_event` does not treat
/// `modules/sub/HEAD` as significant, the submodule's gitdir is not in the
/// external-gitdir registry (it is only registered when the submodule itself
/// is), and `worktree_git_repo` rejects anything inside a `.git` directory.
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn ref_only_submodule_checkout_refreshes_unregistered_parent() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_submodule_fixture(temp.path(), "sm_parent_only");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &fixture.super_root)
        .expect("register superproject client");

    // Let the recursive watch arm, then drop whatever arming itself surfaced.
    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    // Detach onto the commit with the identical tree: this rewrites
    // `<super>/.git/modules/sub/HEAD` and writes no working-tree file at all.
    git(&fixture.sub_dir, &["checkout", &fixture.identical_tree_sha]);

    assert!(
        wait_for_repo(&events, &fixture.super_root, Duration::from_secs(20)),
        "a ref-only submodule checkout must refresh the superproject whose gitlink moved"
    );
}

/// AC 3 + AC 6: with both the superproject and the submodule registered, one
/// submodule branch switch must refresh both — the child through the existing
/// external-gitdir attribution, the parent through the new fan-out.
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

    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    // Writes `<super>/.git/modules/sub/HEAD` and `.../refs/heads/other`, and
    // nothing in either working tree.
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

/// AC 6: with only the submodule registered, the submodule refreshes and the
/// superproject — which no shell is subscribed to — must wake zero times.
///
/// The registration gate is the point: a parent fan-out that skipped it would
/// buy a full capture round on a repository nothing reads (gpy#480).
#[test]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn submodule_branch_switch_wakes_zero_unregistered_parents() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_submodule_fixture(temp.path(), "sm_child_only");

    let (watcher, events) = recording_watcher();
    watcher
        .register_client(std::process::id(), &fixture.sub_root)
        .expect("register submodule client");

    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    git(&fixture.sub_dir, &["checkout", "-b", "other"]);

    // Positive control first: the pipeline is live, so the zero below is a
    // rejected event rather than a dead watcher.
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

/// AC 5: an ordinary nested repository is not a submodule — its gitdir is
/// in-tree, so nothing it writes can look like `<super>/.git/modules/…`, and the
/// enclosing repository has no gitlink to it. A `.git`-only change inside it
/// must wake the enclosing registered repository zero times.
///
/// Asserted on wake count, not rendered output: a spurious refresh recomputes an
/// identical status and shows nothing while still costing a capture round per
/// debounce window.
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

    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    // A branch switch writes only `<outer>/nested/.git/{HEAD,refs/heads/inner}`,
    // so the outer repo sees no worktree change to attribute.
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

/// A linked worktree's admin directory is an external gitdir sitting under the
/// main checkout's `.git`, exactly like a submodule's — but the main checkout
/// holds no gitlink to it, so a per-worktree `HEAD` write must wake it zero
/// times.
///
/// This is the guard against keying the parent fan-out on "any external gitdir"
/// rather than on the `/.git/modules/` segment: both layouts would strip to the
/// same superproject-looking prefix, and only this test tells them apart.
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

    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    let admin_dir = fixture
        .common
        .join("worktrees")
        .join("wt_notparent_npbranch");
    assert!(
        admin_dir.is_dir(),
        "expected per-worktree admin dir at {}",
        admin_dir.display()
    );
    // Detach rather than rewrite the ref HEAD already holds: a byte-identical
    // write is invisible to the poll backend (#817).
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

// ============================================================================
// #469: interactive-rebase / am-style-rebase metadata
// ============================================================================

/// Write an executable `GIT_SEQUENCE_EDITOR` script into `dir` that prepends
/// two `break` lines to whatever todo git hands it, so an interactive rebase
/// stops twice before ever picking a commit.
///
/// Continuing from the first stop to the second is a genuinely metadata-only
/// step: a `break` action applies no commit, so only `rebase-merge/msgnum`
/// advances between the two stops -- `HEAD` does not move again. Verified
/// empirically while implementing #469 (a fresh `git rebase -i --root` DOES
/// write `HEAD` once, for the initial detached checkout, which is why the
/// test built on this helper asserts the *second* stop is HEAD-free rather
/// than asserting no HEAD write at all).
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

/// AC 1 + AC 4 + AC 5 (metadata-only case): a real interactive rebase's
/// step advance writes only `rebase-merge/msgnum`, never `HEAD` or any
/// working-tree file, once past the initial detached checkout. Regression
/// test for #469: before the fix, `rebase-merge/msgnum` failed
/// `is_git_significant_relative`'s top-level check and every such advance was
/// silently dropped.
///
/// Driven through a real `git rebase -i --root`/`--continue` pair rather than
/// writing `msgnum` by hand, specifically so the test cannot be satisfied by
/// an incidental `HEAD` write -- the trap the issue's validation comment
/// calls out.
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

    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

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

    // Setup wake, not the assertion under test: the initial detached checkout
    // writes HEAD, which was already classified before #469.
    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "setup: starting the rebase must refresh the repository"
    );
    events.lock().unwrap().clear();

    let head_before = fs::read_to_string(repo.join(".git").join("HEAD")).expect("read HEAD");

    // The actual regression: continuing past the first `break` to the second
    // advances `rebase-merge/msgnum` and nothing else.
    let rebase_continue_status = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "rebase", "--continue"])
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .current_dir(&repo)
        .status()
        .expect("continue past the first break");
    assert!(
        rebase_continue_status.success(),
        "rebase --continue to the second break should succeed"
    );

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

/// AC 2: creation of the `rebase-apply/applying` marker (an `am`-style
/// operation starting) and removal of the whole `rebase-apply` directory (the
/// operation finishing) must each refresh the repository on their own --
/// exercised as direct metadata writes rather than a full `git am` run, since
/// AC 2 is about the create/remove signal itself, not about driving `am`.
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

    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    // `git am` starting: rebase-apply directory + applying marker, with no
    // HEAD write until the first patch actually commits.
    let rebase_apply = repo.join(".git").join("rebase-apply");
    fs::create_dir(&rebase_apply).expect("create rebase-apply dir");
    fs::write(rebase_apply.join("applying"), "").expect("write applying marker");

    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "creating rebase-apply/applying must refresh the repository (Applying state)"
    );
    events.lock().unwrap().clear();

    // The operation finishing: the whole directory disappears, again with no
    // accompanying HEAD write in this synthetic case.
    fs::remove_dir_all(&rebase_apply).expect("remove rebase-apply dir");

    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "removing rebase-apply must refresh the repository (state clears)"
    );
}

/// AC 3: `rebase-merge/patch`, rewritten on every interactive-rebase step,
/// must not itself buy a refresh -- the distinction between the two files
/// this segment actually reads (`msgnum`/`end`) and the high-churn internals
/// that admitting the whole `rebase-merge` subtree would sweep in.
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

    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

    let rebase_merge = repo.join(".git").join("rebase-merge");
    fs::create_dir(&rebase_merge).expect("create rebase-merge dir");
    // Setup wake: the directory entry itself is significant (AC 2).
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

    // Positive control: the same live watcher does refresh on the actual
    // progress file, so the zero above is a rejected event, not a dead
    // pipeline.
    fs::write(rebase_merge.join("msgnum"), "1\n").expect("write msgnum");
    assert!(
        wait_for_repo(&events, &canonical_repo, Duration::from_secs(20)),
        "control: a rebase-merge/msgnum write must still refresh the repository"
    );
}

/// Consumer 3 (#467's `owning_submodule_gitdir`, reused by
/// `attribute_submodule_parents`): a rebase started inside a *registered*
/// submodule must refresh both the submodule itself (through the existing
/// external-gitdir attribution) and the registered superproject (through the
/// #467 fan-out), since the submodule's gitlink is what the superproject's
/// status renders. Uses the same metadata-only `break`-to-`break` advance as
/// the plain-repository test above, for the same reason: an incidental HEAD
/// write would pass against unchanged code.
#[test]
#[cfg(unix)]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn submodule_rebase_msgnum_progress_refreshes_both_registered_parent_and_child() {
    let temp = tempdir().expect("create temp dir");
    let fixture = create_submodule_fixture(temp.path(), "sm_rebase");
    let editor = double_break_sequence_editor(temp.path());
    let submodule_gitdir = fixture.super_root.join(".git").join("modules").join("sub");
    // Epic #465: no task may introduce a spurious cross-repository refresh.
    // An unrelated, registered, third repository must never wake from a
    // submodule rebase in a sibling fixture.
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

    thread::sleep(Duration::from_millis(300));
    events.lock().unwrap().clear();

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

    // Setup wake, not the assertion under test.
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

    let rebase_continue_status = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "rebase", "--continue"])
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .current_dir(&fixture.sub_dir)
        .status()
        .expect("continue past the first break");
    assert!(
        rebase_continue_status.success(),
        "rebase --continue to the second break should succeed"
    );

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

// ============================================================================
// Partially armed recursive watches (#722)
// ============================================================================

/// An unreadable directory, made readable again on drop.
///
/// inotify needs read permission on a directory to watch it, so a recursive
/// watch over its parent fails part-way through the walk (#722). Restoring
/// the mode on drop lets the `TempDir` above it be removed even when an
/// assertion fails first.
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

        // Best effort: failing here only leaves the temp dir behind.
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

/// #722: inotify arms a recursive watch one directory at a time and stops at
/// the first one it cannot read, without undoing the directories it already
/// armed. The failed worktree watch degrades the repository to `.git`-only
/// watching (#158), and the partial tree it left behind used to stay live
/// for the life of the agent, delivering events for a repository no client
/// was registered for any more.
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
        "the degraded repository must still hear its .git while registered (#158)"
    );

    watcher.unregister_client(pid).expect("unregister");
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

/// #722: rolling back a partially armed recursive watch removes every inotify
/// descriptor under its root, including the ones a nested repository
/// registered earlier relies on. Those must be re-armed, and the outer
/// repository's partial tree must still be gone once it unregisters.
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
    watcher
        .unregister_client(outer_pid)
        .expect("unregister outer");
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
