//! Cached active palette, refreshed on config change.

use crate::config::Config;
use crate::template::Palette;
use std::sync::{Arc, RwLock};

/// Thread-safe cached copy of the active palette.
///
/// Resolved once at startup from the active config, then refreshed via
/// [`PaletteCache::refresh`] when `ui.palette` changes.  Cloning the cached
/// `Palette` on each render is far cheaper than re-parsing the TOML.
#[derive(Clone)]
pub struct PaletteCache {
    inner: Arc<RwLock<Palette>>,
}

impl PaletteCache {
    /// Build the initial cache from the active config.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let palette = crate::palette::active_palette(config);
        Self {
            inner: Arc::new(RwLock::new(palette)),
        }
    }

    /// Return a clone of the currently cached palette.
    ///
    /// If the lock was poisoned by a panic in another thread the cached palette is
    /// still valid, so the guard is recovered rather than propagating the panic.
    #[must_use]
    pub fn get(&self) -> Palette {
        self.inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replace the cached palette with one derived from `config`.
    ///
    /// Called from `handle_config_reload` when `ui.palette` changes. A poisoned
    /// lock is recovered (the cached palette remains valid) rather than panicking.
    pub fn refresh(&self, config: &Config) {
        let new_palette = crate::palette::active_palette(config);
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = new_palette;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::PaletteCache;
    use crate::config::Config;

    #[test]
    fn get_returns_initial_palette() {
        let config = Config::default();
        let cache = PaletteCache::from_config(&config);
        let palette = cache.get();
        // The default palette must define the standard ANSI color names.
        assert!(
            palette.get("green").is_some(),
            "initial palette must define green"
        );
    }

    #[test]
    fn refresh_updates_cached_palette() {
        use crate::config::types::PaletteName;

        let config = Config::default();
        let cache = PaletteCache::from_config(&config);

        // Baseline: default palette resolves "green".
        assert!(cache.get().get("green").is_some());

        // Switch to an unknown palette name — active_palette() degrades to an
        // empty Palette.  After refresh the cache should hold that empty palette.
        let mut new_config = Config::default();
        new_config.ui.palette =
            PaletteName::new("__nonexistent_palette__".to_owned()).expect("valid name");
        cache.refresh(&new_config);

        // The refreshed palette for a nonexistent name is Palette::default() which
        // has no entries.
        let refreshed = cache.get();
        assert!(
            refreshed.get("green").is_none(),
            "refreshed empty palette should not define green"
        );
    }
}
