//! `gpy lang` command handlers.
//!
//! These commands expose the language prompt configuration in a user-facing
//! form, then persist changes through the shared config utilities. Runtime
//! language detection remains in [`crate::language`]; this module only manages
//! the CLI toggles and reload signaling needed after configuration edits.

use super::utils::{
    active_config_path, load_active_config, reload_agent_and_notify, save_config_to,
};
use crate::Result;

/// Set whether to show language versions
///
/// # Errors
///
/// Returns an error if the active config file exists but cannot be read, parsed,
/// or validated, or if the config cannot be saved.
pub fn set_versions(enabled: bool) -> Result<()> {
    let path = active_config_path()?;
    let mut config = load_active_config(&path)?;

    config.language.show_versions = enabled;
    save_config_to(&config, &path)?;

    let status = if enabled { "enabled" } else { "disabled" };
    println!("✅ Language versions {status}");

    reload_agent_and_notify();
    Ok(())
}
