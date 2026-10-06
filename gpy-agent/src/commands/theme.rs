//! `gpy theme` command handlers.
//!
//! Theme commands list, select, inspect, create, and validate prompt themes.
//! They coordinate [`crate::theme::ThemeManager`] and configuration persistence
//! without owning theme parsing rules directly, keeping CLI presentation
//! separate from the runtime theme loader.

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
    /// Human-readable target label.
    pub target: String,
    /// Source descriptor (active theme, named theme, or explicit file path).
    pub source: String,
}

/// List available themes
///
/// # Errors
///
/// Returns an error if the active config file exists but cannot be read, parsed,
/// or validated.
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
/// When `force` is set and the theme declares `[ui.recommended]` settings that
/// differ from the current config (segment order, directory display, language
/// icons, …), those settings are written into `config.ui` so the preset's
/// intended look takes effect without manual config edits — **except** for
/// fields the user has explicitly configured, which are preserved (see
/// [`crate::config::recommended_layout::PendingLayout::partition_preserving`]
/// for how "explicit" is determined).
/// Without the flag the config layout is left untouched and a hint is printed
/// listing what would change. When the recommendation already matches the config
/// nothing is printed.
///
/// # Errors
///
/// Returns an error if the theme doesn't exist or config cannot be saved.
pub fn use_theme(name: &str, force: bool) -> Result<()> {
    // Validate theme exists and read any recommended settings it declares.
    let manager = ThemeManager::new(name)?;
    let theme = manager.get();

    // Update config
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();

    // The theme active *before* this switch. A recommended field whose on-disk
    // value equals this outgoing theme's recommendation was almost certainly
    // written by a prior `--force` of that theme, not hand-edited by the user,
    // so it stays overwritable. This keeps theme→theme `--force` re-layout clean.
    let outgoing_recommended = ThemeManager::new(config.ui.theme.as_str())
        .ok()
        .and_then(|outgoing| outgoing.get().ui.recommended.clone());

    config.ui.theme = config::types::ThemeName::new(name.to_owned())
        .ok_or_else(|| Error::config(format!("Invalid theme name: {name}")))?;

    // Compute only the settings that would actually change, against the current
    // config, before deciding whether to apply or hint.
    let pending = pending_layout(theme.ui.recommended.as_ref(), &config.ui);
    // The theme's recommended detection mode lives in config.language, so it is
    // handled alongside — but separately from — the `config.ui` PendingLayout.
    let recommended_detection = theme
        .ui
        .recommended
        .as_ref()
        .and_then(|rec| rec.language_detection)
        .filter(|mode| *mode != config.language.detection_mode);
    // The theme's recommended palette similarly lives in config.ui but is handled
    // outside PendingLayout (it is a simple newtype, not a layout field).
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

        println!("✅ Switched to '{name}' theme");
        let detection_hint = recommended_detection.map(|_| "language detection".to_owned());
        let parts: Vec<String> = pending
            .field_names()
            .into_iter()
            .chain(detection_hint)
            .collect();
        if !parts.is_empty() {
            println!(
                "💡 Re-run with --force to apply recommended {}",
                parts.join(", ")
            );
        }
    }

    // Reload agent if running
    reload_agent_and_notify();
    Ok(())
}

