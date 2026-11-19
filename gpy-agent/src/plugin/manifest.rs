//! Plugin manifest (`plugin.toml`) schema and validated plugin identifiers.

use super::newtype::{deserialize_via_new, display_inner};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;

/// A validated, normalized plugin identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct PluginId(#[cfg_attr(test, schemars(regex(pattern = r"^[a-z0-9-]+$")))] String);

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

deserialize_via_new!(PluginId, keep_prefix);

display_inner!(PluginId);

/// A validated, normalized segment name
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct SegmentName(#[cfg_attr(test, schemars(regex(pattern = r"^[a-z0-9_-]+$")))] String);

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

    /// The token naming this segment's shell variables (`__gpy_segment_<token>_*`):
    /// `-` becomes `_`, nothing else changes.
    ///
    /// The plugin scaffold applies the same mapping. Distinct names can share a
    /// token (`a-b`, `a_b`); see [`first_token_collision`].
    #[must_use]
    pub fn variable_token(&self) -> String {
        self.0.replace('-', "_")
    }
}

impl From<SegmentName> for String {
    fn from(name: SegmentName) -> Self {
        name.0
    }
}

/// Two distinct segment names that map to the same shell variable token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentTokenCollision {
    /// The name seen first.
    pub first: SegmentName,
    /// The later name that collides with `first`.
    pub second: SegmentName,
}

impl fmt::Display for SegmentTokenCollision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "segments '{}' and '{}' both map to the shell variable token '{}'; rename one of them",
            self.first,
            self.second,
            self.first.variable_token()
        )
    }
}

/// The first pair of distinct names in `names` sharing a [`SegmentName::variable_token`].
///
/// Repeats of one name are not collisions.
pub fn first_token_collision<'a>(
    names: impl IntoIterator<Item = &'a SegmentName>,
) -> Option<SegmentTokenCollision> {
    let mut seen: HashMap<String, &SegmentName> = HashMap::new();
    for name in names {
        match seen.entry(name.variable_token()) {
            Entry::Occupied(entry) if *entry.get() != name => {
                return Some(SegmentTokenCollision {
                    first: (*entry.get()).clone(),
                    second: name.clone(),
                });
            }
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(name);
            }
        }
    }
    None
}

deserialize_via_new!(SegmentName, keep_prefix);

display_inner!(SegmentName);

/// Entry type for a plugin
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    /// The first-party built-in segments, drawn by `gpy prompt`. Never read from
    /// a manifest.
    #[serde(skip)]
    Builtin,
    /// The plugin provides `segments/<name>.toml` script segment manifests, each
    /// naming a POSIX `sh` script that every shell runs.
    Script,
    /// Reserved custom-command runtime; parsed for diagnostics but not loadable.
    Command,
}

impl EntryType {
    /// Stable lowercase identifier matching the manifest schema.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Script => "script",
            Self::Command => "command",
        }
    }

    /// Whether the current runtime can load plugins with this entry type.
    #[must_use]
    pub const fn is_runtime_supported(&self) -> bool {
        matches!(self, Self::Builtin | Self::Script)
    }
}

/// The error for a manifest declaring the removed `entry_type = "file"`
/// (Fish `.fish` plugins), or `None` when it declares something else.
fn removed_entry_type_error(toml_content: &str) -> Option<crate::Error> {
    let table: toml::Table = toml::from_str(toml_content).ok()?;
    if table.get("entry_type").and_then(toml::Value::as_str) != Some("file") {
        return None;
    }
    let id = table
        .get("id")
        .and_then(toml::Value::as_str)
        .unwrap_or("<unknown>");
    Some(crate::Error::invalid(format!(
        "plugin '{}': entry_type \"file\" (Fish .fish plugins) is no longer supported; \
         rewrite it as a script plugin (entry_type = \"script\"), see docs/dev/plugins.md",
        id.escape_debug()
    )))
}

