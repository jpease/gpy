//! `gpy config` command handlers.

use super::utils::{
    active_config_path, load_active_config, read_config, reload_agent_and_notify, save_config_to,
};
use crate::{Error, Result, config};
use std::path::PathBuf;
use std::process::Command;

/// Contents of a config file freshly created by `gpy config open`.
const NEW_CONFIG_HEADER: &str = "# GPY configuration\n# Every key is optional; omitted keys use built-in defaults.\n# See docs/user/configuration-reference.md for all available settings.\n";

/// Show one config section, or all sections.
///
/// # Errors
/// Returns an error if the section is invalid or the config cannot be loaded.
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
            // Reuse the parsed config rather than re-reading per section.
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
/// Rendered from the serialized config so new fields appear automatically;
/// headers are TOML comments so the full output parses as one document.
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

/// Get a config value.
///
/// # Errors
/// Returns an error if the key is unknown or the config cannot be loaded.
pub fn get(key: &str) -> Result<()> {
    let config = read_config()?;
    let value = config::metadata::get_config_value(&config, key)?;
    println!("{value}");
    Ok(())
}

/// Set a config value.
///
/// Loads, mutates, and saves the active config file. Load or validation
/// failures abort before any write, leaving the existing file untouched.
///
/// # Errors
/// Returns an error if the key or value is invalid or the config cannot be loaded.
pub fn set(key: &str, value: &str) -> Result<()> {
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();

    config::metadata::set_config_value(&mut config, key, value)?;
    validate_resource_backed_key(key, &config)?;

    let explicit_key = config::metadata::canonical_config_key(key);
    save_config_to(&path, &original, &config, &[explicit_key.as_str()])?;
    println!("Set {key} = {value}");

    reload_agent_and_notify();
    Ok(())
}

/// Filesystem-backed checks that `set_config_value` cannot do: it only
/// validates syntax, so a nonexistent or malformed theme/palette would
/// otherwise be writable.
fn validate_resource_backed_key(key: &str, config: &config::Config) -> Result<()> {
    match key {
        "ui.theme" => validate_theme_selection(config.ui.theme.as_str()),
        "ui.palette" => validate_palette_selection(config.ui.palette.as_str()),
        _ => Ok(()),
    }
}

/// Validate that `name` is a discoverable, parseable palette, printing the
/// `gpy palette validate` diagnostic on failure.
fn validate_palette_selection(name: &str) -> Result<()> {
    let Err(error) = crate::commands::palette::validate_by_name(name) else {
        return Ok(());
    };

    crate::commands::palette::print_palette_validation_error("config set ui.palette", &error);
    Err(Error::config("palette validation failed".to_owned()))
}

/// Validate that `name` is a discoverable, parseable theme, printing the
/// `gpy theme validate` diagnostic on failure.
fn validate_theme_selection(name: &str) -> Result<()> {
    let Err(error) = crate::commands::theme::validate_by_name(name) else {
        return Ok(());
    };

    crate::commands::theme::print_theme_validation_error("config set ui.theme", &error);
    Err(Error::config("theme validation failed".to_owned()))
}

/// Open the config file in the user's editor, creating a comment-only file
/// if none exists.
///
/// # Errors
/// Returns an error if the file cannot be created or the editor fails.
pub fn open() -> Result<()> {
    let config_path = PathBuf::from(active_config_path()?);

    if !config_path.exists() {
        config::loader::write_new_config(&config_path.to_string_lossy(), NEW_CONFIG_HEADER)?;
    }

    launch_editor(&config_path)?;
    println!("Opened config: {}", config_path.display());
    Ok(())
}

/// List all available configuration keys.
pub fn list(filter: Option<&str>) {
    use crate::config::metadata::CONFIG_KEYS;

    println!("Available configuration keys:\n");

    let mut count = 0_i32;
    for meta in CONFIG_KEYS {
        if let Some(f) = filter
            && !meta.key.starts_with(f)
        {
            continue;
        }

        let access = if meta.set.is_some() {
            "read-write"
        } else {
            "read-only "
        };

        println!("  {} [{}]", meta.key, access);
        println!("    {}", meta.description);
        println!();

        count += 1_i32;
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

/// Launch `$VISUAL`, `$EDITOR`, or the platform opener on `config_path`.
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

/// `xdg-open`, except under WSL where `wslview` (from `wslu`) is preferred
/// when on `PATH`, since `xdg-open` is usually not installed there.
#[cfg(all(unix, not(target_os = "macos")))]
fn linux_opener() -> &'static str {
    const WSL_OPENER: &str = "wslview";
    if crate::wsl::is_wsl()
        && std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join(WSL_OPENER).is_file())
        })
    {
        WSL_OPENER
    } else {
        "xdg-open"
    }
}

/// Launch the platform-default file opener for the config path.
fn launch_default_editor(config_path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut cmd = Command::new("open");
        cmd.arg("-t").arg(config_path);
        cmd
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut cmd = Command::new(linux_opener());
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
