//! Cache management and policy abstractions.

pub(crate) mod bounded;
pub mod instant_prompt;
pub mod policy;
pub(crate) mod ttl_map;

pub use instant_prompt::InstantPromptCache;
pub use policy::CachePolicy;
