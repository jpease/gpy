//! Theme loading, shell export, and hot-reload behind an `Arc<RwLock<Arc<ThemeState>>>`.

pub mod export;
mod locate;
pub mod manager;
pub mod model;
mod parse;
mod released_hashes;
pub mod retire;
pub mod user_copy;

pub use manager::{ThemeManager, ThemeSource};
pub use model::{
    CharacterTheme, ClockTheme, DirectoryTheme, DurationTheme, GitState, GitTextElement, GitTheme,
    PluginSegmentTheme, RecommendedDirectory, RecommendedUi, SegmentThemes, StatusTheme,
    ThemeConfig, UiTheme,
};
pub use parse::parse;
