use super::refresh::{
    agent_refresh_required, git_refresh_required, language_refresh_required, ui_refresh_required,
};
use super::*;
use crate::config::Config;
use crate::git::{FileStatus, RepositoryState, StatusAggregate};
use crate::ipc::ClientDirectory;
use crate::theme::ThemeManager;
use crate::watcher::{DebouncedEvent, FileEvent, WatcherConfig, multi_repo::MultiRepoWatcher};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

fn make_theme_manager() -> Arc<ThemeManager> {
    Arc::new(ThemeManager::builtin("default").expect("theme manager"))
}

fn make_client_registry() -> Arc<ClientDirectory> {
    ClientDirectory::new().shared()
}

fn make_git_cache() -> Arc<GitStatusCache> {
    Arc::new(GitStatusCache::new())
}

fn make_config_manager() -> Arc<crate::config::manager::ConfigManager> {
    Arc::new(crate::config::manager::ConfigManager::with_defaults().expect("config"))
}

fn make_instant_cache() -> Arc<InstantPromptCache> {
    Arc::new(InstantPromptCache::new_for_test())
}

/// A fresh directory for theme exports, so reloads never touch the user's cache.
fn temp_export_dir() -> PathBuf {
    tempdir().expect("theme export dir").keep()
}

fn make_agent_context() -> AgentContext {
    let config_manager = make_config_manager();
    let initial_config = config_manager.get();
    AgentContext {
        registry: make_client_registry(),
        cache: make_git_cache(),
        config_manager,
        theme_manager: make_theme_manager(),
        instant_cache: make_instant_cache(),
        language_cache: crate::language::DetectionCache::new(),
        palette_cache: Arc::new(crate::palette::PaletteCache::from_config(&initial_config)),
        dispatch_guard: Arc::new(RepoRefreshCoordinator::default()),
        watch_registry: Arc::new(crate::watcher::WatchRegistry::new()),
        theme_export_dir: temp_export_dir(),
        announce_gate: Arc::new(Mutex::new(())),
    }
}

/// Swap `ctx`'s registry for one that counts doorbell rings (this process is
/// its one client, subscribed to every repository).
#[cfg(unix)]
fn count_rings(ctx: &mut AgentContext) -> Arc<std::sync::atomic::AtomicU64> {
    let (registry, rings) = ClientDirectory::counting_rings();
    ctx.registry = registry.shared();
    rings
}

/// Doorbell rings counted so far.
#[cfg(unix)]
fn rung(rings: &std::sync::atomic::AtomicU64) -> u64 {
    rings.load(std::sync::atomic::Ordering::SeqCst)
}

/// Supervisor settings render nothing (no refresh) but change the theme
/// export, so each rings the reload doorbell; an unchanged reload rings nothing.
#[cfg(unix)]
#[test]
#[serial_test::serial(global_ttl)]
fn supervisor_only_changes_ring_the_reload_doorbell() {
    use crate::config::types::{SupervisorCheckInterval, SupervisorMaxRestartAttempts};

    let original = Config::default();
    let mut ctx = make_agent_context();
    let counted = count_rings(&mut ctx);
    // Resolved the way `reload` re-resolves it, so re-reading it changes nothing.
    ctx.theme_manager = Arc::new(ThemeManager::new(original.ui.theme.as_str()).expect("theme"));
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
    let rings = |old: &Config, new: &Config| {
        let before = rung(&counted);
        handle_config_reload(
            &ctx,
            &watcher_slot,
            WatcherConfig::default(),
            true,
            old,
            new,
        )
        .expect("config reload should succeed");
        rung(&counted) > before
    };

    rings(&original, &original); // Writes the first export.
    assert!(!rings(&original, &original), "an unchanged reload rang");

    let mut disabled = original.clone();
    disabled.agent.supervisor.enabled = false;
    let mut interval = disabled.clone();
    interval.agent.supervisor.check_interval_seconds =
        SupervisorCheckInterval::new(45).expect("valid interval");
    let mut attempts = interval.clone();
    attempts.agent.supervisor.max_restart_attempts =
        SupervisorMaxRestartAttempts::new(2).expect("valid attempts");
    for (field, old, new) in [
        ("enabled", &original, &disabled),
        ("check_interval_seconds", &disabled, &interval),
        ("max_restart_attempts", &interval, &attempts),
    ] {
        assert!(
            !config_refresh_required(old, new),
            "agent.supervisor.{field} must not force a re-render"
        );
        assert!(rings(old, new), "agent.supervisor.{field} must ring");
    }
}

/// A theme reload and a config reload racing on one stale export ring once
/// between them: the gate makes the later one see the earlier one's export.
#[cfg(unix)]
#[test]
#[serial_test::serial(global_ttl)]
fn racing_theme_and_config_reloads_ring_once() {
    let config = Config::default();
    let mut ctx = make_agent_context();
    let rings = count_rings(&mut ctx);
    // Resolved the way `reload` re-resolves it, so re-reading it changes nothing.
    ctx.theme_manager = Arc::new(ThemeManager::new(config.ui.theme.as_str()).expect("theme"));
    let racer = Mutex::new(None);
    // Stands in for a swap that left the (empty) export stale; the config
    // reload starts mid-swap and, ungated, would write that export and ring.
    let swap = || {
        let (thread_ctx, thread_config) = (ctx.clone(), config.clone());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        *racer.lock().expect("racer lock") = Some(thread::spawn(move || {
            let slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
            handle_config_reload(
                &thread_ctx,
                &slot,
                WatcherConfig::default(),
                true,
                &thread_config,
                &thread_config,
            )
            .expect("config reload should succeed");
            let _ = done_tx.send(());
        }));
        let _ = done_rx.recv_timeout(Duration::from_millis(300));
        true
    };

    handle_theme_reload(&ctx, &swap);
    racer
        .lock()
        .expect("racer lock")
        .take()
        .expect("config reload started")
        .join()
        .expect("config reload thread");

    assert_eq!(rung(&rings), 1, "one theme change must ring once");
}

