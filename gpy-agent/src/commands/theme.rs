//! `gpy theme` command handlers.

use super::utils::{
    active_config_path, load_active_config, print_discovered, read_config, reload_agent_and_notify,
};
use super::validation::{self, Failure, FailureKind, ValidatableResource, ValidationReport};
use crate::config::discovery::validate_config_name;
use crate::config::loader::save_config;
use crate::config::recommended_layout::{
    PendingLayout, pending_layout, resolve_forced_detection, resolve_forced_palette,
    user_set_layout_fields,
};
use crate::{
    Error, Result, config,
    theme::{RecommendedUi, ThemeConfig, ThemeManager},
};
use std::path::Path;

/// List available themes.
///
/// # Errors
/// Returns an error if the active config cannot be read, parsed, or validated.
pub fn list() -> Result<()> {
    let config = read_config()?;
    print_discovered(
        "Available Themes:",
        config.ui.theme.as_str(),
        &ThemeManager::discover_available_themes(),
    );
    Ok(())
}

/// Switch to a different theme.
///
/// With `force`, applies the theme's `[ui.recommended]` settings except fields
/// the user set explicitly; without it, prints a hint listing what would change.
///
/// # Errors
/// Returns an error if the theme doesn't exist or config cannot be saved.
pub fn use_theme(name: &str, force: bool) -> Result<()> {
    let manager = ThemeManager::new(name)?;
    let theme = manager.get();

    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();

    // An on-disk value equal to the outgoing theme's recommendation was written
    // by a prior `--force`, not by hand, so it stays overwritable.
    let outgoing_recommended = ThemeManager::new(config.ui.theme.as_str())
        .ok()
        .and_then(|outgoing| outgoing.get().ui.recommended.clone());

    config.ui.theme = config::types::ThemeName::try_from(name.to_owned()).map_err(Error::config)?;

    let pending = pending_layout(theme.ui.recommended.as_ref(), &config.ui);
    // Detection mode lives in config.language, outside PendingLayout.
    let recommended_detection = theme
        .ui
        .recommended
        .as_ref()
        .and_then(|rec| rec.language_detection)
        .filter(|mode| *mode != config.language.detection_mode);
    let recommended_palette = theme
        .ui
        .recommended
        .as_ref()
        .and_then(|rec| rec.palette.clone());

    if force {
        apply_forced_switch(
            name,
            &original,
            &mut config,
            &path,
            &theme,
            pending,
            recommended_detection,
            recommended_palette,
            outgoing_recommended.as_ref(),
        )?;
    } else {
        validate_prospective_activation(&theme, config.ui.palette.as_str())?;
        save_config(&path, &original, &config, &[])?;

        println!("Switched to '{name}' theme");
        let detection_hint = recommended_detection.map(|_| "language detection".to_owned());
        let parts: Vec<String> = pending
            .field_names()
            .into_iter()
            .chain(detection_hint)
            .collect();
        if !parts.is_empty() {
            println!(
                "Re-run with --force to apply recommended {}",
                parts.join(", ")
            );
        }
    }

    reload_agent_and_notify();
    Ok(())
}

/// The `--force` branch of [`use_theme`].
///
/// # Errors
/// Returns an error if the updated config cannot be saved.
#[expect(
    clippy::too_many_arguments,
    reason = "independent inputs; a params struct would only rename them"
)]
fn apply_forced_switch(
    name: &str,
    original: &config::Config,
    config: &mut config::Config,
    path: &Path,
    theme: &ThemeConfig,
    pending: PendingLayout,
    recommended_detection: Option<config::types::DetectionMode>,
    recommended_palette: Option<config::types::PaletteName>,
    outgoing_recommended: Option<&RecommendedUi>,
) -> Result<()> {
    // Provenance comes from the raw TOML on disk, refined by the outgoing
    // theme's recommendation.
    let user_set = user_set_layout_fields(path);
    let (applied, preserved) =
        pending.partition_preserving(&config.ui, &user_set, outgoing_recommended);
    applied.apply_to(&mut config.ui);
    let (detection_applied, detection_preserved) = resolve_forced_detection(
        config,
        recommended_detection,
        &user_set,
        outgoing_recommended,
    );
    let (palette_applied, palette_preserved) =
        resolve_forced_palette(config, recommended_palette, &user_set, outgoing_recommended);
    validate_prospective_activation(theme, config.ui.palette.as_str())?;
    save_config(path, original, config, &[])?;

    println!("Switched to '{name}' theme");
    print_summary(
        "Applied recommended settings",
        join_summary(applied.summary(), detection_applied),
        palette_applied,
    );
    print_summary(
        "Preserved your explicit settings",
        join_summary(preserved.summary(), detection_preserved),
        palette_preserved,
    );
    Ok(())
}
/// Validate prospective theme activation against the prospective active palette.
fn validate_prospective_activation(theme: &ThemeConfig, palette_name: &str) -> Result<()> {
    let palette_mgr = crate::palette::PaletteManager::new(palette_name)?;
    let palette = palette_mgr.get().to_template_palette();
    crate::config::validation::templates::validate_segment_templates(theme, &palette)
        .map_err(|e| Error::config(e.to_string()))?;
    Ok(())
}

