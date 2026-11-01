//! Plugin discovery and registry construction.

use super::manifest::{
    EntryType, GpyVersion, PluginApiVersion, PluginId, PluginManifest, SegmentName,
    first_token_collision,
};
use super::script::ScriptSegmentManifest;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

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

impl std::fmt::Display for PluginSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FirstParty => "first-party",
            Self::Bundled => "bundled",
            Self::User => "user",
        })
    }
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

/// A valid plugin discovered on disk.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    /// Parsed plugin manifest.
    pub manifest: PluginManifest,
    /// Filesystem root directory containing `plugin.toml`; `None` for first-party metadata.
    pub root: Option<PathBuf>,
    /// Plugin source location.
    pub source: PluginSource,
    /// Validated segment manifests of a `script` plugin, one per provided
    /// segment; empty for other entry types.
    pub script_segments: BTreeMap<SegmentName, ScriptSegmentManifest>,
}

impl DiscoveredPlugin {
    /// The root, or an empty path for first-party metadata.
    fn root_or_default(&self) -> PathBuf {
        self.root.clone().unwrap_or_default()
    }

    /// Load every provided segment's manifest of a `script` plugin.
    ///
    /// # Errors
    /// Returns the first manifest error, naming its file and key.
    pub fn load_script_segments(
        manifest: &PluginManifest,
        root: &Path,
    ) -> crate::Result<BTreeMap<SegmentName, ScriptSegmentManifest>> {
        if !matches!(manifest.entry_type, EntryType::Script) {
            return Ok(BTreeMap::new());
        }
        manifest
            .provided_segments
            .iter()
            .map(|segment| {
                ScriptSegmentManifest::load(root, segment).map(|loaded| (segment.clone(), loaded))
            })
            .collect()
    }
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
    crate::paths::config_root().join("plugins")
}

/// Return the bundled plugin root: `GPY_BUNDLED_PLUGIN_DIR` when set, else
/// `plugins` in the data root, where `gpy setup` installs the plugins GPY ships
/// (`username`, `hostname`).
///
/// Deprecated fallback, for one minor release: a data root without `plugins`
/// beside an old `<config root>/bundled-plugins` (a package upgrade with no
/// `gpy setup` since) uses the old directory, so those segments keep rendering.
#[must_use]
pub fn bundled_plugins_dir() -> PathBuf {
    if let Some(dir) = crate::env_config::BUNDLED_PLUGIN_DIR.path() {
        return crate::paths::absolutize(&dir);
    }
    let legacy = || crate::paths::config_root().join(BUNDLED_PLUGINS_DIR_NAME);
    crate::paths::data_root().map_or_else(legacy, |data| {
        let current = data.join(BUNDLED_PLUGINS_SOURCE_DIR);
        if !current.exists() && legacy().is_dir() {
            legacy()
        } else {
            current
        }
    })
}

/// Directory name of the old bundled plugin root under the config root
/// (deprecated fallback of [`bundled_plugins_dir`]).
pub const BUNDLED_PLUGINS_DIR_NAME: &str = "bundled-plugins";

/// Directory name of the bundled plugin root under the data root.
const BUNDLED_PLUGINS_SOURCE_DIR: &str = "plugins";

fn plugin_roots() -> Vec<(PluginSource, PathBuf)> {
    vec![
        (PluginSource::Bundled, bundled_plugins_dir()),
        (PluginSource::User, user_plugins_dir()),
    ]
}

/// Plugin id of the built-in segment metadata.
pub const FIRST_PARTY_ID: &str = "gpy-core";

/// Metadata for the built-in segments. Every value is statically known; the
/// `first_party_manifest_*` tests pin that construction succeeds.
static FIRST_PARTY: LazyLock<DiscoveredPlugin> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "inputs are compile-time constants validated by tests"
    )]
    let manifest = PluginManifest {
        id: PluginId::new(FIRST_PARTY_ID).expect("first-party id is valid"),
        name: "GPY Core Segments".to_owned(),
        version: GpyVersion::new(env!("CARGO_PKG_VERSION")).expect("crate version is valid"),
        api_version: PluginApiVersion::V1,
        description: Some("First-party metadata for built-in GPY prompt segments".to_owned()),
        provided_segments: super::BUILTIN_ORDER
            .iter()
            .map(|builtin| SegmentName::new(builtin.as_str()).expect("builtin name is valid"))
            .collect(),
        entry_type: EntryType::File,
        compatibility_constraints: None,
    };
    DiscoveredPlugin {
        manifest,
        root: None,
        source: PluginSource::FirstParty,
        script_segments: BTreeMap::new(),
    }
});

