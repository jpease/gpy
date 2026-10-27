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
/// Static checks of script segment files for `gpy plugin validate`.
mod script_lint;
/// Timed trial run of a script segment for `gpy doctor`.
mod script_run;
/// Prompt segment management.
pub mod segments;
/// `gpy setup` — install and wire the per-shell scripts.
pub mod setup;
/// Theme management commands.
pub mod theme;
/// `gpy theme import` (starship.toml converter).
pub mod theme_import;
/// Shared utility functions for commands.
pub mod utils;
/// Shared resolve/validate/report plumbing for theme and palette validation.
pub mod validation;
/// Interactive configuration wizard (`gpy config wizard`).
pub mod wizard;
