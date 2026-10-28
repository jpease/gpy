//! Command implementations for the GPY CLI.

#[cfg(unix)]
mod backup_stamp;
/// Hidden `gpy __complete <kind>` command used by shell completion glue.
pub mod complete;
/// Configuration management commands.
pub mod config;
/// Debugging tools and diagnostics.
pub mod debug;
/// `gpy debug paths` — the Rust half of the cross-shell path parity contract.
pub mod debug_paths;
/// System health checks.
pub mod doctor;
mod doctor_bundled;
mod doctor_themes;
/// Icon-style bootstrap run by `gpy setup`.
pub mod init;
/// Language configuration commands.
pub mod lang;
/// Agent lifecycle management (start/stop/restart).
pub mod lifecycle;
/// Pass/fail/warn markers for CLI status lines.
pub mod output;
/// Palette management commands.
pub mod palette;
/// Plugin discovery and validation commands.
pub mod plugin;
/// Theme files shipped in a plugin's `themes/` directory, for `gpy plugin validate`.
mod plugin_themes;
/// Static checks of script segment files for `gpy plugin validate`.
mod script_lint;
/// Timed trial run of a script segment for `gpy doctor`.
mod script_run;
/// Prompt segment management.
pub mod segments;
/// `gpy setup` — install and wire the per-shell scripts.
pub mod setup;
/// `gpy setup` step that retires unedited copies of builtin themes.
mod setup_themes;
/// Theme management commands.
pub mod theme;
/// `gpy theme import` (starship.toml converter).
pub mod theme_import;
/// `gpy theme migrate`: rewrite keys removed by breaking changes.
pub mod theme_migrate;
/// Versioned rewrite rules for `gpy theme migrate`.
mod theme_migrate_rules;
/// `gpy theme diff` and `gpy theme reset` for user copies of builtin themes.
pub mod theme_override;
/// `gpy theme rebase`: rewrite a modified full copy as an overlay.
pub mod theme_rebase;
/// `gpy theme new` and `gpy theme save`: write overlays, or full copies with `--full`.
pub mod theme_write;
/// Unified line diff for `gpy theme diff`.
mod unified_diff;
/// Shared utility functions for commands.
pub mod utils;
/// Shared resolve/validate/report plumbing for theme and palette validation.
pub mod validation;
/// Interactive configuration wizard (`gpy config wizard`).
pub mod wizard;
