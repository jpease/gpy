//! Plugin framework and extensibility types

/// Plugin manifest data models and parser
pub mod manifest;
/// Plugin discovery and registry model
pub mod registry;

pub use manifest::{
    CompatibilityConstraints, EntryType, GpyVersion, PluginApiVersion, PluginId, PluginManifest,
    SegmentName,
};
pub use registry::{
    DiscoveredPlugin, PluginDiagnostic, PluginDiscovery, PluginLoadStatus, PluginSource,
    discover_plugins, user_plugins_dir,
};
