//! Resolve the active color palette for a render request.
//!
//! Load order: `config.ui.palette` → [`PaletteManager`] → [`PaletteConfig`]
//! → [`PaletteConfig::to_template_palette`] → [`crate::template::Palette`].
//! A load failure degrades to an empty palette (standard color names still
//! resolve) so a misconfigured `ui.palette` never breaks the live prompt;
//! `gpy doctor` surfaces the misconfiguration loudly instead.

use crate::config::Config;
use crate::palette::PaletteManager;
use crate::template::Palette;

/// Build the active [`Palette`] from `config.ui.palette`.
///
/// Returns an empty palette (logging a warning) when the configured palette
/// cannot be loaded or parsed.
#[must_use]
pub fn active_palette(config: &Config) -> Palette {
    palette_by_name(config.ui.palette.as_str())
}

/// Build a [`Palette`] for an arbitrary palette `name`, independent of
/// `config.ui.palette`.
///
/// Added for the config wizard's live preview (`commands::wizard::preview`,
/// #373), which needs to render whichever palette is currently *selected* in
/// the wizard — potentially different from the config's on-disk active
/// palette while the user is still browsing. [`active_palette`] is now a thin
/// wrapper around this function using `config.ui.palette` as `name`.
///
/// Kept infallible-with-fallback, matching [`active_palette`]'s existing
/// behavior, rather than returning `Result`: a load failure degrades to an
/// empty palette (standard color names still resolve) so a preview render
/// never hard-fails on a bad palette name. The wizard's palette *list* is
/// already sourced from real on-disk discovery
/// (`PaletteManager::discover_available_palettes`), so a name reaching this
/// function is normally valid; a load failure here is the same
/// "misconfigured/removed since discovery" edge case `active_palette` already
/// tolerates for `config.ui.palette`, so it gets the same treatment.
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{active_palette, palette_by_name};
    use crate::config::Config;

    #[test]
    fn default_config_resolves_builtin_default_palette() {
        // `ui.palette` defaults to "default", which SP1 ships as a builtin.
        let config = Config::default();
        let palette = active_palette(&config);
        // The default builtin maps the standard ANSI names to themselves (SP1),
        // so at minimum a standard name resolves.
        assert!(
            palette.get("green").is_some(),
            "default palette must define green"
        );
    }

    #[test]
    fn default_palette_defines_required_named_colors() {
        let config = Config::default();
        let palette = active_palette(&config);
        // Identity ANSI names (default.toml/text.toml use these directly).
        for name in [
            "black", "red", "green", "yellow", "blue", "cyan", "white", "magenta",
        ] {
            assert!(
                palette.get(name).is_some(),
                "default palette must define standard name '{name}'"
            );
        }
    }

    #[test]
    fn default_palette_defines_orange_and_brown() {
        let config = Config::default();
        let palette = active_palette(&config);
        assert!(
            palette.get("orange").is_some(),
            "default palette must define orange"
        );
        assert!(
            palette.get("brown").is_some(),
            "default palette must define brown"
        );
    }

    #[test]
    fn palette_by_name_resolves_builtin_default_palette() {
        let palette = palette_by_name("default");
        assert!(
            palette.get("green").is_some(),
            "palette_by_name(\"default\") must define green"
        );
    }

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
