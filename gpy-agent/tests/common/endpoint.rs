//! In-process IPC server wiring shared by the `EndpointHandle` tests.

#![allow(clippy::expect_used)]

use gpy_agent::cache::InstantPromptCache;
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::ipc::ClientDirectory;
#[cfg(unix)]
use gpy_agent::ipc::server::DirRepair;
use gpy_agent::ipc::server::{EndpointDeps, EndpointHandle};
use gpy_agent::language::DetectionCache;
use gpy_agent::palette::PaletteCache;
use gpy_agent::security::GuardSettings;
use gpy_agent::theme::ThemeManager;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// An `EndpointHandle` on `socket_path` with default subsystems (builtin
/// theme, default config, no watcher).
#[must_use]
pub fn endpoint(socket_path: PathBuf) -> EndpointHandle {
    endpoint_with_config(
        socket_path,
        Arc::new(ConfigManager::with_defaults().expect("default config should load")),
    )
}

/// Like [`endpoint`], but serving the caller's `config_manager`.
#[must_use]
pub fn endpoint_with_config(
    socket_path: PathBuf,
    config_manager: Arc<ConfigManager>,
) -> EndpointHandle {
    EndpointHandle::new(endpoint_deps_with_config(socket_path, config_manager))
}

/// The dependencies behind [`endpoint_with_config`], for tests that override
/// one of them (for example the guard settings).
#[must_use]
pub fn endpoint_deps_with_config(
    socket_path: PathBuf,
    config_manager: Arc<ConfigManager>,
) -> EndpointDeps {
    let palette_cache = Arc::new(PaletteCache::from_config(&config_manager.get()));
    EndpointDeps {
        socket_path,
        #[cfg(unix)]
        socket_dir_repair: DirRepair::Refuse,
        security_config: GuardSettings::default(),
        watcher_slot: Arc::new(Mutex::new(None)),
        client_registry: ClientDirectory::new().shared(),
        git_cache: Arc::new(GitStatusCache::new()),
        config_manager,
        theme_manager: Arc::new(
            ThemeManager::builtin("default").expect("default theme should load"),
        ),
        instant_cache: Arc::new(InstantPromptCache::new_for_test()),
        language_cache: DetectionCache::new(),
        palette_cache,
    }
}

/// Wait for `socket_path` to exist or panic after `timeout`. Sleeps on the
/// tokio timer so a same-runtime server task can progress.
pub async fn wait_for_socket(socket_path: &Path, timeout: Duration) {
    let deadline = tokio::time::Instant::now()
        .checked_add(timeout)
        .expect("timeout too large");
    while !socket_path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "socket {} was not created within {timeout:?}",
            socket_path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
