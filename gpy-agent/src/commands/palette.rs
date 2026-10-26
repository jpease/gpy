//! `gpy palette` command handlers.

use super::utils::{
    active_config_path, load_active_config, print_discovered, read_config, reload_agent_and_notify,
};
use super::validation::{self, Failure, FailureKind, ValidatableResource, ValidationReport};
use crate::config::discovery::validate_config_name;
use crate::config::loader::save_config;
use crate::{Error, Result, config, palette::PaletteManager};
use std::path::Path;

/// List available palettes.
///
/// # Errors
/// Returns an error if the active config cannot be read, parsed, or validated.
pub fn list() -> Result<()> {
    let config = read_config()?;
    print_discovered(
        "Available Palettes:",
        config.ui.palette.as_str(),
        &PaletteManager::discover_available_palettes(),
    );
    Ok(())
}

/// Switch to a different palette.
///
/// # Errors
/// Returns an error if the palette doesn't exist or config cannot be saved.
pub fn use_palette(name: &str) -> Result<()> {
    PaletteManager::new(name)?;

    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();
    config.ui.palette =
        config::types::PaletteName::try_from(name.to_owned()).map_err(Error::config)?;
    save_config(&path, &original, &config, &[])?;

    println!("Switched to '{name}' palette");

    reload_agent_and_notify();
    Ok(())
}

/// Show current palette name.
///
/// # Errors
/// Returns an error if the active config cannot be read, parsed, or validated.
pub fn show() -> Result<()> {
    let config = read_config()?;
    println!("{}", config.ui.palette);
    Ok(())
}

/// Validate a palette target: the active palette, a TOML file path, or a name.
///
/// # Errors
/// Returns a short summary error; the detailed diagnostic is already printed.
pub fn validate(target: Option<&str>) -> Result<()> {
    validation::validate::<PaletteResource>(target)
}

/// Validate a palette and return structured information for callers like `doctor`.
///
/// # Errors
/// Returns an error if the palette fails to parse/validate.
pub fn validate_target(target: Option<&str>) -> std::result::Result<ValidationReport, Failure> {
    validation::validate_target::<PaletteResource>(target)
}

/// Validate a palette strictly by name, never treating it as a file path.
///
/// # Errors
/// Returns an error if the named palette cannot be loaded or validated.
pub fn validate_by_name(name: &str) -> std::result::Result<ValidationReport, Failure> {
    validation::validate_by_name::<PaletteResource>(name)
}

/// Print remediation-focused diagnostics for a palette validation error.
pub fn print_palette_validation_error(context: &str, failure: &Failure) {
    validation::print_validation_error::<PaletteResource>(context, failure);
}

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

    fn validate_name(name: &str) -> std::result::Result<(), Failure> {
        validate_palette_name(name)
    }

    fn validate_file(path: &Path) -> std::result::Result<(), Failure> {
        validate_palette_file(path)
    }

    fn not_a_file_error(path: &Path) -> Error {
        Error::config(format!("palette path '{}' is not a file", path.display()))
    }

    fn hints(kind: FailureKind) -> &'static [&'static str] {
        match kind {
            FailureKind::Parse => &[
                "Use named colors, ANSI 0-255, or #RRGGBB values.",
                "Run `gpy palette validate <palette-file>` after fixing TOML syntax.",
            ],
            FailureKind::NotAFile => &["Pass a palette name or a direct path to a .toml file."],
            FailureKind::UnknownName => {
                &["Confirm the palette is in ~/.config/gpy/palettes or is a known builtin."]
            }
            _ => &["Re-run `gpy palette validate` after correcting the reported field."],
        }
    }
}

fn validate_palette_name(palette_name: &str) -> std::result::Result<(), Failure> {
    validate_config_name("palette", palette_name)
        .map_err(|error| Failure::new(FailureKind::UnknownName, error))?;
    let manager = PaletteManager::new(palette_name).map_err(|error| {
        let discovered = PaletteManager::discover_available_palettes()
            .iter()
            .any(|palette| palette.name == palette_name);
        if discovered {
            Failure::from(error)
        } else {
            Failure::new(FailureKind::UnknownName, error)
        }
    })?;
    warn_aliased_keys(&manager.get());
    Ok(())
}

fn validate_palette_file(path: &Path) -> std::result::Result<(), Failure> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| Error::config(format!("failed to read palette file: {e}")))?;
    let palette: crate::palette::PaletteConfig = toml::from_str(&content).map_err(Error::from)?;
    palette.validate()?;
    warn_aliased_keys(&palette);
    Ok(())
}

/// Report keys that alias one ANSI color; legal, but only one answers other spellings.
fn warn_aliased_keys(palette: &crate::palette::PaletteConfig) {
    for warning in palette.alias_warnings() {
        super::output::warn_stderr(format_args!("palette '{}': {warning}", palette.name));
    }
}

/// Import a base16/base24 scheme file into a GPY user palette.
///
/// # Errors
/// Returns an error on read/parse/write failure, or without `force` on name clashes.
pub fn import(file: &Path, name_override: Option<&str>, force: bool) -> Result<()> {
    let input = std::fs::read_to_string(file)
        .map_err(|e| Error::io_context("failed to read scheme file", &e))?;
    let mut palette = crate::import::base16::import_scheme(&input)?;
    if let Some(name) = name_override {
        name.clone_into(&mut palette.name);
    }
    let stem = palette.name.clone();
    validate_config_name("palette", &stem)?;
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
    let already_exists = || {
        Error::config(format!(
            "palette '{stem}' already exists at {}; pass --force to overwrite",
            dest.display()
        ))
    };
    if !force && dest.exists() {
        return Err(already_exists());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::config(format!("failed to create palettes directory: {e}")))?;
    }
    let toml = toml::to_string_pretty(&palette)
        .map_err(|e| Error::config(format!("failed to serialize palette: {e}")))?;
    let written = if force {
        crate::fs_util::write_atomically(&dest, toml.as_bytes())
    } else {
        crate::fs_util::create_new(&dest, toml.as_bytes())
    };
    written.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            already_exists()
        } else {
            Error::config(format!("failed to write palette: {e}"))
        }
    })?;
    println!("Imported palette '{stem}' → {}", dest.display());
    Ok(())
}

// Builtin `default` validation lives in `tests/palette_manager_tests.rs`, where
// a developer's own `palettes/default.toml` cannot shadow it.
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    #[test]
    fn validate_target_rejects_unknown() {
        let err = super::validate_target(Some("no-such-palette-xyz")).expect_err("unknown palette");
        assert_eq!(err.kind(), super::FailureKind::UnknownName);
    }

    #[test]
    fn validate_by_name_rejects_traversal() {
        let err = super::validate_by_name("../../etc/passwd").expect_err("traversal must fail");
        assert_eq!(err.kind(), super::FailureKind::UnknownName);
        assert!(err.to_string().contains("invalid palette name"), "{err}");
    }
}
