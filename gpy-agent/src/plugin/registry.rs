//! Plugin discovery and registry construction.
//!
//! The registry scans first-party, bundled, and user plugin locations, parses
//! manifests with [`crate::plugin::manifest`], and returns both valid plugins
//! and diagnostics for invalid entries. CLI commands and prompt export code
//! consume this discovery result rather than walking plugin directories
//! themselves.

use super::manifest::{
    EntryType, GpyVersion, PluginApiVersion, PluginId, PluginManifest, SegmentName,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Cap on `plugin.toml` reads: a manifest with declared fields, config-sized.
const PLUGIN_MANIFEST_READ_CAP: usize = 65_536;

/// Source location for a discovered plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSource {
    /// First-party metadata for built-in GPY segments.
    FirstParty,
    /// Bundled plugin distributed with GPY.
    Bundled,
    /// User-installed plugin from configuration directory.
    User,
}

impl PluginSource {
    const fn precedence(self) -> u8 {
        match self {
            Self::FirstParty => 0,
            Self::Bundled => 1,
            Self::User => 2,
        }
    }
}

/// Load status for a discovered plugin entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginLoadStatus {
    /// Plugin manifest parsed and accepted.
    Ready,
}

/// A valid plugin discovered on disk.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    /// Parsed plugin manifest.
    pub manifest: PluginManifest,
    /// Filesystem root directory containing `plugin.toml`.
    pub root: PathBuf,
    /// Plugin source location.
    pub source: PluginSource,
    /// Current plugin load status.
    pub status: PluginLoadStatus,
}

/// A non-fatal plugin discovery/parse issue.
#[derive(Debug, Clone)]
pub struct PluginDiagnostic {
    /// Directory where the diagnostic originated.
    pub plugin_dir: PathBuf,
    /// Source root being scanned.
    pub source: PluginSource,
    /// Human-readable diagnostic message.
    pub message: String,
}

/// Result of plugin discovery.
#[derive(Debug, Default, Clone)]
pub struct PluginDiscovery {
    /// Successfully discovered plugins after precedence resolution.
    pub plugins: Vec<DiscoveredPlugin>,
    /// Non-fatal diagnostics collected during discovery.
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// Return the user plugin installation root.
#[must_use]
pub fn user_plugins_dir() -> PathBuf {
    crate::paths::config_root_for(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        crate::paths::home_dir().as_deref(),
    )
    .join("plugins")
}

fn bundled_plugins_dir() -> Option<PathBuf> {
    std::env::var("GPY_BUNDLED_PLUGIN_DIR")
        .ok()
        .map(PathBuf::from)
}

fn plugin_roots() -> Vec<(PluginSource, PathBuf)> {
    let mut roots = Vec::new();
    if let Some(path) = bundled_plugins_dir() {
        roots.push((PluginSource::Bundled, path));
    }
    roots.push((PluginSource::User, user_plugins_dir()));
    roots
}

fn first_party_plugins() -> Vec<DiscoveredPlugin> {
    let Ok(plugin_id) = PluginId::new("gpy-core") else {
        return Vec::new();
    };
    let provided_segments: Vec<SegmentName> = [
        "clock",
        "duration",
        "language",
        "directory",
        "git",
        "status",
    ]
    .iter()
    .filter_map(|name| SegmentName::new(name).ok())
    .collect();
    if provided_segments.is_empty() {
        return Vec::new();
    }

    let Ok(version) = GpyVersion::new(env!("CARGO_PKG_VERSION")) else {
        return Vec::new();
    };

    let manifest = PluginManifest {
        id: plugin_id,
        name: "GPY Core Segments".to_owned(),
        version,
        api_version: PluginApiVersion::V1,
        description: Some("First-party metadata for built-in GPY prompt segments".to_owned()),
        provided_segments,
        entry_type: EntryType::File,
        compatibility_constraints: None,
    };

    vec![DiscoveredPlugin {
        manifest,
        root: PathBuf::from("<first-party>"),
        source: PluginSource::FirstParty,
        status: PluginLoadStatus::Ready,
    }]
}

/// Read and parse `plugin.toml` from a plugin directory.
///
/// # Errors
///
/// Returns a `PluginDiagnostic` when the manifest cannot be read or parsed.
fn read_plugin_manifest(
    plugin_dir: &Path,
    source: PluginSource,
) -> std::result::Result<PluginManifest, PluginDiagnostic> {
    let manifest_path = plugin_dir.join("plugin.toml");
    let content = crate::fs_util::read_small_file(&manifest_path, PLUGIN_MANIFEST_READ_CAP)
        .map_err(|e| PluginDiagnostic {
            plugin_dir: plugin_dir.to_path_buf(),
            source,
            message: format!("Failed to read {}: {e}", manifest_path.display()),
        })?;

    PluginManifest::parse(&content).map_err(|e| PluginDiagnostic {
        plugin_dir: plugin_dir.to_path_buf(),
        source,
        message: format!("Invalid manifest {}: {e}", manifest_path.display()),
    })
}

/// Parse, validate, and register a single candidate plugin directory.
///
/// Pushes a diagnostic (and registers nothing) for malformed manifests and
/// unsupported entry types (#175). For a duplicate id, a strictly higher-precedence
/// source overrides; a same-source duplicate keeps the existing (lexically smaller)
/// winner and reports the conflict (#172).
fn register_discovered_plugin(
    plugin_dir: PathBuf,
    source: PluginSource,
    winners: &mut HashMap<String, DiscoveredPlugin>,
    diagnostics: &mut Vec<PluginDiagnostic>,
) {
    let manifest = match read_plugin_manifest(&plugin_dir, source) {
        Ok(manifest) => manifest,
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            return;
        }
    };

