//! In-process IPC server wiring shared by the `EndpointHandle` tests.

#![allow(dead_code)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::cache::InstantPromptCache;
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::ipc::server::{EndpointHandle, EndpointHandleBuilder};
use gpy_agent::ipc::{ClientDirectory, LatencyTracker};
use gpy_agent::language::DetectionCache;
use gpy_agent::theme::ThemeManager;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// An `EndpointHandle` builder bound to `socket_path` with every subsystem set
/// to a fresh default (built-in `default` theme, default config, no watcher).
///
/// Builder setters overwrite, so a test that needs to observe a subsystem
/// passes its own handle by calling that setter again, e.g.
/// `endpoint_builder(path).client_registry(Arc::clone(&registry))`.
#[must_use]
pub fn endpoint_builder(socket_path: PathBuf) -> EndpointHandleBuilder {
    EndpointHandle::builder()
        .socket_path(socket_path)
        .client_registry(ClientDirectory::new().shared())
        .git_cache(Arc::new(GitStatusCache::new()))
        .config_manager(Arc::new(
            ConfigManager::with_defaults().expect("default config should load"),
        ))
        .watcher_slot(Arc::new(Mutex::new(None)))
        .theme_manager(Arc::new(
            ThemeManager::builtin("default").expect("default theme should load"),
        ))
        .instant_cache(Arc::new(InstantPromptCache::new_for_test()))
        .latency_tracker(Arc::new(LatencyTracker::new(100)))
        .language_cache(DetectionCache::new())
}

/// Wait until `socket_path` exists, panicking if it does not appear within
/// `timeout`. Sleeps on the tokio timer so a server task spawned on the same
/// runtime can make progress.
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
