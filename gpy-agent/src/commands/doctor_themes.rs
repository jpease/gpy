//! Doctor check for user theme files that shadow a builtin theme.

use crate::commands::output;
use crate::theme::user_copy;
use std::path::Path;

/// Validate every user theme file that shadows a builtin, other than the active one.
///
/// Overlays (files with `extends`) are listed apart from full copies: an
/// overlay keeps receiving the builtin's changes, so it gets no fork hint, and
/// a full copy is a fork that `gpy theme rebase` converts to an overlay.
///
/// The active theme is validated (and failed) by the configuration check, so it
/// is listed here without being reloaded. A broken inactive override only warns:
/// it affects nothing until the user switches to it, so it never fails doctor.
pub(super) fn check_theme_overrides(active_theme: Option<&str>) {
    let (overlays, copies): (Vec<_>, Vec<_>) = user_copy::builtin_names()
        .filter_map(|name| user_copy::shadowing_copy(name).map(|path| (name, path)))
        .partition(|(_, path)| user_copy::is_overlay_file(path));
    if overlays.is_empty() && copies.is_empty() {
        return;
    }

    output::blank();
    output::section("Theme overrides");
    if !overlays.is_empty() {
        output::plain("Overlays (extend the builtin and keep receiving its changes):");
        for (name, path) in overlays {
            check_override(name, &path, active_theme);
        }
    }
    if !copies.is_empty() {
        output::plain("Full copies (forks that no longer receive builtin changes):");
        for (name, path) in copies {
            check_override(name, &path, active_theme);
            output::detail(format_args!(
                "Run `gpy theme rebase {name}` to turn it into an overlay of the builtin"
            ));
        }
    }
}

/// Check one override file, reporting a chain error for an overlay.
fn check_override(name: &str, path: &Path, active_theme: Option<&str>) {
    output::step(format_args!(
        "Checking override '{name}' ({})",
        path.display()
    ));
    if active_theme == Some(name) {
        output::plain("Active theme, validated above");
        return;
    }
    // A full copy's load error already carries `user_copy::recovery_hint`;
    // an overlay's is the chain error.
    match crate::theme::load::load_theme_file(path) {
        Ok(loaded) if loaded.chain.is_overlay() => {
            output::ok(format_args!("Loaded successfully ({})", loaded.chain));
        }
        Ok(_) => output::ok("Loaded successfully"),
        Err(err) => {
            output::warn("Fails to load (inactive, not blocking)");
            for line in err.to_string().lines() {
                output::detail(line);
            }
        }
    }
}