fn join_summary(base: Option<String>, extra: Option<String>) -> Option<String> {
    let parts: Vec<String> = base.into_iter().chain(extra).collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

fn print_summary(prefix: &str, base: Option<String>, extra: Option<String>) {
    if let Some(message) = join_summary(base, extra) {
        println!("{prefix}: {message}");
    }
}

/// Show current theme name.
///
/// # Errors
/// Returns an error if the active config cannot be read, parsed, or validated.
pub fn show() -> Result<()> {
    let config = read_config()?;
    println!("{}", config.ui.theme);
    Ok(())
}

/// Validate a theme target: the active theme, a TOML file path, or a theme name.
///
/// # Errors
/// Returns a short summary error; the detailed diagnostic is already printed.
pub fn validate(target: Option<&str>) -> Result<()> {
    validation::validate::<ThemeResource>(target)
}

/// Validate a theme and return structured information for callers like `doctor`.
///
/// # Errors
/// Returns an error if the theme fails to parse/validate.
pub fn validate_target(target: Option<&str>) -> std::result::Result<ValidationReport, Failure> {
    validation::validate_target::<ThemeResource>(target)
}

/// Validate a theme strictly by name, never treating it as a file path.
///
/// # Errors
/// Returns an error if the named theme cannot be loaded or validated.
pub fn validate_by_name(name: &str) -> std::result::Result<ValidationReport, Failure> {
    validation::validate_by_name::<ThemeResource>(name)
}

/// Print remediation-focused diagnostics for a theme validation failure.
pub fn print_theme_validation_error(context: &str, failure: &Failure) {
    validation::print_validation_error::<ThemeResource>(context, failure);
}

/// Create a new theme from the default template or by cloning `from`.
///
/// # Errors
/// Returns an error if `from` cannot be read or the theme file cannot be created.
pub fn new(name: &str, from: Option<&str>) -> Result<()> {
    validate_config_name("theme", name)?;

    let themes_dir = ThemeManager::user_themes_dir();

    std::fs::create_dir_all(&themes_dir)
        .map_err(|e| Error::config(format!("failed to create themes directory: {e}")))?;

    let new_theme_path = themes_dir.join(format!("{name}.toml"));

    let source_content = match from {
        Some(base) => ThemeManager::theme_source_content(base)?,
        None => crate::config::defaults::DEFAULT_THEME_CONTENT.to_owned(),
    };

    crate::fs_util::create_new(&new_theme_path, source_content.as_bytes()).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            Error::config(format!("theme '{name}' already exists"))
        } else {
            Error::config(format!("failed to write theme file: {e}"))
        }
    })?;

    match from {
        Some(base) => println!(
            "Created new theme: {} (cloned from '{base}')",
            new_theme_path.display()
        ),
        None => println!("Created new theme: {}", new_theme_path.display()),
    }
    println!("Edit: {}", new_theme_path.display());
    println!("Activate with: gpy theme use {name}");

    Ok(())
}

