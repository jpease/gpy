//! Palette (color theme) loading, discovery, and management.
//!
//! A palette defines which colors exist; the prompt theme decides where they are used.

pub mod cache;
pub mod config;
pub mod manager;
pub mod resolve;

pub use cache::PaletteCache;
pub use config::PaletteConfig;
pub use manager::{DiscoveredPalette, PaletteManager, PaletteSource};
pub use resolve::{active_palette, load_active_palette};
