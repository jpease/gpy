//! Regression tests for instant-prompt cache invalidation bugs
//!
//! These tests ensure three specific bugs don't resurface:
//! 1. GitHandler writes instant-prompt cache after IPC requests (not just watcher)
//! 2. ClientHandler triggers initial scan on registration
//! 3. Cache age validation (tested in Fish shell separately)

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use gpy_agent::cache::InstantPromptCache;
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::formatter::Format;
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::git::{RepositoryState, RepositoryStatus};
use gpy_agent::ipc::Message;
use gpy_agent::ipc::handlers::{GitHandler, RenderDeps, RequestHandler};
use gpy_agent::ipc::registry::ClientDirectory;
use gpy_agent::security::SafePath;
use gpy_agent::theme::ThemeManager;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create a test git repository
fn create_git_repo() -> TempDir {
    let temp_dir = TempDir::new().expect("create temp dir");
    let repo_path = temp_dir.path();

    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .status()
            .expect("git command should execute")
    };

    assert!(run(&["init"]).success());
    assert!(run(&["config", "user.email", "test@example.com"]).success());
    assert!(run(&["config", "user.name", "Test User"]).success());
    assert!(run(&["config", "commit.gpgsign", "false"]).success());

    // Create initial commit so we have a valid repository
    std::fs::write(repo_path.join("README.md"), "# Test").expect("write file");
    assert!(run(&["add", "."]).success());
    assert!(run(&["commit", "-m", "Initial commit"]).success());

    temp_dir
}

/// Build a palette cache from the manager's active config, matching the wiring
/// the agent uses so instant-cache renders resolve the same active palette.
fn palette_cache_for(config_manager: &Arc<ConfigManager>) -> Arc<gpy_agent::palette::PaletteCache> {
    Arc::new(gpy_agent::palette::PaletteCache::from_config(
        &config_manager.get(),
    ))
}

/// Bundle the handles every publish-and-repaint site shares (#587).
///
/// Uses the embedded builtin theme so the tests stay hermetic: a stale on-disk
/// ~/.config/gpy/themes/default.toml (predating the format+palette migration)
/// would render empty segments and defeat the content-change assertions.
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

/// Regression test for Fix #2: GitHandler must write instant-prompt cache after IPC requests
///
/// Bug: Originally, instant-prompt cache was only written by watcher events.
/// If a client made an IPC request for git status, the cache would not be updated.
/// This led to stale cache being read on subsequent prompts.
///
/// Fix: GitHandler now writes instant-prompt cache in all 4 success paths:
/// - Cache hit (using stale git cache)
/// - Fresh status (within 100ms timeout)
/// - Stale status (after 100ms timeout, using stale git cache)
/// - Delayed status (after 100ms timeout, no stale cache available)
#[test]
fn test_git_handler_writes_instant_cache_on_ipc_request() {
    let repo = create_git_repo();
    let repo_path = repo.path().to_str().expect("valid utf8 path");

    // Create dependencies
    let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
    let git_cache = Arc::new(GitStatusCache::new());
    let instant_cache = Arc::new(InstantPromptCache::new().expect("instant cache"));

    // Create GitHandler
    let client_registry = Arc::new(ClientDirectory::new());
    let handler = GitHandler::new(
        render_deps(&config_manager, &instant_cache, &client_registry),
        Arc::clone(&git_cache),
    );

    // Before IPC request, there should be no instant-prompt cache file
    // Use canonicalized path to handle macOS /var -> /private/var symlink
    let canonical_repo_path = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let cache_file = InstantPromptCache::cache_file_for_dir(&canonical_repo_path, "git", None)
        .expect("cache path");

    // Clean up any existing cache file
    let _ = std::fs::remove_file(&cache_file);
    assert!(
        !cache_file.exists(),
        "Cache file should not exist before IPC request"
    );

    // Make IPC request for git status
    let message = Message::RepositoryStatus {
        path: SafePath::new(repo_path).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };

    let response = handler.handle(&message);
    eprintln!("Handler response: {response:?}");
    assert!(response.is_ok(), "Git status request should succeed");

    // Give a moment for async operations to complete
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Check what files exist in cache directory
    if let Some(parent) = cache_file.parent() {
        eprintln!("Cache directory contents:");
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                let entry_display = entry_path.display();
                eprintln!("  - {entry_display}");
            }
        }
    }

    // After IPC request, instant-prompt cache file MUST exist
    // This is the regression test - if GitHandler doesn't write the cache,
    // this assertion will fail and the bug has resurfaced
    assert!(
        cache_file.exists(),
        "REGRESSION: GitHandler did not write instant-prompt cache after IPC request!\n\
         This was Fix #2 - GitHandler must write cache, not just watcher.\n\
         Expected cache file: {}",
        cache_file.display()
    );

    // Verify cache content is valid (not empty or corrupted)
    let cache_content = std::fs::read_to_string(&cache_file).expect("read cache file");
    assert!(!cache_content.is_empty(), "Cache file should have content");
}

