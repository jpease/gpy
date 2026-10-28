//! `gpy plugin validate` for the theme files in a plugin's `themes/` directory
//! (ADR-0012 section 3): each loads, merged and validated like `gpy theme
//! validate <file>`, and a plugin overlay may extend only a builtin theme.

use super::output;
use super::theme::validate_target;
use crate::config::discovery::toml_stems;
use crate::{Error, Result};
use std::path::Path;

/// Validate every `*.toml` file of `<root>/themes`, in name order, and return
/// their names. A plugin without a `themes/` directory has none.
///
/// # Errors
/// Returns an error naming the first theme file that does not load or validate.
pub(super) fn validate_plugin_themes(root: &Path) -> Result<Vec<String>> {
    let mut files: Vec<_> = toml_stems(&root.join("themes")).collect();
    files.sort();
    for (_, path) in &files {
        validate_target(Some(&path.to_string_lossy())).map_err(|failure| {
            Error::invalid(format!("plugin theme {}: {failure}", path.display()))
        })?;
    }
    Ok(files.into_iter().map(|(name, _)| name).collect())
}

/// Print the validated theme `names`, if any.
pub(super) fn print_plugin_themes(names: &[String]) {
    if !names.is_empty() {
        output::detail(format_args!("themes: {}", names.join(", ")));
    }
}
