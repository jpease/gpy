//! `gpy lang` command handlers.

use super::utils::{
    active_config_path, load_active_config, reload_agent_and_notify, save_config_to,
};
use crate::Result;

/// Set whether to show language versions.
///
/// # Errors
/// Returns an error if the active config cannot be read, validated, or saved.
pub fn set_versions(enabled: bool) -> Result<()> {
    let path = active_config_path()?;
    let original = load_active_config(&path)?;
    let mut config = original.clone();

    config.language.show_versions = enabled;
    save_config_to(&path, &original, &config, &[])?;

    let status = if enabled { "enabled" } else { "disabled" };
    println!("Language versions {status}");

    reload_agent_and_notify();
    Ok(())
}
