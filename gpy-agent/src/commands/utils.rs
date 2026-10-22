//! Config load/save, agent binary lookup and agent reload helpers shared by CLI commands.

use crate::{Error, Result, config};
use std::path::{Path, PathBuf};

/// The config path CLI commands read and write: the highest-priority existing
/// config file, else the highest-priority candidate, so a new file is never
/// shadowed by a higher-priority one.
///
/// # Errors
/// Returns an error if no candidate path can be constructed.
#[allow(
    clippy::map_err_ignore,
    reason = "into_string's error is the OsString itself, which is not an Error and cannot be a source"
)]
pub fn active_config_path() -> Result<String> {
    let paths = config::schema::get_config_paths();

    let (write_path, _existing) =
        config::schema::resolve_active_config(&paths).ok_or_else(|| {
            Error::config(
                "Unable to determine a config path from GPY_CONFIG_PATH, XDG_CONFIG_HOME, or HOME"
                    .to_owned(),
            )
        })?;
    write_path
        .into_os_string()
        .into_string()
        .map_err(|_| Error::config("Config path contains invalid UTF-8".to_owned()))
}

/// Load the config at `path`; a missing file yields defaults, but an invalid
/// one is an error, so a failed mutation never overwrites it.
///
/// # Errors
/// Returns an error if the file exists but cannot be read, parsed, or validated.
pub fn load_active_config(path: &str) -> Result<config::Config> {
    if Path::new(path).exists() {
        config::loader::load_config_from_file(path)
    } else {
        Ok(config::Config::default())
    }
}

/// Save the difference between `original` and `updated` to `path`, preserving
/// everything else in the file.
///
/// # Errors
/// Returns an error if validation or the write fails.
pub fn save_config_to(
    path: &str,
    original: &config::Config,
    updated: &config::Config,
    explicit_keys: &[&str],
) -> Result<()> {
    config::loader::save_config(path, original, updated, explicit_keys)
}

/// Load the active configuration; an existing-but-invalid config is an error.
///
/// # Errors
/// Returns an error if the active config exists but cannot be read, parsed, or validated.
pub fn read_config() -> Result<config::Config> {
    let path = active_config_path()?;
    load_active_config(&path)
}

/// The `gpy-agent` installed beside this `gpy`, else `gpy-agent` from `PATH`
/// (split installs).
#[must_use]
pub fn agent_binary() -> PathBuf {
    std::env::current_exe().map_or_else(
        |_| PathBuf::from("gpy-agent"),
        |exe| agent_binary_beside(&exe),
    )
}

fn agent_binary_beside(exe: &Path) -> PathBuf {
    exe.parent()
        .map(|dir| dir.join(format!("gpy-agent{}", std::env::consts::EXE_SUFFIX)))
        .filter(|sibling| sibling.is_file())
        .unwrap_or_else(|| PathBuf::from("gpy-agent"))
}

/// Printed when the running agent did not confirm a reload.
const NOT_RELOADED_NOTICE: &str =
    "Agent not reloaded; the new config applies the next time it starts.";

/// Reload the running agent's config, warning when it was not applied. The
/// config is already written, so there is nothing to return.
#[cfg(unix)]
pub fn reload_agent_and_notify() {
    use crate::agent::lifecycle::ConfigReload;

    let outcome = tokio::runtime::Runtime::new()
        .map_err(|e| Error::process("runtime".to_owned(), e.to_string()))
        .and_then(|runtime| {
            runtime.block_on(crate::agent::lifecycle::send_config_reload_command())
        });

    match outcome {
        Ok(ConfigReload::Applied) => {}
        Ok(ConfigReload::NotRunning) => crate::commands::output::warn(NOT_RELOADED_NOTICE),
        Ok(ConfigReload::Rejected(reason)) => crate::commands::output::warn(format_args!(
            "Agent rejected the new config and kept its previous one: {reason}"
        )),
        Err(e) => crate::commands::output::warn(format_args!("Agent not reloaded: {e}")),
    }
}

/// Native Windows stub; see `crate::ipc::native_windows_unsupported`.
#[cfg(not(unix))]
pub fn reload_agent_and_notify() {
    crate::commands::output::warn(NOT_RELOADED_NOTICE);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn agent_binary_falls_back_to_path_without_a_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("gpy");
        std::fs::write(&exe, "").unwrap();
        assert_eq!(agent_binary_beside(&exe), PathBuf::from("gpy-agent"));
    }

    #[test]
    fn agent_binary_prefers_the_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("gpy");
        let sibling = dir
            .path()
            .join(format!("gpy-agent{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&exe, "").unwrap();
        std::fs::write(&sibling, "").unwrap();
        assert_eq!(agent_binary_beside(&exe), sibling);
    }
}