/// Supported public plugin API versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
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
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
#[cfg_attr(test, schemars(transparent))]
pub struct GpyVersion {
    #[serde(skip)]
    core: [u64; 3],
    #[cfg_attr(
        test,
        schemars(regex(pattern = r"^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]*)?$"))
    )]
    raw: String,
}

impl GpyVersion {
    /// Create a new parsed GPY version string.
    ///
    /// # Errors
    /// Returns an error if the value is empty, does not start with
    /// `MAJOR.MINOR.PATCH`, a component does not fit in a `u64`, or the
    /// `-pre`/`+build` suffix holds anything but ASCII letters, digits, `.`,
    /// `+` and `-` (the CLI prints the version, so it must not carry terminal
    /// escapes). Error messages escape the rejected value.
    pub fn new(version: &str) -> crate::Result<Self> {
        let normalized = version.trim();
        if normalized.is_empty() {
            return Err(crate::Error::invalid("Version cannot be empty"));
        }

        let numeric_core = Self::numeric_core(normalized);

        let parts = numeric_core.split('.').collect::<Vec<_>>();
        if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
            return Err(crate::Error::invalid(format!(
                "Version must start with MAJOR.MINOR.PATCH, got '{}'",
                normalized.escape_debug()
            )));
        }
        if parts
            .iter()
            .any(|part| !part.chars().all(|c| c.is_ascii_digit()))
        {
            return Err(crate::Error::invalid(format!(
                "Version core must contain only digits, got '{}'",
                normalized.escape_debug()
            )));
        }
        let suffix = normalized.strip_prefix(numeric_core).unwrap_or_default();
        if !suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
        {
            return Err(crate::Error::invalid(format!(
                "Version suffix may contain only ASCII letters, digits, '.', '+' and '-', got '{}'",
                normalized.escape_debug()
            )));
        }

        let mut core = [0u64; 3];
        for (slot, part) in core.iter_mut().zip(&parts) {
            *slot = part.parse().map_err(|e| {
                crate::Error::invalid(format!(
                    "Version component '{part}' is too large in '{normalized}': {e}"
                ))
            })?;
        }

        Ok(Self {
            core,
            raw: normalized.to_owned(),
        })
    }

    /// Get the normalized string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
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
        self.core > other.core
    }
}

deserialize_via_new!(GpyVersion, keep_prefix);

impl fmt::Display for GpyVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

/// Constraints for plugin compatibility
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct CompatibilityConstraints {
    /// Minimum required GPY version
    pub min_gpy_version: Option<GpyVersion>,
}

