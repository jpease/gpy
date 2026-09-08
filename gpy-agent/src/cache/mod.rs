//! Cache management and policy abstractions
//!
//! This module provides unified caching infrastructure used across GPY agent:
//!
//! - **Policy**: Timing parameters (TTL, cooldown) for different cache types
//! - **Git cache**: Repository status caching (see `git::cache`)
//! - **Language cache**: Version detection caching (see `language::cache`)
//! - **Instant prompt**: Shell-formatted prompt caching for 0ms perceived latency
//!
//! ## Architecture
//!
//! GPY uses three timing mechanisms to balance performance and freshness:
//!
//! 1. **Debounce** (100ms): File watcher batches rapid events
//! 2. **Cooldown** (150ms): Prevents re-querying after recent cache updates
//! 3. **TTL** (30s-24h): Expires stale entries based on cache type
//!
//! See [`crate::cache::policy`] module documentation for detailed timing relationship explanation.

pub(crate) mod bounded;
pub mod instant_prompt;
pub mod policy;
pub mod theme_export;
pub(crate) mod ttl_map;

pub use instant_prompt::InstantPromptCache;
pub use policy::{CachePolicy, CachePolicyBuilder, PolicyDriven};
pub use theme_export::{get_theme_export_cache_path, write_theme_export_cache};