/// Save the active theme's contents as `<name>.toml`, or over the active theme.
///
/// Overwriting an existing file, or omitting `name`, prompts for confirmation.
///
/// # Errors
/// Returns an error on unresolvable/unsafe names, prompt failure, or write failure.
pub fn save(name: Option<&str>) -> Result<()> {
    let config = read_config()?;
    let active_theme = config.ui.theme.as_str().to_owned();

    let target_name = name.unwrap_or(active_theme.as_str());
    validate_config_name("theme", target_name)?;

    let source_content = ThemeManager::theme_source_content(&active_theme)?;

    let themes_dir = ThemeManager::user_themes_dir();
    std::fs::create_dir_all(&themes_dir)
        .map_err(|e| Error::config(format!("failed to create themes directory: {e}")))?;
    let target_path = themes_dir.join(format!("{target_name}.toml"));

    let target_exists = target_path.exists();
    if save_requires_confirmation(name, target_exists) {
        let prompt = if name.is_none() {
            format!(
                "Overwrite current theme '{target_name}' with its current configuration? [y/N] "
            )
        } else {
            format!("Theme '{target_name}' already exists. Overwrite? [y/N] ")
        };
        if !confirm(&prompt)? {
            println!("Aborted.");
            return Ok(());
        }
    }

    let written = if target_exists {
        crate::fs_util::write_atomically(&target_path, source_content.as_bytes())
    } else {
        crate::fs_util::create_new(&target_path, source_content.as_bytes())
    };
    written.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            Error::config(format!("theme '{target_name}' already exists"))
        } else {
            Error::config(format!("failed to write theme file: {e}"))
        }
    })?;

    println!("Saved theme: {}", target_path.display());
    println!("Edit: {}", target_path.display());
    if target_name != active_theme {
        println!("Activate with: gpy theme use {target_name}");
    }

    Ok(())
}

/// Omitting `name` always overwrites the theme in use, so it always confirms,
/// even if the active theme is an unmaterialized builtin.
const fn save_requires_confirmation(name: Option<&str>, target_exists: bool) -> bool {
    name.is_none() || target_exists
}

fn confirm(prompt: &str) -> Result<bool> {
    use std::io::Write;

    print!("{prompt}");
    std::io::stdout()
        .flush()
        .map_err(|e| Error::config(format!("failed to write prompt: {e}")))?;

    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| Error::config(format!("failed to read confirmation: {e}")))?;

    Ok(parse_confirmation(&answer))
}

/// Only (case-insensitive) `y`/`yes` confirm.
fn parse_confirmation(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

struct ThemeResource;

impl ValidatableResource for ThemeResource {
    const KIND: &'static str = "Theme";
    const VALIDATE_CONTEXT: &'static str = "theme validate";
    const ACTIVE_LABEL: &'static str = "active config theme";
    const NAME_LABEL: &'static str = "named theme";
    const SHORT_ERROR: &'static str = "theme validation failed";

    fn active_value() -> Result<String> {
        let cfg = config::loader::load_config()?;
        Ok(cfg.ui.theme.to_string())
    }

    fn validate_name(name: &str) -> std::result::Result<(), Failure> {
        validate_theme_name(name)
    }

    fn validate_file(path: &Path) -> std::result::Result<(), Failure> {
        validate_theme_file(path)
    }

    fn not_a_file_error(path: &Path) -> Error {
        Error::config(format!("theme path '{}' is not a file", path.display()))
    }

    fn hints(kind: FailureKind) -> &'static [&'static str] {
        match kind {
            FailureKind::Color => &["Use named colors, ANSI 0-255, or #RRGGBB values."],
            FailureKind::IconControlChars => &["Remove control characters from icon values."],
            FailureKind::Parse => {
                &["Run `gpy theme validate <theme-file>` after fixing TOML syntax."]
            }
            FailureKind::Schema => &[
                "Use named colors, ANSI 0-255, or #RRGGBB values.",
                "Remove control characters from icon values.",
            ],
            FailureKind::NotAFile => &["Pass a theme name or a direct path to a .toml file."],
            FailureKind::UnknownName => {
                &["Confirm the theme is in ~/.config/gpy/themes or provided by a plugin."]
            }
            FailureKind::Template => &["Fix the segment `format` template named in the reason."],
            FailureKind::Other => {
                &["Re-run `gpy theme validate` after correcting the reported field."]
            }
        }
    }
}

