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
#[cfg(unix)]
mod script_acl;
mod script_ancestry;
/// Typed settings declared by script segments
pub mod settings;

pub(crate) use builtin::{BUILTIN_ORDER, BuiltinSegment};
pub use manifest::{
    CompatibilityConstraints, EntryType, GpyVersion, PluginApiVersion, PluginId, PluginManifest,
    SegmentName, SegmentTokenCollision, first_token_collision,
};
pub use registry::{
    DiscoveredPlugin, PluginDiagnostic, PluginDiscovery, PluginSource, bundled_plugins_dir,
    discover_plugins, user_plugins_dir,
};
pub use script::{
    CacheScope, EnvName, Extension, InputName, RESERVED_SETTING_KEYS, ScanName, ScriptPath,
    ScriptSegmentManifest, TIMEOUT_MS_KEY, TTL_SECS_KEY, TimeoutMs, TtlSecs, VariantColorKey,
    VariantColorPart, VariantName, VariantStyleKey, check_owned_and_not_shared,
    script_segment_manifest_path,
};
pub use settings::{SettingDecl, SettingKey, SettingValue};

/// Segments provided by the plugins `gpy setup` installs under the bundled
/// plugin root (`plugins/` in the release archive). Before 0.7 these were
/// builtin segments, so an upgrade without `gpy setup` silently drops them.
/// Every builtin theme styles them, so a theme section for one is known even
/// before they are installed.
pub const BUNDLED_PLUGIN_NAMES: [&str; 2] = ["username", "hostname"];

#[cfg(test)]
mod schema_tests;