/// Root structure of a plugin manifest (plugin.toml)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
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
    /// Returns an error if the TOML is invalid or field validation fails.
    pub fn parse(toml_content: &str) -> crate::Result<Self> {
        let manifest: Self = toml::from_str(toml_content).map_err(|e| {
            removed_entry_type_error(toml_content).unwrap_or_else(|| {
                crate::Error::invalid(format!("Failed to parse plugin manifest: {e}"))
            })
        })?;

        if manifest.provided_segments.is_empty() {
            return Err(crate::Error::invalid(
                "provided_segments must list at least one segment".to_owned(),
            ));
        }

        let mut unique_segments = std::collections::HashSet::new();
        for segment in &manifest.provided_segments {
            if !unique_segments.insert(segment.as_str()) {
                return Err(crate::Error::invalid(format!(
                    "Duplicate segment declaration: '{}'",
                    segment.as_str()
                )));
            }
        }
        if let Some(collision) = first_token_collision(&manifest.provided_segments) {
            return Err(crate::Error::invalid(format!(
                "provided_segments: {collision}"
            )));
        }
        Ok(manifest)
    }

    /// Reject a plugin whose `min_gpy_version` is newer than `current`.
    ///
    /// # Errors
    /// Returns an error naming both versions when the plugin needs a newer GPY.
    pub fn check_min_gpy_version(&self, current: &GpyVersion) -> crate::Result<()> {
        match self
            .compatibility_constraints
            .as_ref()
            .and_then(|constraints| constraints.min_gpy_version.as_ref())
        {
            Some(min_version) if min_version.is_greater_than(current) => {
                Err(crate::Error::invalid(format!(
                    "Plugin '{}' requires GPY {min_version} or newer (compatibility_constraints.min_gpy_version); this is GPY {current}",
                    self.id
                )))
            }
            _ => Ok(()),
        }
    }

    /// Reject a plugin that provides a built-in segment name (`git`,
    /// `directory`, ...): the built-in always wins, so the plugin's segment
    /// would never run. The first-party metadata is built in code and never
    /// goes through this check.
    ///
    /// # Errors
    /// Returns an error naming the first built-in name the plugin provides.
    pub fn check_no_builtin_segments(&self) -> crate::Result<()> {
        self.provided_segments
            .iter()
            .find(|segment| super::BuiltinSegment::try_from(segment.as_str()).is_ok())
            .map_or(Ok(()), |segment| {
                Err(crate::Error::invalid(format!(
                    "Plugin '{}' provides '{segment}', the name of a built-in segment; rename that segment",
                    self.id
                )))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts `result` failed and its message contains `needle`.
    fn assert_rejected<T: std::fmt::Debug>(result: crate::Result<T>, needle: &str) {
        let err = result.unwrap_err().to_string();
        assert!(err.contains(needle), "expected `{needle}` in: {err}");
    }

    #[test]
    fn plugin_id_is_trimmed_and_lowercased() {
        assert_eq!(PluginId::new("my-plugin").unwrap().as_str(), "my-plugin");
        assert_eq!(PluginId::new(" My-Plugin ").unwrap().as_str(), "my-plugin");
        assert_eq!(PluginId::new("123-ABC").unwrap().as_str(), "123-abc");
    }

    #[test]
    fn plugin_id_rejects_empty_and_non_alphanumeric_hyphen_input() {
        assert_rejected(PluginId::new(""), "cannot be empty");
        assert_rejected(PluginId::new("  "), "cannot be empty");
        assert_rejected(
            PluginId::new("my plugin"),
            "alphanumeric characters and hyphens",
        );
        assert_rejected(
            PluginId::new("my_plugin"),
            "alphanumeric characters and hyphens",
        );
    }

    #[test]
    fn segment_name_is_trimmed_and_lowercased() {
        assert_eq!(SegmentName::new("my_seg").unwrap().as_str(), "my_seg");
        assert_eq!(SegmentName::new(" MY-seg ").unwrap().as_str(), "my-seg");
    }

    #[test]
    fn segment_name_rejects_empty_and_invalid_characters() {
        assert_rejected(SegmentName::new(""), "cannot be empty");
        assert_rejected(SegmentName::new("my seg"), "underscores, and hyphens");
        assert_rejected(SegmentName::new("my@seg"), "underscores, and hyphens");
    }

    #[test]
    fn variable_token_maps_only_hyphens() {
        for (name, token) in [
            ("my-seg", "my_seg"),
            ("my--seg", "my__seg"),
            ("my__seg", "my__seg"),
            ("seg_", "seg_"),
            ("-seg", "_seg"),
        ] {
            assert_eq!(SegmentName::new(name).unwrap().variable_token(), token);
        }
    }

    #[test]
    fn token_collision_reports_distinct_names_only() {
        let names: Vec<SegmentName> = ["a-b", "c", "a-b", "a_b"]
            .iter()
            .map(|name| SegmentName::new(name).unwrap())
            .collect();
        let collision = first_token_collision(&names).unwrap();
        assert_eq!(collision.first.as_str(), "a-b");
        assert_eq!(collision.second.as_str(), "a_b");
        assert!(first_token_collision(names.iter().take(3)).is_none());
    }

    #[test]
    fn parse_rejects_colliding_segment_tokens() {
        let toml = r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["k8s-ctx", "k8s_ctx"]
            entry_type = "script"
        "#;
        let err = PluginManifest::parse(toml).unwrap_err().to_string();
        assert!(
            err.contains("k8s-ctx") && err.contains("k8s_ctx"),
            "error must name both segments: {err}"
        );
    }

    #[test]
    fn parse_accepts_a_valid_manifest() {
        let toml = r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            description = "A test plugin"
            provided_segments = ["test_seg1", "test-seg2"]
            entry_type = "script"
        "#;
        let manifest = PluginManifest::parse(toml).unwrap();
        assert_eq!(manifest.id.as_str(), "my-test-plugin");
        assert_eq!(manifest.provided_segments.len(), 2);
        assert_eq!(manifest.entry_type, EntryType::Script);
        assert_eq!(manifest.api_version, PluginApiVersion::V1);
        assert_eq!(manifest.version.as_str(), "1.0.0");
        assert!(manifest.compatibility_constraints.is_none());
    }

    #[test]
    fn parse_rejects_a_manifest_missing_required_fields() {
        let toml = r#"
            name = "My Test Plugin"
            version = "1.0.0"
        "#;
        assert_rejected(PluginManifest::parse(toml), "missing field");
    }

    #[test]
    fn parse_rejects_missing_provided_segments() {
        for key_line in ["", "provided_segment = [\"x\"]"] {
            let toml = format!(
                r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            entry_type = "script"
            {key_line}
        "#
            );
            let err = PluginManifest::parse(&toml).unwrap_err();
            assert!(
                err.to_string().contains("provided_segments"),
                "error should name provided_segments: {err}"
            );
        }
    }

    #[test]
    fn parse_rejects_empty_provided_segments() {
        let toml = r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = []
            entry_type = "script"
        "#;
        let err = PluginManifest::parse(toml).unwrap_err();
        assert!(err.to_string().contains("provided_segments"), "{err}");
    }

    #[test]
    fn parse_rejects_duplicate_segments() {
        let toml = r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["test_seg1", "test_seg1"]
            entry_type = "script"
        "#;
        assert_rejected(
            PluginManifest::parse(toml),
            "Duplicate segment declaration: 'test_seg1'",
        );
    }

    #[test]
    fn entry_type_runtime_support() {
        assert!(EntryType::Builtin.is_runtime_supported());
        assert!(EntryType::Script.is_runtime_supported());
        assert!(!EntryType::Command.is_runtime_supported());
        assert_eq!(EntryType::Builtin.as_str(), "builtin");
        assert_eq!(EntryType::Script.as_str(), "script");
        assert_eq!(EntryType::Command.as_str(), "command");
    }

    #[test]
    fn parse_rejects_removed_file_entry_type_with_a_pointer() {
        let toml = r#"
            id = "old-fish"
            name = "Old"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["x"]
            entry_type = "file"
        "#;
        assert_rejected(
            PluginManifest::parse(toml),
            "plugin 'old-fish': entry_type \"file\" (Fish .fish plugins) is no longer supported; \
             rewrite it as a script plugin (entry_type = \"script\"), see docs/dev/plugins.md",
        );
        assert_rejected(
            PluginManifest::parse(&toml.replace("\"file\"", "\"builtin\"")),
            "unknown variant `builtin`",
        );
    }

    fn manifest_with_min_version(min: &str) -> PluginManifest {
        PluginManifest::parse(&format!(
            r#"
            id = "versioned"
            name = "Versioned"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["x"]
            entry_type = "script"

            [compatibility_constraints]
            min_gpy_version = "{min}"
        "#
        ))
        .unwrap()
    }

    #[test]
    fn min_gpy_version_is_enforced() {
        let current = GpyVersion::new("1.2.3").unwrap();
        for ok in ["1.2.3", "1.2.0", "0.9.9", "1.2.3-beta.1"] {
            manifest_with_min_version(ok)
                .check_min_gpy_version(&current)
                .unwrap();
        }
        let err = manifest_with_min_version("1.3.0")
            .check_min_gpy_version(&current)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("1.3.0") && err.contains("1.2.3") && err.contains("min_gpy_version"),
            "{err}"
        );
    }

    #[test]
    fn missing_min_gpy_version_always_passes() {
        let manifest = PluginManifest::parse(
            r#"
            id = "plain"
            name = "Plain"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["x"]
            entry_type = "script"
        "#,
        )
        .unwrap();
        assert_eq!(manifest.entry_type, EntryType::Script);
        manifest
            .check_min_gpy_version(&GpyVersion::new("0.0.1").unwrap())
            .unwrap();
    }

    #[test]
    fn parse_accepts_command_entry_type_for_diagnostics() {
        // Must parse so discovery can tell unsupported from malformed.
        let toml = r#"
            id = "cmd-plugin"
            name = "Cmd"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["x"]
            entry_type = "command"
        "#;
        let manifest = PluginManifest::parse(toml).unwrap();
        assert_eq!(manifest.entry_type, EntryType::Command);
        assert!(!manifest.entry_type.is_runtime_supported());
    }

    #[test]
    fn parse_rejects_an_invalid_id() {
        let toml = r#"
            id = "my_invalid_id"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["x"]
            entry_type = "script"
        "#;
        assert_rejected(
            PluginManifest::parse(toml),
            "alphanumeric characters and hyphens",
        );
    }

    #[test]
    fn parse_rejects_unknown_api_version() {
        let toml = r#"
            id = "my-test-plugin"
            name = "My Test Plugin"
            version = "1.0.0"
            api_version = "v2"
            provided_segments = ["x"]
            entry_type = "script"
        "#;
        assert_rejected(PluginManifest::parse(toml), "unknown variant `v2`");
    }

    #[test]
    fn gpy_version_accepts_release_and_prerelease() {
        assert_eq!(GpyVersion::new("0.1.0").unwrap().as_str(), "0.1.0");
        assert_eq!(
            GpyVersion::new("1.2.3-beta.1").unwrap().as_str(),
            "1.2.3-beta.1"
        );
    }

    #[test]
    fn gpy_version_rejects_empty_and_malformed_cores() {
        assert_rejected(GpyVersion::new(""), "cannot be empty");
        assert_rejected(GpyVersion::new("1"), "MAJOR.MINOR.PATCH");
        assert_rejected(GpyVersion::new("1.2"), "MAJOR.MINOR.PATCH");
        assert_rejected(GpyVersion::new("1.2.x"), "only digits");
    }

    #[test]
    fn gpy_version_suffix_rejects_terminal_escapes() {
        assert!(GpyVersion::new("1.0.0+build.7-x").is_ok());
        for bad in ["1.0.0+\u{1b}]0;x\u{7}", "1.0.0-a b", "1.0.0-\u{9b}31m"] {
            let err = GpyVersion::new(bad).unwrap_err().to_string();
            assert!(err.contains("Version suffix"), "{err}");
            assert!(!err.chars().any(char::is_control), "{err:?}");
        }
    }

    #[test]
    fn builtin_segment_names_are_rejected() {
        let manifest = PluginManifest::parse(
            r#"
            id = "shadow"
            name = "Shadow"
            version = "1.0.0"
            api_version = "v1"
            provided_segments = ["demo", "git"]
            entry_type = "script"
        "#,
        )
        .unwrap();
        assert_rejected(
            manifest.check_no_builtin_segments(),
            "'git', the name of a built-in segment",
        );
    }

    #[test]
    fn gpy_version_component_overflow_is_rejected() {
        assert_rejected(GpyVersion::new("99999999999999999999.0.0"), "too large");
        assert_rejected(GpyVersion::new("0.0.18446744073709551616"), "too large");
        assert!(GpyVersion::new("18446744073709551615.0.0").is_ok());
    }
}
