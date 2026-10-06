//! Plugin framework and extensibility types

/// The builtin segment set (single source of truth)
mod builtin;
/// Plugin manifest data models and parser
pub mod manifest;
/// Plugin discovery and registry model
pub mod registry;

pub(crate) use builtin::{BUILTIN_ORDER, BuiltinSegment};
pub use manifest::{
    CompatibilityConstraints, EntryType, GpyVersion, PluginApiVersion, PluginId, PluginManifest,
    SegmentName,
};
pub use registry::{
    DiscoveredPlugin, PluginDiagnostic, PluginDiscovery, PluginLoadStatus, PluginSource,
    discover_plugins, user_plugins_dir,
};
