//! `gpy config` command handlers.
//!
//! This module bridges the CLI surface to the typed configuration system. It
//! reads and writes [`crate::config::Config`] through loader and metadata
//! helpers, persists accepted edits, and asks the running agent to reload when
//! user-visible settings change.

use super::utils::{
    active_config_path, load_active_config, read_config, reload_agent_and_notify, save_config_to,
};
use crate::{Error, Result, config};
use std::path::PathBuf;
use std::process::Command;

/// Contents of a config file freshly created by `gpy config open`.
const NEW_CONFIG_HEADER: &str = "# GPY configuration\n# Every key is optional; omitted keys use built-in defaults.\n# See docs/user/configuration-reference.md for all available settings.\n";

/// Show config section or all sections
///
/// # Errors
///
/// Returns an error if the section name is invalid or the active config file
/// exists but cannot be read, parsed, or validated.
pub fn show(section: Option<&str>) -> Result<()> {
    let config = read_config()?;

    match section {
        Some("agent") => show_section(&config, "agent", "Agent")?,
        Some("git") => show_section(&config, "git", "Git")?,
        Some("language") => show_section(&config, "language", "Language")?,
        Some("ui") => show_section(&config, "ui", "UI")?,
        Some(other) => {
            return Err(Error::config(format!(
                "Invalid section: {other}. Valid sections: agent, git, language, ui"
            )));
        }
        None => {
            // Show all sections, reusing the config already parsed above
            // instead of recursing back into `show(Some(...))` — which would
            // re-resolve the active config path and re-parse the file once
            // per section (5 reads total for `show(None)` instead of 1).
            show_section(&config, "agent", "Agent")?;
            println!();
            show_section(&config, "git", "Git")?;
            println!();
            show_section(&config, "language", "Language")?;
            println!();
            show_section(&config, "ui", "UI")?;
        }
    }

    Ok(())
}

/// Print one `Config` section as TOML under a comment header.
///
/// The body is rendered from the serialized [`config::Config`] (sorted keys,
/// scalars before sub-tables, lists as arrays), so every field is shown and a
/// new field cannot be forgotten here. The headers are TOML comments, so the
/// output of `gpy config show` parses as one TOML document.
///
/// # Errors
///
/// Returns an error if the config cannot be serialized to TOML.
fn show_section(config: &config::Config, section: &str, title: &str) -> Result<()> {
    let table = toml::Table::try_from(config)
        .map_err(|e| Error::config(format!("Failed to serialize config to TOML: {e}")))?;
    let mut selected = toml::Table::new();
    if let Some(value) = table.get(section) {
        selected.insert(section.to_owned(), value.clone());
    }
    let body = toml::to_string(&selected)
        .map_err(|e| Error::config(format!("Failed to render config as TOML: {e}")))?;

    let heading = format!("{title} Configuration:");
    println!("# {heading}");
    println!("# {}\n", "=".repeat(heading.chars().count()));
    println!("{}", body.trim_end());
    Ok(())
}

/// Get a config value
///
/// # Errors
///
/// Returns an error if the key is unknown or the active config file exists but
/// cannot be read, parsed, or validated.
pub fn get(key: &str) -> Result<()> {
    let config = read_config()?;
    let value = config::metadata::get_config_value(&config, key)?;
    println!("{value}");
    Ok(())
}

/// Set a config value
///
/// Loads, mutates, and saves the same active config file, so the requested
/// change is written exactly where the agent will read it. A load failure aborts
/// before any write, leaving an existing (possibly malformed) file untouched.
/// Resource-backed keys (`ui.theme`, `ui.palette`) are additionally checked
/// against the filesystem before the write — see
/// [`validate_resource_backed_key`] — so a failed check also leaves the file
/// untouched, the same as a load failure.
///
/// # Errors
///
/// Returns an error if the key is unknown, the value is invalid (including a
/// resource-backed value, such as a theme name, that does not exist or
/// fails to parse), or the active config file exists but cannot be read,
/// parsed, or validated.
pub fn set(key: &str, value: &str) -> Result<()> {
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();

    config::metadata::set_config_value(&mut config, key, value)?;
    validate_resource_backed_key(key, &config)?;

    let explicit_key = config::metadata::canonical_config_key(key);
    save_config_to(&path, &original, &config, &[explicit_key.as_str()])?;
    println!("✅ Set {key} = {value}");

    reload_agent_and_notify();
    Ok(())
}

