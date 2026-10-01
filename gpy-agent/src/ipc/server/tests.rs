//! Unit tests for IPC server construction and guard behavior.
//!
//! These tests live beside the server internals so they can exercise private
//! builder and handle details without exposing test-only APIs from the crate.

use super::EndpointHandle;
use super::handle::ServerDeps;
use crate::config::Config;
use crate::config::manager::ConfigManager;
use crate::formatter::Format;
use crate::ipc::{ClientDirectory, Message, Response};
use crate::security::GuardSettings;
use crate::theme::ThemeManager;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

/// # Panics
///
/// Panics if a temporary repository cannot be created or initialized.
fn create_git_repo() -> TempDir {
    let dir = TempDir::new().expect("create temp git repo");
    std::process::Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output()
        .expect("git init");
    dir
}

/// # Panics
///
/// Panics if a temporary project directory cannot be created.
fn create_language_project() -> TempDir {
    let dir = TempDir::new().expect("create temp project");
    std::fs::write(dir.path().join("package.json"), "{\"name\":\"demo\"}")
        .expect("write package.json");
    dir
}

// `notify_clients` rings the repaint doorbell for live-update subscribers (#540).
#[cfg(unix)]
/// # Panics
///
/// Panics if default configuration or theme initialization fails during test setup.
#[test]
fn notify_clients_respects_agent_live_updates_flag() {
    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    let config_manager =
        Arc::new(crate::config::manager::ConfigManager::with_defaults().expect("config"));
    let watcher = Arc::new(Mutex::new(None));
    let theme_manager =
        Arc::new(ThemeManager::new("default").expect("default theme should always load"));
    let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100));
    let language_cache = crate::language::DetectionCache::new();

    let initial_config = config_manager.get();
    let palette_cache = Arc::new(crate::palette::PaletteCache::from_config(&initial_config));
    let handle = EndpointHandle::with_path_and_state(
        PathBuf::from("/tmp/test.sock"),
        ServerDeps {
            client_registry: Arc::clone(&registry),
            git_cache: Arc::clone(&git_cache),
            config_manager: Arc::clone(&config_manager),
            theme_manager: Arc::clone(&theme_manager),
            instant_cache,
            latency_tracker: Arc::clone(&latency_tracker),
            language_cache,
            palette_cache,
        },
        Arc::clone(&watcher),
        GuardSettings::default(),
    );

    let mut disabled_config = Config::default();
    disabled_config.agent.live_updates = false;
    config_manager.overwrite_for_tests(disabled_config);

    let response = Response::RepositoryStatus(crate::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: crate::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    });

    handle.notify_clients(&registry, &response, None);
    assert_eq!(
        registry.notify_invocations(),
        0,
        "live updates disabled should suppress notifications"
    );

    let mut enabled_config = Config::default();
    enabled_config.agent.live_updates = true;
    config_manager.overwrite_for_tests(enabled_config);

    handle.notify_clients(&registry, &response, None);
    assert_eq!(
        registry.notify_invocations(),
        1,
        "live updates enabled should allow notifications"
    );
}

/// # Panics
///
/// Panics if the test configuration or temporary git repository cannot be initialized.
#[test]
fn hot_reload_git_enabled_blocks_requests() {
    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    let config_manager =
        Arc::new(crate::config::manager::ConfigManager::with_defaults().expect("config"));
    let watcher = Arc::new(Mutex::new(None));
    let theme_manager =
        Arc::new(ThemeManager::new("default").expect("default theme should always load"));
    let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100));
    let language_cache = crate::language::DetectionCache::new();

    let mut cfg = Config::default();
    cfg.git.enabled = true;
    config_manager.overwrite_for_tests(cfg.clone());

    let initial_config_for_git = config_manager.get();
    let palette_cache_for_git = Arc::new(crate::palette::PaletteCache::from_config(
        &initial_config_for_git,
    ));
    let handle = EndpointHandle::with_path_and_state(
        PathBuf::from("/tmp/hot-reload-git.sock"),
        ServerDeps {
            client_registry: Arc::clone(&registry),
            git_cache: Arc::clone(&git_cache),
            config_manager: Arc::clone(&config_manager),
            theme_manager: Arc::clone(&theme_manager),
            instant_cache,
            latency_tracker: Arc::clone(&latency_tracker),
            language_cache,
            palette_cache: palette_cache_for_git,
        },
        Arc::clone(&watcher),
        GuardSettings::default(),
    );

    let repo = create_git_repo();
    let path_str = repo
        .path()
        .to_str()
        .expect("test repository path should be valid UTF-8");
    let path = crate::security::SafePath::new(path_str).expect("Valid test path");

    let response = handle
        .handler_registry
        .route(&git_status_message(path.clone()));
    assert!(matches!(response, Ok(Response::RepositoryStatus { .. })));

    let mut disabled = cfg.clone();
    disabled.git.enabled = false;
    config_manager.overwrite_for_tests(disabled.clone());
    config_manager.trigger_reload_for_tests(&cfg, &disabled);

    assert!(matches!(
        handle.handler_registry.route(&git_status_message(path)),
        Err(ref err) if err.to_string().contains("Git segment disabled")
    ));
}