/// `path` as a `git` URL argument; `git` rejects Windows `\\?\` paths.
fn git_url(path: &Path) -> String {
    let text = path.to_str().expect("utf-8 path");
    text.strip_prefix(r"\\?\")
        .filter(|plain| !plain.starts_with("UNC\\"))
        .unwrap_or(text)
        .to_owned()
}

fn create_temp_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().expect("create temp dir");
    std::process::Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output()
        .expect("git init");
    let canonical = fs::canonicalize(dir.path()).expect("canonical repo");
    (dir, canonical)
}

/// A `GitPaths::WholeRepo` event, as a rescan or path-bound overflow emits.
fn whole_repo_event(repo: &Path) -> DebouncedEvent {
    DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::WholeRepo,
        },
        repo: repo.to_path_buf(),
    }
}

fn commit_all(repo: &Path) {
    for args in [
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
        vec!["add", "-A"],
        vec!["commit", "-m", "init"],
    ] {
        // Global `commit.gpgsign` makes gpg time out under parallel tests.
        let output = std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .args(&args)
            .current_dir(repo)
            .output()
            .expect("git command");
        assert!(output.status.success(), "git {args:?} failed");
    }
}

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

/// Records each `complete_status` call's `paths` (`None` = full scan).
struct CountingGitBackend {
    calls: Mutex<Vec<Option<Vec<PathBuf>>>>,
}

impl CountingGitBackend {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("calls lock").len()
    }

    fn call_paths(&self, index: usize) -> Option<Vec<PathBuf>> {
        self.calls
            .lock()
            .expect("calls lock")
            .get(index)
            .cloned()
            .flatten()
    }
}

impl GitBackend for CountingGitBackend {
    fn complete_status(
        &self,
        path: &Path,
        paths: Option<&[PathBuf]>,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(paths.map(<[PathBuf]>::to_vec));
        NativeGitBackend.complete_status(path, paths, max_ahead_behind, stash_enabled, timeout)
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
        backend.call_count(),
        1,
        "a .git/index write must cost exactly one complete git capture round"
    );
    assert_eq!(
        backend.call_paths(0),
        None,
        "the single capture round must be a full scan (paths == None)"
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
    assert_eq!(backend.call_count(), 1, "warm-up scan should be one call");

    let paths_hint = git_paths_hint(&GitPaths::single(source.clone()), &repo);
    assert_eq!(
        paths_hint,
        Some(vec![source]),
        "precondition: an ordinary file write must hint a single-path incremental scan"
    );

    refresh_repo_status_with(&backend, &ctx, &repo, &config, paths_hint.as_deref());

    assert_eq!(
        backend.call_count(),
        2,
        "one warm-up call plus exactly one incremental call"
    );
    assert_eq!(
        backend.call_paths(1),
        Some(vec![PathBuf::from("a.txt")]),
        "the incremental capture round must be pathspec-limited to the changed file"
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

    assert!(
        backend.call_count() > 1,
        "the incremental call site must share the caller's backend, or a \
         capture-round counter is blind to it (saw {} call(s))",
        backend.call_count()
    );
    assert_eq!(
        backend.call_paths(0),
        Some(vec![PathBuf::from(".git/index")]),
        "the first round is the pathspec-limited attempt"
    );
    assert_eq!(
        backend.call_paths(backend.call_count() - 1),
        None,
        "the last round is the full scan the event always needed"
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

fn git_in(repo: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repo)
        .output()
        .expect("spawn git")
        .status
        .success()
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

fn status(untracked: u32, state: crate::git::RepositoryState) -> RepositoryStatus {
    RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked,
        conflicts: 0,
        state,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    }
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

#[test]
#[serial_test::serial(global_ttl)]
fn config_reload_disables_and_reenables_watcher() {
    let (
        theme_manager,
        watcher_config,
        watcher_slot,
        client_registry,
        git_cache,
        config_manager,
        instant_cache,
    ) = setup_reload_context();

    let initial_config = config_manager.get();
    let ctx = AgentContext {
        registry: client_registry,
        cache: git_cache,
        config_manager,
        theme_manager,
        instant_cache,
        language_cache: crate::language::DetectionCache::new(),
        palette_cache: Arc::new(crate::palette::PaletteCache::from_config(&initial_config)),
        dispatch_guard: Arc::new(RepoRefreshCoordinator::default()),
        watch_registry: Arc::new(crate::watcher::WatchRegistry::new()),
        theme_export_dir: temp_export_dir(),
        announce_gate: Arc::new(Mutex::new(())),
    };

    let apply = |old_cfg: &Config, new_cfg: &Config| {
        handle_config_reload(&ctx, &watcher_slot, watcher_config, false, old_cfg, new_cfg)
            .expect("config reload should succeed");
    };

    let original = Config::default();
    let mut git_disabled = original.clone();
    git_disabled.git.enabled = false;
    apply(&original, &git_disabled);
    expect_watcher_state(
        &watcher_slot,
        true,
        "watcher should remain active when language live updates still need it",
    );

    let mut disabled = git_disabled.clone();
    disabled.language.enabled = false;
    apply(&git_disabled, &disabled);
    expect_watcher_state(
        &watcher_slot,
        false,
        "watcher should be cleared when both git and language updates are disabled",
    );

    let mut reenabled = disabled.clone();
    reenabled.git.enabled = true;
    apply(&disabled, &reenabled);
    expect_watcher_state(
        &watcher_slot,
        true,
        "watcher should be recreated when git.enabled returns to true",
    );

    let mut no_live_updates = reenabled.clone();
    no_live_updates.agent.live_updates = false;
    apply(&reenabled, &no_live_updates);
    expect_watcher_state(
        &watcher_slot,
        false,
        "watcher should stop when agent.live_updates is false",
    );
}

#[cfg(unix)]
#[test]
#[serial_test::serial(global_ttl)]
fn config_reload_notifies_on_language_toggles() {
    let mut ctx = make_agent_context();
    let rings = count_rings(&mut ctx);
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
    let watcher_config = WatcherConfig::default();

    let original = Config::default();
    let mut toggled = original.clone();
    toggled.language.show_versions = !original.language.show_versions;

    let before = rung(&rings);
    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        true,
        &original,
        &toggled,
    )
    .expect("config reload should succeed");
    let after = rung(&rings);
    assert!(
        after > before,
        "toggling show_versions should trigger notification"
    );

    let mut toggled_enabled = toggled.clone();
    toggled_enabled.language.enabled = !toggled.language.enabled;
    let before_enabled = rung(&rings);
    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        true,
        &toggled,
        &toggled_enabled,
    )
    .expect("config reload should succeed");
    let after_enabled = rung(&rings);
    assert!(
        after_enabled > before_enabled,
        "toggling language.enabled should trigger notification"
    );

    // A real builtin theme: unresolvable theme names are a hard error.
    let mut theme_changed = toggled_enabled.clone();
    theme_changed.ui.theme =
        crate::config::types::ThemeName::new("text".to_owned()).expect("valid theme name");
    let before_theme = rung(&rings);
    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        true,
        &toggled_enabled,
        &theme_changed,
    )
    .expect("config reload should succeed");
    let after_theme = rung(&rings);
    assert!(
        after_theme > before_theme,
        "changing ui.theme should trigger notification"
    );
}

#[allow(clippy::type_complexity)]
fn setup_reload_context() -> (
    Arc<ThemeManager>,
    WatcherConfig,
    SharedWatcherSlot,
    Arc<ClientDirectory>,
    Arc<GitStatusCache>,
    Arc<crate::config::manager::ConfigManager>,
    Arc<InstantPromptCache>,
) {
    let theme_manager = make_theme_manager();
    let watcher_config = WatcherConfig::default();
    let mock_watcher = Arc::new(
        MultiRepoWatcher::builder()
            .config(watcher_config)
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .expect("mock watcher should initialize"),
    );
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(Some(mock_watcher)));
    let client_registry = make_client_registry();
    let git_cache = make_git_cache();
    let config_manager = make_config_manager();
    let instant_cache = make_instant_cache();

