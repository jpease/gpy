//! `gpy config` command handlers.

use super::output;
use super::utils::{active_config_path, load_active_config, read_config, reload_agent_and_notify};
use crate::config::loader::save_config;
use crate::config::plugin_segments::{self, PLUGIN_KEY_PREFIX};
use crate::{Error, Result, config};
use std::process::Command;

/// Contents of a config file freshly created by `gpy config open`.
const NEW_CONFIG_HEADER: &str = "# GPY configuration\n# Every key is optional; omitted keys use built-in defaults.\n# See docs/user/configuration-reference.md for all available settings.\n";

/// Show one config section, or all sections.
///
/// # Errors
/// Returns an error if the section is invalid or the config cannot be loaded.
pub fn show(section: Option<&str>) -> Result<()> {
    const SECTIONS: [(&str, &str); 5] = [
        ("agent", "Agent"),
        ("git", "Git"),
        ("language", "Language"),
        ("ui", "UI"),
        ("plugin", "Plugin segment overrides"),
    ];
    let config = read_config()?;

    match section {
        Some(name) => {
            let (key, title) = SECTIONS
                .iter()
                .find(|(key, _)| *key == name)
                .ok_or_else(|| {
                    Error::config(format!(
                        "invalid section '{name}'; valid sections: agent, git, language, ui, plugin"
                    ))
                })?;
            show_section(&config, key, title)?;
        }
        None => {
            // Reuse the parsed config rather than re-reading per section.
            for (index, (key, title)) in SECTIONS.iter().enumerate() {
                if index > 0 {
                    println!();
                }
                show_section(&config, key, title)?;
            }
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
        .map_err(|e| Error::config(format!("failed to serialize config to TOML: {e}")))?;
    let mut selected = toml::Table::new();
    if let Some(value) = table.get(section) {
        selected.insert(section.to_owned(), value.clone());
    }
    let body = toml::to_string(&selected)
        .map_err(|e| Error::config(format!("failed to render config as TOML: {e}")))?;

    let heading = format!("{title} Configuration:");
    println!("# {heading}");
    println!("# {}\n", output::underline(&heading));
    println!("{}", body.trim_end());
    Ok(())
}

/// Get a config value.
///
/// # Errors
/// Returns an error if the key is unknown or the config cannot be loaded.
pub fn get(key: &str) -> Result<()> {
    let config = read_config()?;
    let value = if key.starts_with(PLUGIN_KEY_PREFIX) {
        plugin_segments::get(&config, &crate::plugin::discover_plugins(), key)?
    } else {
        config::metadata::get_config_value(&config, key)?
    };
    println!("{value}");
    Ok(())
}

/// Set a config value.
///
/// Loads, mutates, and saves the active config file. Load or validation
/// failures abort before any write, leaving the existing file untouched.
/// `plugin.<segment>.<key>` is checked against the installed segment's manifest.
///
/// # Errors
/// Returns an error if the key or value is invalid or the config cannot be loaded.
pub fn set(key: &str, value: &str) -> Result<()> {
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();

    let explicit_key = if key.starts_with(PLUGIN_KEY_PREFIX) {
        plugin_segments::set(&mut config, &crate::plugin::discover_plugins(), key, value)?;
        plugin_segments::canonical_key(key)?
    } else {
        config::metadata::set_config_value(&mut config, key, value)?;
        validate_resource_backed_key(key, &config)?;
        config::metadata::canonical_config_key(key)
    };

    save_config(&path, &original, &config, &[explicit_key.as_str()])?;
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
    let Err(failure) = crate::commands::palette::validate_by_name(name) else {
        return Ok(());
    };

    crate::commands::palette::print_palette_validation_error("config set ui.palette", &failure);
    Err(Error::config("palette validation failed".to_owned()))
}

/// Validate that `name` is a discoverable, parseable theme, printing the
/// `gpy theme validate` diagnostic on failure.
fn validate_theme_selection(name: &str) -> Result<()> {
    let Err(failure) = crate::commands::theme::validate_by_name(name) else {
        return Ok(());
    };

    crate::commands::theme::print_theme_validation_error("config set ui.theme", &failure);
    Err(Error::config("theme validation failed".to_owned()))
}

/// Open the config file in the user's editor, creating a comment-only file
/// if none exists.
///
/// # Errors
/// Returns an error if the file cannot be created or the editor fails.
pub fn open() -> Result<()> {
    let config_path = active_config_path()?;

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::config(format!("failed to create config directory: {e}")))?;
    }
    let created = match crate::fs_util::create_new(&config_path, NEW_CONFIG_HEADER.as_bytes()) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(e) => {
            return Err(Error::config(format!(
                "failed to create config file {}: {e}",
                config_path.display()
            )));
        }
    };
    if created {
        println!("Created config: {}", config_path.display());
    }

    if let Some(program) = launch_editor(&config_path)? {
        println!(
            "Opened {} in the default editor ({program})",
            config_path.display()
        );
    }
    Ok(())
}