/// A minimal, non-last/non-first `RepositoryStatus` request, for tests that
/// only care about routing behavior (enabled/disabled), not chain position.
fn git_status_message(path: crate::security::SafePath) -> Message {
    Message::RepositoryStatus {
        path,
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
    }
}

#[allow(clippy::missing_panics_doc)]
fn build_test_endpoint(
    sock_name: &str,
) -> (
    Arc<ConfigManager>,
    EndpointHandle,
    crate::security::SafePath,
    TempDir,
) {
    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    let config_manager =
        Arc::new(crate::config::manager::ConfigManager::with_defaults().expect("config"));
    let watcher = Arc::new(Mutex::new(None));
    let theme_manager =
        Arc::new(ThemeManager::new("default").expect("default theme should always load"));
    let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100));
    let language_cache = crate::language::DetectionCache::new();
    let mut cfg = Config::default();
    cfg.language.enabled = true;
    config_manager.overwrite_for_tests(cfg);
    let palette_cache = Arc::new(crate::palette::PaletteCache::from_config(
        &config_manager.get(),
    ));
    let handle = EndpointHandle::with_path_and_state(
        PathBuf::from(sock_name),
        ServerDeps {
            client_registry: Arc::clone(&registry),
            git_cache: Arc::clone(&git_cache),
            config_manager: Arc::clone(&config_manager),
            theme_manager: Arc::clone(&theme_manager),
            instant_cache,
            latency_tracker: Arc::clone(&latency_tracker),
            language_cache,
            palette_cache,
        },
        Arc::clone(&watcher),
        GuardSettings::default(),
    );
    let project = create_language_project();
    let path_str = project.path().to_str().expect("valid UTF-8");
    let path = crate::security::SafePath::new(path_str).expect("Valid test path");
    (config_manager, handle, path, project)
}

/// # Panics
///
/// Panics if the test configuration or temporary language project cannot be created.
#[test]
fn hot_reload_language_enabled_updates_responses() {
    let (config_manager, handle, path, _project) = build_test_endpoint("/tmp/hot-reload-lang.sock");
    let mut cfg = Config::default();
    cfg.language.enabled = true;

    let lang_msg = Message::LanguageDetect {
        path: path.clone(),
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
        virtual_env: None,
    };
    assert!(matches!(
        handle.handler_registry.route(&lang_msg),
        Ok(Response::Language { languages }) if !languages.is_empty()
    ));

    let mut disabled = cfg.clone();
    disabled.language.enabled = false;
    config_manager.overwrite_for_tests(disabled.clone());
    config_manager.trigger_reload_for_tests(&cfg, &disabled);

    let message = Message::LanguageDetect {
        path,
        format: Format::Json,
        is_last: false,
        is_first: false,
        prev_bg: None,
        virtual_env: None,
    };
    assert!(matches!(
        handle.handler_registry.route(&message),
        Ok(Response::Language { languages }) if languages.is_empty()
    ));
}

#[test]
#[allow(clippy::missing_panics_doc)]
fn test_endpoint_handle_builder_success() {
    // Test successful builder construction with all required fields
    let registry = Arc::new(ClientDirectory::new());
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    let config_manager = Arc::new(
        crate::config::manager::ConfigManager::with_defaults().expect("default config should load"),
    );
    let theme_manager = Arc::new(ThemeManager::new("default").expect("default theme should load"));
    let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100));
    let language_cache = crate::language::DetectionCache::new();

    let result = EndpointHandle::builder()
        .client_registry(registry)
        .git_cache(git_cache)
        .config_manager(config_manager)
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(language_cache)
        .build();

    assert!(
        result.is_ok(),
        "Builder should succeed with all required fields"
    );
}