/// Regression test for Fix #2: GitHandler writes cache even when using cached git status
///
/// Bug: Even if git_cache has stale data, instant-prompt cache should be updated.
/// This test specifically covers the "cache hit" path in GitHandler.
#[test]
fn test_git_handler_writes_instant_cache_even_with_stale_git_cache() {
    let repo = create_git_repo();
    let repo_path = repo.path().to_str().expect("valid utf8 path");

    // Create dependencies
    let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
    let git_cache = Arc::new(GitStatusCache::new());
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

    // Create GitHandler
    let client_registry = Arc::new(ClientDirectory::new());
    let handler = GitHandler::new(
        render_deps(&config_manager, &instant_cache, &client_registry),
        Arc::clone(&git_cache),
    );

    // Delete instant-prompt cache if it exists
    // Use canonicalized path to handle macOS /var -> /private/var symlink
    let canonical_repo_path = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let cache_file = InstantPromptCache::cache_file_for_dir(&canonical_repo_path, "git", None)
        .expect("cache path");
    let _ = std::fs::remove_file(&cache_file);
    assert!(!cache_file.exists(), "Cache should be deleted");

    // Make IPC request - will hit git_cache (stale data)
    let message = Message::RepositoryStatus {
        path: SafePath::new(repo_path).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };

    let response = handler.handle(&message);
    assert!(response.is_ok(), "Git status request should succeed");

    // Give a moment for async operations
    std::thread::sleep(std::time::Duration::from_millis(50));

    // REGRESSION TEST: Even though GitHandler used cached (stale) git status,
    // it MUST still write the instant-prompt cache
    assert!(
        cache_file.exists(),
        "REGRESSION: GitHandler did not write instant-prompt cache when using cached git status!\n\
         This is the 'cache hit' path in Fix #2."
    );
}

