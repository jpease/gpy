//! Plugin manifest schema and validated plugin identifiers.
//!
//! GPY plugins are described by `plugin.toml` files. This module defines the
//! typed manifest representation, validates public identifiers and versions at
//! parse time, and models plugin-provided segments before discovery registers
//! them with the rest of the system.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A validated, normalized plugin identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct PluginId(String);

impl PluginId {
    /// Create a new `PluginId`, normalizing and validating the input
    ///
    /// # Errors
    /// Returns an error if the ID is empty or contains invalid characters.
    pub fn new(id: &str) -> crate::Result<Self> {
        let normalized = id.trim().to_lowercase();
        if normalized.is_empty() {
            return Err(crate::Error::invalid("Plugin ID cannot be empty"));
        }
        if !normalized
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(crate::Error::invalid(
                "Plugin ID must contain only alphanumeric characters and hyphens",
            ));
        }
        Ok(Self(normalized))
    }

    /// Get the normalized string
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for PluginId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A validated, normalized segment name
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SegmentName(String);

impl SegmentName {
    /// Create a new `SegmentName`, normalizing and validating the input
    ///
    /// # Errors
    /// Returns an error if the name is empty or contains invalid characters.
    pub fn new(name: &str) -> crate::Result<Self> {
        let normalized = name.trim().to_lowercase();
        if normalized.is_empty() {
            return Err(crate::Error::invalid("Segment name cannot be empty"));
        }
        if !normalized
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(crate::Error::invalid(
                "Segment name must contain only alphanumeric characters, underscores, and hyphens",
            ));
        }
        Ok(Self(normalized))
    }

    /// Get the normalized string
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SegmentName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for SegmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Entry type for a plugin
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    /// The plugin provides `segments/<name>.fish` files sourced at prompt init.
    File,
    /// Reserved entry type for a custom-command runtime. Parsed for precise
    /// diagnostics, but not loadable by the current runtime (see
    /// [`Self::is_runtime_supported`]).
    Command,
}

impl EntryType {
    /// Stable lowercase identifier matching the manifest schema.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Command => "command",
        }
    }

    /// Whether the current runtime can load plugins with this entry type.
    ///
    /// Only `file` plugins (sourced `segments/<name>.fish` files) have a runtime
    /// integration today; `command` is reserved and rejected by validation and
    /// discovery so unsupported plugins fail loudly rather than at prompt init
    /// (#175).
    #[must_use]
    pub const fn is_runtime_supported(&self) -> bool {
        matches!(self, Self::File)
    }
}

/// Supported public plugin API versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginApiVersion {
    /// The first public plugin contract.
    #[serde(rename = "v1")]
    V1,
}

impl fmt::Display for PluginApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V1 => write!(f, "v1"),
        }
    }
}

/// A parsed GPY semantic version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct GpyVersion(String);

impl GpyVersion {
    /// Create a new parsed GPY version string.
    ///
    /// # Errors
    /// Returns an error if the value is empty or does not start with `MAJOR.MINOR.PATCH`.
    pub fn new(version: &str) -> crate::Result<Self> {
        let normalized = version.trim();
        if normalized.is_empty() {
            return Err(crate::Error::invalid("Version cannot be empty"));
        }

        let numeric_core = Self::numeric_core(normalized);

        let parts = numeric_core.split('.').collect::<Vec<_>>();
        if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
            return Err(crate::Error::invalid(format!(
                "Version must start with MAJOR.MINOR.PATCH, got '{normalized}'"
            )));
        }
        if parts
            .iter()
            .any(|part| !part.chars().all(|c| c.is_ascii_digit()))
        {
            return Err(crate::Error::invalid(format!(
                "Version core must contain only digits, got '{normalized}'"
            )));
        }

        Ok(Self(normalized.to_owned()))
    }

    /// Get the normalized string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn numeric_parts(&self) -> [u64; 3] {
        let mut parts = Self::numeric_core(&self.0)
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or_default());
        [
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        ]
    }

    fn numeric_core(version: &str) -> &str {
        let without_prerelease = version.split_once('-').map_or(version, |(core, _)| core);
        without_prerelease
            .split_once('+')
            .map_or(without_prerelease, |(core, _)| core)
    }

    /// Compare two versions using their numeric semver core.
    #[must_use]
    pub fn is_greater_than(&self, other: &Self) -> bool {
        self.numeric_parts() > other.numeric_parts()
    }
}

impl<'de> Deserialize<'de> for GpyVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for GpyVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Constraints for plugin compatibility
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CompatibilityConstraints {
    /// Minimum required GPY version
    pub min_gpy_version: Option<GpyVersion>,
}

/// Root structure of a plugin manifest (plugin.toml)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Canonical plugin identifier
    pub id: PluginId,
    /// Human-readable plugin name
    pub name: String,
    /// Plugin version
    pub version: GpyVersion,
    /// Plugin API version
    pub api_version: PluginApiVersion,
    /// Optional plugin description
    pub description: Option<String>,
    /// List of segment names provided by this plugin
    #[serde(default)]
    pub provided_segments: Vec<SegmentName>,
    /// Type of execution entry point
    pub entry_type: EntryType,
    /// Environment compatibility constraints
    #[serde(default)]
    pub compatibility_constraints: Option<CompatibilityConstraints>,
}

