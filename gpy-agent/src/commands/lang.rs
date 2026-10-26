//! `gpy lang` command handlers.

use super::utils::{active_config_path, load_active_config, reload_agent_and_notify};
use crate::Result;
use crate::config::loader::save_config;

/// Set whether to show language versions.
///
/// # Errors
/// Returns an error if the active config cannot be read, validated, or saved.
pub fn set_versions(enabled: bool) -> Result<()> {
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();

    config.language.show_versions = enabled;
    save_config(&path, &original, &config, &[])?;

    let status = if enabled { "enabled" } else { "disabled" };
    println!("Language versions {status}");

    reload_agent_and_notify();
    Ok(())
}