    (
        theme_manager,
        watcher_config,
        watcher_slot,
        client_registry,
        git_cache,
        config_manager,
        instant_cache,
    )
}

fn expect_watcher_state(watcher_slot: &SharedWatcherSlot, expect_some: bool, message: &str) {
    let watcher_guard = watcher_slot.lock().expect("watcher lock poisoned");
    assert_eq!(watcher_guard.is_some(), expect_some, "{message}");
}

#[test]
#[serial_test::serial(global_ttl)]
fn config_reload_updates_language_ttl() {
    use crate::cache::ttl_map::shared_cache_ttl_seconds;
    use crate::language::version::set_version_cache_ttl;

    let ctx = make_agent_context();
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
    let watcher_config = WatcherConfig::default();

    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    let before = shared_cache_ttl_seconds();
    let initial_hours = before / 3600;

    let mut old_config = Config::default();
    old_config.language.cache_ttl_hours =
        crate::config::types::CacheTtlHours::new(initial_hours).expect("valid ttl");

    let mut updated = Config::default();
    updated.language.cache_ttl_hours =
        crate::config::types::CacheTtlHours::new(36).expect("valid ttl");

    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        true,
        &old_config,
        &updated,
    )
    .expect("config reload should succeed");

    let after = shared_cache_ttl_seconds();
    assert_eq!(
        after,
        36 * 3600,
        "cache TTL should update to new value after reload"
    );
    assert_ne!(after, before, "TTL should change from previous value");

    // Restore default for other tests.
    set_version_cache_ttl(24);
}

#[test]
fn palette_change_triggers_refresh_required() {
    use crate::config::types::PaletteName;
    let old = Config::default();
    let mut new_cfg = Config::default();
    new_cfg.ui.palette = PaletteName::new("other-palette".to_owned()).expect("valid name");
    assert!(
        config_refresh_required(&old, &new_cfg),
        "a ui.palette change must be recognized as requiring a prompt refresh"
    );
}

#[test]
fn plugin_segment_override_change_triggers_refresh_required() {
    let old = Config::default();
    let new_cfg = crate::config::loader::parse_config_toml("[plugin.k8s]\nttl_secs = 30\n")
        .expect("valid plugin override");
    assert!(
        config_refresh_required(&old, &new_cfg),
        "a [plugin.<segment>] change must be recognized as requiring a prompt refresh"
    );
}

#[test]
fn unchanged_config_does_not_require_refresh() {
    let config = Config::default();
    assert!(
        !config_refresh_required(&config, &config),
        "identical configs must not trigger a prompt refresh"
    );
}

#[test]
fn ui_show_icons_change_triggers_refresh_required() {
    let old = crate::config::UiSettings::default();
    let new = crate::config::UiSettings {
        show_icons: !old.show_icons,
        ..crate::config::UiSettings::default()
    };
    assert!(
        ui_refresh_required(&old, &new),
        "ui.show_icons is rendered directly; changing it must require a refresh"
    );
}

/// Shells re-source `ui.transient_prompt` only after a reload doorbell.
#[test]
fn ui_transient_prompt_change_triggers_refresh_required() {
    let old = crate::config::UiSettings::default();
    let new = crate::config::UiSettings {
        transient_prompt: !old.transient_prompt,
        ..crate::config::UiSettings::default()
    };
    assert!(ui_refresh_required(&old, &new));
}

#[test]
fn git_stash_enabled_change_triggers_refresh_required() {
    let old = crate::config::GitSettings::default();
    let new = crate::config::GitSettings {
        stash_enabled: !old.stash_enabled,
        ..crate::config::GitSettings::default()
    };
    assert!(
        git_refresh_required(&old, &new),
        "git.stash_enabled affects the rendered $stash indicator; must require a refresh"
    );
}

