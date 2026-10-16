//! `gpy theme` command handlers.

use super::utils::{
    active_config_path, load_active_config, read_config, reload_agent_and_notify, save_config_to,
};
use super::validation::{self, ValidatableResource};
use crate::config::recommended_layout::{
    PendingLayout, pending_layout, resolve_forced_detection, resolve_forced_palette,
    user_set_layout_fields,
};
use crate::{
    Error, Result, config,
    theme::{RecommendedUi, ThemeConfig, ThemeManager, ThemeSource},
};
use std::path::Path;

/// Result of validating a theme target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeValidationResult {
    /// Target label.
    pub target: String,
    /// Active theme, named theme, or explicit file path.
    pub source: String,
}

/// List available themes.
///
/// # Errors
/// Returns an error if the active config cannot be read, parsed, or validated.
pub fn list() -> Result<()> {
    let config = read_config()?;
    let active = &config.ui.theme;

    println!("Available Themes:");
    println!("=================\n");

    let themes = ThemeManager::discover_available_themes();

    for theme in themes {
        let marker = if theme.name == active.as_str() {
            " *"
        } else {
            ""
        };
        let source = match &theme.source {
            ThemeSource::Builtin => "builtin".to_owned(),
            ThemeSource::User => "user".to_owned(),
            ThemeSource::Plugin { plugin_id } => format!("plugin:{plugin_id}"),
        };
        println!("  {}{} [{}]", theme.name, marker, source);
    }

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

    config.ui.theme = config::types::ThemeName::new(name.to_owned())
        .ok_or_else(|| Error::config(format!("Invalid theme name: {name}")))?;

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
        save_config_to(&path, &original, &config, &[])?;

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
    path: &str,
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
    save_config_to(path, original, config, &[])?;

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
pub fn validate_target(target: Option<&str>) -> Result<ThemeValidationResult> {
    let report = validation::validate_target::<ThemeResource>(target)?;
    Ok(ThemeValidationResult {
        target: report.target,
        source: report.source.to_owned(),
    })
}

/// Validate a theme strictly by name, never treating it as a file path.
///
/// # Errors
/// Returns an error if the named theme cannot be loaded or validated.
pub fn validate_by_name(name: &str) -> Result<ThemeValidationResult> {
    let report = validation::validate_by_name::<ThemeResource>(name)?;
    Ok(ThemeValidationResult {
        target: report.target,
        source: report.source.to_owned(),
    })
}

/// Which stage of validating a named theme failed.
#[derive(Debug)]
pub enum ThemeCheckError {
    /// The theme does not exist, is unsafe, or failed to load/parse.
    Load(Error),
    /// The theme loaded but a segment `format` template is invalid.
    Templates(Error),
}

/// Like [`validate_by_name`], but reports which validation stage failed.
///
/// # Errors
/// Returns the [`ThemeCheckError`] stage that failed.
pub fn validate_by_name_detailed(
    name: &str,
) -> std::result::Result<ThemeValidationResult, ThemeCheckError> {
    validate_theme_name_detailed(name)?;
    Ok(ThemeValidationResult {
        target: name.to_owned(),
        source: ThemeResource::NAME_LABEL.to_owned(),
    })
}

/// Print remediation-focused diagnostics for a theme validation error.
pub fn print_theme_validation_error(context: &str, error: &Error) {
    validation::print_validation_error::<ThemeResource>(context, error);
}

/// Create a new theme from the default template or by cloning `from`.
///
/// # Errors
/// Returns an error if `from` cannot be read or the theme file cannot be created.
pub fn new(name: &str, from: Option<&str>) -> Result<()> {
    let _theme_name = config::types::ThemeName::new(name.to_owned())
        .ok_or_else(|| Error::config(format!("Invalid theme name: '{name}'")))?;

    let themes_dir = ThemeManager::user_themes_dir();

    std::fs::create_dir_all(&themes_dir)
        .map_err(|e| Error::config(format!("Failed to create themes directory: {e}")))?;

    let new_theme_path = themes_dir.join(format!("{name}.toml"));

    if new_theme_path.exists() {
        return Err(Error::config(format!("Theme '{name}' already exists")));
    }

    let source_content = match from {
        Some(base) => ThemeManager::theme_source_content(base)?,
        None => ThemeManager::default_theme_template().to_owned(),
    };

    std::fs::write(&new_theme_path, &source_content)
        .map_err(|e| Error::config(format!("Failed to write theme file: {e}")))?;

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
    if !config::types::is_safe_config_name(target_name) {
        return Err(Error::config(format!(
            "invalid theme name '{target_name}': must not contain path separators, '..', or control characters"
        )));
    }

    let source_content = ThemeManager::theme_source_content(&active_theme)?;

    let themes_dir = ThemeManager::user_themes_dir();
    std::fs::create_dir_all(&themes_dir)
        .map_err(|e| Error::config(format!("Failed to create themes directory: {e}")))?;
    let target_path = themes_dir.join(format!("{target_name}.toml"));

    if save_requires_confirmation(name, target_path.exists()) {
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

    std::fs::write(&target_path, &source_content)
        .map_err(|e| Error::config(format!("Failed to write theme file: {e}")))?;

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
        .map_err(|e| Error::config(format!("Failed to write prompt: {e}")))?;

    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| Error::config(format!("Failed to read confirmation: {e}")))?;

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

    fn validate_name(name: &str) -> Result<()> {
        validate_theme_name(name)
    }

    fn validate_file(path: &Path) -> Result<()> {
        validate_theme_file(path)
    }

    fn not_a_file_error(path: &Path) -> Error {
        Error::config(format!("Theme path '{}' is not a file", path.display()))
    }

    fn hints(message: &str) -> Vec<&'static str> {
        theme_validation_hints(message)
    }
}

