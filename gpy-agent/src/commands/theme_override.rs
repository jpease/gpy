//! `gpy theme diff` and `gpy theme reset`: inspect or retire a user theme file
//! that shadows a builtin theme or customises one as an overlay (ADR-0012).

use super::output;
use super::unified_diff;
use super::utils::{read_config, reload_agent_and_notify};
use crate::config::discovery::{Source, validate_config_name};
use crate::config::loader::load_named_theme;
use crate::theme::overlay::{load_link, resolve_top};
use crate::theme::{ThemeChain, ThemeManager, normalised_table, parse_table, user_copy};
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// Show how the user file of theme `name` differs from what it builds on.
///
/// - A full copy of a builtin: a text diff of the raw source, `-` lines only in
///   the builtin, `+` lines only in the copy.
/// - An overlay: the keys whose effective value the overlay changes, compared
///   between the merged base and the merged overlay.
///
/// # Errors
/// Returns an error when `name` is neither a builtin theme nor a user overlay,
/// or the file cannot be read or loaded.
pub fn diff(name: &str) -> Result<()> {
    let overlay = check_target(name, "diff")?;
    let Some(path) = existing_user_file(name) else {
        output::plain(format!(
            "No user copy of '{name}': the builtin theme is in use, so there is nothing to compare."
        ));
        return Ok(());
    };
    if overlay {
        return diff_overlay(name, &path);
    }

    let builtin = user_copy::builtin_content(name).ok_or_else(|| not_builtin(name, "diff"))?;
    let copy = std::fs::read_to_string(&path)
        .map_err(|e| Error::io_context(format_args!("read {}", path.display()), &e))?;

    match unified_diff::unified(
        &format!("builtin:{name} (embedded)"),
        &path.display().to_string(),
        builtin,
        &copy,
    ) {
        Some(text) => output::plain(text.trim_end()),
        None => output::plain(format!(
            "{} is identical to the builtin theme '{name}'.",
            path.display()
        )),
    }
    Ok(())
}

/// The keys the overlay `name` changes against its base, both merged results
/// normalised and serialized alike.
fn diff_overlay(name: &str, path: &Path) -> Result<()> {
    let loaded = load_named_theme(name)?;
    let Some(base_link) = loaded.chain.iter().nth(1).cloned() else {
        // Not an overlay after all (the file lost its `extends`): nothing to compare.
        output::plain(format!(
            "{} is a full theme, not an overlay.",
            path.display()
        ));
        return Ok(());
    };
    let base_label = format!("{base_link} (merged)");
    let base = load_link(base_link)?;
    let before = flatten(&normalised_table(&base.theme)?);
    let after = flatten(&normalised_table(&loaded.theme)?);

    let mut lines = Vec::new();
    for (key, old) in &before {
        match after.get(key) {
            Some(new) if new == old => {}
            Some(new) => {
                lines.push(format!("-{key} = {old}"));
                lines.push(format!("+{key} = {new}"));
            }
            None => lines.push(format!("-{key} = {old}")),
        }
    }
    for (key, new) in &after {
        if !before.contains_key(key) {
            lines.push(format!("+{key} = {new}"));
        }
    }
    if lines.is_empty() {
        output::plain(format!(
            "{} changes nothing: it is identical to its base {base_label}.",
            path.display()
        ));
        return Ok(());
    }
    output::plain(format!(
        "--- {base_label}\n+++ {} (merged)\n{}",
        path.display(),
        lines.join("\n")
    ));
    Ok(())
}