/// List all available configuration keys, then the `plugin.<segment>.<key>`
/// overrides the installed script segments accept.
pub fn list(filter: Option<&str>) {
    use crate::config::metadata::CONFIG_KEYS;

    output::heading("Available configuration keys:");

    let core = CONFIG_KEYS.iter().map(|meta| {
        let access = if meta.set.is_some() {
            "read-write"
        } else {
            "read-only"
        };
        (meta.key.to_owned(), access, meta.description.to_owned())
    });
    let plugin = plugin_segments::list_keys(&crate::plugin::discover_plugins())
        .into_iter()
        .map(|(key, description)| (key, "read-write", description));

    let entries: Vec<_> = core
        .chain(plugin)
        .filter(|(key, _, _)| filter.is_none_or(|prefix| key.starts_with(prefix)))
        .collect();

    for (key, access, description) in &entries {
        output::detail(format_args!("{key} [{access:<ACCESS_WIDTH$}]"));
        output::detail_at(2, description);
        println!();
    }

    match (entries.len(), filter) {
        (0, Some(prefix)) => println!("No config keys found matching filter: {prefix}"),
        (0, None) => println!("No config keys available."),
        (1, _) => println!("Total: 1 key"),
        (count, _) => println!("Total: {count} keys"),
    }
}

/// Width of the `read-write`/`read-only` column, so brackets line up.
const ACCESS_WIDTH: usize = "read-write".len();

/// Launch `$VISUAL`, `$EDITOR`, or the platform opener on `config_path`.
/// Returns the opener's program name when the platform opener ran (it returns
/// before the user is done editing), `None` after a blocking editor.
fn launch_editor(config_path: &std::path::Path) -> Result<Option<String>> {
    let configured = ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|editor| !editor.trim().is_empty());
    configured.map_or_else(
        || launch_default_editor(config_path).map(Some),
        |editor| launch_shell_editor(&editor, config_path).map(|()| None),
    )
}

/// Launch an editor command through the system shell.
fn launch_shell_editor(editor: &str, config_path: &std::path::Path) -> Result<()> {
    let quoted_path = shell_quote(config_path);

    #[cfg(windows)]
    let status = Command::new("cmd")
        .args(["/C", &format!("{editor} {quoted_path}")])
        .status()
        .map_err(|e| Error::process(editor, e.to_string()))?;

    #[cfg(not(windows))]
    let status = Command::new("sh")
        .args(["-c", &format!("{editor} {quoted_path}")])
        .status()
        .map_err(|e| Error::process(editor, e.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(Error::process(
            editor,
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
fn launch_default_editor(config_path: &std::path::Path) -> Result<String> {
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
    let status = command.status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::config(format!(
                "no editor found: set $VISUAL or $EDITOR (config file: {})",
                config_path.display()
            ))
        } else {
            Error::process(program.clone(), e.to_string())
        }
    })?;

    if status.success() {
        Ok(program)
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