#[test]
fn excluded_field_changes_do_not_trigger_refresh_required() {
    type Case = (&'static str, fn() -> bool);
    let cases: [Case; 3] = [
        ("git.timeout_seconds", || {
            let old = crate::config::GitSettings::default();
            let new = crate::config::GitSettings {
                timeout_seconds: crate::config::types::GitTimeout::new(
                    old.timeout_seconds.get() + 1,
                )
                .expect("valid git timeout"),
                ..crate::config::GitSettings::default()
            };
            assert_ne!(old.timeout_seconds, new.timeout_seconds);
            git_refresh_required(&old, &new)
        }),
        ("language.cache_ttl_hours", || {
            let old = crate::config::LanguageSettings::default();
            let new = crate::config::LanguageSettings {
                cache_ttl_hours: crate::config::types::CacheTtlHours::new(
                    old.cache_ttl_hours.get() + 1,
                )
                .expect("valid cache ttl"),
                ..crate::config::LanguageSettings::default()
            };
            assert_ne!(old.cache_ttl_hours, new.cache_ttl_hours);
            language_refresh_required(&old, &new)
        }),
        ("agent.timeout_seconds", || {
            let old = crate::config::AgentSettings::default();
            let new = crate::config::AgentSettings {
                timeout_seconds: crate::config::types::AgentTimeout::new(
                    old.timeout_seconds.get() + 1,
                )
                .expect("valid agent timeout"),
                ..crate::config::AgentSettings::default()
            };
            assert_ne!(old.timeout_seconds, new.timeout_seconds);
            agent_refresh_required(&old, &new)
        }),
    ];
    for (field, refresh_required) in cases {
        assert!(
            !refresh_required(),
            "{field} does not affect rendering; must not require a refresh"
        );
    }
}

#[test]
fn language_filter_change_triggers_refresh_required() {
    let old = crate::config::LanguageSettings::default();
    let new = crate::config::LanguageSettings {
        filter: crate::config::types::LanguageFilter::Primary,
        ..crate::config::LanguageSettings::default()
    };
    assert_ne!(
        old.filter, new.filter,
        "the mutation must actually change the field for this test to mean anything"
    );
    assert!(
        language_refresh_required(&old, &new),
        "language.filter is a render-time decision over already-detected languages; must require a refresh"
    );
}

#[test]
fn agent_live_updates_change_triggers_refresh_required() {
    let old = crate::config::AgentSettings::default();
    let new = crate::config::AgentSettings {
        live_updates: !old.live_updates,
        ..crate::config::AgentSettings::default()
    };
    assert!(
        agent_refresh_required(&old, &new),
        "agent.live_updates gates whether renders are pushed at all; must require a refresh"
    );
}

/// Agent context with an instant cache in `cache_dir` and the builtin theme.
fn make_hermetic_ctx(cache_dir: &Path) -> AgentContext {
    let config_manager = make_config_manager();
    let initial_config = config_manager.get();
    AgentContext {
        registry: make_client_registry(),
        cache: make_git_cache(),
        config_manager,
        theme_manager: Arc::new(ThemeManager::builtin("default").expect("builtin default")),
        instant_cache: Arc::new(
            InstantPromptCache::new_in_dir(cache_dir.to_path_buf()).expect("instant cache"),
        ),
        language_cache: crate::language::DetectionCache::new(),
        palette_cache: Arc::new(crate::palette::PaletteCache::from_config(&initial_config)),
        dispatch_guard: Arc::new(RepoRefreshCoordinator::default()),
        watch_registry: Arc::new(crate::watcher::WatchRegistry::new()),
        theme_export_dir: temp_export_dir(),
        announce_gate: Arc::new(Mutex::new(())),
    }
}

fn cache_file_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .expect("read cache dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// A theme change regenerates warm git instant caches so the repaint does not flicker.
#[test]
fn theme_change_regenerates_git_caches_for_active_path() {
    use crate::git::RepositoryState;
    use std::collections::HashMap;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    ctx.registry.register(4321, Some(repo.clone()));
    ctx.cache
        .set(&repo, status(0, RepositoryState::Clean), HashMap::new());

    regenerate_instant_caches_for_theme_change(&ctx, &Config::default());

    let files = cache_file_names(cache_tmp.path());

    let git_none = files
        .iter()
        .find(|f| f.ends_with(".git.none.ansi"))
        .expect("context-free git cache must be regenerated");
    for dialect in crate::formatter::PromptDialect::ALL {
        let ext = format!(".{}", dialect.cache_ext());
        assert!(
            files
                .iter()
                .any(|f| f.contains(".git.none.") && f.ends_with(&ext)),
            "context-free {ext} git cache must be regenerated: {files:?}"
        );
        // A non-`none` token file proves the new theme's `prev_bg` was seeded.
        assert!(
            files.iter().any(|f| {
                f.contains(".git.")
                    && f.ends_with(&ext)
                    && !f.contains(".git.none.")
                    && !f.contains(".git_last.none.")
            }),
            "a seeded prev_bg {ext} git cache must exist: {files:?}"
        );
    }

    let rendered = fs::read_to_string(cache_tmp.path().join(git_none)).expect("read git cache");
    assert!(
        rendered.contains("main"),
        "regenerated git cache must carry the cached branch: {rendered:?}"
    );
    assert!(
        rendered.contains('\u{1b}'),
        "regenerated git cache must be styled ANSI: {rendered:?}"
    );
}

/// A repo cached before it was skipped must not get its git cache republished on reload.
#[test]
fn theme_change_does_not_publish_git_caches_for_skipped_repo() {
    use crate::git::RepositoryState;
    use std::collections::HashMap;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    ctx.registry.register(4321, Some(repo.clone()));
    ctx.cache
        .set(&repo, status(0, RepositoryState::Clean), HashMap::new());

    let mut config = Config::default();
    config.git.skip_paths = vec![repo.to_string_lossy().into_owned()];
    regenerate_instant_caches_for_theme_change(&ctx, &config);

    let files = cache_file_names(cache_tmp.path());
    assert!(
        !files.iter().any(|f| f.contains(".git")),
        "a skipped repo must not get git caches: {files:?}"
    );
}

#[test]
fn show_icons_toggle_regenerates_git_caches_without_caps() {
    use crate::git::RepositoryState;
    use std::collections::HashMap;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());
    ctx.registry.register(4321, Some(repo.clone()));
    ctx.cache
        .set(&repo, status(0, RepositoryState::Clean), HashMap::new());

    let git_cache_text = || -> String {
        cache_file_names(cache_tmp.path())
            .iter()
            .filter(|f| {
                f.contains(".git.")
                    && Path::new(f)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("ansi"))
            })
            .map(|f| fs::read_to_string(cache_tmp.path().join(f)).expect("read git cache"))
            .collect()
    };
    let has_private_use = |text: &str| text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));

    let mut config = Config::default();
    config.ui.show_icons = true;
    regenerate_instant_caches_for_theme_change(&ctx, &config);
    let with_icons = git_cache_text();
    assert!(
        with_icons.contains("main") && has_private_use(&with_icons),
        "control: show_icons=true cache must carry the caps: {with_icons:?}"
    );

    config.ui.show_icons = false;
    regenerate_instant_caches_for_theme_change(&ctx, &config);
    let without_icons = git_cache_text();
    assert!(
        without_icons.contains("main"),
        "regenerated cache must still carry the branch: {without_icons:?}"
    );
    assert!(
        !has_private_use(&without_icons),
        "show_icons=false cache must not contain private-use glyphs: {without_icons:?}"
    );
}