/// Verify that `theme_name` names a discovered theme.
///
/// [`ThemeManager::new`] falls back to the default theme for unknown names so
/// rendering keeps working; validation must not, or it would validate the
/// default theme and report success for a name that exists nowhere.
fn ensure_theme_discovered(theme_name: &str) -> Result<()> {
    if ThemeManager::discover_available_themes()
        .iter()
        .any(|theme| theme.name == theme_name)
    {
        return Ok(());
    }
    Err(Error::config(format!(
        "No discovered theme named '{theme_name}'"
    )))
}

/// Every theme-name validation entry point (`theme validate`, `doctor`,
/// `config set ui.theme`) funnels through here.
fn validate_theme_name(theme_name: &str) -> Result<()> {
    validate_theme_name_detailed(theme_name).map_err(|failure| match failure {
        ThemeCheckError::Load(e) | ThemeCheckError::Templates(e) => e,
    })
}

fn validate_theme_name_detailed(theme_name: &str) -> std::result::Result<(), ThemeCheckError> {
    // Load first: `ThemeManager::new` gives the more specific name-safety and
    // parse errors; only a name loaded via the default fallback reaches discovery.
    let manager = ThemeManager::new(theme_name).map_err(ThemeCheckError::Load)?;
    ensure_theme_discovered(theme_name).map_err(ThemeCheckError::Load)?;
    let theme = manager.get();
    let config = config::loader::load_config().unwrap_or_default();
    let palette = crate::palette::active_palette(&config);
    crate::config::validation::templates::validate_segment_templates(&theme, &palette)
        .map_err(|e| ThemeCheckError::Templates(Error::config(e.to_string())))?;
    Ok(())
}

fn validate_theme_file(path: &Path) -> Result<()> {
    let theme = config::loader::load_theme_from_path(&path.display().to_string())?;
    let config = config::loader::load_config().unwrap_or_default();
    let palette = crate::palette::active_palette(&config);
    crate::config::validation::templates::validate_segment_templates(&theme, &palette)
        .map_err(|e| Error::config(e.to_string()))?;
    Ok(())
}

fn theme_validation_hints(message: &str) -> Vec<&'static str> {
    let lowered = message.to_ascii_lowercase();
    let mut hints = Vec::new();

    if lowered.contains("invalid color") {
        hints.push("Use named colors, ANSI 0-255, or #RRGGBB values.");
    }
    if lowered.contains("icon") && lowered.contains("empty") {
        hints.push("Set icon fields to a non-empty glyph or remove the key.");
    }
    if lowered.contains("control characters") {
        hints.push("Remove control characters from icon values.");
    }
    if lowered.contains("toml parsing failed") || lowered.contains("parse theme toml") {
        hints.push("Run `gpy theme validate <theme-file>` after fixing TOML syntax.");
    }
    if lowered.contains("theme path") && lowered.contains("not a file") {
        hints.push("Pass a theme name or a direct path to a .toml file.");
    }
    if lowered.contains("invalid theme name") || lowered.contains("no discovered theme") {
        hints.push("Confirm the theme is in ~/.config/gpy/themes or provided by a plugin.");
    }
    if hints.is_empty() {
        hints.push("Re-run `gpy theme validate` after correcting the reported field.");
    }
    hints
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
