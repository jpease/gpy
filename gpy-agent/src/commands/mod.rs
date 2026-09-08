//! Command implementations for the GPY CLI.

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
/// First-run configuration bootstrap (`gpy-agent init`).
pub mod init;
/// Language configuration commands.
pub mod lang;
/// Agent lifecycle management (start/stop/restart).
pub mod lifecycle;
/// Palette management commands.
pub mod palette;
/// Plugin discovery and validation commands.
pub mod plugin;
/// Prompt segment management.
pub mod segments;
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