/// Verify that `theme_name` names a discovered theme.
///
/// [`ThemeManager::new`] falls back to the default theme for unknown names so
/// rendering keeps working; validation must not, or it would validate the
/// default theme and report success for a name that exists nowhere.
fn ensure_theme_discovered(theme_name: &str) -> std::result::Result<(), Failure> {
    if ThemeManager::discover_available_themes()
        .iter()
        .any(|theme| theme.name == theme_name)
    {
        return Ok(());
    }
    Err(Failure::new(
        FailureKind::UnknownName,
        Error::config(format!("no discovered theme named '{theme_name}'")),
    ))
}

/// Tag a theme load `error` by what is wrong with the theme `file`: the theme
/// parser reports syntax and value errors as plain text, so the file is
/// re-read. Without a file, the typed source of `error` decides.
fn classify_load_failure(error: Error, file: Option<&Path>) -> Failure {
    let file_problem = file
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| {
            if toml::from_str::<toml::Table>(&text).is_err() {
                Some(FailureKind::Parse)
            } else if toml::from_str::<ThemeConfig>(&text).is_err() {
                Some(FailureKind::Schema)
            } else {
                None
            }
        });
    match file_problem {
        Some(kind) => Failure::new(kind, error),
        None => Failure::from(error),
    }
}

/// The discovered theme called `name`, if any.
fn discovered_theme(name: &str) -> Option<config::discovery::Discovered> {
    ThemeManager::discover_available_themes()
        .into_iter()
        .find(|theme| theme.name == name)
}

/// Every theme-name validation entry point (`theme validate`, `doctor`,
/// `config set ui.theme`) funnels through here.
fn validate_theme_name(theme_name: &str) -> std::result::Result<(), Failure> {
    validate_config_name("theme", theme_name)
        .map_err(|error| Failure::new(FailureKind::UnknownName, error))?;
    // Load first: `ThemeManager::new` gives the more specific parse errors;
    // only a name loaded via the default fallback reaches discovery.
    let manager =
        ThemeManager::new(theme_name).map_err(|error| match discovered_theme(theme_name) {
            Some(theme) => classify_load_failure(error, theme.path.as_deref()),
            None => Failure::new(FailureKind::UnknownName, error),
        })?;
    ensure_theme_discovered(theme_name)?;
    let theme = manager.get();
    let config = config::loader::load_config().unwrap_or_default();
    let palette = crate::palette::active_palette(&config);
    validate_templates(&theme, &palette)
}

fn validate_theme_file(path: &Path) -> std::result::Result<(), Failure> {
    let theme = config::loader::load_theme_from_path(path)
        .map_err(|error| classify_load_failure(error, Some(path)))?;
    let config = config::loader::load_config().unwrap_or_default();
    let palette = crate::palette::active_palette(&config);
    validate_templates(&theme, &palette)
}

fn validate_templates(
    theme: &ThemeConfig,
    palette: &crate::template::Palette,
) -> std::result::Result<(), Failure> {
    crate::config::validation::templates::validate_segment_templates(theme, palette)
        .map_err(|error| Failure::new(FailureKind::Template, Error::from(error)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{parse_confirmation, save_requires_confirmation};

    #[test]
    fn no_confirmation_needed_for_new_named_theme() {
        assert!(!save_requires_confirmation(Some("mytheme"), false));
    }

    #[test]
    fn confirmation_needed_when_named_theme_already_exists() {
        assert!(save_requires_confirmation(Some("mytheme"), true));
    }

    #[test]
    fn confirmation_always_needed_when_name_omitted() {
        assert!(save_requires_confirmation(None, false));
        assert!(save_requires_confirmation(None, true));
    }

    #[test]
    fn parses_yes_variants_as_confirmed() {
        assert!(parse_confirmation("y"));
        assert!(parse_confirmation("Y"));
        assert!(parse_confirmation("yes"));
        assert!(parse_confirmation("YES"));
        assert!(parse_confirmation("  y  \n"));
    }

    #[test]
    fn parses_everything_else_as_declined() {
        assert!(!parse_confirmation("n"));
        assert!(!parse_confirmation(""));
        assert!(!parse_confirmation("\n"));
        assert!(!parse_confirmation("maybe"));
    }
}
