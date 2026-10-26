//! Plugin framework and extensibility types

/// The builtin segment set (single source of truth)
mod builtin;
/// Plugin manifest data models and parser
pub mod manifest;
/// Shared plumbing for validated string newtypes
pub(crate) mod newtype;
/// Plugin discovery and registry model
pub mod registry;
/// Script segment manifests (`entry_type = "script"`)
pub mod script;
/// Typed settings declared by script segments
pub mod settings;

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
    CacheScope, EnvName, Extension, InputName, RESERVED_SETTING_KEYS, ScanName, ScriptPath,
    ScriptSegmentManifest, TIMEOUT_MS_KEY, TTL_SECS_KEY, TimeoutMs, TtlSecs, VariantColorKey,
    VariantColorPart, VariantName, script_segment_manifest_path,
};
pub use settings::{SettingDecl, SettingKey, SettingValue};

#[cfg(test)]
mod schema_tests;
