//! In-process IPC server wiring shared by the `EndpointHandle` tests.

#![allow(dead_code)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::cache::InstantPromptCache;
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::ipc::server::{EndpointDeps, EndpointHandle, EndpointHandleBuilder};
use gpy_agent::ipc::{ClientDirectory, LatencyTracker};
use gpy_agent::language::DetectionCache;
use gpy_agent::theme::ThemeManager;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// An `EndpointHandle` builder on `socket_path` with default subsystems
/// (builtin theme, default config, no watcher).
#[must_use]
pub fn endpoint_builder(socket_path: PathBuf) -> EndpointHandleBuilder {
    endpoint_builder_with_config(
        socket_path,
        Arc::new(ConfigManager::with_defaults().expect("default config should load")),
    )
}

/// Like [`endpoint_builder`], but serving the caller's `config_manager`.
#[must_use]
pub fn endpoint_builder_with_config(
    socket_path: PathBuf,
    config_manager: Arc<ConfigManager>,
) -> EndpointHandleBuilder {
    EndpointHandle::builder(EndpointDeps {
        client_registry: ClientDirectory::new().shared(),
        git_cache: Arc::new(GitStatusCache::new()),
        config_manager,
        theme_manager: Arc::new(
            ThemeManager::builtin("default").expect("default theme should load"),
        ),
        instant_cache: Arc::new(InstantPromptCache::new_for_test()),
        latency_tracker: Arc::new(LatencyTracker::new(100)),
        language_cache: DetectionCache::new(),
    })
    .socket_path(socket_path)
    .watcher_slot(Arc::new(Mutex::new(None)))
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
