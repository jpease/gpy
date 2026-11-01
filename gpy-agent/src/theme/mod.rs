//! Theme loading, shell export, and hot-reload behind an `Arc<RwLock<Arc<ThemeState>>>`.

pub mod export;
mod locate;
pub mod manager;
pub mod model;
pub mod overlay;
mod parse;
mod released_hashes;
pub mod retire;
pub mod user_copy;
pub mod warnings;

pub use manager::{ThemeManager, ThemeSource};
pub use model::{
    CharacterTheme, ClockTheme, DirectoryTheme, DurationTheme, GitState, GitTextElement, GitTheme,
    PluginSegmentTheme, RecommendedDirectory, RecommendedUi, SegmentThemes, StatusTheme,
    ThemeConfig, UiTheme, normalise, normalised_table,
};
pub use overlay::{ChainLink, LoadedTheme, ThemeChain};
pub use parse::{FromTableError, RequiredTable, from_table, parse, parse_table};
