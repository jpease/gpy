//! Theme management with hot-reload support
//!
//! This module provides centralized theme management for GPY, including:
//! - Loading themes from TOML files
//! - Exporting themes to Fish shell format
//! - Hot-reloading themes when files change
//! - Thread-safe theme access for concurrent requests
//!
//! # Architecture
//!
//! The `ThemeManager` wraps a `ThemeConfig` in an `Arc<RwLock<>>` to enable:
//! - Concurrent read access for IPC requests
//! - Exclusive write access for hot-reload updates
//!
//! # File Watching
//!
//! When enabled, the theme manager watches `~/.config/gpy/themes/*.toml` for changes.
//! File events are debounced (5s default) to batch rapid editor saves.
//!
//! # Export Format
//!
//! The `export_fish()` method converts theme TOML to Fish shell variables:
//! ```fish
//! set -g __gpy_theme_name "default"
//! set -g __icon_prompt "❯"
//! set -g __prompt_color "cyan"
//! ```
//!
//! These variables are sourced by `conf.d/gpy.fish` to render the prompt.

pub mod export;
pub mod manager;
pub mod model;
mod parse;

pub use manager::{ThemeManager, ThemeSource};
pub use model::{
    CharacterTheme, ClockTheme, DirectoryTheme, DurationTheme, GitState, GitTextElement, GitTheme,
    HostnameTheme, PluginSegmentTheme, RecommendedDirectory, RecommendedUi, SegmentThemes,
    StatusTheme, ThemeConfig, UiTheme, UsernameTheme,
};
pub use parse::parse;