#[test]
#[allow(clippy::missing_panics_doc)]
fn test_endpoint_handle_builder_with_socket_path() {
    // Test builder with custom socket path
    let registry = Arc::new(ClientDirectory::new());
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    let config_manager = Arc::new(
        crate::config::manager::ConfigManager::with_defaults().expect("default config should load"),
    );
    let theme_manager = Arc::new(ThemeManager::new("default").expect("default theme should load"));
    let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100));
    let language_cache = crate::language::DetectionCache::new();

    let custom_path = PathBuf::from("/tmp/custom-test.sock");
    let result = EndpointHandle::builder()
        .socket_path(custom_path.clone())
        .client_registry(registry)
        .git_cache(git_cache)
        .config_manager(config_manager)
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(language_cache)
        .build();

    assert!(result.is_ok(), "Builder should accept custom socket path");
    let handle = result.unwrap();
    assert_eq!(
        handle.socket_path, custom_path,
        "Builder should use custom socket path"
    );
}

#[test]
#[allow(clippy::missing_panics_doc)]
fn test_endpoint_handle_builder_with_watcher_slot() {
    // Test builder with watcher slot (simulating agent passing shared slot)
    let registry = Arc::new(ClientDirectory::new());
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    let config_manager = Arc::new(
        crate::config::manager::ConfigManager::with_defaults().expect("default config should load"),
    );
    let theme_manager = Arc::new(ThemeManager::new("default").expect("default theme should load"));
    let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100));
    let language_cache = crate::language::DetectionCache::new();

    // Create a shared watcher slot (as the agent would)
    let watcher_slot = Arc::new(Mutex::new(None));

    let result = EndpointHandle::builder()
        .client_registry(Arc::clone(&registry))
        .git_cache(Arc::clone(&git_cache))
        .config_manager(Arc::clone(&config_manager))
        .theme_manager(Arc::clone(&theme_manager))
        .instant_cache(Arc::clone(&instant_cache))
        .latency_tracker(Arc::clone(&latency_tracker))
        .language_cache(language_cache)
        .watcher_slot(Arc::clone(&watcher_slot))
        .build();

    assert!(result.is_ok(), "Builder should accept shared watcher slot");

    // Verify the server got the same slot (this is critical for live updates)
    let handle = result.unwrap();
    assert!(
        Arc::ptr_eq(&handle.watcher, &watcher_slot),
        "Builder must use the exact shared watcher slot, not create a new one"
    );
}

#[test]
#[allow(clippy::missing_panics_doc)]
fn test_endpoint_handle_builder_missing_required_field() {
    // Test that builder fails when required field is missing
    let registry = Arc::new(ClientDirectory::new());
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    // Intentionally omit config_manager

    let result = EndpointHandle::builder()
        .client_registry(registry)
        .git_cache(git_cache)
        // Missing: config_manager
        // Missing: theme_manager
        // Missing: latency_tracker
        .build();

    assert!(
        result.is_err(),
        "Builder should fail with missing required fields"
    );
    if let Err(e) = result {
        let err_msg = e.to_string();
        assert!(
            err_msg.contains("missing required field"),
            "Error should mention missing required field: {err_msg}"
        );
    }
}

#[test]
#[allow(clippy::missing_panics_doc)]
fn test_endpoint_handle_builder_fluent_api() {
    // Test fluent API chaining
    let result = EndpointHandle::builder()
        .client_registry(Arc::new(ClientDirectory::new()))
        .git_cache(Arc::new(crate::git::cache::GitStatusCache::new()))
        .config_manager(Arc::new(
            crate::config::manager::ConfigManager::with_defaults()
                .expect("default config should load"),
        ))
        .theme_manager(Arc::new(
            ThemeManager::new("default").expect("default theme should load"),
        ))
        .instant_cache(Arc::new(crate::cache::InstantPromptCache::new_for_test()))
        .latency_tracker(Arc::new(crate::ipc::LatencyTracker::new(100)))
        .language_cache(crate::language::DetectionCache::new())
        .build();

    assert!(result.is_ok(), "Fluent API should work correctly");
}
