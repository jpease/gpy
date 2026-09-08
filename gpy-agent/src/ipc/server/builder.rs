//! Builder for constructing the IPC server endpoint.
//!
//! The server needs several shared runtime collaborators: client registry, git
//! cache, configuration manager, theme manager, latency tracker, and optional
//! watcher access. This module gathers those dependencies explicitly before
//! producing an [`EndpointHandle`] that can accept shell client requests.

use super::handle::{EndpointHandle, ServerDeps, WatcherRef, WatcherSlot};
use crate::{
    Error, Result, config::manager::ConfigManager, git::cache::GitStatusCache,
    ipc::ClientDirectory, palette::PaletteCache, security::GuardSettings, theme::ThemeManager,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Builder for `EndpointHandle` providing a clearer API for constructing servers.
///
/// This builder simplifies the construction of `EndpointHandle` instances by:
/// - Making required vs optional parameters explicit
/// - Providing sensible defaults for optional fields
/// - Offering a fluent API that's easier to read and maintain
/// - Reducing errors from parameter ordering mistakes
///
/// # Example
///
/// ```rust
/// # use gpy_agent::ipc::server::EndpointHandle;
/// # use std::sync::Arc;
/// # use gpy_agent::ipc::registry::ClientDirectory;
/// # use gpy_agent::git::cache::GitStatusCache;
/// # use gpy_agent::config::manager::ConfigManager;
/// # use gpy_agent::theme::manager::ThemeManager;
/// # use gpy_agent::ipc::LatencyTracker;
/// # use gpy_agent::cache::InstantPromptCache;
/// # use gpy_agent::language::DetectionCache;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let server = EndpointHandle::builder()
///     .client_registry(Arc::new(ClientDirectory::new()))
///     .git_cache(Arc::new(GitStatusCache::new()))
///     .config_manager(Arc::new(ConfigManager::with_defaults()?))
///     .theme_manager(Arc::new(ThemeManager::new("default")?))
///     .instant_cache(Arc::new(InstantPromptCache::new()?))
///     .latency_tracker(Arc::new(LatencyTracker::new(100)))
///     .language_cache(DetectionCache::new())
///     .build()?;
/// # Ok(())
/// # }
/// ```
#[derive(Default)]
pub struct EndpointHandleBuilder {
    socket_path: Option<PathBuf>,
    client_registry: Option<Arc<ClientDirectory>>,
    git_cache: Option<Arc<GitStatusCache>>,
    config_manager: Option<Arc<ConfigManager>>,
    watcher_slot: WatcherSlot,
    theme_manager: Option<Arc<ThemeManager>>,
    instant_cache: Option<Arc<crate::cache::InstantPromptCache>>,
    latency_tracker: Option<Arc<crate::ipc::LatencyTracker>>,
    language_cache: Option<crate::language::DetectionCache>,
    security_config: Option<GuardSettings>,
    palette_cache: Option<Arc<PaletteCache>>,
}

impl EndpointHandleBuilder {
    /// Create a new builder with no fields set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the socket path (optional, defaults to platform-specific path).
    #[must_use]
    pub fn socket_path(mut self, path: PathBuf) -> Self {
        self.socket_path = Some(path);
        self
    }

    /// Set the client registry (required).
    #[must_use]
    pub fn client_registry(mut self, registry: Arc<ClientDirectory>) -> Self {
        self.client_registry = Some(registry);
        self
    }

    /// Set the git status cache (required).
    #[must_use]
    pub fn git_cache(mut self, cache: Arc<GitStatusCache>) -> Self {
        self.git_cache = Some(cache);
        self
    }

    /// Set the language cache (required).
    #[must_use]
    pub fn language_cache(mut self, cache: crate::language::DetectionCache) -> Self {
        self.language_cache = Some(cache);
        self
    }

    /// Set the configuration manager (required).
    #[must_use]
    pub fn config_manager(mut self, manager: Arc<ConfigManager>) -> Self {
        self.config_manager = Some(manager);
        self
    }

    /// Set the security configuration (optional).
    #[must_use]
    pub const fn security_config(mut self, config: GuardSettings) -> Self {
        self.security_config = Some(config);
        self
    }

    /// Set the watcher slot (optional, defaults to empty slot).
    ///
    /// **IMPORTANT**: Pass the SHARED watcher slot from the agent to enable live updates.
    /// Creating a new slot here will break watcher functionality as the agent and server
    /// won't share the same watcher instance.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use std::sync::{Arc, Mutex};
    /// # use gpy_agent::ipc::server::EndpointHandle;
    /// # use gpy_agent::watcher::multi_repo::MultiRepoWatcher;
    /// # fn example(watcher_slot: Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>) {
    /// // Correct: Share the slot created by the agent
    /// let server = EndpointHandle::builder()
    ///     // ... other required fields ...
    ///     .watcher_slot(watcher_slot)  // Pass the agent's slot
    ///     .build();
    /// # }
    /// ```
    #[must_use]
    pub fn watcher_slot(mut self, watcher_slot: WatcherRef) -> Self {
        self.watcher_slot = Some(watcher_slot);
        self
    }

    /// Set the theme manager (required).
    #[must_use]
    pub fn theme_manager(mut self, manager: Arc<ThemeManager>) -> Self {
        self.theme_manager = Some(manager);
        self
    }

    /// Set the instant-prompt cache (required).
    #[must_use]
    pub fn instant_cache(mut self, cache: Arc<crate::cache::InstantPromptCache>) -> Self {
        self.instant_cache = Some(cache);
        self
    }

    /// Set the latency tracker (required).
    #[must_use]
    pub fn latency_tracker(mut self, tracker: Arc<crate::ipc::LatencyTracker>) -> Self {
        self.latency_tracker = Some(tracker);
        self
    }

    /// Set the palette cache (optional, built from config when not provided).
    #[must_use]
    pub fn palette_cache(mut self, cache: Arc<PaletteCache>) -> Self {
        self.palette_cache = Some(cache);
        self
    }

    /// Build the `EndpointHandle` from the configured builder.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any required field is not set
    /// - Socket path cannot be determined (when using default)
    ///
    /// # Example
    ///
    /// ```rust
    /// # use gpy_agent::ipc::server::EndpointHandle;
    /// # use std::sync::Arc;
    /// # use gpy_agent::ipc::registry::ClientDirectory;
    /// # use gpy_agent::git::cache::GitStatusCache;
    /// # use gpy_agent::config::manager::ConfigManager;
    /// # use gpy_agent::theme::manager::ThemeManager;
    /// # use gpy_agent::ipc::LatencyTracker;
    /// # use gpy_agent::cache::InstantPromptCache;
    /// # use gpy_agent::language::DetectionCache;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let server = EndpointHandle::builder()
    ///     .client_registry(Arc::new(ClientDirectory::new()))
    ///     .git_cache(Arc::new(GitStatusCache::new()))
    ///     .config_manager(Arc::new(ConfigManager::with_defaults()?))
    ///     .theme_manager(Arc::new(ThemeManager::new("default")?))
    ///     .instant_cache(Arc::new(InstantPromptCache::new()?))
    ///     .latency_tracker(Arc::new(LatencyTracker::new(100)))
    ///     .language_cache(DetectionCache::new())
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn build(self) -> Result<EndpointHandle> {
        // Determine socket path (use default if not specified)
        let socket_path = match self.socket_path {
            Some(path) => path,
            None => EndpointHandle::default_socket_path()?,
        };

        // Extract required fields, returning error if any are missing
        let client_registry = self.client_registry.ok_or_else(|| {
            Error::ipc("Builder missing required field: client_registry".to_owned())
        })?;
        let git_cache = self
            .git_cache
            .ok_or_else(|| Error::ipc("Builder missing required field: git_cache".to_owned()))?;
        let config_manager = self.config_manager.ok_or_else(|| {
            Error::ipc("Builder missing required field: config_manager".to_owned())
        })?;
        let theme_manager = self.theme_manager.ok_or_else(|| {
            Error::ipc("Builder missing required field: theme_manager".to_owned())
        })?;
        let instant_cache = self.instant_cache.ok_or_else(|| {
            Error::ipc("Builder missing required field: instant_cache".to_owned())
        })?;
        let latency_tracker = self.latency_tracker.ok_or_else(|| {
            Error::ipc("Builder missing required field: latency_tracker".to_owned())
        })?;
        let language_cache = self.language_cache.ok_or_else(|| {
            Error::ipc("Builder missing required field: language_cache".to_owned())
        })?;

        // Watcher slot is optional - default to empty slot if not set
        // NOTE: For live updates to work, the agent MUST pass its shared watcher slot
        let watcher_slot = self
            .watcher_slot
            .unwrap_or_else(|| Arc::new(Mutex::new(None)));

        let security_config = self.security_config.unwrap_or_default();

        // Palette cache: build from config when not explicitly provided.
        let palette_cache = self.palette_cache.unwrap_or_else(|| {
            let config = config_manager.get();
            Arc::new(PaletteCache::from_config(&config))
        });

        // Use the existing with_path_and_state constructor
        Ok(EndpointHandle::with_path_and_state(
            socket_path,
            ServerDeps {
                client_registry,
                git_cache,
                config_manager,
                theme_manager,
                instant_cache,
                latency_tracker,
                language_cache,
                palette_cache,
            },
            watcher_slot,
            security_config,
        ))
    }
}
