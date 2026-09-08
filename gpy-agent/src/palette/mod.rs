//! Palette (color theme) loading, discovery, and management.
//!
//! A palette is a named `color-name -> color-value` map (`[colors]` table). It
//! owns *what colors exist*; the prompt theme owns *where they are used*.
//!
//! # How to add a builtin palette
//!
//! 1. Add `config/palettes/<name>.toml` with a `[colors]` table.
//! 2. Embed it via `include_str!` in `config/defaults.rs`.
//! 3. Register it in `PaletteManager`'s `BUILTIN_PALETTES` table.

pub mod cache;
pub mod config;
pub mod manager;
pub mod resolve;

pub use cache::PaletteCache;
pub use config::PaletteConfig;
pub use manager::{DiscoveredPalette, PaletteManager, PaletteSource};
pub use resolve::active_palette;
