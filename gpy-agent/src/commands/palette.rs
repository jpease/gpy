//! `gpy palette` command handlers.
//!
//! Palette commands list, select, inspect, and validate named color palettes.
//! They coordinate [`crate::palette::PaletteManager`] and configuration persistence
//! without owning palette parsing rules directly, keeping CLI presentation
//! separate from the runtime palette loader.

use super::utils::{
    active_config_path, load_active_config, read_config, reload_agent_and_notify, save_config_to,
};
use super::validation::{self, ValidatableResource};
use crate::{
    Error, Result, config,
    palette::{PaletteManager, PaletteSource},
};
use std::path::Path;

/// Result of validating a palette target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteValidationResult {
    /// Human-readable target label.
    pub target: String,
    /// Source descriptor (active palette, named palette, or explicit file path).
    pub source: String,
}

/// List available palettes.
///
/// # Errors
///
/// Returns an error if the active config file exists but cannot be read, parsed,
/// or validated.
pub fn list() -> Result<()> {
    let config = read_config()?;
    let active = &config.ui.palette;

    println!("Available Palettes:");
    println!("===================\n");

    let palettes = PaletteManager::discover_available_palettes();

    for palette in palettes {
        let marker = if palette.name == active.as_str() {
            " *"
        } else {
            ""
        };
        let source = match &palette.source {
            PaletteSource::Builtin => "builtin".to_owned(),
            PaletteSource::User => "user".to_owned(),
            PaletteSource::Plugin { plugin_id } => format!("plugin:{plugin_id}"),
        };
        println!("  {}{} [{}]", palette.name, marker, source);
    }

    Ok(())
}

/// Switch to a different palette.
///
/// # Errors
///
/// Returns an error if the palette doesn't exist or config cannot be saved.
pub fn use_palette(name: &str) -> Result<()> {
    // Validate palette exists
    PaletteManager::new(name)?;

    // Update config
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();
    config.ui.palette = config::types::PaletteName::new(name.to_owned())
        .ok_or_else(|| Error::config(format!("Invalid palette name: {name}")))?;
    save_config_to(&path, &original, &config, &[])?;

    println!("✅ Switched to '{name}' palette");

    // Reload agent if running
    reload_agent_and_notify();
    Ok(())
}

/// Show current palette name.
///
/// # Errors
///
/// Returns an error if the active config file exists but cannot be read, parsed,
/// or validated.
pub fn show() -> Result<()> {
    let config = read_config()?;
    println!("{}", config.ui.palette);
    Ok(())
}

/// Validate a palette target.
///
/// If `target` is omitted, validates the currently configured active palette.
/// If `target` is an existing file path, validates that TOML file directly.
/// Otherwise, treats `target` as a discovered palette name.
///
/// # Errors
///
/// Returns an error when parsing or validation fails. On failure the detailed
/// diagnostic has already been printed once; the returned error is a short,
/// generic summary so the process exits non-zero without duplicating that
/// diagnostic (see `commands::validation::validate`).
pub fn validate(target: Option<&str>) -> Result<()> {
    validation::validate::<PaletteResource>(target)
}

/// Validate a palette and return structured information for callers like `doctor`.
///
/// # Errors
///
/// Returns an error if the palette fails to parse/validate.
pub fn validate_target(target: Option<&str>) -> Result<PaletteValidationResult> {
    let report = validation::validate_target::<PaletteResource>(target)?;
    Ok(PaletteValidationResult {
        target: report.target,
        source: report.source.to_owned(),
    })
}

/// Validate a palette strictly by name, bypassing the CWD-sensitive path resolver.
///
/// Use this when the argument is always a palette name (e.g., from `gpy doctor`)
/// and must not be treated as a file path regardless of the working directory.
///
/// # Errors
///
/// Returns an error if the named palette cannot be loaded or validated.
pub fn validate_by_name(name: &str) -> Result<PaletteValidationResult> {
    let report = validation::validate_by_name::<PaletteResource>(name)?;
    Ok(PaletteValidationResult {
        target: report.target,
        source: report.source.to_owned(),
    })
}

/// Print remediation-focused diagnostics for a palette validation error.
pub fn print_palette_validation_error(context: &str, error: &Error) {
    validation::print_validation_error::<PaletteResource>(context, error);
}

/// Marker type wiring palette validation into the shared
/// [`validation::ValidatableResource`] orchestration.
struct PaletteResource;

impl ValidatableResource for PaletteResource {
    const KIND: &'static str = "Palette";
    const VALIDATE_CONTEXT: &'static str = "palette validate";
    const ACTIVE_LABEL: &'static str = "active config palette";
    const NAME_LABEL: &'static str = "named palette";
    const SHORT_ERROR: &'static str = "palette validation failed";

