//! `gpy theme migrate`: rewrite a user theme file for keys removed by breaking
//! changes, moving the ones that became `config.toml` settings.

use super::output;
use super::theme::{print_theme_validation_error, validate_by_name};
use super::theme_migrate_rules::{self, SettingMove};
use super::theme_override::is_active;
use super::utils::{active_config_path, load_active_config, reload_agent_and_notify};
use crate::config::Config;
use crate::config::discovery::validate_config_name;
use crate::config::loader::save_config;
use crate::plugin::{SegmentName, SettingKey, SettingValue};
use crate::theme::user_copy;
use crate::{Error, Result};
use std::path::Path;

/// Rewrite the user theme file `name` for keys removed by breaking changes.
///
/// Settings that moved to `config.toml` are added to `[plugin.<segment>]`
/// without replacing a value already there. The theme file and `config.toml`
/// are copied to `.bak` before either is written, and the result must pass
/// `gpy theme validate`.
///
/// # Errors
/// Returns an error when there is no user theme file `name`, the theme or the
/// config cannot be parsed or written, or the migrated theme still fails validation.
pub fn migrate(name: &str) -> Result<()> {
    validate_config_name("theme", name)?;
    let path = user_copy::user_copy_path(name);
    if !path.is_file() {
        return Err(Error::config(format!(
            "No user theme file for '{name}' to migrate\n  \
             Diagnostic: {} does not exist; builtin themes are always current.\n  \
             Remediation: Run `gpy theme list` to see themes; migrate rewrites files in the themes directory.",
            path.display()
        )));
    }
    let original = std::fs::read_to_string(&path)
        .map_err(|e| Error::io_context(format_args!("read {}", path.display()), &e))?;
    let mut theme: toml_edit::DocumentMut = original.parse().map_err(|e| {
        Error::config(format!(
            "Failed to parse {} as TOML:\n  {e}\n  Remediation: Fix the syntax error, then run `gpy theme migrate {name}` again.",
            path.display()
        ))
    })?;

    let migration = theme_migrate_rules::apply(&mut theme);
    let migrated = theme.to_string();
    if migrated == original {
        for note in &migration.notes {
            output::plain(note);
        }
        output::plain(format!(
            "Nothing to migrate: {} has no keys removed by breaking changes.",
            path.display()
        ));
        output::plain(format!(
            "If it still fails to load, run `gpy theme validate {name}` for the reason."
        ));
        return Ok(());
    }

    let config_path = active_config_path()?;
    let config_before = load_active_config(&config_path)?;
    let mut config_after = config_before.clone();
    let mut notes = migration.notes;
    for moved in &migration.moves {
        notes.push(place_setting(&mut config_after, moved)?);
    }
    let config_changed = config_after.plugin != config_before.plugin;

    output::plain(format!("Migrating {}", path.display()));
    for note in &notes {
        output::detail(note);
    }

    // Both backups exist before either file changes. The config goes first:
    // if the theme write then fails, the settings are merely duplicated.
    let theme_backup = user_copy::copy_aside(&path)?;
    let config_backup = if config_changed && config_path.is_file() {
        Some(user_copy::copy_aside(&config_path)?)
    } else {
        None
    };
    if config_changed {
        save_config(&config_path, &config_before, &config_after, &[])?;
    }
    crate::fs_util::write_atomically(&path, migrated.as_bytes())
        .map_err(|e| Error::io_context(format_args!("write {}", path.display()), &e))?;

    output::plain(format!(
        "Rewrote {} (original kept at {})",
        path.display(),
        theme_backup.display()
    ));
    if config_changed {
        match config_backup {
            Some(backup) => output::plain(format!(
                "Updated {} (original kept at {})",
                config_path.display(),
                backup.display()
            )),
            None => output::plain(format!("Created {}", config_path.display())),
        }
    }

    let validation = validate_by_name(name);
    if config_changed || is_active(name) {
        reload_agent_and_notify();
    }
    match validation {
        Ok(_) => {
            output::ok(format!("Theme '{name}' validates"));
            Ok(())
        }
        Err(failure) => {
            print_theme_validation_error("theme migrate", &failure);
            Err(still_invalid(name, &path, &theme_backup))
        }
    }
}

/// Set `plugin.<segment>.<key>` in `config` unless it already has a value, and
/// describe what happened.
fn place_setting(config: &mut Config, moved: &SettingMove) -> Result<String> {
    let SettingMove {
        since,
        segment,
        key,
        ref value,
    } = *moved;
    let segment_name = SegmentName::new(segment)?;
    let setting = SettingKey::new(key)?;
    let existing = config
        .plugin
        .get(&segment_name)
        .and_then(|table| table.settings.get(&setting));
    Ok(match existing {
        Some(current) if current == value => format!(
            "{since}: `{key}` of [segments.{segment}] is already plugin.{segment}.{key} = {} in config.toml",
            render(current)
        ),
        Some(current) => format!(
            "{since}: dropped `{key}` = {} from [segments.{segment}]; kept plugin.{segment}.{key} = {} from config.toml",
            render(value),
            render(current)
        ),
        None => {
            config
                .plugin
                .entry(segment_name)
                .or_default()
                .settings
                .insert(setting, value.clone());
            format!(
                "{since}: moved `{key}` = {} from [segments.{segment}] to config.toml [plugin.{segment}]",
                render(value)
            )
        }
    })
}

fn render(value: &SettingValue) -> String {
    match value {
        SettingValue::Bool(flag) => flag.to_string(),
        SettingValue::Int(number) => number.to_string(),
        SettingValue::String(text) => format!("{text:?}"),
    }
}

fn still_invalid(name: &str, path: &Path, backup: &Path) -> Error {
    Error::config(format!(
        "Theme '{name}' still does not validate after migrating\n  \
         Diagnostic: The rewrites cover only keys removed by breaking changes; {} has another problem.\n  \
         Remediation: Fix it using the reason above, or restore the original from {}.",
        path.display(),
        backup.display()
    ))
}