    // Reject entry types the runtime cannot load, with a diagnostic that is
    // distinct from a malformed manifest (#175).
    if !manifest.entry_type.is_runtime_supported() {
        diagnostics.push(PluginDiagnostic {
            plugin_dir,
            source,
            message: format!(
                "Unsupported entry_type '{}' for plugin '{}': only 'file' plugins are loadable by this runtime",
                manifest.entry_type.as_str(),
                manifest.id
            ),
        });
        return;
    }

    let plugin_id = manifest.id.as_str().to_owned();
    let candidate = DiscoveredPlugin {
        manifest,
        root: plugin_dir,
        source,
        status: PluginLoadStatus::Ready,
    };

    let Some(existing) = winners.get(&plugin_id) else {
        winners.insert(plugin_id, candidate);
        return;
    };

    let candidate_precedence = candidate.source.precedence();
    let existing_precedence = existing.source.precedence();
    if candidate_precedence > existing_precedence {
        // Strictly higher-precedence source overrides (user > bundled >
        // first-party). Deterministic regardless of visit order.
        winners.insert(plugin_id, candidate);
    } else if candidate_precedence == existing_precedence {
        // Same-source duplicate id. The deterministic winner is the lexically
        // smaller root, already inserted because entries are visited in sorted
        // order; keep it and report the conflicting directories (#172).
        diagnostics.push(PluginDiagnostic {
            plugin_dir: candidate.root.clone(),
            source,
            message: format!(
                "Duplicate plugin id '{plugin_id}': keeping '{}', ignoring '{}'",
                existing.root.display(),
                candidate.root.display()
            ),
        });
    }
    // Lower-precedence candidate: keep the existing winner silently.
}

fn discover_plugins_in_roots(roots: &[(PluginSource, PathBuf)]) -> PluginDiscovery {
    let mut winners: HashMap<String, DiscoveredPlugin> = first_party_plugins()
        .into_iter()
        .map(|plugin| (plugin.manifest.id.as_str().to_owned(), plugin))
        .collect();
    let mut diagnostics = Vec::new();

    for (source, root) in roots {
        if !root.exists() {
            continue;
        }
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) => {
                diagnostics.push(PluginDiagnostic {
                    plugin_dir: root.clone(),
                    source: *source,
                    message: format!("Failed to read plugin root {}: {e}", root.display()),
                });
                continue;
            }
        };

        // Visit directory entries in a deterministic (lexical by path) order so the
        // selected winner never depends on filesystem `read_dir` ordering (#172).
        let mut plugin_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.join("plugin.toml").exists())
            .collect();
        plugin_dirs.sort();

        for plugin_dir in plugin_dirs {
            register_discovered_plugin(plugin_dir, *source, &mut winners, &mut diagnostics);
        }
    }

    let mut plugins: Vec<DiscoveredPlugin> = winners.into_values().collect();
    plugins.sort_by(|a, b| a.manifest.id.as_str().cmp(b.manifest.id.as_str()));

    PluginDiscovery {
        plugins,
        diagnostics,
    }
}

