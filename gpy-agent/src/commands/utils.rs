//! Shared helpers for CLI command modules.
//!
//! This module collects small cross-command operations such as saving the typed
//! configuration, signaling the daemon to reload, and formatting common command
//! results. Keeping these helpers here avoids duplicating persistence and reload
//! behavior across individual `gpy` subcommands.

use crate::{Error, Result, config};
use std::path::Path;

/// Resolve the single active configuration path that CLI commands must use for
/// both reading and writing.
///
/// The active path is the highest-priority **existing** config file, matching
/// the discovery order used by [`config::loader::load_config`]
/// (`GPY_CONFIG_PATH`, then XDG, HOME, and `.gpy.toml`). When no config file
/// exists yet, the highest-priority candidate is returned so a new file is
/// created where it will actually be loaded from — never at a lower-priority
/// location that would be shadowed by an active higher-priority source.
///
/// # Errors
///
/// Returns an error only if no candidate path can be constructed at all.
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

/// Load configuration from the active path for a mutating or reading command.
///
/// A missing file yields defaults (a fresh, never-saved config is legitimate),
/// but an existing file that fails to read, parse, or validate propagates its
/// error instead of being silently replaced with defaults. This guarantees a
/// failed mutation leaves the original file byte-for-byte unchanged, because the
/// caller never reaches its save step.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read, parsed, or validated.
pub fn load_active_config(path: &str) -> Result<config::Config> {
    if Path::new(path).exists() {
        config::loader::load_config_from_file(path)
    } else {
        Ok(config::Config::default())
    }
}

/// Save config to the given active path with validation.
///
/// # Errors
///
/// Returns an error if validation fails, directory creation fails, or write fails.
pub fn save_config_to(config: &config::Config, path: &str) -> Result<()> {
    // Use the config loader's save function which validates before writing.
    config::loader::save_config(config, path)
}

/// Load the active configuration for a read-only command.
///
/// Reports the underlying read/parse/validation error for an existing-but-invalid
/// config instead of masking it with defaults, so callers can exit non-zero.
///
/// # Errors
///
/// Returns an error if the active config file exists but cannot be read, parsed,
/// or validated.
pub fn read_config() -> Result<config::Config> {
    let path = active_config_path()?;
    load_active_config(&path)
}

/// The one line a mutating command prints when the running agent did not
/// confirm the reload, so the user knows the change is on disk but not yet
/// live.
const NOT_RELOADED_NOTICE: &str =
    "⚠️  Agent not reloaded; the new config applies the next time it starts.";

/// Reload the running agent's config, printing exactly one notice when the
/// reload was not confirmed (#573).
///
/// Every mutating CLI command calls this rather than
/// [`crate::agent::lifecycle::send_config_reload_command`] directly, so the
/// notice reads the same across all of them. It deliberately does not return a
/// `Result`: the predecessor (`reload_agent`) returned `Ok(())` no matter what
/// the agent replied and every one of its eight call sites discarded that with
/// `let _ =`, which is how a failed reload came to be reported to the user as a
/// plain success. There is nothing here for a caller to handle -- the config is
/// already written; the only outstanding question is whether the daemon picked
/// it up, and the answer is printed.
#[cfg(unix)]
pub fn reload_agent_and_notify() {
    // The lifecycle sender resolves the socket with the canonical resolver --
    // the same one the server binds with. A second copy used to live here and
    // omitted the XDG_CACHE_HOME step, so with XDG_CACHE_HOME set and
    // XDG_RUNTIME_DIR unset it connected to a path nothing listened on and
    // reported success while the running agent never reloaded (#475).
    let outcome = tokio::runtime::Runtime::new()
        .map_err(|e| Error::process("runtime".to_owned(), e.to_string()))
        .and_then(|runtime| {
            runtime.block_on(crate::agent::lifecycle::send_config_reload_command())
        });

    match outcome {
        Ok(true) => {}
        Ok(false) => println!("{NOT_RELOADED_NOTICE}"),
        Err(e) => println!("⚠️  Agent not reloaded: {e}"),
    }
}

/// Reload the running agent's config -- native Windows stub (#284).
///
/// GPY's IPC transport is Unix-socket based and not yet implemented on native
/// Windows, so there is never a daemon to reload; prints the same notice the
/// Unix path prints when no agent is running.
#[cfg(not(unix))]
pub fn reload_agent_and_notify() {
    println!("{NOT_RELOADED_NOTICE}");
}
