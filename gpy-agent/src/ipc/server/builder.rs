//! Builder for the IPC server endpoint; required collaborators come in
//! [`EndpointDeps`] so a missing one is a compile error.

use super::handle::{EndpointHandle, ServerDeps, WatcherRef};
#[cfg(unix)]
use super::socket_dir::DirRepair;
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
    socket_dir_repair: DirRepair,
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
            socket_dir_repair: DirRepair::Refuse,
            #[cfg(unix)]
            socket_ownership_check: None,
        }
    }

    /// Set the socket path (defaults to [`crate::paths::get_socket_path`]).
    #[must_use]
    pub fn socket_path(mut self, path: PathBuf) -> Self {
        self.socket_path = Some(path);
        self
    }

    /// Set how a socket directory we own but others can write is treated
    /// (defaults to [`DirRepair::Refuse`]: never change a directory's mode
    /// unless the caller knows GPY owns it).
    #[cfg(unix)]
    #[must_use]
    pub const fn socket_dir_repair(mut self, repair: DirRepair) -> Self {
        self.socket_dir_repair = repair;
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
    /// Fails if no socket path was set and the default is unavailable.
    pub fn build(self) -> Result<EndpointHandle> {
        let socket_path = match self.socket_path {
            Some(path) => path,
            None => crate::paths::get_socket_path()?,
        };

        let EndpointDeps {
            client_registry,
            git_cache,
            config_manager,
            theme_manager,
            instant_cache,
            language_cache,
        } = self.deps;

        let palette_cache = self.palette_cache.unwrap_or_else(|| {
            let config = config_manager.get();
            Arc::new(PaletteCache::from_config(&config))
        });

        #[cfg_attr(
            not(unix),
            expect(unused_mut, reason = "only unix applies the socket overrides")
        )]
        let mut handle = EndpointHandle::with_path_and_state(
            socket_path,
            ServerDeps {
                client_registry,
                git_cache,
                config_manager,
                theme_manager,
                instant_cache,
                language_cache,
                palette_cache,
            },
            self.watcher_slot,
            self.security_config,
        );
        #[cfg(unix)]
        {
            handle.socket_dir_repair = self.socket_dir_repair;
            if let Some(period) = self.socket_ownership_check {
                handle.socket_ownership_check = period;
            }
        }
        Ok(handle)
    }
}
