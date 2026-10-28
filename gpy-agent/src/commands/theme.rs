//! `gpy theme` command handlers.

use super::output;
use super::utils::{
    active_config_path, load_active_config, print_discovered_noted, read_config,
    reload_agent_and_notify,
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
    theme::{LoadedTheme, RecommendedUi, ThemeConfig, ThemeManager, user_copy as theme_user_copy},
};
use std::path::Path;

/// List available themes.
///
/// # Errors
/// Returns an error if the active config cannot be read, parsed, or validated.
pub fn list() -> Result<()> {
    let config = read_config()?;
    print_discovered_noted(
        "Available Themes:",
        config.ui.theme.as_str(),
        &ThemeManager::discover_available_themes(),
        overlay_note,
    );
    Ok(())
}

/// The `extends <source> <name>` note for a discovered overlay: the base its
/// chain resolves to today. A file that does not load says so instead.
fn overlay_note(theme: &config::discovery::Discovered) -> Option<String> {
    let path = theme.path.as_deref()?;
    if !theme_user_copy::is_overlay_file(path) {
        return None;
    }
    Some(match config::loader::load_theme_file(path) {
        Ok(loaded) => loaded.chain.iter().nth(1).map_or_else(
            || "overlay".to_owned(),
            |base| format!("extends {} {}", base.source, base.name),
        ),
        Err(_) => "overlay, does not load".to_owned(),
    })
}

/// Switch to a different theme.
///
/// With `force`, applies the theme's `[ui.recommended]` settings except fields
/// the user set explicitly; without it, prints a hint listing what would change.
///
/// # Errors
/// Returns an error if the theme doesn't exist or config cannot be saved.
pub fn use_theme(name: &str, force: bool) -> Result<()> {
    validate_config_name("theme", name)?;
    let loaded = config::loader::load_named_theme(name)?;
    let theme = &loaded.theme;

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
            &loaded,
            pending,
            recommended_detection,
            recommended_palette,
            outgoing_recommended.as_ref(),
        )?;
    } else {
        validate_prospective_activation(&loaded, config.ui.palette.as_str())?;
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
    loaded: &LoadedTheme,
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
    validate_prospective_activation(loaded, config.ui.palette.as_str())?;
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
/// Validate prospective theme activation against the prospective active
/// palette. A template error in an overlay names the file and line that set it.
fn validate_prospective_activation(loaded: &LoadedTheme, palette_name: &str) -> Result<()> {
    let palette_mgr = crate::palette::PaletteManager::new(palette_name)?;
    let palette = palette_mgr.get().to_template_palette();
    crate::config::validation::templates::validate_segment_templates(&loaded.theme, &palette)
        .map_err(|e| loaded.template_error(e))?;
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
    // Load first: the loader gives the more specific parse errors; only a
    // name that loaded reaches discovery.
    let loaded = config::loader::load_named_theme(theme_name).map_err(|error| {
        match discovered_theme(theme_name) {
            Some(theme) => classify_load_failure(error, theme.path.as_deref()),
            None => Failure::new(FailureKind::UnknownName, error),
        }
    })?;
    ensure_theme_discovered(theme_name)?;
    let config = config::loader::load_config().unwrap_or_default();
    let palette = crate::palette::active_palette(&config);
    validate_templates(&loaded, &palette)
}

fn validate_theme_file(path: &Path) -> std::result::Result<(), Failure> {
    let loaded = config::loader::load_theme_file(path)
        .map_err(|error| classify_load_failure(error, Some(path)))?;
    let config = config::loader::load_config().unwrap_or_default();
    let palette = crate::palette::active_palette(&config);
    validate_templates(&loaded, &palette)
}

/// Template validation; an overlay's failure names the file and line that set
/// the failing key.
fn validate_templates(
    loaded: &LoadedTheme,
    palette: &crate::template::Palette,
) -> std::result::Result<(), Failure> {
    crate::config::validation::templates::validate_segment_templates(&loaded.theme, palette)
        .map_err(|error| Failure::new(FailureKind::Template, loaded.template_error(error)))?;
    let top = loaded.chain.top();
    if top.on_disk {
        for warning in crate::theme::overlay::unset_warnings(&top.path) {
            output::warn(warning);
        }
    }
    Ok(())
}
