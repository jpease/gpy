//! `gpy theme import`: convert a `starship.toml` into a GPY palette + theme.
//!
//! Pure translation lives in [`crate::import::starship`]; unsupported Starship
//! constructs become stderr warnings and the import still exits 0.

use crate::commands::output;
use crate::config::discovery::{shadowed_provider, validate_config_name};
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
    pub path: &'a Path,
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
pub fn derive_name(path: &Path, explicit: Option<&str>) -> String {
    if let Some(name) = explicit
        && !name.is_empty()
    {
        return name.to_owned();
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map_or_else(|| "starship-import".to_owned(), str::to_owned)
}

/// Run the import.
///
/// # Errors
/// Returns an error on unreadable/unparseable source, name clashes, or write failure.
pub fn run(opts: &ImportOptions<'_>) -> Result<()> {
    let source_path = opts.path;
    if !source_path.is_file() {
        return Err(Error::config(format!(
            "starship config not found: '{}' (pass the path to a starship.toml)",
            source_path.display()
        )));
    }
    let input = std::fs::read_to_string(source_path).map_err(|error| {
        Error::io_context(
            format_args!("failed to read '{}'", source_path.display()),
            &error,
        )
    })?;
    let model = parse(&input)?;
    let name = derive_name(source_path, opts.name);
    validate_config_name("artifact", &name)?;
    let artifacts = build(&model, &name)?;

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

fn print_stdout(artifacts: &ImportArtifacts) -> Result<()> {
    let palette_toml = artifacts.palette_toml()?;
    let theme_toml = artifacts.theme_toml()?;
    println!("# ---- palette ----");
    println!("{palette_toml}");
    println!("# ---- theme ----");
    println!("{theme_toml}");
    Ok(())
}

/// Write palette + theme files, refusing to overwrite or shadow without `force`.
fn write_files(artifacts: &ImportArtifacts, name: &str, force: bool) -> Result<()> {
    let palette_path = PaletteManager::user_palettes_dir().join(format!("{name}.toml"));
    let theme_path = ThemeManager::user_themes_dir().join(format!("{name}.toml"));

    if !force {
        refuse_shadowing(name)?;
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
                Error::io_context(
                    format_args!("failed to create {}", parent.display()),
                    &error,
                )
            })?;
        }
    }
    let palette_toml = artifacts.palette_toml()?;
    let theme_toml = artifacts.theme_toml()?;
    std::fs::write(&palette_path, &palette_toml).map_err(|error| {
        Error::io_context(
            format_args!("failed to write {}", palette_path.display()),
            &error,
        )
    })?;
    std::fs::write(&theme_path, &theme_toml).map_err(|error| {
        Error::io_context(
            format_args!("failed to write {}", theme_path.display()),
            &error,
        )
    })?;
    Ok(())
}

/// Refuse a `name` matching a builtin or plugin theme/palette: the user file
/// would silently replace it (and the live prompt, if it is active).
fn refuse_shadowing(name: &str) -> Result<()> {
    let theme = shadowed_provider(&ThemeManager::discover_available_themes(), name)
        .map(|provider| format!("{provider} theme"));
    let palette = shadowed_provider(&PaletteManager::discover_available_palettes(), name)
        .map(|provider| format!("{provider} palette"));
    let shadowed: Vec<String> = theme.into_iter().chain(palette).collect();
    if shadowed.is_empty() {
        return Ok(());
    }
    Err(Error::config(format!(
        "'{name}' is a {}; importing would shadow it. Pass --name <other>, or --force to override",
        shadowed.join(" and ")
    )))
}

fn print_activation_hints(artifacts: &ImportArtifacts, name: &str) {
    println!("Imported starship config into palette + theme '{name}'");
    output::detail(format_args!("gpy palette use {name}"));
    output::detail(format_args!("gpy theme use {name}"));
    if !artifacts.segments.is_empty() {
        let quoted: Vec<String> = artifacts
            .segments
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect();
        output::detail(
            "# recommended segment order (paste into config.toml, or re-run with --apply-layout):",
        );
        output::detail(format_args!("enabled_segments = [{}]", quoted.join(", ")));
    }
}

fn apply_layout(segments: &[String]) -> Result<()> {
    let path = crate::commands::utils::active_config_path()?;
    let original = crate::commands::utils::load_active_config(&path)?;
    let mut config = original.clone();
    config.ui.enabled_segments = segments.to_vec();
    crate::config::loader::save_config(&path, &original, &config, &[])?;
    crate::commands::utils::reload_agent_and_notify();
    println!("Applied enabled_segments to {}", path.display());
    Ok(())
}

fn print_warnings(artifacts: &ImportArtifacts) {
    if artifacts.warnings.is_empty() {
        return;
    }
    eprint!("{}", artifacts.warnings.render());
    eprintln!("imported with {} warning(s)", artifacts.warnings.len());
}

#[cfg(test)]
mod tests {

    use super::derive_name;
    use std::path::Path;

    #[test]
    fn derive_name_prefers_explicit() {
        assert_eq!(
            derive_name(Path::new("/x/starship.toml"), Some("nord")),
            "nord"
        );
    }

    #[test]
    fn derive_name_falls_back_to_file_stem() {
        assert_eq!(
            derive_name(Path::new("/x/my-prompt.toml"), None),
            "my-prompt"
        );
    }

    #[test]
    fn derive_name_uses_default_when_no_stem() {
        assert_eq!(derive_name(Path::new(""), None), "starship-import");
    }
}
