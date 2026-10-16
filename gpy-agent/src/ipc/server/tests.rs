//! Unit tests for IPC server construction and guard behavior.

use super::handle::ServerDeps;
use super::{EndpointDeps, EndpointHandle, EndpointHandleBuilder};
use crate::config::Config;
use crate::config::manager::ConfigManager;
use crate::formatter::Format;
use crate::ipc::{ClientDirectory, Message, Response};
use crate::security::GuardSettings;
use crate::theme::ThemeManager;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn create_git_repo() -> TempDir {
    let dir = TempDir::new().expect("create temp git repo");
    std::process::Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output()
        .expect("git init");
    dir
}

fn create_language_project() -> TempDir {
    let dir = TempDir::new().expect("create temp project");
    std::fs::write(dir.path().join("package.json"), "{\"name\":\"demo\"}")
        .expect("write package.json");
    dir
}

#[cfg(unix)]
#[test]
fn notify_clients_respects_agent_live_updates_flag() {
    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    let config_manager =
        Arc::new(crate::config::manager::ConfigManager::with_defaults().expect("config"));
    let watcher = Arc::new(Mutex::new(None));
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should always load"));
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

#[test]
fn hot_reload_git_enabled_blocks_requests() {
    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
    let config_manager =
        Arc::new(crate::config::manager::ConfigManager::with_defaults().expect("config"));
    let watcher = Arc::new(Mutex::new(None));
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should always load"));
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
    let Ok(Response::RepositoryStatus(status)) = &response else {
        panic!("expected a RepositoryStatus for an enabled git segment, got {response:?}");
    };
    // A freshly `git init`ed repo has nothing pending.
    assert_eq!(status.state, crate::git::RepositoryState::Clean);

    let mut disabled = cfg.clone();
    disabled.git.enabled = false;
    config_manager.overwrite_for_tests(disabled.clone());
    config_manager.trigger_reload_for_tests(&cfg, &disabled);

    assert!(matches!(
        handle.handler_registry.route(&git_status_message(path)),
        Err(ref err) if err.to_string().contains("Git segment disabled")
    ));
}

/// A minimal mid-chain `RepositoryStatus` request.
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
        Arc::new(ThemeManager::builtin("default").expect("default theme should always load"));
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

/// A builder over fresh default collaborators with no optional setter applied.
#[allow(clippy::missing_panics_doc)]
fn default_builder() -> EndpointHandleBuilder {
    EndpointHandle::builder(EndpointDeps {
        client_registry: Arc::new(ClientDirectory::new()),
        git_cache: Arc::new(crate::git::cache::GitStatusCache::new()),
        config_manager: Arc::new(
            crate::config::manager::ConfigManager::with_defaults()
                .expect("default config should load"),
        ),
        theme_manager: Arc::new(
            ThemeManager::builtin("default").expect("default theme should load"),
        ),
        instant_cache: Arc::new(crate::cache::InstantPromptCache::new_for_test()),
        latency_tracker: Arc::new(crate::ipc::LatencyTracker::new(100)),
        language_cache: crate::language::DetectionCache::new(),
    })
}

#[test]
#[allow(clippy::missing_panics_doc)]
fn test_endpoint_handle_builder_optional_defaults() {
    let handle = default_builder()
        .build()
        .expect("builder should succeed with only the required collaborators");

    assert_eq!(
        handle.socket_path,
        EndpointHandle::default_socket_path().expect("default socket path"),
        "Socket path should default to the platform path"
    );
    assert!(
        handle
            .watcher
            .lock()
            .expect("watcher lock poisoned")
            .is_none(),
        "Watcher slot should default to an empty slot"
    );
}

#[test]
#[allow(clippy::missing_panics_doc)]
fn test_endpoint_handle_builder_with_socket_path() {
    let custom_path = PathBuf::from("/tmp/custom-test.sock");
    let handle = default_builder()
        .socket_path(custom_path.clone())
        .build()
        .expect("builder should accept a custom socket path");

    assert_eq!(
        handle.socket_path, custom_path,
        "Builder should use custom socket path"
    );
}

#[test]
#[allow(clippy::missing_panics_doc)]
fn test_endpoint_handle_builder_with_watcher_slot() {
    let watcher_slot = Arc::new(Mutex::new(None));

    let handle = default_builder()
        .watcher_slot(Arc::clone(&watcher_slot))
        .build()
        .expect("builder should accept a shared watcher slot");

    // Must be the same slot, or live updates break.
    assert!(
        Arc::ptr_eq(&handle.watcher, &watcher_slot),
        "Builder must use the exact shared watcher slot, not create a new one"
    );
}
