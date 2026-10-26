//! Palette manager: loading, discovery, and path resolution.
//!
//! Precedence: user `<config>/gpy/palettes/<name>.toml` over builtins.

use crate::config::defaults::{
    CATPPUCCIN_FRAPPE_PALETTE_CONTENT, CATPPUCCIN_LATTE_PALETTE_CONTENT,
    CATPPUCCIN_MACCHIATO_PALETTE_CONTENT, CATPPUCCIN_MOCHA_PALETTE_CONTENT,
    DEFAULT_PALETTE_CONTENT, GRUVBOX_DARK_MEDIUM_PALETTE_CONTENT, NORD_PALETTE_CONTENT,
    STARSHIP_PALETTE_CONTENT,
};
use crate::config::discovery::{self, Discovered, Source};
use crate::palette::PaletteConfig;
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Builtin palettes: name → embedded TOML content.
pub(crate) const BUILTIN_PALETTES: &[(&str, &str)] = &[
    ("default", DEFAULT_PALETTE_CONTENT),
    ("starship", STARSHIP_PALETTE_CONTENT),
    ("catppuccin-latte", CATPPUCCIN_LATTE_PALETTE_CONTENT),
    ("catppuccin-frappe", CATPPUCCIN_FRAPPE_PALETTE_CONTENT),
    ("catppuccin-macchiato", CATPPUCCIN_MACCHIATO_PALETTE_CONTENT),
    ("catppuccin-mocha", CATPPUCCIN_MOCHA_PALETTE_CONTENT),
    ("nord", NORD_PALETTE_CONTENT),
    ("gruvbox-dark-medium", GRUVBOX_DARK_MEDIUM_PALETTE_CONTENT),
];

/// Source metadata for a discovered palette (`Plugin` is never used).
pub type PaletteSource = Source;

/// A palette entry exposed to CLI/runtime discovery.
pub type DiscoveredPalette = Discovered;

/// Loads a palette by name. Short-lived per call, so no hot-reload here;
/// long-lived state lives in [`crate::palette::PaletteCache`].
pub struct PaletteManager {
    palette_name: String,
    palette: Arc<PaletteConfig>,
}

impl PaletteManager {
    /// Load the named palette: user path, then builtin.
    ///
    /// # Errors
    /// Returns an error if the name is invalid or no such palette exists.
    pub fn new(name: &str) -> Result<Self> {
        discovery::validate_config_name("palette", name)?;
        let user_path = Self::palette_path(name);
        let palette = if user_path.exists() {
            Self::load_from_path(&user_path, name)?
        } else {
            Self::load_builtin(name)?
        };

        Ok(Self {
            palette_name: name.to_owned(),
            palette: Arc::new(palette),
        })
    }

    /// Return the active palette configuration.
    #[must_use]
    pub fn get(&self) -> Arc<PaletteConfig> {
        Arc::clone(&self.palette)
    }

    /// Return the name of the active palette.
    #[must_use]
    pub fn palette_name(&self) -> &str {
        &self.palette_name
    }

    /// Return the embedded default palette TOML template.
    #[must_use]
    pub const fn default_palette_template() -> &'static str {
        DEFAULT_PALETTE_CONTENT
    }

    /// Discover all palettes; user sources override builtins of the same name.
    #[must_use]
    pub fn discover_available_palettes() -> Vec<DiscoveredPalette> {
        let mut merged: BTreeMap<String, DiscoveredPalette> = BTreeMap::new();

        discovery::insert_builtins(&mut merged, BUILTIN_PALETTES);
        Self::insert_user_palettes(&mut merged);

        merged.into_values().collect()
    }

    /// Return the user palettes directory (honors `$XDG_CONFIG_HOME`).
    #[must_use]
    pub fn user_palettes_dir() -> PathBuf {
        crate::paths::config_root_for(
            crate::paths::root_var("XDG_CONFIG_HOME").as_deref(),
            crate::paths::home_dir().as_deref(),
            crate::paths::Os::host(),
        )
        .join("palettes")
    }

    /// Return the canonical user path for a palette by name.
    #[must_use]
    pub fn palette_path(name: &str) -> PathBuf {
        Self::user_palettes_dir().join(format!("{name}.toml"))
    }

    /// # Errors
    /// Returns an error for an unknown builtin or invalid embedded TOML.
    fn load_builtin(name: &str) -> Result<PaletteConfig> {
        let Some((_, content)) = BUILTIN_PALETTES.iter().find(|(n, _)| *n == name) else {
            return Err(Error::config(format!(
                "No palette named '{name}' found (checked user path and builtins)"
            )));
        };
        let mut cfg: PaletteConfig = toml::from_str(content)?;
        if cfg.name.is_empty() {
            name.clone_into(&mut cfg.name);
        }
        cfg.validate()?;
        Ok(cfg)
    }

    /// # Errors
    /// Returns an error if the file cannot be read or its TOML is invalid.
    fn load_from_path(path: &std::path::Path, stem_fallback: &str) -> Result<PaletteConfig> {
        let content = std::fs::read_to_string(path).map_err(|e| Error::config(e.to_string()))?;
        let mut cfg: PaletteConfig = toml::from_str(&content)?;
        if cfg.name.is_empty() {
            stem_fallback.clone_into(&mut cfg.name);
        }
        cfg.validate()?;
        Ok(cfg)
    }

    fn insert_user_palettes(merged: &mut BTreeMap<String, DiscoveredPalette>) {
        discovery::insert_toml_dir(merged, &Self::user_palettes_dir(), &PaletteSource::User);
    }
}

// Builtin loading/discovery is tested in `tests/palette_manager_tests.rs` (needs empty XDG_CONFIG_HOME).
#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::PaletteManager;

    #[test]
    fn missing_palette_errors() {
        assert!(PaletteManager::new("no-such-palette-xyz").is_err());
    }
}
