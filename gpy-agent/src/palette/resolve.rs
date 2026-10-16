//! Resolve the active color palette for a render request.
//!
//! Load failures degrade to an empty palette so a bad `ui.palette` never breaks the prompt.

use crate::config::Config;
use crate::palette::PaletteManager;
use crate::template::Palette;

/// Build the active [`Palette`] from `config.ui.palette`; empty on load failure.
#[must_use]
pub fn active_palette(config: &Config) -> Palette {
    palette_by_name(config.ui.palette.as_str())
}

/// Build a [`Palette`] for an arbitrary `name` (e.g. wizard preview); empty on load failure.
#[must_use]
pub fn palette_by_name(name: &str) -> Palette {
    match PaletteManager::new(name) {
        Ok(manager) => manager.get().to_template_palette(),
        Err(error) => {
            crate::debug::warn_fallback(
                &format!("Palette '{name}' loading"),
                "using empty palette",
                &error,
            );
            Palette::default()
        }
    }
}

// Builtin default resolution is tested in `tests/palette_manager_tests.rs`.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{active_palette, palette_by_name};
    use crate::config::Config;

    #[test]
    fn palette_by_name_falls_back_to_empty_for_unknown_name() {
        let palette = palette_by_name("definitely-not-a-real-palette-name");
        assert!(
            palette.get("green").is_none(),
            "unknown palette name should fall back to an empty palette, not error"
        );
    }

    #[test]
    fn active_palette_matches_palette_by_name_for_configured_palette() {
        let config = Config::default();
        let via_active = active_palette(&config);
        let via_name = palette_by_name(config.ui.palette.as_str());
        assert_eq!(
            via_active.get("green"),
            via_name.get("green"),
            "active_palette must delegate to palette_by_name for config.ui.palette"
        );
    }
}