/// Discover plugins from bundled and user plugin roots.
///
/// Discovery is deterministic and independent of filesystem `read_dir` ordering:
/// - Source precedence: user > bundled > first-party. A strictly higher-precedence
///   source always overrides a lower one.
/// - Duplicate ids within the *same* source resolve to the lexically smallest
///   plugin directory, and every conflicting directory is reported as a diagnostic.
/// - Final plugins are ordered lexically by plugin id for stable display.
/// - Diagnostics are collected for malformed/unreadable plugins and unsupported
///   entry types without aborting discovery.
///
#[must_use]
pub fn discover_plugins() -> PluginDiscovery {
    discover_plugins_in_roots(&plugin_roots())
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn valid_manifest(id: &str) -> String {
        format!(
            r#"
id = "{id}"
name = "{id}"
version = "0.1.0"
api_version = "v1"
provided_segments = ["{id}"]
entry_type = "file"
"#
        )
    }

    fn command_manifest(id: &str) -> String {
        format!(
            r#"
id = "{id}"
name = "{id}"
version = "0.1.0"
api_version = "v1"
provided_segments = ["{id}"]
entry_type = "command"
"#
        )
    }

    fn write_plugin(root: &Path, dir_name: &str, manifest: &str) {
        let plugin_dir = root.join(dir_name);
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.toml"), manifest).expect("write manifest");
    }

    #[test]
    fn discover_plugins_same_source_duplicate_is_deterministic() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("gpy").join("plugins");
        // Two sibling directories declaring the same id within one (user) source,
        // created in reverse lexical order to prove ordering is independent of
        // creation/read_dir order (#172).
        write_plugin(&root, "z-dup", &valid_manifest("demo"));
        write_plugin(&root, "a-dup", &valid_manifest("demo"));

        let roots = vec![(PluginSource::User, root)];
        let discovery = discover_plugins_in_roots(&roots);

        let demo = discovery
            .plugins
            .iter()
            .find(|plugin| plugin.manifest.id.as_str() == "demo")
            .expect("demo discovered");
        assert!(
            demo.root.ends_with("a-dup"),
            "lexically smaller root must win, got {}",
            demo.root.display()
        );
        assert!(
            discovery.diagnostics.iter().any(|diagnostic| {
                diagnostic.message.contains("Duplicate plugin id 'demo'")
                    && diagnostic.message.contains("z-dup")
            }),
            "expected a duplicate diagnostic naming the ignored directory"
        );
    }

    #[test]
    fn discover_plugins_rejects_unsupported_entry_type() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("gpy").join("plugins");
        write_plugin(&root, "cmd", &command_manifest("cmd-demo"));

        let roots = vec![(PluginSource::User, root)];
        let discovery = discover_plugins_in_roots(&roots);

        assert!(
            !discovery
                .plugins
                .iter()
                .any(|plugin| plugin.manifest.id.as_str() == "cmd-demo"),
            "command plugin must not be registered as discoverable"
        );
        assert!(
            discovery.diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("Unsupported entry_type 'command'")),
            "expected an unsupported-entry-type diagnostic"
        );
    }

    #[test]
    fn discover_plugins_orders_results_by_id() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("gpy").join("plugins");
        for id in ["zeta", "alpha", "mike"] {
            write_plugin(&root, id, &valid_manifest(id));
        }
        let roots = vec![(PluginSource::User, root)];
        let discovery = discover_plugins_in_roots(&roots);

        let ids: Vec<&str> = discovery
            .plugins
            .iter()
            .map(|plugin| plugin.manifest.id.as_str())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(
            ids, sorted,
            "plugins must be ordered by id for stable display"
        );
    }

    #[test]
    fn discover_plugins_skips_missing_roots() {
        let discovery = discover_plugins_in_roots(&[]);
        assert_eq!(discovery.plugins.len(), 1);
        let first = discovery.plugins.first().expect("first-party plugin");
        assert_eq!(first.source, PluginSource::FirstParty);
    }

    #[test]
    fn discover_plugins_collects_invalid_manifest_diagnostics() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("gpy").join("plugins");
        let plugin_dir = root.join("bad");
        fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        fs::write(plugin_dir.join("plugin.toml"), "id =").expect("write invalid manifest");

        let roots = vec![(PluginSource::User, root)];
        let discovery = discover_plugins_in_roots(&roots);
        assert_eq!(discovery.plugins.len(), 1);
        let first = discovery
            .plugins
            .first()
            .expect("first-party metadata plugin should be present");
        assert_eq!(first.source, PluginSource::FirstParty);
        assert_eq!(discovery.diagnostics.len(), 1);
    }

    #[test]
    fn discover_plugins_user_precedence_over_bundled() {
        let user_temp = TempDir::new().expect("tempdir");
        let bundled_temp = TempDir::new().expect("tempdir");

        let user_root = user_temp.path().join("gpy").join("plugins").join("demo");
        let bundled_root = bundled_temp.path().join("demo");
        fs::create_dir_all(&user_root).expect("create user plugin");
        fs::create_dir_all(&bundled_root).expect("create bundled plugin");

        fs::write(user_root.join("plugin.toml"), valid_manifest("demo"))
            .expect("write user plugin");
        fs::write(bundled_root.join("plugin.toml"), valid_manifest("demo"))
            .expect("write bundled plugin");

        let roots = vec![
            (PluginSource::Bundled, bundled_temp.path().to_path_buf()),
            (
                PluginSource::User,
                user_temp.path().join("gpy").join("plugins"),
            ),
        ];
        let discovery = discover_plugins_in_roots(&roots);
        let demo = discovery
            .plugins
            .iter()
            .find(|plugin| plugin.manifest.id.as_str() == "demo")
            .expect("demo plugin discovered");
        assert_eq!(demo.source, PluginSource::User);
    }

    #[test]
    fn discover_plugins_includes_first_party_builtins_metadata() {
        let discovery = discover_plugins_in_roots(&[]);
        let first_party = discovery
            .plugins
            .iter()
            .find(|plugin| plugin.manifest.id.as_str() == "gpy-core")
            .expect("first-party metadata plugin should be present");

        let provided: Vec<String> = first_party
            .manifest
            .provided_segments
            .iter()
            .map(ToString::to_string)
            .collect();
        for segment in [
            "clock",
            "duration",
            "language",
            "directory",
            "git",
            "status",
        ] {
            assert!(
                provided.contains(&segment.to_owned()),
                "first-party metadata should include '{segment}'"
            );
        }
    }
}
