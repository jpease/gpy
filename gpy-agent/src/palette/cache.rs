//! Cached active palette, replaced only by a successfully loaded palette.

use crate::config::Config;
use crate::sync_ext::RwLockExt;
use crate::template::Palette;
use std::sync::{Arc, RwLock};

/// Thread-safe cached copy of the active palette.
#[derive(Clone)]
pub struct PaletteCache {
    inner: Arc<RwLock<Palette>>,
}

impl PaletteCache {
    /// Build the initial cache from the active config. With no earlier palette
    /// to keep, an unusable one starts empty (warned) rather than failing startup.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let palette = crate::palette::active_palette(config);
        Self {
            inner: Arc::new(RwLock::new(palette)),
        }
    }

    /// Return a clone of the cached palette; a poisoned lock is recovered.
    #[must_use]
    pub fn get(&self) -> Palette {
        self.inner.read_or_recover().clone()
    }

    /// Store `palette` and report whether its contents differ from the cached one.
    #[must_use]
    pub fn replace(&self, palette: Palette) -> bool {
        let mut guard = self.inner.write_or_recover();
        let changed = *guard != palette;
        if changed {
            *guard = palette;
        }
        changed
    }
}

// Config-driven loading is tested in `tests/palette_manager_tests.rs` (needs empty XDG_CONFIG_HOME).
#[cfg(test)]
mod tests {

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
