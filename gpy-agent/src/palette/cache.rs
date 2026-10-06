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
    /// A poisoned lock is recovered (the cached palette remains valid) rather
    /// than panicking.
    pub fn refresh(&self, config: &Config) {
        self.replace(crate::palette::active_palette(config));
    }

    /// Store `palette` and report whether its contents differ from the cached one.
    ///
    /// Called from `handle_config_reload` on every reload so an edit to the
    /// active palette file (same `ui.palette` name) is picked up (#772).
    pub fn replace(&self, palette: Palette) -> bool {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = *guard != palette;
        if changed {
            *guard = palette;
        }
        changed
    }
}

// Loading and refreshing the configured palette is tested in
// `tests/palette_manager_tests.rs` against an empty `XDG_CONFIG_HOME`; here a
// developer's own `palettes/default.toml` would shadow the builtin (#664).
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::PaletteCache;
    use crate::config::Config;

    #[test]
    fn replace_reports_content_change() {
        use crate::template::{Color, Palette};
        use std::collections::HashMap;

        let cache = PaletteCache::from_config(&Config::default());
        let build = |color: &str| {
            Palette::new(HashMap::from([(
                "red".to_owned(),
                Color::Named(color.to_owned()),
            )]))
        };

        assert!(
            cache.replace(build("blue")),
            "different red must report a change"
        );
        assert_eq!(
            cache.get().get("red"),
            Some(Color::Named("blue".to_owned()))
        );
        assert!(
            !cache.replace(build("blue")),
            "equal palette must report no change"
        );
    }
}
