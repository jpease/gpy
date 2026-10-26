//! Shared dependency fixtures for the in-tree IPC tests.

use crate::config::manager::ConfigManager;
use crate::ipc::ClientDirectory;
use crate::ipc::handlers::RenderDeps;
#[cfg(unix)]
use crate::ipc::server::DirRepair;
use crate::ipc::server::EndpointDeps;
use crate::palette::PaletteCache;
use crate::security::GuardSettings;
use crate::theme::ThemeManager;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Render dependencies over `config_manager`: the built-in theme (hermetic
/// against on-disk themes), its palette, a private instant cache and no clients.
pub(crate) fn render_deps(config_manager: Arc<ConfigManager>) -> RenderDeps {
    RenderDeps {
        palette_cache: Arc::new(PaletteCache::from_config(&config_manager.get())),
        config_manager,
        theme_manager: Arc::new(ThemeManager::builtin("default").expect("default theme")),
        instant_cache: Arc::new(crate::cache::InstantPromptCache::new_for_test()),
        client_registry: ClientDirectory::new().shared(),
    }
}

/// [`render_deps`] over the default configuration.
pub(crate) fn default_render_deps() -> RenderDeps {
    render_deps(Arc::new(
        ConfigManager::with_defaults().expect("default config"),
    ))
}

/// Endpoint construction deps for an unbound `socket_path` around `render`,
/// with empty git and language caches and an empty watcher slot.
pub(crate) fn endpoint_deps(socket_path: &Path, render: RenderDeps) -> EndpointDeps {
    EndpointDeps {
        socket_path: socket_path.to_path_buf(),
        #[cfg(unix)]
        socket_dir_repair: DirRepair::Refuse,
        security_config: GuardSettings::default(),
        watcher_slot: Arc::new(Mutex::new(None)),
        client_registry: render.client_registry,
        git_cache: Arc::new(crate::git::cache::GitStatusCache::new()),
        config_manager: render.config_manager,
        theme_manager: render.theme_manager,
        instant_cache: render.instant_cache,
        language_cache: crate::language::DetectionCache::new(),
        palette_cache: render.palette_cache,
    }
}
