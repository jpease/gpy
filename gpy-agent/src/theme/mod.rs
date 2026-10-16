//! Theme loading, Fish export, and hot-reload behind an `Arc<RwLock<ThemeConfig>>`.

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