/// Validate a resource (filesystem)-backed config key's newly set value
/// before it is written to disk.
///
/// `config::metadata::set_config_value` only enforces each field's own type
/// invariants — for `ui.theme` that's `ThemeName::new`, which checks the
/// string is a syntactically safe config name (non-empty, no path
/// separators, no control characters) but never touches the filesystem. That
/// leaves a syntactically-fine but nonexistent or malformed theme name
/// writable through `gpy config set`, even though the same selection made
/// through the setup wizard is caught by `wizard::save::save_to`'s explicit
/// `theme::validate_by_name` call (see that function's doc comment for the
/// full rationale). This closes the same gap for `config set`.
///
/// Expressed as a match over `key` (rather than folding the check into
/// `set_config_value`'s closure) so a future resource-backed key only needs
/// one more arm here — `set_config_value` and its direct callers in
/// `config_tests.rs`/`language_tests.rs` stay pure in-memory mutation with no
/// filesystem dependency.
///
/// # Errors
///
/// Returns an error if `key` names a resource-backed field whose newly set
/// value fails validation.
fn validate_resource_backed_key(key: &str, config: &config::Config) -> Result<()> {
    match key {
        "ui.theme" => validate_theme_selection(config.ui.theme.as_str()),
        "ui.palette" => validate_palette_selection(config.ui.palette.as_str()),
        _ => Ok(()),
    }
}

/// Validate that `name` is a discoverable, parseable palette.
///
/// On failure, prints the same remediation-focused diagnostic
/// `gpy palette validate` uses before returning a short, generic error.
///
/// # Errors
///
/// Returns an error if `name` is not a discovered palette, or if the
/// discovered palette fails to parse or validate.
fn validate_palette_selection(name: &str) -> Result<()> {
    let Err(error) = crate::commands::palette::validate_by_name(name) else {
        return Ok(());
    };

    crate::commands::palette::print_palette_validation_error("config set ui.palette", &error);
    Err(Error::config("palette validation failed".to_owned()))
}

/// Validate that `name` is both a discoverable and a parseable theme.
///
/// `theme::validate_by_name` covers both failure shapes — a name that matches
/// no builtin, user, or plugin theme, and a discovered theme whose file fails
/// TOML parsing, schema validation, or segment template rendering. (Until
/// #459 it only covered the second: `ThemeManager::new` falls back to the
/// default theme for an undiscovered name, so this function had to check
/// `ThemeManager::discover_available_themes` itself first. That check now
/// lives centrally in `theme::validate_theme_name`.)
///
/// On failure, prints the same remediation-focused diagnostic
/// `gpy theme validate` uses (via `print_theme_validation_error`) before
/// returning a short, generic error — mirroring
/// `commands::validation::validate`'s pattern of not duplicating the
/// detailed diagnostic in the propagated error.
///
/// # Errors
///
/// Returns an error if `name` is not a discovered theme, or if the
/// discovered theme fails to parse or validate.
fn validate_theme_selection(name: &str) -> Result<()> {
    let Err(error) = crate::commands::theme::validate_by_name(name) else {
        return Ok(());
    };

    crate::commands::theme::print_theme_validation_error("config set ui.theme", &error);
    Err(Error::config("theme validation failed".to_owned()))
}

