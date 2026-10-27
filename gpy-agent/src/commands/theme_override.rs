//! `gpy theme diff` and `gpy theme reset`: inspect or retire a user theme file
//! that shadows a builtin theme of the same name.

use super::output;
use super::unified_diff;
use super::utils::{read_config, reload_agent_and_notify};
use crate::config::discovery::validate_config_name;
use crate::theme::user_copy;
use crate::{Error, Result};

/// Show how the user copy of builtin theme `name` differs from the current
/// builtin: `-` lines exist only in the builtin, `+` lines only in the copy.
///
/// # Errors
/// Returns an error when `name` is not a builtin theme or the copy cannot be read.
pub fn diff(name: &str) -> Result<()> {
    let builtin = builtin_content(name, "diff")?;
    let Some(path) = user_copy::shadowing_copy(name) else {
        output::plain(format!(
            "No user copy of '{name}': the builtin theme is in use, so there is nothing to compare."
        ));
        return Ok(());
    };
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

/// Move the user copy of builtin theme `name` aside to `.bak` so the builtin
/// takes over, and reload the agent when that theme is the active one.
///
/// # Errors
/// Returns an error when `name` is not a builtin theme, no user copy exists,
/// or the copy cannot be moved.
pub fn reset(name: &str) -> Result<()> {
    builtin_content(name, "reset")?;
    let Some(path) = user_copy::shadowing_copy(name) else {
        return Err(Error::config(format!(
            "No user copy of builtin theme '{name}' to reset\n  \
             Diagnostic: {} does not exist, so the builtin theme is already in use.\n  \
             Remediation: Nothing to do; `gpy theme diff {name}` compares a copy once one exists.",
            user_copy::user_copy_path(name).display()
        )));
    };
    let backup = user_copy::move_aside(&path)?;
    output::plain(format!("Moved {} to {}", path.display(), backup.display()));
    output::plain(format!("The builtin theme '{name}' is in use again."));

    if is_active(name) {
        reload_agent_and_notify();
    }
    Ok(())
}

/// Whether the active config selects theme `name`; an unreadable config counts as no.
pub(super) fn is_active(name: &str) -> bool {
    read_config().is_ok_and(|config| config.ui.theme.as_str() == name)
}

/// The embedded content of builtin `name`; any other name is rejected, so a
/// custom theme is never compared against or moved aside as if it were an override.
fn builtin_content(name: &str, verb: &str) -> Result<&'static str> {
    validate_config_name("theme", name)?;
    user_copy::builtin_content(name).ok_or_else(|| not_builtin(name, verb))
}

fn not_builtin(name: &str, verb: &str) -> Error {
    let builtins: Vec<&str> = user_copy::builtin_names().collect();
    Error::config(format!(
        "Cannot {verb} theme '{name}': it is not a builtin theme\n  \
         Diagnostic: Only a user copy of a builtin theme ({}) has a builtin to compare with or fall back to.\n  \
         Remediation: Pass one of those names; `gpy theme list` shows every available theme.",
        builtins.join(", ")
    ))
}
