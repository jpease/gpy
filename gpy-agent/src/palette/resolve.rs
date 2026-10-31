//! Resolve the active color palette for a render request.
//!
//! [`load_palette`] reports read/parse/validation failures so a long-lived owner
//! (the daemon's reload) can keep its last good palette. The render-path helpers
//! degrade to the embedded builtin default palette so a bad `ui.palette` never
//! breaks the prompt.

use crate::Result;
use crate::config::Config;
use crate::config::types::PaletteName;
use crate::palette::PaletteManager;
use crate::template::Palette;

/// Load the [`Palette`] named by `config.ui.palette`.
///
/// # Errors
/// Returns an error if the palette cannot be found, read, parsed, or validated.
pub fn load_active_palette(config: &Config) -> Result<Palette> {
    load_palette(config.ui.palette.as_str())
}

/// Load the [`Palette`] named `name` (user file, then builtin).
///
/// # Errors
/// Returns an error if the palette cannot be found, read, parsed, or validated.
pub fn load_palette(name: &str) -> Result<Palette> {
    Ok(PaletteManager::new(name)?.get().to_template_palette())
}

/// Build the active [`Palette`] from `config.ui.palette`; the builtin default on load failure.
#[must_use]
pub fn active_palette(config: &Config) -> Palette {
    palette_by_name(config.ui.palette.as_str())
}

/// Build a [`Palette`] for an arbitrary `name` (e.g. wizard preview); the
/// builtin default palette on load failure (warned).
#[must_use]
pub fn palette_by_name(name: &str) -> Palette {
    load_palette(name).unwrap_or_else(|error| {
        crate::debug::warn_fallback(
            &format!("Palette '{name}' loading"),
            "using the builtin default palette",
            &error,
        );
        builtin_default_palette()
    })
}

/// The embedded builtin default palette; never reads a user file.
fn builtin_default_palette() -> Palette {
    PaletteManager::load_builtin(PaletteName::DEFAULT)
        .ok()
        // Empty only with a corrupt binary: the embedded palette always parses.
        .map_or_else(Palette::default, |cfg| cfg.to_template_palette())
}

// Builtin default resolution is tested in `tests/palette_manager_tests.rs`.
#[cfg(test)]
mod tests {

    use super::{active_palette, load_palette, palette_by_name};
    use crate::config::Config;
    use crate::palette::PaletteManager;

    #[test]
    fn palette_by_name_falls_back_to_builtin_default_for_unknown_name() {
        let palette = palette_by_name("definitely-not-a-real-palette-name");
        assert!(
            palette.get("orange").is_some(),
            "unknown palette name should fall back to the builtin default palette, not error"
        );
    }

    #[test]
    fn palette_by_name_ignores_a_broken_user_default_palette() {
        let path = PaletteManager::palette_path("default");
        std::fs::create_dir_all(path.parent().expect("palettes dir")).expect("create dir");
        std::fs::write(&path, "name = [\n").expect("write broken palette");
        let palette = palette_by_name("default");
        std::fs::remove_file(&path).expect("remove broken palette");
        assert!(
            palette.get("orange").is_some(),
            "a broken user default.toml must not shadow the embedded fallback"
        );
    }

    #[test]
    fn load_palette_reports_unknown_name() {
        assert!(load_palette("definitely-not-a-real-palette-name").is_err());
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