/// Open the config file in the user's editor.
///
/// Creates a comment-only config file pointing at the configuration reference
/// if it does not exist yet; defaults are not written out.
///
/// # Errors
///
/// Returns an error if the config path cannot be resolved, the config file cannot
/// be created, or the editor command cannot be launched successfully.
pub fn open() -> Result<()> {
    let config_path = PathBuf::from(active_config_path()?);

    if !config_path.exists() {
        config::loader::write_new_config(&config_path.to_string_lossy(), NEW_CONFIG_HEADER)?;
    }

    launch_editor(&config_path)?;
    println!("Opened config: {}", config_path.display());
    Ok(())
}

/// List all available configuration keys
pub fn list(filter: Option<&str>) {
    use crate::config::metadata::CONFIG_KEYS;

    println!("Available configuration keys:\n");

    let mut count = 0_i32;
    for meta in CONFIG_KEYS {
        // Apply filter if provided
        if let Some(f) = filter
            && !meta.key.starts_with(f)
        {
            continue;
        }

        // Determine if key is read-only
        let access = if meta.set.is_some() {
            "read-write"
        } else {
            "read-only "
        };

        println!("  {} [{}]", meta.key, access);
        println!("    {}", meta.description);
        println!();

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "counts entries in the static config-key metadata table; cannot realistically approach i32::MAX"
        )]
        {
            count += 1_i32;
        }
    }

    if count == 0_i32 {
        if let Some(f) = filter {
            println!("No config keys found matching filter: {f}");
        } else {
            println!("No config keys available.");
        }
    } else {
        println!(
            "Total: {} key{}",
            count,
            if count == 1_i32 { "" } else { "s" }
        );
    }
}

/// Launch the user's preferred editor for the provided config path.
///
/// # Errors
///
/// Returns an error if the configured editor or platform fallback cannot be
/// started successfully.
fn launch_editor(config_path: &std::path::Path) -> Result<()> {
    if let Ok(editor) = std::env::var("VISUAL")
        && !editor.trim().is_empty()
    {
        return launch_shell_editor(&editor, config_path);
    }

    if let Ok(editor) = std::env::var("EDITOR")
        && !editor.trim().is_empty()
    {
        return launch_shell_editor(&editor, config_path);
    }

    launch_default_editor(config_path)
}

/// Launch an editor command through the system shell.
///
/// # Errors
///
/// Returns an error if the shell command cannot be started or if the editor
/// exits with a non-zero status.
fn launch_shell_editor(editor: &str, config_path: &std::path::Path) -> Result<()> {
    let quoted_path = shell_quote(config_path);

    #[cfg(windows)]
    let status = Command::new("cmd")
        .args(["/C", &format!("{editor} {quoted_path}")])
        .status()
        .map_err(|e| Error::process(editor.to_owned(), e.to_string()))?;

    #[cfg(not(windows))]
    let status = Command::new("sh")
        .args(["-c", &format!("{editor} {quoted_path}")])
        .status()
        .map_err(|e| Error::process(editor.to_owned(), e.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(Error::process(
            editor.to_owned(),
            format!("exited with status {status}"),
        ))
    }
}

/// Launch the platform-default file opener for the config path.
///
/// # Errors
///
/// Returns an error if the platform opener cannot be started or if it exits
/// with a non-zero status.
fn launch_default_editor(config_path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut cmd = Command::new("open");
        cmd.arg("-t").arg(config_path);
        cmd
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(config_path);
        cmd
    };

    #[cfg(windows)]
    let mut command = {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", &config_path.to_string_lossy()]);
        cmd
    };

    let program = command.get_program().to_string_lossy().into_owned();
    let status = command
        .status()
        .map_err(|e| Error::process(program.clone(), e.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(Error::process(
            program,
            format!("exited with status {status}"),
        ))
    }
}

fn shell_quote(path: &std::path::Path) -> String {
    let path_str = path.to_string_lossy();
    format!("'{}'", path_str.replace('\'', "'\"'\"'"))
}
