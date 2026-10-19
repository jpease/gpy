//! Instant-prompt cache population through the IPC handlers: `GitHandler`
//! writes it on every status request, and `ClientHandler` scans on register.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use gpy_agent::cache::InstantPromptCache;
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::formatter::{Format, PromptDialect};
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::git::{RepositoryState, RepositoryStatus};
use gpy_agent::ipc::Message;
use gpy_agent::ipc::handlers::{GitHandler, RenderDeps, RequestHandler};
use gpy_agent::ipc::registry::ClientDirectory;
use gpy_agent::security::SafePath;
use gpy_agent::theme::ThemeManager;
use serial_test::serial;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

#[path = "common/env_guard.rs"]
mod env_guard;
#[path = "common/git.rs"]
mod git;
#[path = "common/wait.rs"]
mod wait;

use env_guard::EnvGuard;

/// Upper bound for handler side effects to land on disk.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Temp XDG cache/config roots, restored on drop. Field order: env is
/// restored before the directory is removed.
struct UserDirs {
    _env: EnvGuard,
    _dir: TempDir,
}

/// Point `XDG_CACHE_HOME`/`XDG_CONFIG_HOME` at a temp dir for the guard's
/// lifetime. The cache is read back via env-resolved
/// `cache_file_for_dir`, so it must use `new()`, not `new_for_test()`.
fn isolate_user_dirs() -> UserDirs {
    let dir = TempDir::new().expect("create temp dir");
    let mut env = EnvGuard::set_one("XDG_CACHE_HOME", dir.path().join("cache"));
    env.set("XDG_CONFIG_HOME", dir.path().join("config"));
    UserDirs {
        _env: env,
        _dir: dir,
    }
}

/// Create a git repository with one commit.
fn create_git_repo() -> TempDir {
    let temp_dir = TempDir::new().expect("create temp dir");
    let repo_path = temp_dir.path();
    git::git_init(repo_path);
    std::fs::write(repo_path.join("README.md"), "# Test").expect("write file");
    git::git(repo_path, &["add", "."]);
    git::git(repo_path, &["commit", "-q", "-m", "Initial commit"]);
    temp_dir
}

/// Wait until `path` exists with non-empty content.
fn wait_for_content(path: &Path) -> bool {
    wait::wait_until(WRITE_TIMEOUT, POLL_INTERVAL, || {
        std::fs::read_to_string(path).is_ok_and(|content| !content.is_empty())
    })
}

/// Palette cache wired the same way the agent wires it.
fn palette_cache_for(config_manager: &Arc<ConfigManager>) -> Arc<gpy_agent::palette::PaletteCache> {
    Arc::new(gpy_agent::palette::PaletteCache::from_config(
        &config_manager.get(),
    ))
}

/// Shared render handles, using the embedded builtin theme so a stale
/// on-disk theme can't defeat the content assertions.
fn render_deps(
    config_manager: &Arc<ConfigManager>,
    instant_cache: &Arc<InstantPromptCache>,
    client_registry: &Arc<ClientDirectory>,
) -> RenderDeps {
    RenderDeps {
        config_manager: Arc::clone(config_manager),
        theme_manager: Arc::new(ThemeManager::builtin("default").expect("theme")),
        palette_cache: palette_cache_for(config_manager),
        instant_cache: Arc::clone(instant_cache),
        client_registry: Arc::clone(client_registry),
    }
}

/// `GitHandler` writes the instant-prompt cache after an IPC status request.
#[test]
#[serial]
fn test_git_handler_writes_instant_cache_on_ipc_request() {
    let repo = create_git_repo();
    let repo_path = repo.path().to_str().expect("valid utf8 path");

    let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
    let git_cache = Arc::new(GitStatusCache::new());
    let _user_dirs = isolate_user_dirs();
    let instant_cache = Arc::new(InstantPromptCache::new().expect("instant cache"));

    let client_registry = Arc::new(ClientDirectory::new());
    let handler = GitHandler::new(
        render_deps(&config_manager, &instant_cache, &client_registry),
        Arc::clone(&git_cache),
    );

    // Canonicalize: macOS /var -> /private/var.
    let canonical_repo_path = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let cache_file = InstantPromptCache::cache_file_for_dir(
        &canonical_repo_path,
        "git",
        None,
        PromptDialect::Ansi,
    )
    .expect("cache path");

    let _ = std::fs::remove_file(&cache_file);
    assert!(
        !cache_file.exists(),
        "Cache file should not exist before IPC request"
    );

    let message = Message::RepositoryStatus {
        path: SafePath::new(repo_path).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };

    let response = handler.handle(&message);
    assert!(response.is_ok(), "Git status request should succeed");

    assert!(
        wait_for_content(&cache_file),
        "GitHandler did not write instant-prompt cache after IPC request: {}",
        cache_file.display()
    );
}