/// Read and parse `plugin.toml` from a plugin directory.
///
/// # Errors
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
/// Higher-precedence sources override; same-source duplicates keep the
/// lexically smaller root and report the conflict.
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

    // Unsupported entry types get a diagnostic distinct from a malformed manifest.
    if !manifest.entry_type.is_runtime_supported() {
        diagnostics.push(PluginDiagnostic {
            plugin_dir,
            source,
            message: format!(
                "Unsupported entry_type '{}' for plugin '{}': only 'file' and 'script' plugins are loadable by this runtime",
                manifest.entry_type.as_str(),
                manifest.id
            ),
        });
        return;
    }

    let checked = GpyVersion::new(env!("CARGO_PKG_VERSION"))
        .and_then(|current| manifest.check_min_gpy_version(&current))
        .and_then(|()| DiscoveredPlugin::load_script_segments(&manifest, &plugin_dir));
    let script_segments = match checked {
        Ok(script_segments) => script_segments,
        Err(e) => {
            diagnostics.push(PluginDiagnostic {
                plugin_dir,
                source,
                message: e.to_string(),
            });
            return;
        }
    };

    let plugin_id = manifest.id.as_str().to_owned();
    let candidate = DiscoveredPlugin {
        manifest,
        root: Some(plugin_dir),
        source,
        script_segments,
    };

    let Some(existing) = winners.get(&plugin_id) else {
        winners.insert(plugin_id, candidate);
        return;
    };

    let candidate_precedence = candidate.source.precedence();
    let existing_precedence = existing.source.precedence();
    if candidate_precedence > existing_precedence {
        // user > bundled > first-party.
        winners.insert(plugin_id, candidate);
    } else if candidate_precedence == existing_precedence {
        // Same source: the lexically smaller root was visited first; keep it.
        diagnostics.push(PluginDiagnostic {
            plugin_dir: candidate.root_or_default(),
            source,
            message: format!(
                "Duplicate plugin id '{plugin_id}': keeping '{}', ignoring '{}'",
                existing.root_or_default().display(),
                candidate.root_or_default().display()
            ),
        });
    }
}

fn discover_plugins_in_roots(roots: &[(PluginSource, PathBuf)]) -> PluginDiscovery {
    let mut winners: HashMap<String, DiscoveredPlugin> = HashMap::from([(
        FIRST_PARTY_ID.to_owned(),
        LazyLock::force(&FIRST_PARTY).clone(),
    )]);
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

        // Sorted so the winner never depends on `read_dir` order.
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

    let mut sorted: Vec<DiscoveredPlugin> = winners.into_values().collect();
    sorted.sort_by(|a, b| a.manifest.id.as_str().cmp(b.manifest.id.as_str()));
    let plugins = reject_token_collisions(sorted, &mut diagnostics);

    PluginDiscovery {
        plugins,
        diagnostics,
    }
}

