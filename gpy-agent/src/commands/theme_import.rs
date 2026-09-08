//! `gpy theme import` command: convert a `starship.toml` into a GPY palette + theme.
//!
//! This module owns all I/O and presentation; pure translation lives in
//! [`crate::import::starship`]. Unsupported Starship constructs become grouped
//! warnings on stderr; the command exits 0 on a successful (if lossy) import.

use crate::config::types::is_safe_config_name;
use crate::import::starship::{ImportArtifacts, build, parse};
use crate::palette::manager::PaletteManager;
use crate::theme::ThemeManager;
use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// Where the import sends its output artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportOutput {
    /// Write palette and theme files to disk (default).
    #[default]
    WriteFiles,
    /// Print both artifacts to stdout instead of writing files.
    Stdout,
}

/// Options for an import run.
#[derive(Debug, Clone)]
pub struct ImportOptions<'a> {
    /// Path to the source `starship.toml`.
    pub path: &'a str,
    /// Optional artifact base name.
    pub name: Option<&'a str>,
    /// Overwrite existing artifacts of the same name.
    pub force: bool,
    /// Where to send the output artifacts.
    pub output: ImportOutput,
    /// Also write the derived `enabled_segments` into the active config.
    pub apply_layout: bool,
}

/// Determine the artifact base name: explicit, else file stem, else fallback.
#[must_use]
pub fn derive_name(path: &str, explicit: Option<&str>) -> String {
    if let Some(name) = explicit
        && !name.is_empty()
    {
        return name.to_owned();
    }
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map_or_else(|| "starship-import".to_owned(), str::to_owned)
}

/// Run the import.
///
/// # Errors
///
/// Returns an error if the source file is missing/unreadable/unparseable, if
/// output files exist without `--force`, or if writing/config-save fails.
pub fn run(opts: &ImportOptions<'_>) -> Result<()> {
    let source_path = Path::new(opts.path);
    if !source_path.is_file() {
        return Err(Error::config(format!(
            "starship config not found: '{}' (pass the path to a starship.toml)",
            opts.path
        )));
    }
    let input = std::fs::read_to_string(source_path)
        .map_err(|error| Error::config(format!("failed to read '{}': {error}", opts.path)))?;
    let model = parse(&input).map_err(|error| Error::config(error.to_string()))?;
    let name = derive_name(opts.path, opts.name);
    if !is_safe_config_name(&name) {
        return Err(Error::config(format!(
            "invalid artifact name '{name}': must not contain path separators, '..', or control characters"
        )));
    }
    let artifacts = build(&model, &name);

    if opts.output == ImportOutput::Stdout {
        print_stdout(&artifacts)?;
    } else {
        write_files(&artifacts, &name, opts.force)?;
        print_activation_hints(&artifacts, &name);
    }

    if opts.apply_layout && !artifacts.segments.is_empty() {
        apply_layout(&artifacts.segments)?;
    }

    print_warnings(&artifacts);
    Ok(())
}

/// Print both artifacts with section headers (for `--stdout`).
///
/// # Errors
///
/// Returns an error if either artifact fails to serialize.
fn print_stdout(artifacts: &ImportArtifacts) -> Result<()> {
    let palette_toml = artifacts
        .palette_toml()
        .map_err(|error| Error::config(error.to_string()))?;
    let theme_toml = artifacts
        .theme_toml()
        .map_err(|error| Error::config(error.to_string()))?;
    println!("# ---- palette ----");
    println!("{palette_toml}");
    println!("# ---- theme ----");
    println!("{theme_toml}");
    Ok(())
}

/// Write palette + theme files, refusing to overwrite without `force`.
///
/// # Errors
///
/// Returns an error if the files already exist without `force`, or if creating
/// directories or writing to disk fails.
fn write_files(artifacts: &ImportArtifacts, name: &str, force: bool) -> Result<()> {
    let palette_path = PaletteManager::user_palettes_dir().join(format!("{name}.toml"));
    let theme_path = ThemeManager::user_themes_dir().join(format!("{name}.toml"));

    if !force {
        let existing: Vec<&PathBuf> = [&palette_path, &theme_path]
            .into_iter()
            .filter(|path| path.exists())
            .collect();
        if !existing.is_empty() {
            let listed: Vec<String> = existing
                .iter()
                .map(|path| path.display().to_string())
                .collect();
            return Err(Error::config(format!(
                "refusing to overwrite existing artifacts (use --force): {}",
                listed.join(", ")
            )));
        }
    }

    for path in [&palette_path, &theme_path] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                Error::config(format!("failed to create {}: {error}", parent.display()))
            })?;
        }
    }
    let palette_toml = artifacts
        .palette_toml()
        .map_err(|error| Error::config(error.to_string()))?;
    let theme_toml = artifacts
        .theme_toml()
        .map_err(|error| Error::config(error.to_string()))?;
    std::fs::write(&palette_path, &palette_toml).map_err(|error| {
        Error::config(format!(
            "failed to write {}: {error}",
            palette_path.display()
        ))
    })?;
    std::fs::write(&theme_path, &theme_toml).map_err(|error| {
        Error::config(format!("failed to write {}: {error}", theme_path.display()))
    })?;
    Ok(())
}

/// Print activation hints + recommended segment order.
fn print_activation_hints(artifacts: &ImportArtifacts, name: &str) {
    println!("✅ Imported starship config into palette + theme '{name}'");
    println!("   gpy theme use {name}");
    println!("   gpy palette use {name}");
    if !artifacts.segments.is_empty() {
        let quoted: Vec<String> = artifacts
            .segments
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect();
        println!(
            "   # recommended segment order (paste into config.toml, or re-run with --apply-layout):"
        );
        println!("   enabled_segments = [{}]", quoted.join(", "));
    }
}

/// Write the derived segment order into the active config and reload the agent.
///
/// # Errors
///
/// Returns an error if resolving the active config path, loading, or saving fails.
fn apply_layout(segments: &[String]) -> Result<()> {
    let path = crate::commands::utils::active_config_path()?;
    let mut config = crate::commands::utils::load_active_config(&path)?;
    config.ui.enabled_segments = segments.to_vec();
    crate::commands::utils::save_config_to(&config, &path)?;
    crate::commands::utils::reload_agent_and_notify();
    println!("✅ Applied enabled_segments to {path}");
    Ok(())
}

/// Print grouped warnings to stderr with a final summary.
fn print_warnings(artifacts: &ImportArtifacts) {
    if artifacts.warnings.is_empty() {
        return;
    }
    eprint!("{}", artifacts.warnings.render());
    eprintln!("imported with {} warning(s)", artifacts.warnings.len());
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::derive_name;

    #[test]
    fn derive_name_prefers_explicit() {
        assert_eq!(derive_name("/x/starship.toml", Some("nord")), "nord");
    }

    #[test]
    fn derive_name_falls_back_to_file_stem() {
        assert_eq!(derive_name("/x/my-prompt.toml", None), "my-prompt");
    }

    #[test]
    fn derive_name_uses_default_when_no_stem() {
        assert_eq!(derive_name("", None), "starship-import");
    }
}
