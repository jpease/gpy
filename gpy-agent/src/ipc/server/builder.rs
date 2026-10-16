//! Builder for the IPC server endpoint; required collaborators come in
//! [`EndpointDeps`] so a missing one is a compile error.

use super::handle::{EndpointHandle, ServerDeps, WatcherRef};
use crate::{
    Result, config::manager::ConfigManager, git::cache::GitStatusCache, ipc::ClientDirectory,
    palette::PaletteCache, security::GuardSettings, theme::ThemeManager,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The shared collaborators every IPC server endpoint requires.
pub struct EndpointDeps {
    /// Registered shell clients, used to target repaint signals.
    pub client_registry: Arc<ClientDirectory>,
    /// Shared git status cache backing both the request path and the watcher.
    pub git_cache: Arc<GitStatusCache>,
    /// Live configuration, re-read per request so a hot reload takes effect.
    pub config_manager: Arc<ConfigManager>,
    /// Active theme, shared rather than reloaded from disk per render.
    pub theme_manager: Arc<ThemeManager>,
    /// On-disk cache the shell reads to render a prompt without any IPC.
    pub instant_cache: Arc<crate::cache::InstantPromptCache>,
    /// Rolling request-latency samples reported by `Message::LatencyStats`.
    pub latency_tracker: Arc<crate::ipc::LatencyTracker>,
    /// Per-repo language detection results shared with the agent event loop.
    pub language_cache: crate::language::DetectionCache,
}

/// Builder for `EndpointHandle`; required deps via [`EndpointDeps`], the rest are optional setters.
pub struct EndpointHandleBuilder {
    deps: EndpointDeps,
    socket_path: Option<PathBuf>,
    watcher_slot: WatcherRef,
    security_config: GuardSettings,
    palette_cache: Option<Arc<PaletteCache>>,
    #[cfg(unix)]
    socket_ownership_check: Option<std::time::Duration>,
}

impl EndpointHandleBuilder {
    /// Create a builder from the shared collaborators every server requires.
    #[must_use]
    pub fn new(deps: EndpointDeps) -> Self {
        Self {
            deps,
            socket_path: None,
            watcher_slot: Arc::new(Mutex::new(None)),
            security_config: GuardSettings::default(),
            palette_cache: None,
            #[cfg(unix)]
            socket_ownership_check: None,
        }
    }

    /// Set the socket path (defaults to the platform-specific path).
    #[must_use]
    pub fn socket_path(mut self, path: PathBuf) -> Self {
        self.socket_path = Some(path);
        self
    }

    /// Shorten the socket-ownership check period for tests.
    #[cfg(all(unix, test))]
    #[must_use]
    pub(crate) const fn socket_ownership_check(mut self, period: std::time::Duration) -> Self {
        self.socket_ownership_check = Some(period);
        self
    }

    /// Set the security configuration (defaults to [`GuardSettings::default`]).
    #[must_use]
    pub fn security_config(mut self, config: GuardSettings) -> Self {
        self.security_config = config;
        self
    }

    /// Set the watcher slot. Must be the agent's shared slot, or live updates break.
    #[must_use]
    pub fn watcher_slot(mut self, watcher_slot: WatcherRef) -> Self {
        self.watcher_slot = watcher_slot;
        self
    }

    /// Set the palette cache (defaults to one built from the current config).
    #[must_use]
    pub fn palette_cache(mut self, cache: Arc<PaletteCache>) -> Self {
        self.palette_cache = Some(cache);
        self
    }

    /// Build the `EndpointHandle`.
    ///
    /// # Errors
    /// Fails if no socket path was set and the platform default is unavailable.
    pub fn build(self) -> Result<EndpointHandle> {
        let socket_path = match self.socket_path {
            Some(path) => path,
            None => EndpointHandle::default_socket_path()?,
        };

        let EndpointDeps {
            client_registry,
            git_cache,
            config_manager,
            theme_manager,
            instant_cache,
            latency_tracker,
            language_cache,
        } = self.deps;

        let palette_cache = self.palette_cache.unwrap_or_else(|| {
            let config = config_manager.get();
            Arc::new(PaletteCache::from_config(&config))
        });

        #[cfg_attr(
            not(unix),
            expect(unused_mut, reason = "only unix applies the ownership-check override")
        )]
        let mut handle = EndpointHandle::with_path_and_state(
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
            self.watcher_slot,
            self.security_config,
        );
        #[cfg(unix)]
        if let Some(period) = self.socket_ownership_check {
            handle.socket_ownership_check = period;
        }
        Ok(handle)
    }
}
