//! Palette manager: loading, discovery, and path resolution.
//!
//! # Sources
//!
//! Palettes are resolved in this precedence order (highest wins):
//!
//! 1. **User** — `~/.config/gpy/palettes/<name>.toml` (or `$XDG_CONFIG_HOME/gpy/palettes/<name>.toml`).
//! 2. **Plugin** — provided by a discovered plugin (reserved for a future sub-project;
//!    plugin manifests do not yet carry a `palettes` field — see SP2).
//! 3. **Builtin** — embedded in the binary (`"default"` → [`DEFAULT_PALETTE_CONTENT`]).
//!
//! # Plugin palettes (deferred)
//!
//! [`PaletteSource::Plugin`] is present for forward-compatibility, but SP1 does **not**
//! wire plugin palette discovery. Adding it requires a plugin-manifest schema change that
//! is out of scope for SP1. The variant is a no-op until a later sub-project lands.

use crate::config::defaults::{
    CATPPUCCIN_FRAPPE_PALETTE_CONTENT, CATPPUCCIN_LATTE_PALETTE_CONTENT,
    CATPPUCCIN_MACCHIATO_PALETTE_CONTENT, CATPPUCCIN_MOCHA_PALETTE_CONTENT,
    DEFAULT_PALETTE_CONTENT, GRUVBOX_DARK_MEDIUM_PALETTE_CONTENT, NORD_PALETTE_CONTENT,
    STARSHIP_PALETTE_CONTENT,
};
use crate::config::discovery::{self, Discovered, Source};
use crate::config::types::is_safe_config_name;
use crate::palette::PaletteConfig;
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Builtin palettes: name → embedded TOML content.
///
/// Adding a new builtin palette requires one entry here; `load_builtin` and
/// `config::discovery::insert_builtins` both iterate this table automatically.
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

/// Source metadata for a discovered palette.
///
/// Themes and palettes shared one enum shape and one precedence ordering, so
/// both names now refer to the single [`crate::config::discovery::Source`]
/// (#588). `Source::Plugin` stays unconstructed for palettes: plugin palette
/// discovery is still deferred (see this module's header).
pub type PaletteSource = Source;

/// A palette entry exposed to CLI/runtime discovery.
///
/// See [`PaletteSource`] on why this is shared with theme discovery.
pub type DiscoveredPalette = Discovered;

/// Manages palette loading and discovery.
///
/// A plain owned struct with no hot-reload, and, unlike its two sibling
/// managers, not a long-lived one: every caller builds a `PaletteManager`,
/// reads a palette out of it, and drops it inside the same expression
/// (`palette::resolve::palette_by_name` does this per render request; the two
/// `commands::palette` sites use `new` purely as a name-validity probe). No
/// struct field, `Arc`, or static holds one anywhere in the crate.
///
/// That is why #588 gave hot-reload to `ConfigManager` and `ThemeManager` but
/// not here: attaching a
/// [`HotReloadSlot`](crate::watcher::hot_reload::HotReloadSlot) to a value with
/// this lifetime would spawn and join a watcher thread per render. The
/// long-lived palette state is [`crate::palette::PaletteCache`], and that — not
/// this type — is where palette hot-reload belongs if it is ever added. It
/// would need a watch that follows `ui.palette` across config reloads, and
/// `ClientDirectory` access to repaint clients the way theme reloads do.
pub struct PaletteManager {
    palette_name: String,
    palette: Arc<PaletteConfig>,
}

