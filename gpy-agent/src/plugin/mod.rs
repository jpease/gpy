//! Plugin framework and extensibility types

/// The builtin segment set (single source of truth)
mod builtin;
/// Plugin manifest data models and parser
pub mod manifest;
/// Plugin discovery and registry model
pub mod registry;
/// Script segment manifests (`entry_type = "script"`)
pub mod script;

pub(crate) use builtin::{BUILTIN_ORDER, BuiltinSegment};
pub use manifest::{
    CompatibilityConstraints, EntryType, GpyVersion, PluginApiVersion, PluginId, PluginManifest,
    SegmentName, SegmentTokenCollision, first_token_collision,
};
pub use registry::{
    DiscoveredPlugin, PluginDiagnostic, PluginDiscovery, PluginSource, discover_plugins,
    user_plugins_dir,
};
pub use script::{
    CacheScope, EnvName, Extension, InputName, ScanName, ScriptPath, ScriptSegmentManifest,
    TimeoutMs, TtlSecs, VariantName, script_segment_manifest_path,
};