impl PluginManifest {
    /// Parse a plugin manifest from a TOML string
    ///
    /// # Errors
    /// Returns an error if the TOML is invalid, missing required fields,
    /// or if validation of strongly typed fields fails.
    pub fn parse(toml_content: &str) -> crate::Result<Self> {
        let manifest: Self = toml::from_str(toml_content)
            .map_err(|e| crate::Error::invalid(format!("Failed to parse plugin manifest: {e}")))?;

        // Ensure provided_segments has no duplicates
        let mut unique_segments = std::collections::HashSet::new();
        for segment in &manifest.provided_segments {
            if !unique_segments.insert(segment.as_str()) {
                return Err(crate::Error::invalid(format!(
                    "Duplicate segment declaration: '{}'",
                    segment.as_str()
                )));
            }
        }

        Ok(manifest)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_plugin_id() {
        assert_eq!(PluginId::new("my-plugin").unwrap().as_str(), "my-plugin");
        assert_eq!(PluginId::new(" My-Plugin ").unwrap().as_str(), "my-plugin");
        assert_eq!(PluginId::new("123-ABC").unwrap().as_str(), "123-abc");
    }

    #[test]
    fn test_invalid_plugin_id() {
        assert!(PluginId::new("").is_err());
        assert!(PluginId::new("  ").is_err());
        assert!(PluginId::new("my plugin").is_err());
        assert!(PluginId::new("my_plugin").is_err());
    }

    #[test]
    fn test_valid_segment_name() {
        assert_eq!(SegmentName::new("my_seg").unwrap().as_str(), "my_seg");
        assert_eq!(SegmentName::new(" MY-seg ").unwrap().as_str(), "my-seg");
    }

    #[test]
    fn test_invalid_segment_name() {
        assert!(SegmentName::new("").is_err());
        assert!(SegmentName::new("my seg").is_err());
        assert!(SegmentName::new("my@seg").is_err());
    }

    #[test]
    fn test_parse_valid_manifest() {
        let toml = r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            description = "A test plugin"
            provided_segments = ["test_seg1", "test-seg2"]
            entry_type = "file"
        "#;
        let manifest = PluginManifest::parse(toml).unwrap();
        assert_eq!(manifest.id.as_str(), "my-test-plugin");
        assert_eq!(manifest.provided_segments.len(), 2);
        assert_eq!(manifest.entry_type, EntryType::File);
        assert_eq!(manifest.api_version, PluginApiVersion::V1);
        assert_eq!(manifest.version.as_str(), "1.0.0");
        assert!(manifest.compatibility_constraints.is_none());
    }

    #[test]
    fn test_parse_missing_required_fields() {
        let toml = r#"
            name = "My Test Plugin"
            version = "1.0.0"
        "#;
        assert!(PluginManifest::parse(toml).is_err());
    }

    #[test]
    fn test_parse_duplicate_segments() {
        let toml = r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["test_seg1", "test_seg1"]
            entry_type = "file"
        "#;
        assert!(PluginManifest::parse(toml).is_err());
    }

    #[test]
    fn entry_type_runtime_support() {
        assert!(EntryType::File.is_runtime_supported());
        assert!(!EntryType::Command.is_runtime_supported());
        assert_eq!(EntryType::File.as_str(), "file");
        assert_eq!(EntryType::Command.as_str(), "command");
    }

    #[test]
    fn test_parse_accepts_command_entry_type_for_diagnostics() {
        // `command` must still parse so discovery can distinguish an unsupported
        // entry type from a malformed manifest (#175).
        let toml = r#"
            id = "cmd-plugin"
            name = "Cmd"
            version = "1.0.0"
            api_version = "v1"
            entry_type = "command"
        "#;
        let manifest = PluginManifest::parse(toml).unwrap();
        assert_eq!(manifest.entry_type, EntryType::Command);
        assert!(!manifest.entry_type.is_runtime_supported());
    }

    #[test]
    fn test_parse_invalid_id() {
        let toml = r#"
            id = "my_invalid_id"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            entry_type = "file"
        "#;
        assert!(PluginManifest::parse(toml).is_err());
    }

    #[test]
    fn test_parse_rejects_unknown_api_version() {
        let toml = r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v2"
            entry_type = "file"
        "#;
        assert!(PluginManifest::parse(toml).is_err());
    }

    #[test]
    fn test_valid_gpy_version() {
        assert_eq!(GpyVersion::new("0.1.0").unwrap().as_str(), "0.1.0");
        assert_eq!(
            GpyVersion::new("1.2.3-beta.1").unwrap().as_str(),
            "1.2.3-beta.1"
        );
    }

    #[test]
    fn test_invalid_gpy_version() {
        assert!(GpyVersion::new("").is_err());
        assert!(GpyVersion::new("1").is_err());
        assert!(GpyVersion::new("1.2").is_err());
        assert!(GpyVersion::new("1.2.x").is_err());
    }
}