impl PaletteManager {
    /// Load the named palette.
    ///
    /// Resolution order: user path → builtin by name → error.
    ///
    /// # Errors
    ///
    /// Returns an error if no palette with the given name exists (neither a user
    /// file at the expected path nor a known builtin).
    pub fn new(name: &str) -> Result<Self> {
        if !is_safe_config_name(name) {
            return Err(Error::config(format!(
                "invalid palette name '{name}': must not contain path separators, '..', or control characters"
            )));
        }
        let user_path = Self::user_palette_path(name);
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

    /// Discover all available palettes, merging builtin and user sources.
    ///
    /// When the same name exists in multiple sources the highest-precedence
    /// source wins (User > Plugin > Builtin), matching [`ThemeSource::precedence`].
    #[must_use]
    pub fn discover_available_palettes() -> Vec<DiscoveredPalette> {
        let mut merged: BTreeMap<String, DiscoveredPalette> = BTreeMap::new();

        discovery::insert_builtins(&mut merged, BUILTIN_PALETTES);
        // Plugin palettes deferred to a later sub-project (SP2+).
        Self::insert_user_palettes(&mut merged);

        merged.into_values().collect()
    }

    /// Return the directory where user palettes are stored.
    ///
    /// Honors `$XDG_CONFIG_HOME` when set; otherwise falls back to
    /// `~/.config/gpy/palettes`.
    #[must_use]
    pub fn user_palettes_dir() -> PathBuf {
        crate::paths::config_root_for(
            std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
            crate::paths::home_dir().as_deref(),
        )
        .join("palettes")
    }

    /// Return the canonical user path for a palette by name.
    #[must_use]
    pub fn palette_path(name: &str) -> PathBuf {
        Self::user_palettes_dir().join(format!("{name}.toml"))
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn user_palette_path(name: &str) -> PathBuf {
        Self::user_palettes_dir().join(format!("{name}.toml"))
    }

    /// # Errors
    ///
    /// Returns an error if `name` does not match any known builtin palette, or
    /// if the embedded TOML for a known builtin fails to deserialize.
    fn load_builtin(name: &str) -> Result<PaletteConfig> {
        let Some((_, content)) = BUILTIN_PALETTES.iter().find(|(n, _)| *n == name) else {
            return Err(Error::config(format!(
                "No palette named '{name}' found (checked user path and builtins)"
            )));
        };
        let mut cfg: PaletteConfig =
            toml::from_str(content).map_err(|e| Error::config(e.to_string()))?;
        if cfg.name.is_empty() {
            name.clone_into(&mut cfg.name);
        }
        Ok(cfg)
    }

    /// # Errors
    ///
    /// Returns an error if the file at `path` cannot be read or its TOML is
    /// invalid.
    fn load_from_path(path: &std::path::Path, stem_fallback: &str) -> Result<PaletteConfig> {
        let content = std::fs::read_to_string(path).map_err(|e| Error::config(e.to_string()))?;
        let mut cfg: PaletteConfig =
            toml::from_str(&content).map_err(|e| Error::config(e.to_string()))?;
        if cfg.name.is_empty() {
            stem_fallback.clone_into(&mut cfg.name);
        }
        Ok(cfg)
    }

    fn insert_user_palettes(merged: &mut BTreeMap<String, DiscoveredPalette>) {
        discovery::insert_toml_dir(merged, &Self::user_palettes_dir(), &PaletteSource::User);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{PaletteManager, PaletteSource};

    #[test]
    fn loads_builtin_default() {
        let mgr = PaletteManager::new("default").expect("default palette loads");
        assert!(mgr.get().colors.contains_key("red"));
    }

    #[test]
    fn missing_palette_errors() {
        assert!(PaletteManager::new("no-such-palette-xyz").is_err());
    }

    #[test]
    fn discovery_includes_builtin_default() {
        let found = PaletteManager::discover_available_palettes();
        assert!(
            found
                .iter()
                .any(|p| p.name == "default" && matches!(p.source, PaletteSource::Builtin))
        );
    }

    #[test]
    fn loads_builtin_starship() {
        let mgr = PaletteManager::new("starship").expect("starship palette loads");
        assert_eq!(
            mgr.get().colors.get("orange").expect("orange").as_str(),
            "202"
        );
        assert_eq!(
            mgr.get()
                .colors
                .get("bright_magenta")
                .expect("bright_magenta")
                .as_str(),
            "147"
        );
        assert_eq!(
            mgr.get()
                .colors
                .get("bright_green")
                .expect("bright_green")
                .as_str(),
            "149"
        );
    }

    #[test]
    fn discovery_includes_builtin_starship() {
        let found = PaletteManager::discover_available_palettes();
        assert!(
            found
                .iter()
                .any(|p| p.name == "starship" && matches!(p.source, PaletteSource::Builtin))
        );
    }

    #[test]
    fn all_builtins_load_and_are_discovered() {
        for (name, _) in super::BUILTIN_PALETTES {
            assert!(PaletteManager::new(name).is_ok(), "{name} loads");
        }
        let found = PaletteManager::discover_available_palettes();
        for (name, _) in super::BUILTIN_PALETTES {
            assert!(found.iter().any(|p| p.name == *name), "{name} discovered");
        }
    }
}