#[test]
fn language_regen_renders_cached_data() {
    use crate::language::DetectedLanguage;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    ctx.language_cache.set(
        &repo,
        vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 128,
        }],
    );

    let wrote = regen_language_caches(&ctx, &repo, &Config::default());
    assert!(wrote, "first language regen must report a write");

    let files = cache_file_names(cache_tmp.path());
    for dialect in crate::formatter::PromptDialect::ALL {
        let ext = format!(".{}", dialect.cache_ext());
        assert!(
            files
                .iter()
                .any(|f| f.contains(".lang.none.") && f.ends_with(&ext)),
            "context-free {ext} language cache must be regenerated: {files:?}"
        );
        assert!(
            files.iter().any(|f| {
                f.contains(".lang.")
                    && f.ends_with(&ext)
                    && !f.contains(".lang.none.")
                    && !f.contains(".lang_last.none.")
            }),
            "a seeded prev_bg {ext} language cache must exist: {files:?}"
        );
    }
}

/// Cold paths are skipped: never run synchronous detection.
#[test]
fn language_regen_skips_cold_path() {
    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    assert!(
        !regen_language_caches(&ctx, &repo, &Config::default()),
        "a path with no cached language data must report no write"
    );
    assert_eq!(
        cache_file_names(cache_tmp.path()).len(),
        0,
        "no language cache files should be written for a cold path"
    );
}

#[cfg(unix)]
#[test]
fn language_refresh_notifies_only_when_a_cache_write_actually_happens() {
    use crate::language::DetectedLanguage;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let mut ctx = make_hermetic_ctx(cache_tmp.path());
    let rings = count_rings(&mut ctx);
    let config = Config::default();
    let theme = ctx.theme_manager.get();

    ctx.language_cache.set(
        &repo,
        vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 128,
        }],
    );

    let before_first = rung(&rings);
    let wrote_first = refresh_language_for(
        &ctx,
        &repo,
        &config,
        LanguageRefreshMode::CachedOnly,
        &theme,
        None,
    );
    let after_first = rung(&rings);

    assert!(
        wrote_first,
        "first render into an empty cache dir must write"
    );
    assert!(
        after_first > before_first,
        "a genuine cache write must notify (before: {before_first}, after: {after_first})"
    );

    let wrote_second = refresh_language_for(
        &ctx,
        &repo,
        &config,
        LanguageRefreshMode::CachedOnly,
        &theme,
        None,
    );
    let after_second = rung(&rings);

    assert!(
        !wrote_second,
        "a re-render of unchanged content must report no write"
    );
    assert_eq!(
        after_second, after_first,
        "a no-op re-render must not notify (after first: {after_first}, after second: {after_second})"
    );
}

#[test]
fn theme_change_skips_paths_without_warm_data() {
    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    ctx.registry.register(4321, Some(repo));

    regenerate_instant_caches_for_theme_change(&ctx, &Config::default());

    assert_eq!(
        cache_file_names(cache_tmp.path()).len(),
        0,
        "no instant-prompt cache files should be written for a cold path"
    );
}

/// A recovery write after a failed cache write must repaint despite unchanged status.
#[test]
#[cfg(unix)]
fn instant_cache_recovery_write_triggers_repaint_on_unchanged_status() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let mut ctx = make_hermetic_ctx(cache_tmp.path());
    let rings = count_rings(&mut ctx);
    let mut config = Config::default();
    // The language side channel notifies on its own thread; it would race the counts.
    config.language.enabled = false;

    let baseline_notifications = rung(&rings);
    assert_eq!(baseline_notifications, 0, "no refresh has run yet");

    let cache_dir = cache_tmp.path();
    let writable_perms = fs::metadata(cache_dir)
        .expect("cache dir metadata")
        .permissions();
    fs::set_permissions(cache_dir, std::fs::Permissions::from_mode(0o500))
        .expect("make cache dir read-only");

    // The write fails: no notify, since a repaint could only serve a stale file.
    refresh_and_notify_if_changed(&ctx, &repo, &config, None);
    let after_failed_write = rung(&rings);

    fs::set_permissions(cache_dir, writable_perms).expect("restore cache dir permissions");

    // The failed write never updated the memo, so this write reports a change.
    refresh_and_notify_if_changed(&ctx, &repo, &config, None);
    let after_recovery_write = rung(&rings);

    assert_eq!(
        after_failed_write, baseline_notifications,
        "a failed cache write must not trigger a repaint that can only serve stale content"
    );
    assert!(
        after_recovery_write > after_failed_write,
        "the first successful recovery write must trigger a repaint even though \
         repository status did not change (before: {baseline_notifications}, \
         after failed write: {after_failed_write}, after recovery: {after_recovery_write})"
    );
}

/// Oracle per-path flags: (staged, unstaged, untracked, conflicted).
type OracleFlags = (bool, bool, bool, bool);