/// The `--force` branch of [`use_theme`].
///
/// Applies the theme's recommended layout/detection/palette settings
/// (preserving explicit user edits), saves, and prints the applied/preserved
/// summary, as a self-contained branch of `use_theme`'s behavior.
///
/// # Errors
///
/// Returns an error if the updated config cannot be saved.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter is a distinct piece of --force's decision state (name, config, path, theme, pending layout, three independent recommendation options); grouping them would only move the same eight names into a params struct"
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
    // Split the recommended changes into those we apply and those the user
    // has explicitly set (preserved). Provenance comes from the raw TOML on
    // disk, refined by the outgoing theme's recommendation.
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
    let palette_applied = resolve_forced_palette(config, recommended_palette, user_set.palette);
    validate_prospective_activation(theme, config.ui.palette.as_str())?;
    save_config_to(path, original, config, &[])?;

    println!("✅ Switched to '{name}' theme");
    print_summary(
        "✅ Applied recommended settings",
        join_summary(applied.summary(), detection_applied),
        palette_applied,
    );
    print_summary(
        "💡 Preserved your explicit settings",
        preserved.summary(),
        detection_preserved,
    );
    Ok(())
}
/// Validate prospective theme activation against the prospective active palette.
///
/// # Errors
///
/// Returns an error if the prospective palette cannot be loaded or if theme
/// segment templates fail rendering validation.
fn validate_prospective_activation(theme: &ThemeConfig, palette_name: &str) -> Result<()> {
    let palette_mgr = crate::palette::PaletteManager::new(palette_name)?;
    let palette = palette_mgr.get().to_template_palette();
    crate::config::validation::templates::validate_segment_templates(theme, &palette)
        .map_err(|e| Error::config(e.to_string()))?;
    Ok(())
}

/// Join a base summary with an optional extra fragment using `; `, or `None`
/// when both are empty.
fn join_summary(base: Option<String>, extra: Option<String>) -> Option<String> {
    let parts: Vec<String> = base.into_iter().chain(extra).collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

/// Print `"{prefix}: {summary}"` when the combined summary is non-empty.
fn print_summary(prefix: &str, base: Option<String>, extra: Option<String>) {
    if let Some(message) = join_summary(base, extra) {
        println!("{prefix}: {message}");
    }
}

/// Show current theme name
///
/// # Errors
///
/// Returns an error if the active config file exists but cannot be read, parsed,
/// or validated.
pub fn show() -> Result<()> {
    let config = read_config()?;
    println!("{}", config.ui.theme);
    Ok(())
}

/// Validate a theme target.
///
/// If `target` is omitted, validates the currently configured active theme.
/// If `target` is an existing file path, validates that TOML file directly.
/// Otherwise, treats `target` as a discovered theme name.
///
/// # Errors
///
/// Returns an error when parsing or validation fails. On failure the detailed
/// diagnostic has already been printed once; the returned error is a short,
/// generic summary so the process exits non-zero without duplicating that
/// diagnostic (see `commands::validation::validate`).
pub fn validate(target: Option<&str>) -> Result<()> {
    validation::validate::<ThemeResource>(target)
}

/// Validate a theme and return structured information for callers like `doctor`.
///
/// # Errors
///
/// Returns an error if the theme fails to parse/validate.
pub fn validate_target(target: Option<&str>) -> Result<ThemeValidationResult> {
    let report = validation::validate_target::<ThemeResource>(target)?;
    Ok(ThemeValidationResult {
        target: report.target,
        source: report.source.to_owned(),
    })
}

/// Validate a theme strictly by name, bypassing the CWD-sensitive path resolver.
///
/// Use this when the argument is always a theme name (e.g., from `gpy doctor`)
/// and must not be treated as a file path regardless of the working directory.
///
/// # Errors
///
/// Returns an error if the named theme cannot be loaded or validated.
pub fn validate_by_name(name: &str) -> Result<ThemeValidationResult> {
    let report = validation::validate_by_name::<ThemeResource>(name)?;
    Ok(ThemeValidationResult {
        target: report.target,
        source: report.source.to_owned(),
    })
}

/// Print remediation-focused diagnostics for a theme validation error.
pub fn print_theme_validation_error(context: &str, error: &Error) {
    validation::print_validation_error::<ThemeResource>(context, error);
}

/// Create a new theme, either from the default template or by cloning the
/// full contents of an existing theme named by `from`.
///
/// # Errors
///
/// Returns an error if the theme file cannot be created, or if `from` names a
/// theme that cannot be discovered or read.
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
            "✅ Created new theme: {} (cloned from '{base}')",
            new_theme_path.display()
        ),
        None => println!("✅ Created new theme: {}", new_theme_path.display()),
    }
    println!("📝 Edit: {}", new_theme_path.display());
    println!("🎨 Activate with: gpy theme use {name}");

    Ok(())
}