/// Regression test for Fix #3: ClientHandler triggers initial scan on registration
///
/// Bug: When a new client registered, watcher would start watching but wouldn't
/// trigger an initial git scan. This meant the instant-prompt cache wouldn't exist
/// until the first file change, causing stale prompts in new shells.
///
/// Fix: ClientHandler now calls trigger_initial_scan() after registering with watcher,
/// which spawns a background thread to scan git status and write instant-prompt cache.
#[test]
fn test_client_handler_triggers_initial_scan_on_registration() {
    use gpy_agent::ipc::handlers::ClientHandler;
    use gpy_agent::ipc::{LatencyTracker, registry::ClientDirectory};
    use std::sync::Mutex;

    let repo = create_git_repo();
    let repo_path = repo.path().to_str().expect("valid utf8 path");

    // Create dependencies
    let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
    let client_registry = Arc::new(ClientDirectory::new());
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = Arc::new(Mutex::new(None)); // No watcher for this test
    let instant_cache = Arc::new(InstantPromptCache::new().expect("instant cache"));
    let latency_tracker = Arc::new(LatencyTracker::new(100));

    // Create ClientHandler
    let handler = ClientHandler::new(
        render_deps(&config_manager, &instant_cache, &client_registry),
        Arc::clone(&git_cache),
        Arc::clone(&watcher),
        Arc::clone(&latency_tracker),
    );

    // Before registration, cache should not exist
    // Use canonicalized path to handle macOS /var -> /private/var symlink
    let canonical_repo_path = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let cache_file = InstantPromptCache::cache_file_for_dir(&canonical_repo_path, "git", None)
        .expect("cache path");
    let _ = std::fs::remove_file(&cache_file);
    assert!(!cache_file.exists(), "Cache should not exist yet");

    // Register client with cwd pointing to git repo
    let message = Message::RegisterClient {
        pid: gpy_agent::config::types::ClientPid::new(std::process::id()).unwrap(),
        cwd: Some(SafePath::new(repo_path).unwrap()),
        shell: Some(gpy_agent::config::types::ShellVariant::Fish),
        shell_version: Some("3.0.0".to_owned()),
    };

    let response = handler.handle(&message);
    assert!(response.is_ok(), "Client registration should succeed");

    // Give background thread time to complete initial scan
    // trigger_initial_scan() spawns a background thread, so we need to wait
    std::thread::sleep(std::time::Duration::from_millis(500));

    // REGRESSION TEST: After registration, instant-prompt cache MUST exist
    // This ensures Fix #3 is working - initial scan was triggered
    assert!(
        cache_file.exists(),
        "REGRESSION: ClientHandler did not trigger initial scan on registration!\n\
         This was Fix #3 - trigger_initial_scan() should write cache immediately."
    );

    // Verify cache content is valid
    let cache_content = std::fs::read_to_string(&cache_file).expect("read cache file");
    assert!(
        !cache_content.is_empty(),
        "Cache file should have content from initial scan"
    );
}

/// Regression test: Verify instant-prompt cache is updated when git status changes
///
/// This is an integration test that verifies the full pipeline:
/// 1. Initial IPC request creates cache
/// 2. Git status changes (new file)
/// 3. Subsequent IPC request updates cache with new status
#[test]
fn test_instant_cache_updates_on_status_change() {
    let repo = create_git_repo();
    let repo_path = repo.path().to_str().expect("valid utf8 path");

    // Create dependencies
    let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
    let git_cache = Arc::new(GitStatusCache::new());
    let instant_cache = Arc::new(InstantPromptCache::new().expect("instant cache"));

    let client_registry = Arc::new(ClientDirectory::new());
    let handler = GitHandler::new(
        render_deps(&config_manager, &instant_cache, &client_registry),
        Arc::clone(&git_cache),
    );

    // Use canonicalized path to handle macOS /var -> /private/var symlink
    let canonical_repo_path = repo
        .path()
        .canonicalize()
        .unwrap_or_else(|_| repo.path().to_path_buf());
    let cache_file = InstantPromptCache::cache_file_for_dir(&canonical_repo_path, "git", None)
        .expect("cache path");
    let _ = std::fs::remove_file(&cache_file);

    // First request - clean repo
    let message = Message::RepositoryStatus {
        path: SafePath::new(repo_path).unwrap(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    };
    assert!(handler.handle(&message).is_ok());
    std::thread::sleep(std::time::Duration::from_millis(100));

    let initial_cache = std::fs::read_to_string(&cache_file).expect("read initial cache");

    // Modify repo - add untracked file
    std::fs::write(repo.path().join("newfile.txt"), "test").expect("write file");

    // Clear git_cache to force fresh scan
    git_cache.clear();

    // Second request - dirty repo
    assert!(handler.handle(&message).is_ok());
    std::thread::sleep(std::time::Duration::from_millis(100));

    let updated_cache = std::fs::read_to_string(&cache_file).expect("read updated cache");

    // Cache should be different (updated with new status)
    assert_ne!(
        initial_cache, updated_cache,
        "Cache should be updated when git status changes"
    );
}