/// Per-path status parsed from git independently of `git::native::parser`.
fn oracle_files(repo: &Path) -> HashMap<PathBuf, FileStatus> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain=v2", "-z"])
        .current_dir(repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("oracle git status");
    assert!(output.status.success(), "oracle git status failed");
    let text = String::from_utf8(output.stdout).expect("utf-8 status");

    let mut files: HashMap<String, OracleFlags> = HashMap::new();
    let mut records = text.split('\0');
    while let Some(record) = records.next() {
        let (path, flags) = match record.chars().next() {
            Some('1') => {
                let mut fields = record.splitn(9, ' ');
                let xy = fields.nth(1).unwrap_or_default();
                let path = fields.nth(6).unwrap_or_default();
                (path, xy_flags(xy))
            }
            Some('2') => {
                let mut fields = record.splitn(10, ' ');
                let xy = fields.nth(1).unwrap_or_default();
                let path = fields.nth(7).unwrap_or_default();
                records.next();
                (path, xy_flags(xy))
            }
            Some('u') => {
                let path = record.splitn(11, ' ').nth(10).unwrap_or_default();
                (path, (false, false, false, true))
            }
            Some('?') => (
                record.get(2..).unwrap_or_default(),
                (false, false, true, false),
            ),
            _ => continue,
        };
        let entry = files.entry(path.to_owned()).or_default();
        *entry = (
            entry.0 || flags.0,
            entry.1 || flags.1,
            entry.2 || flags.2,
            entry.3 || flags.3,
        );
    }

    files
        .into_iter()
        .map(|(path, (staged, unstaged, untracked, conflicted))| {
            (
                PathBuf::from(path),
                FileStatus {
                    staged,
                    unstaged,
                    untracked,
                    conflicted,
                },
            )
        })
        .collect()
}

fn xy_flags(xy: &str) -> OracleFlags {
    let mut chars = xy.chars();
    let staged = chars.next().is_some_and(|c| c != '.');
    let unstaged = chars.next().is_some_and(|c| c != '.');
    (staged, unstaged, false, false)
}

/// xorshift64: deterministic, so a failing seed replays.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13_u32;
        self.0 ^= self.0 >> 7_u32;
        self.0 ^= self.0 << 17_u32;
        let bound64 = u64::try_from(bound).expect("small bound");
        usize::try_from(self.0.checked_rem(bound64).expect("non-zero bound")).expect("fits usize")
    }
}

/// Names that are easy to mishandle: TAB, leading `:` (pathspec magic), spaces, new dirs.
/// Windows forbids TAB and `:`.
#[cfg(not(windows))]
const ORACLE_NAMES: [&str; 8] = [
    "a.txt",
    "b c.txt",
    "tab\tname.txt",
    ":colon.txt",
    "src/lib.txt",
    "newdir/x.txt",
    "newdir/deep/y.txt",
    "other/z.txt",
];

#[cfg(windows)]
const ORACLE_NAMES: [&str; 6] = [
    "a.txt",
    "b c.txt",
    "src/lib.txt",
    "newdir/x.txt",
    "newdir/deep/y.txt",
    "other/z.txt",
];

/// Applies one random mutation; returns watcher-reported paths and a log line.
/// Reuses the previous file half the time so per-path op chains are common.
fn apply_random_op(rng: &mut Rng, repo: &Path, previous: &mut usize) -> (Vec<PathBuf>, String) {
    if rng.below(2) == 0 {
        *previous = rng.below(ORACLE_NAMES.len());
    }
    let name = ORACLE_NAMES.get(*previous).copied().unwrap_or("a.txt");
    let path = repo.join(name);
    let index = vec![repo.join(".git/index")];
    match rng.below(6) {
        0 | 1 => {
            // A watcher reports each created directory plus the file.
            let mut reported = Vec::new();
            let mut dir = path.parent();
            while let Some(d) = dir {
                if d == repo || d.exists() {
                    break;
                }
                reported.push(d.to_path_buf());
                dir = d.parent();
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            let body = format!("{}\n", rng.below(1000));
            fs::write(&path, body).expect("write file");
            reported.push(path);
            (reported, format!("write {name:?}"))
        }
        2 => {
            let _ = fs::remove_file(&path);
            // git stops reporting empty dirs; delete them like an editor would.
            let mut reported = vec![path.clone()];
            let mut dir = path.parent();
            while let Some(d) = dir {
                if d == repo || fs::remove_dir(d).is_err() {
                    break;
                }
                reported.push(d.to_path_buf());
                dir = d.parent();
            }
            (reported, format!("rm {name:?}"))
        }
        3 => {
            git_in(repo, &["add", "--", &format!(":(literal){name}")]);
            (index, format!("git add {name:?}"))
        }
        4 => {
            git_in(
                repo,
                &["rm", "-q", "--cached", "--", &format!(":(literal){name}")],
            );
            (index, format!("git rm --cached {name:?}"))
        }
        _ => {
            git_in(repo, &["commit", "-q", "-m", "step", "--allow-empty"]);
            (index, "git commit".to_owned())
        }
    }
}

#[test]
/// Incremental refresh must equal an independent full `git status` after each random op.
fn incremental_refresh_matches_full_git_status_oracle() {
    for seed in 1..=24_u64 {
        let (_tmp, repo) = create_temp_repo();
        fs::create_dir_all(repo.join("src")).expect("src dir");
        fs::write(repo.join("a.txt"), "a\n").expect("seed a.txt");
        fs::write(repo.join("src/lib.txt"), "lib\n").expect("seed lib");
        commit_all(&repo);

        let ctx = make_agent_context();
        let config = Config::default();
        let backend = NativeGitBackend;
        refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut log = Vec::new();
        let mut previous = 0_usize;
        for _ in 0_u32..16_u32 {
            let (reported, op) = apply_random_op(&mut rng, &repo, &mut previous);
            log.push(op);
            let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
            let refreshed =
                refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
            let status = refreshed.status.expect("repository status");
            let want_files = oracle_files(&repo);
            let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
            assert_eq!(
                got_files, want_files,
                "seed {seed}: cached file map diverged from git after {log:#?}"
            );
            let got = StatusAggregate {
                staged: status.staged,
                unstaged: status.unstaged,
                untracked: status.untracked,
                conflicts: status.conflicts,
            };
            assert_eq!(
                got,
                StatusAggregate::from_file_statuses(want_files.values()),
                "seed {seed}: status counts diverged from git after {log:#?}"
            );
        }
    }
}

/// Parent repo with a committed submodule `sub`; the first `TempDir` must outlive use.
fn repo_with_submodule() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let (source_tmp, source) = create_temp_repo();
    fs::write(source.join("s.txt"), "s\n").expect("write s.txt");
    commit_all(&source);

    let (parent_tmp, repo) = create_temp_repo();
    fs::write(repo.join("t.txt"), "t\n").expect("write t.txt");
    commit_all(&repo);
    let source_arg = git_url(&source);
    let added = std::process::Command::new("git")
        .args([
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &source_arg,
            "sub",
        ])
        .current_dir(&repo)
        .output()
        .expect("spawn git submodule add");
    assert!(
        added.status.success(),
        "git submodule add failed: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    commit_all(&repo);
    let sub = repo.join("sub");
    assert!(git_in(&sub, &["config", "user.email", "test@example.com"]));
    assert!(git_in(&sub, &["config", "user.name", "Test"]));
    (source_tmp, parent_tmp, repo)
}

fn deliver_path(
    ctx: &AgentContext,
    repo: &Path,
    config: &Config,
    path: PathBuf,
) -> RepositoryStatus {
    let event = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(path),
        },
        repo: repo.to_path_buf(),
    };
    handle_file_event(ctx, &event, config);
    ctx.cache.get(repo).expect("status after event")
}