/// `GitHandler` writes the instant-prompt cache on a git-cache hit too.
#[test]
#[serial]
fn test_git_handler_writes_instant_cache_even_with_stale_git_cache() {
    let repo = create_git_repo();
    let repo_path = repo.path().to_str().expect("valid utf8 path");

    let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
    let git_cache = Arc::new(GitStatusCache::new());
    let _user_dirs = isolate_user_dirs();
    let instant_cache = Arc::new(InstantPromptCache::new().expect("instant cache"));

    // Pre-populate git_cache with stale data
    let stale_status = RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    };
    git_cache.set(repo.path(), stale_status, HashMap::new());

    let client_registry = Arc::new(ClientDirectory::new());
    let handler = GitHandler::new(
        render_deps(&config_manager, &instant_cache, &client_registry),
        Arc::clone(&git_cache),
    );

    // Canonicalize: macOS /var -> /private/var.
    let canonical_repo_path = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let cache_file = InstantPromptCache::cache_file_for_dir(
        &canonical_repo_path,
        "git",
        None,
        PromptDialect::Ansi,
    )
    .expect("cache path");
    let _ = std::fs::remove_file(&cache_file);
    assert!(!cache_file.exists(), "Cache should be deleted");

    let message = Message::RepositoryStatus {
        path: SafePath::new(repo_path).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };

    let response = handler.handle(&message);
    assert!(response.is_ok(), "Git status request should succeed");

    assert!(
        wait::wait_until(WRITE_TIMEOUT, POLL_INTERVAL, || cache_file.exists()),
        "GitHandler did not write instant-prompt cache when answering from cached git status"
    );
}

/// `ClientHandler` triggers an initial scan on registration, populating the
/// cache before any file change.
#[test]
#[serial]
fn test_client_handler_triggers_initial_scan_on_registration() {
    use gpy_agent::ipc::handlers::ClientHandler;
    use gpy_agent::ipc::registry::ClientDirectory;
    use std::sync::Mutex;

    let repo = create_git_repo();
    let repo_path = repo.path().to_str().expect("valid utf8 path");

    let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
    let client_registry = Arc::new(ClientDirectory::new());
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = Arc::new(Mutex::new(None)); // No watcher for this test
    let _user_dirs = isolate_user_dirs();
    let instant_cache = Arc::new(InstantPromptCache::new().expect("instant cache"));

    let handler = ClientHandler::new(
        render_deps(&config_manager, &instant_cache, &client_registry),
        Arc::clone(&git_cache),
        Arc::clone(&watcher),
    );

    // Canonicalize: macOS /var -> /private/var.
    let canonical_repo_path = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let cache_file = InstantPromptCache::cache_file_for_dir(
        &canonical_repo_path,
        "git",
        None,
        PromptDialect::Ansi,
    )
    .expect("cache path");
    let _ = std::fs::remove_file(&cache_file);
    assert!(!cache_file.exists(), "Cache should not exist yet");

    let message = Message::RegisterClient {
        pid: gpy_agent::config::types::ClientPid::new(std::process::id()).unwrap(),
        cwd: Some(SafePath::new(repo_path).unwrap()),
        shell: Some(gpy_agent::config::types::ShellVariant::Fish),
        shell_version: Some("3.0.0".to_owned()),
    };

    let response = handler.handle(&message);
    assert!(response.is_ok(), "Client registration should succeed");

    // The initial scan runs on a background thread.
    assert!(
        wait_for_content(&cache_file),
        "ClientHandler did not trigger an initial scan on registration: {}",
        cache_file.display()
    );
}

/// The cached render follows clean -> dirty status across requests.
#[test]
#[serial]
fn test_instant_cache_updates_on_status_change() {
    let repo = create_git_repo();
    let repo_path = repo.path().to_str().expect("valid utf8 path");

    let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
    let git_cache = Arc::new(GitStatusCache::new());
    let _user_dirs = isolate_user_dirs();
    let instant_cache = Arc::new(InstantPromptCache::new().expect("instant cache"));

    let client_registry = Arc::new(ClientDirectory::new());
    let handler = GitHandler::new(
        render_deps(&config_manager, &instant_cache, &client_registry),
        Arc::clone(&git_cache),
    );

    // Canonicalize: macOS /var -> /private/var.
    let canonical_repo_path = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let cache_file = InstantPromptCache::cache_file_for_dir(
        &canonical_repo_path,
        "git",
        None,
        PromptDialect::Ansi,
    )
    .expect("cache path");
    let _ = std::fs::remove_file(&cache_file);

    let message = Message::RepositoryStatus {
        path: SafePath::new(repo_path).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };
    assert!(handler.handle(&message).is_ok());
    assert!(wait_for_content(&cache_file), "initial cache not written");
    let initial_cache = std::fs::read_to_string(&cache_file).expect("read initial cache");

    std::fs::write(repo.path().join("newfile.txt"), "test").expect("write file");

    // Clear git_cache to force fresh scan
    git_cache.clear();

    assert!(handler.handle(&message).is_ok());

    assert!(
        wait::wait_until(WRITE_TIMEOUT, POLL_INTERVAL, || {
            std::fs::read_to_string(&cache_file)
                .is_ok_and(|content| !content.is_empty() && content != initial_cache)
        }),
        "Cache should be updated when git status changes"
    );
}
