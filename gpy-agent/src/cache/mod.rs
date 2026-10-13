//! Cache management and policy abstractions.
//!
//! - **Policy**: git status cache timing (TTL, cooldown); see [`policy`] for how
//!   it relates to the watcher debounce.
//! - **Instant prompt**: shell-formatted prompt caching for 0ms perceived latency.
//! - **Theme export**: cached `theme export` output for shell startup.

pub(crate) mod bounded;
pub mod instant_prompt;
pub mod policy;
pub mod theme_export;
pub(crate) mod ttl_map;

pub use instant_prompt::InstantPromptCache;
pub use policy::{CachePolicy, PolicyDriven};
pub use theme_export::{get_theme_export_cache_path, write_theme_export_cache};