#[test]
/// Submodule child paths match no superproject pathspec; map them to the gitlink.
fn incremental_refresh_maps_submodule_child_paths_to_gitlink() {
    let (_source, _tmp, repo) = repo_with_submodule();
    let ctx = make_agent_context();
    let config = Config::default();
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);

    let file = repo.join("sub/s.txt");
    fs::write(&file, "s\nmodified\n").expect("modify submodule file");
    let incremental = deliver_path(&ctx, &repo, &config, file);
    assert_eq!(
        incremental.unstaged, 1,
        "the submodule must show as modified"
    );

    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    let full = ctx.cache.get(&repo).expect("status after full scan");
    assert_eq!(incremental, full);
}

#[test]
fn incremental_refresh_clears_gitlink_after_submodule_revert() {
    let (_source, _tmp, repo) = repo_with_submodule();
    let ctx = make_agent_context();
    let config = Config::default();

    let file = repo.join("sub/s.txt");
    fs::write(&file, "s\nmodified\n").expect("modify submodule file");
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(ctx.cache.get(&repo).expect("warm").unstaged, 1);

    assert!(
        git_in(&repo.join("sub"), &["checkout", "--", "s.txt"]),
        "git checkout failed"
    );
    let incremental = deliver_path(&ctx, &repo, &config, file);
    assert_eq!(incremental.state, RepositoryState::Clean);

    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    let full = ctx.cache.get(&repo).expect("status after full scan");
    assert_eq!(incremental, full);
}

#[test]
fn incremental_refresh_maps_nested_repo_child_paths_to_nested_root() {
    let (_tmp, repo) = create_temp_repo();
    fs::write(repo.join("t.txt"), "t\n").expect("write t.txt");
    commit_all(&repo);
    let ctx = make_agent_context();
    let config = Config::default();
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);

    let child = repo.join("deep/child");
    fs::create_dir_all(&child).expect("create nested repo dir");
    assert!(git_in(&child, &["init", "-q"]), "git init failed");
    let file = child.join("f.txt");
    fs::write(&file, "f\n").expect("write nested file");
    let incremental = deliver_path(&ctx, &repo, &config, file);
    assert_eq!(incremental.untracked, 1);

    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    let full = ctx.cache.get(&repo).expect("status after full scan");
    assert_eq!(incremental, full);
}

#[test]
fn incremental_refresh_matches_git_status_for_submodule_edits() {
    for seed in 1..=8_u64 {
        let (_source, _tmp, repo) = repo_with_submodule();
        let sub = repo.join("sub");
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = NativeGitBackend;
        refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut log = Vec::new();
        for _ in 0_u32..10_u32 {
            let (reported, op) = match rng.below(4) {
                0 => {
                    let file = sub.join("s.txt");
                    fs::write(&file, format!("{}\n", rng.below(1000))).expect("write");
                    (vec![file], "edit s.txt")
                }
                1 => {
                    git_in(&sub, &["checkout", "--", "s.txt"]);
                    (vec![sub.join("s.txt")], "revert s.txt")
                }
                2 => {
                    let file = sub.join("extra/new.txt");
                    fs::create_dir_all(sub.join("extra")).expect("mkdir");
                    fs::write(&file, "n\n").expect("write");
                    (vec![sub.join("extra"), file], "add untracked")
                }
                _ => {
                    git_in(&sub, &["commit", "-q", "-a", "-m", "step", "--allow-empty"]);
                    (vec![sub.join("s.txt")], "commit in submodule")
                }
            };
            log.push(op);
            let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
            let refreshed =
                refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
            let status = refreshed.status.expect("repository status");
            let want_files = oracle_files(&repo);
            let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
            assert_eq!(
                got_files, want_files,
                "seed {seed}: cached file map diverged from git after {log:#?}"
            );
            assert_eq!(
                StatusAggregate {
                    staged: status.staged,
                    unstaged: status.unstaged,
                    untracked: status.untracked,
                    conflicts: status.conflicts,
                },
                StatusAggregate::from_file_statuses(want_files.values()),
                "seed {seed}: status counts diverged from git after {log:#?}"
            );
        }
    }
}

/// Whether git set `core.ignorecase` for `repo`.
fn repo_ignores_case(repo: &Path) -> bool {
    let output = std::process::Command::new("git")
        .args(["config", "--type=bool", "--default", "false"])
        .arg("core.ignorecase")
        .current_dir(repo)
        .output()
        .expect("git config");
    String::from_utf8_lossy(&output.stdout).trim() == "true"
}

/// Case-only rename via a temp name so it works on every volume.
fn rename_via_temp(repo: &Path, from: &str, to: &str) {
    let temp = repo.join("case-rename.tmp");
    fs::rename(repo.join(from), &temp).expect("rename to temp");
    fs::rename(&temp, repo.join(to)).expect("rename from temp");
}