/// Every leaf of `table` by dotted path, as TOML value text. An empty table is a leaf.
fn flatten(table: &toml::Table) -> BTreeMap<String, String> {
    fn walk(prefix: &str, table: &toml::Table, out: &mut BTreeMap<String, String>) {
        for (key, value) in table {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match value {
                toml::Value::Table(inner) if !inner.is_empty() => walk(&path, inner, out),
                other => {
                    out.insert(path, other.to_string());
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    walk("", table, &mut out);
    out
}

/// Move the user file of theme `name` aside to `.bak` so what lies below it
/// (a plugin theme or the builtin) takes over, and reload the agent when the
/// file was part of the active theme.
///
/// Refuses, leaving the file in place, when the active theme loads now but
/// would not without the file, wherever the file sits in its chain.
///
/// # Errors
/// Returns an error when `name` is neither a builtin theme nor a user overlay,
/// no user file exists, the move fails, or the active theme would stop loading.
pub fn reset(name: &str) -> Result<()> {
    check_target(name, "reset")?;
    let Some(path) = existing_user_file(name) else {
        return Err(Error::config(format!(
            "No user copy of builtin theme '{name}' to reset\n  \
             Diagnostic: {} does not exist, so the builtin theme is already in use.\n  \
             Remediation: Nothing to do; `gpy theme diff {name}` compares a copy once one exists.",
            user_copy::user_copy_path(name).display()
        )));
    };

    let active = read_config()
        .ok()
        .map(|config| config.ui.theme.as_str().to_owned());
    let before = active.as_deref().map(load_named_theme);
    let in_chain = matches!(&before, Some(Ok(loaded)) if chain_has(&loaded.chain, &path));
    let dependents = overlays_extending(name, &path);

    let backup = user_copy::move_aside(&path)?;
    let after = active.as_deref().map(load_named_theme);
    if let (Some(active_name), Some(Ok(_)), Some(Err(error))) = (&active, &before, &after) {
        let restored = std::fs::rename(&backup, &path);
        let restore_note = match restored {
            Ok(()) => format!("{} was left in place.", path.display()),
            Err(e) => format!(
                "Restoring {} from {} failed ({e}); move it back by hand.",
                path.display(),
                backup.display()
            ),
        };
        return Err(Error::config(format!(
            "Cannot reset theme '{name}': the active theme '{active_name}' would not load without it\n  \
             Diagnostic: {error}\n  \
             Remediation: Run `gpy theme use <other>` first, then reset '{name}'. {restore_note}"
        )));
    }
    let fixed = matches!((&before, &after), (Some(Err(_)), Some(Ok(_))));

    output::plain(format!("Moved {} to {}", path.display(), backup.display()));
    output::plain(afterwards(name));
    if !in_chain && !dependents.is_empty() {
        output::warn(format!(
            "Other theme(s) extend '{name}': {}. They now build on what '{name}' resolves to above.",
            dependents.join(", ")
        ));
    }

    if in_chain || fixed {
        reload_agent_and_notify();
    }
    Ok(())
}

/// What `name` resolves to once its user file is gone.
fn afterwards(name: &str) -> String {
    match resolve_top(name) {
        None => format!("No theme '{name}' remains."),
        Some(link) => match link.source {
            Source::Builtin => format!("The builtin theme '{name}' is in use again."),
            Source::Plugin { plugin_id } => {
                format!("The plugin theme '{name}' from plugin '{plugin_id}' is in use now.")
            }
            Source::User => format!("The user theme {} is in use now.", link.path.display()),
        },
    }
}

/// Whether `chain` has a link that reads the file `path`.
pub(super) fn chain_has(chain: &ThemeChain, path: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let wanted = canonical(path);
    chain
        .iter()
        .any(|link| link.on_disk && canonical(&link.path) == wanted)
}

/// Whether the active theme loads and has `path` in its chain.
pub(super) fn in_active_chain(path: &Path) -> bool {
    read_config().is_ok_and(|config| {
        load_named_theme(config.ui.theme.as_str())
            .is_ok_and(|loaded| chain_has(&loaded.chain, path))
    })
}

/// Whether the active config selects theme `name`; an unreadable config counts as no.
pub(super) fn is_active(name: &str) -> bool {
    read_config().is_ok_and(|config| config.ui.theme.as_str() == name)
}

/// The user theme file for `name`, if it exists.
fn existing_user_file(name: &str) -> Option<std::path::PathBuf> {
    let path = user_copy::user_copy_path(name);
    path.is_file().then_some(path)
}

/// The `extends` name of the theme file `path`, when it has a string one.
fn extends_of(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let table = parse_table(&text, &path.display().to_string()).ok()?;
    table.get("extends")?.as_str().map(str::to_owned)
}

/// Stems of the other user theme files that extend `name`, sorted.
fn overlays_extending(name: &str, own_path: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(ThemeManager::user_themes_dir()) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path != own_path && path.extension().is_some_and(|ext| ext == "toml"))
        .filter(|path| extends_of(path).as_deref() == Some(name))
        .filter_map(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    found.sort();
    found
}

/// Validate that `name` can be diffed or reset: a builtin theme, or a user
/// theme file that is an overlay. Returns whether the user file is an overlay.
/// Any other custom theme is rejected, so a full custom theme is never compared
/// against or moved aside as if it were an override.
fn check_target(name: &str, verb: &str) -> Result<bool> {
    validate_config_name("theme", name)?;
    let overlay = existing_user_file(name)
        .as_deref()
        .and_then(extends_of)
        .is_some();
    if overlay || user_copy::is_builtin(name) {
        Ok(overlay)
    } else {
        Err(not_builtin(name, verb))
    }
}

fn not_builtin(name: &str, verb: &str) -> Error {
    let builtins: Vec<&str> = user_copy::builtin_names().collect();
    Error::config(format!(
        "Cannot {verb} theme '{name}': it is not a builtin theme\n  \
         Diagnostic: Only a user copy of a builtin theme ({}) or a user overlay (a theme with `extends`) has a base to compare with or fall back to.\n  \
         Remediation: Pass one of those names; `gpy theme list` shows every available theme.",
        builtins.join(", ")
    ))
}
