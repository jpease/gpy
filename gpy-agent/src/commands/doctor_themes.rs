//! Doctor check for user theme files that shadow a builtin theme.

use crate::commands::output;
use crate::config;
use crate::theme::manager::BUILTIN_THEMES;
use crate::theme::user_copy;

/// Validate every user theme file that shadows a builtin, other than the active one.
///
/// The active theme is validated (and failed) by the configuration check, so it
/// is listed here without being reloaded. A broken inactive override only warns:
/// it affects nothing until the user switches to it, so it never fails doctor.
pub(super) fn check_theme_overrides(active_theme: Option<&str>) {
    let overrides: Vec<_> = BUILTIN_THEMES
        .iter()
        .filter_map(|(name, _)| user_copy::shadowing_copy(name).map(|path| (*name, path)))
        .collect();
    if overrides.is_empty() {
        return;
    }

    output::blank();
    output::section("Theme overrides");
    for (name, path) in overrides {
        output::step(format_args!(
            "Checking override '{name}' ({})",
            path.display()
        ));
        if active_theme == Some(name) {
            output::plain("Active theme, validated above");
            continue;
        }
        // The load error already carries `user_copy::recovery_hint`.
        match config::loader::load_theme_from_path(&path) {
            Ok(_) => output::ok("Loaded successfully"),
            Err(err) => {
                output::warn("Fails to load (inactive, not blocking)");
                for line in err.to_string().lines() {
                    output::detail(line);
                }
            }
        }
    }
}