#[test]
/// Index spells `Readme.md`, disk `README.md`; the edit and revert must be seen.
fn incremental_refresh_handles_index_case_mismatch() {
    let (_tmp, repo) = create_temp_repo();
    if !repo_ignores_case(&repo) {
        return;
    }
    fs::write(repo.join("Readme.md"), "a\n").expect("write Readme.md");
    commit_all(&repo);
    rename_via_temp(&repo, "Readme.md", "README.md");

    let ctx = make_agent_context();
    let config = Config::default();
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(
        ctx.cache.get(&repo).expect("warm").state,
        RepositoryState::Clean
    );

    let file = repo.join("README.md");
    fs::write(&file, "b\n").expect("modify README.md");
    let edited = deliver_path(&ctx, &repo, &config, file.clone());
    assert_eq!(edited.unstaged, 1, "the edit must be seen incrementally");
    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(edited, ctx.cache.get(&repo).expect("full scan"));

    fs::write(&file, "a\n").expect("restore README.md");
    let reverted = deliver_path(&ctx, &repo, &config, file);
    assert_eq!(
        reverted.state,
        RepositoryState::Clean,
        "the revert must clear the entry cached under the index spelling"
    );
    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(reverted, ctx.cache.get(&repo).expect("full scan"));
}

/// A tracked path whose first component can be respelled in case.
struct CaseSlot {
    index: &'static str,
    variant: &'static str,
    tail: &'static str,
    on_disk: &'static str,
}

impl CaseSlot {
    fn file(&self, repo: &Path) -> PathBuf {
        repo.join(format!("{}{}", self.on_disk, self.tail))
    }
}

fn apply_case_op(rng: &mut Rng, repo: &Path, slots: &mut [CaseSlot]) -> (Vec<PathBuf>, String) {
    let choice = rng.below(slots.len());
    let slot = slots.get_mut(choice).expect("slot in range");
    match rng.below(7) {
        0 | 1 => {
            let file = slot.file(repo);
            fs::write(&file, format!("{}\n", rng.below(1000))).expect("write");
            (vec![file], format!("edit {}", slot.on_disk))
        }
        2 => {
            let file = slot.file(repo);
            fs::write(&file, "orig\n").expect("restore");
            (vec![file], format!("revert {}", slot.on_disk))
        }
        3 | 4 => {
            let old = repo.join(slot.on_disk);
            let target = if slot.on_disk == slot.index {
                slot.variant
            } else {
                slot.index
            };
            rename_via_temp(repo, slot.on_disk, target);
            let op = format!("rename {} -> {target}", slot.on_disk);
            slot.on_disk = target;
            (vec![old, repo.join(target), slot.file(repo)], op)
        }
        5 => {
            git_in(repo, &["add", "-u"]);
            (vec![repo.join(".git/index")], "git add -u".to_owned())
        }
        _ => {
            git_in(repo, &["commit", "-q", "-m", "step", "--allow-empty"]);
            (vec![repo.join(".git/index")], "git commit".to_owned())
        }
    }
}

#[test]
fn incremental_refresh_matches_git_status_for_case_variant_edits() {
    for seed in 1..=12_u64 {
        let (_tmp, repo) = create_temp_repo();
        if !repo_ignores_case(&repo) {
            return;
        }
        let mut slots = [
            ("Readme.md", "README.md", ""),
            ("Notes.txt", "NOTES.TXT", ""),
            ("Src", "SRC", "/lib.txt"),
        ]
        .map(|(index, variant, tail)| CaseSlot {
            index,
            variant,
            tail,
            on_disk: index,
        });
        fs::create_dir_all(repo.join("Src")).expect("src dir");
        for slot in &slots {
            fs::write(slot.file(&repo), "orig\n").expect("seed file");
        }
        commit_all(&repo);

        let ctx = make_agent_context();
        let config = Config::default();
        let backend = NativeGitBackend;
        refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut log = Vec::new();
        for _ in 0_u32..14_u32 {
            let (reported, op) = apply_case_op(&mut rng, &repo, &mut slots);
            log.push(op);
            let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
            let refreshed =
                refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
            let status = refreshed.status.expect("repository status");
            let want_files = oracle_files(&repo);
            let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
            assert_eq!(
                got_files, want_files,
                "seed {seed}: cached file map diverged from git after {log:#?}"
            );
            assert_eq!(
                StatusAggregate {
                    staged: status.staged,
                    unstaged: status.unstaged,
                    untracked: status.untracked,
                    conflicts: status.conflicts,
                },
                StatusAggregate::from_file_statuses(want_files.values()),
                "seed {seed}: status counts diverged from git after {log:#?}"
            );
        }
    }
}

/// A language-marker event hands re-detection and its version probes to a
/// background job: the event thread returns while a 2 s probe runs, and the
/// probed version is published and rung when it lands.
#[cfg(unix)]
#[test]
fn language_event_does_not_wait_on_a_version_probe() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, repo) = create_temp_repo();
    fs::write(repo.join("main.py"), "print('hi')\n").expect("main.py");
    fs::write(repo.join("lib.py"), "x = 1\n").expect("lib.py");
    // A forwarded venv with no version in pyvenv.cfg: only its 2 s
    // interpreter can answer.
    let venv_tmp = tempdir().expect("venv dir");
    let venv = venv_tmp.path().join("activated");
    let python = crate::language::venv::venv_python_binary(&venv);
    fs::create_dir_all(python.parent().expect("bin dir")).expect("bin dir");
    fs::write(&python, "#!/bin/sh\nsleep 2\necho 'Python 3.7.4'\n").expect("python");
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).expect("chmod python");
    crate::language::venv::stash_project_venv(&repo, &venv);

    let cache_tmp = tempdir().expect("cache dir");
    let mut ctx = make_hermetic_ctx(cache_tmp.path());
    let rings = count_rings(&mut ctx);
    let notified_before = rung(&rings);
    let event = DebouncedEvent {
        event: FileEvent::Language {
            path: repo.join("main.py"),
        },
        repo: repo.clone(),
    };

    let started = std::time::Instant::now();
    handle_file_event(&ctx, &event, &Config::default());
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(1000),
        "the event thread waited {elapsed:?} on a 2 s version probe"
    );

    let published = || {
        cache_file_names(cache_tmp.path()).iter().any(|name| {
            name.contains(".lang")
                && fs::read_to_string(cache_tmp.path().join(name))
                    .is_ok_and(|content| content.contains("3.7.4"))
        })
    };
    // The job writes the instant cache first and rings the doorbell after, so
    // a cache that already holds the version says nothing about the ring yet:
    // wait for both, not the ring right after the cache.
    let rung = || rung(&rings) > notified_before;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !(published() && rung()) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        published(),
        "the probed version must reach the instant cache"
    );
    assert!(rung(), "the publish must ring the shells");
}