/// Save the currently active theme's contents as a theme file.
///
/// With `name`, saves to `<name>.toml`: created without prompting if no such
/// theme exists yet, or overwritten after a `[y/N]` confirmation if it does.
/// Without `name`, saves over the currently active theme's own name, always
/// prompting first since that always overwrites "the theme in use".
///
/// # Errors
///
/// Returns an error if the active theme cannot be read/resolved, if `name` is
/// an unsafe theme name, if the confirmation prompt cannot be read, or if the
/// theme file cannot be written.
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

    println!("✅ Saved theme: {}", target_path.display());
    println!("📝 Edit: {}", target_path.display());
    if target_name != active_theme {
        println!("🎨 Activate with: gpy theme use {target_name}");
    }

    Ok(())
}

/// Whether saving to `target_name` (or, when `None`, the active theme) must
/// prompt for confirmation before overwriting.
///
/// Pure decision logic, kept separate from the I/O in [`save`] so it can be
/// tested directly. Omitting `name` always overwrites "the theme in use", so
/// it always requires confirmation regardless of whether a file happens to
/// exist on disk yet (e.g. the active theme may still be an unmaterialized
/// builtin).
const fn save_requires_confirmation(name: Option<&str>, target_exists: bool) -> bool {
    name.is_none() || target_exists
}

/// Prompt on stdout and read a `y`/`N` answer from stdin.
///
/// # Errors
///
/// Returns an error if the prompt cannot be flushed or stdin cannot be read.
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

/// Parse a confirmation answer: only (case-insensitive) `y`/`yes` confirm.
fn parse_confirmation(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Marker type wiring theme validation into the shared
/// [`validation::ValidatableResource`] orchestration.
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

/// Verify that `theme_name` names a theme that exists somewhere — builtin,
/// user, or plugin.
///
/// This is the strict half of a deliberate split (#459). [`ThemeManager::new`]
/// falls back to the default theme for an undiscovered name so a user whose
/// theme file went missing still gets a working prompt; that is right for the
/// *rendering* path and wrong for the *validation* path, where it silently
/// validated the default theme's templates and reported success for a name
/// that exists nowhere. Checking discovery keeps validation honest without
/// touching the runtime fallback (still pinned by
/// `tests/theme_manager_tests.rs::test_theme_manager_new_loads_default_when_missing`),
/// and brings themes in line with `PaletteManager::new`, which already errors
/// on a missing palette name.
///
/// # Errors
///
/// Returns an error when no discovered theme has this name.
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

/// Validate a discovered theme by name.
///
/// Every theme-name validation entry point funnels through here — `validate`,
/// `validate_target`, `validate_by_name`, and therefore `gpy theme validate`,
/// `gpy doctor`, and `gpy config set ui.theme`.
///
/// # Errors
///
/// Returns an error when no theme of this name exists, or when the theme
/// cannot be loaded, validated, or contains a broken segment format template.
fn validate_theme_name(theme_name: &str) -> Result<()> {
    // Load first, then check discovery: `ThemeManager::new` owns the
    // name-safety rejection (path separators, `..`, control characters) and
    // the parse/schema errors for a malformed file, and those are the more
    // specific diagnostics. Only a name that loads cleanly *because* of the
    // default-theme fallback reaches the discovery check.
    let manager = ThemeManager::new(theme_name)?;
    ensure_theme_discovered(theme_name)?;
    let theme = manager.get();
    let config = config::loader::load_config().unwrap_or_default();
    let palette = crate::palette::active_palette(&config);
    crate::config::validation::templates::validate_segment_templates(&theme, &palette)
        .map_err(|e| Error::config(e.to_string()))?;
    Ok(())
}

/// Validate a concrete theme TOML file by path.
///
/// # Errors
///
/// Returns an error when file reading, TOML parsing, theme validation, or
/// segment format template rendering fails.
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
