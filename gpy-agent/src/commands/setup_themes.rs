//! The `gpy setup` step that retires unedited user copies of builtin themes
//! (`theme::retire`), unless `ui.retire_unmodified_themes` is `false`.

use crate::commands::output;
use crate::commands::utils::read_config;
use crate::theme::ThemeManager;
use crate::theme::retire::{Outcome, retire_unmodified_copies};
use crate::theme::user_copy;

/// Retire unedited copies of builtin themes and report what moved; copies
/// that stay get one quiet line. Never fails: the shell integration is
/// already in place, so problems are warnings.
pub(crate) fn retire_unmodified_themes() {
    match read_config() {
        Ok(config) if !config.ui.retire_unmodified_themes => return,
        Ok(_) => {}
        Err(err) => {
            output::warn(format_args!(
                "Left the theme files alone: the config could not be read ({err})"
            ));
            return;
        }
    }
    let themes_dir = ThemeManager::user_themes_dir();
    let mut kept = Vec::new();
    let mut overlays = Vec::new();
    for copy in retire_unmodified_copies(&themes_dir) {
        match copy.outcome {
            Outcome::Retired(backup) => println!(
                "Retired {}, an unedited copy of the builtin '{}' theme: it is now {} and the builtin applies",
                copy.path.display(),
                copy.name,
                backup
                    .file_name()
                    .unwrap_or(backup.as_os_str())
                    .to_string_lossy()
            ),
            Outcome::Modified if user_copy::is_overlay_file(&copy.path) => {
                overlays.push(format!("{}.toml", copy.name));
            }
            Outcome::Modified => kept.push(format!("{}.toml", copy.name)),
            Outcome::Symlink => kept.push(format!("{}.toml (symlink)", copy.name)),
            Outcome::Failed(err) => output::warn(format_args!(
                "Could not retire {} ({err}); it still overrides the builtin '{}' theme",
                copy.path.display(),
                copy.name
            )),
        }
    }
    if !kept.is_empty() {
        println!(
            "Kept your theme files that override builtins in {}: {}",
            themes_dir.display(),
            kept.join(", ")
        );
    }
    if !overlays.is_empty() {
        println!(
            "Kept your theme overlays of builtins in {}: {}",
            themes_dir.display(),
            overlays.join(", ")
        );
    }
}