/// Drop each plugin (visited in id order) that provides a segment whose shell
/// variable token an earlier plugin's differently named segment already uses;
/// loading both would let one segment's file and styles shadow the other's.
fn reject_token_collisions(
    plugins: Vec<DiscoveredPlugin>,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> Vec<DiscoveredPlugin> {
    let mut accepted: Vec<DiscoveredPlugin> = Vec::with_capacity(plugins.len());
    for plugin in plugins {
        let claimed = accepted
            .iter()
            .flat_map(|earlier| earlier.manifest.provided_segments.iter());
        let Some(collision) =
            first_token_collision(claimed.chain(&plugin.manifest.provided_segments))
        else {
            accepted.push(plugin);
            continue;
        };
        let owner = accepted
            .iter()
            .find(|earlier| {
                earlier
                    .manifest
                    .provided_segments
                    .contains(&collision.first)
            })
            .map_or("another plugin", |earlier| earlier.manifest.id.as_str());
        let message = format!(
            "Plugin '{}' ignored: {collision} (segment '{}' comes from plugin '{owner}')",
            plugin.manifest.id, collision.first
        );
        diagnostics.push(PluginDiagnostic {
            plugin_dir: plugin.root.unwrap_or_default(),
            source: plugin.source,
            message,
        });
    }
    accepted
}

/// Discover plugins from bundled and user plugin roots.
///
/// Deterministic: precedence user > bundled > first-party, same-source
/// duplicates resolve lexically, output is sorted by id, and a plugin whose
/// segment token collides with an earlier plugin's is dropped with a diagnostic.
#[must_use]
pub fn discover_plugins() -> PluginDiscovery {
    discover_plugins_in_roots(&plugin_roots())
}

#[cfg(test)]
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
        // Created in reverse lexical order to prove independence from read_dir order.
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
            demo.root
                .as_deref()
                .is_some_and(|dir| dir.ends_with("a-dup")),
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
    fn discover_plugins_rejects_unmet_min_gpy_version() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("gpy").join("plugins");
        let manifest = format!(
            "{}\n[compatibility_constraints]\nmin_gpy_version = \"999.0.0\"\n",
            valid_manifest("future")
        );
        write_plugin(&root, "future", &manifest);

        let discovery = discover_plugins_in_roots(&[(PluginSource::User, root)]);

        assert!(
            !discovery
                .plugins
                .iter()
                .any(|plugin| plugin.manifest.id.as_str() == "future"),
            "a plugin needing a newer GPY must not load"
        );
        assert!(
            discovery
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("requires GPY 999.0.0")),
            "expected a min_gpy_version diagnostic"
        );
    }

    #[test]
    fn discover_plugins_loads_script_segments_or_reports_them() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("gpy").join("plugins");
        for id in ["good", "bad"] {
            write_plugin(
                &root,
                id,
                &valid_manifest(id).replace("entry_type = \"file\"", "entry_type = \"script\""),
            );
            let dir = root.join(id);
            fs::create_dir_all(dir.join("segments")).expect("segments");
            fs::write(dir.join("run.sh"), "echo hi\n").expect("script");
        }
        fs::write(
            root.join("good/segments/good.toml"),
            "command = \"run.sh\"\ninputs = [\"pwd\"]\n",
        )
        .expect("good manifest");
        fs::write(
            root.join("bad/segments/bad.toml"),
            "command = \"run.sh\"\nenv = [\"a-b\"]\n",
        )
        .expect("bad manifest");

        let discovery = discover_plugins_in_roots(&[(PluginSource::User, root)]);

        let good = discovery
            .plugins
            .iter()
            .find(|plugin| plugin.manifest.id.as_str() == "good")
            .expect("good script plugin discovered");
        let segment = good
            .script_segments
            .get(&SegmentName::new("good").unwrap())
            .expect("segment manifest loaded");
        assert_eq!(segment.cache_scope(), super::super::CacheScope::Folder);
        assert!(
            !discovery
                .plugins
                .iter()
                .any(|plugin| plugin.manifest.id.as_str() == "bad")
        );
        assert!(
            discovery.diagnostics.iter().any(|diagnostic| {
                diagnostic.message.contains("bad.toml") && diagnostic.message.contains("a-b")
            }),
            "{:?}",
            discovery.diagnostics
        );
    }

    #[test]
    fn discover_plugins_drops_later_plugin_with_colliding_segment_token() {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().join("gpy").join("plugins");
        let manifest = |id: &str, segment: &str| {
            valid_manifest(id).replace(
                &format!("provided_segments = [\"{id}\"]"),
                &format!("provided_segments = [\"{segment}\"]"),
            )
        };
        write_plugin(&root, "first", &manifest("aaa-ctx", "k8s-ctx"));
        write_plugin(&root, "second", &manifest("zzz-ctx", "k8s_ctx"));

        let discovery = discover_plugins_in_roots(&[(PluginSource::User, root)]);

        let ids: Vec<&str> = discovery
            .plugins
            .iter()
            .map(|plugin| plugin.manifest.id.as_str())
            .collect();
        assert!(ids.contains(&"aaa-ctx"), "earlier plugin kept: {ids:?}");
        assert!(
            !ids.contains(&"zzz-ctx"),
            "colliding plugin dropped: {ids:?}"
        );
        assert!(
            discovery.diagnostics.iter().any(|diagnostic| {
                diagnostic.plugin_dir.ends_with("second")
                    && diagnostic.message.contains("k8s-ctx")
                    && diagnostic.message.contains("k8s_ctx")
            }),
            "expected a collision diagnostic naming both segments: {:?}",
            discovery.diagnostics
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
            .find(|plugin| plugin.manifest.id.as_str() == FIRST_PARTY_ID)
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

    #[test]
    fn first_party_manifest_provides_exactly_the_builtin_segments() {
        let provided: Vec<String> = FIRST_PARTY
            .manifest
            .provided_segments
            .iter()
            .map(ToString::to_string)
            .collect();
        let builtin: Vec<String> = crate::plugin::BUILTIN_ORDER
            .iter()
            .map(|segment| segment.as_str().to_owned())
            .collect();
        assert_eq!(provided, builtin);
    }
}