    fn active_value() -> Result<String> {
        let cfg = config::loader::load_config()?;
        Ok(cfg.ui.palette.to_string())
    }

    fn validate_name(name: &str) -> Result<()> {
        validate_palette_name(name)
    }

    fn validate_file(path: &Path) -> Result<()> {
        validate_palette_file(path)
    }

    fn not_a_file_error(path: &Path) -> Error {
        Error::config(format!("Palette path '{}' is not a file", path.display()))
    }

    fn hints(message: &str) -> Vec<&'static str> {
        palette_validation_hints(message)
    }
}

/// Validate a discovered palette by name.
///
/// # Errors
///
/// Returns an error when the palette cannot be loaded or validated.
fn validate_palette_name(palette_name: &str) -> Result<()> {
    PaletteManager::new(palette_name)?;
    Ok(())
}

/// Validate a concrete palette TOML file by path.
///
/// `ColorSpec` deserialization validates each color value; `PaletteConfig::validate`
/// then confirms every palette reference resolves to a concrete color.
///
/// # Errors
///
/// Returns an error when file reading or TOML parsing fails.
fn validate_palette_file(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| Error::config(format!("Failed to read palette file: {e}")))?;
    let palette: crate::palette::PaletteConfig =
        toml::from_str(&content).map_err(|e| Error::config(format!("TOML parsing failed: {e}")))?;
    palette.validate()
}

/// Import a base16/base24 scheme file into a GPY user palette.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the scheme cannot be parsed,
/// or the destination cannot be written. Without `--force`, also errors when
/// the name matches a builtin or plugin palette (the user file would shadow
/// it) or the destination already exists.
pub fn import(file: &Path, name_override: Option<&str>, force: bool) -> Result<()> {
    let input = std::fs::read_to_string(file)
        .map_err(|e| Error::config(format!("Failed to read scheme file: {e}")))?;
    let mut palette =
        crate::import::base16::import_scheme(&input).map_err(|e| Error::config(e.to_string()))?;
    if let Some(name) = name_override {
        name.clone_into(&mut palette.name);
    }
    let stem = palette.name.clone();
    if !config::types::is_safe_config_name(&stem) {
        return Err(Error::config(format!(
            "Invalid palette name derived from scheme: '{stem}'"
        )));
    }
    if !force
        && let Some(provider) = config::discovery::shadowed_provider(
            &PaletteManager::discover_available_palettes(),
            &stem,
        )
    {
        return Err(Error::config(format!(
            "'{stem}' is a {provider} palette; importing would shadow it. Pass --name <other>, or --force to override"
        )));
    }
    let dest = PaletteManager::palette_path(&stem);
    if dest.exists() && !force {
        return Err(Error::config(format!(
            "Palette '{stem}' already exists at {}; pass --force to overwrite",
            dest.display()
        )));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::config(format!("Failed to create palettes dir: {e}")))?;
    }
    let toml = toml::to_string_pretty(&palette)
        .map_err(|e| Error::config(format!("Failed to serialize palette: {e}")))?;
    std::fs::write(&dest, toml)
        .map_err(|e| Error::config(format!("Failed to write palette: {e}")))?;
    println!("✅ Imported palette '{stem}' → {}", dest.display());
    Ok(())
}

fn palette_validation_hints(message: &str) -> Vec<&'static str> {
    let lowered = message.to_ascii_lowercase();
    let mut hints = Vec::new();

    if lowered.contains("invalid color") {
        hints.push("Use named colors, ANSI 0-255, or #RRGGBB values.");
    }
    if lowered.contains("toml parsing failed") {
        hints.push("Run `gpy palette validate <palette-file>` after fixing TOML syntax.");
    }
    if lowered.contains("palette path") && lowered.contains("not a file") {
        hints.push("Pass a palette name or a direct path to a .toml file.");
    }
    if lowered.contains("invalid palette name") || lowered.contains("no palette named") {
        hints.push("Confirm the palette is in ~/.config/gpy/palettes or is a known builtin.");
    }
    if hints.is_empty() {
        hints.push("Re-run `gpy palette validate` after correcting the reported field.");
    }
    hints
}

// Validating the builtin `default` palette is tested in
// `tests/palette_manager_tests.rs` against an empty `XDG_CONFIG_HOME`; here a
// developer's own `palettes/default.toml` would be validated instead (#664).
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    #[test]
    fn validate_target_rejects_unknown() {
        assert!(super::validate_target(Some("no-such-palette-xyz")).is_err());
    }

    #[test]
    fn validate_by_name_rejects_traversal() {
        let err = super::validate_by_name("../../etc/passwd").expect_err("traversal must fail");
        assert!(err.to_string().contains("invalid palette name"), "{err}");
    }
}
