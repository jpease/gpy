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

mod cache_regen;
mod config_reload;
#[cfg(unix)]
mod doorbell;
mod git_hints;
mod git_routing;
mod incremental_oracle;

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

/// Counts full scans and records each pathspec scan's paths.
struct CountingGitBackend {
    full_scans: Mutex<usize>,
    pathspec_scans: Mutex<Vec<Vec<PathBuf>>>,
}

impl CountingGitBackend {
    fn new() -> Self {
        Self {
            full_scans: Mutex::new(0),
            pathspec_scans: Mutex::new(Vec::new()),
        }
    }

    fn full_count(&self) -> usize {
        *self.full_scans.lock().expect("full scans lock")
    }

    fn pathspec_count(&self) -> usize {
        self.pathspec_scans
            .lock()
            .expect("pathspec scans lock")
            .len()
    }

    fn pathspec_paths(&self, index: usize) -> Option<Vec<PathBuf>> {
        self.pathspec_scans
            .lock()
            .expect("pathspec scans lock")
            .get(index)
            .cloned()
    }
}

impl GitBackend for CountingGitBackend {
    fn complete_status(
        &self,
        path: &Path,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>> {
        {
            let mut scans = self.full_scans.lock().expect("full scans lock");
            *scans = scans.saturating_add(1);
        }
        NativeGitBackend.complete_status(path, max_ahead_behind, stash_enabled, timeout)
    }

    fn pathspec_status(
        &self,
        repo_root: &Path,
        paths: &[PathBuf],
        timeout: Duration,
    ) -> Result<crate::git::PathspecScan> {
        self.pathspec_scans
            .lock()
            .expect("pathspec scans lock")
            .push(paths.to_vec());
        NativeGitBackend.pathspec_status(repo_root, paths, timeout)
    }
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
