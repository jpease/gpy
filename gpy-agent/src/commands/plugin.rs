//! `gpy plugin` command handlers.

use crate::plugin::{
    DiscoveredPlugin, EntryType, GpyVersion, PluginApiVersion, PluginManifest, PluginSource,
    ScriptSegmentManifest, SegmentName, discover_plugins, script_segment_manifest_path,
    user_plugins_dir,
};
use crate::{Error, Result, VERSION};
use std::collections::BTreeMap;
use std::path::Path;
use std::{fs, path::PathBuf};

/// Validated segment manifests of a script plugin, keyed by segment.
type ScriptSegments = BTreeMap<SegmentName, ScriptSegmentManifest>;

pub(crate) const fn render_source(source: PluginSource) -> &'static str {
    match source {
        PluginSource::FirstParty => "first-party",
        PluginSource::Bundled => "bundled",
        PluginSource::User => "user",
    }
}

const PUBLIC_PLUGIN_HELPERS: &[&str] = &[
    "gpy_section_start",
    "gpy_section_append",
    "gpy_section_end",
    "gpy_section_standalone",
];

/// Parse the current GPY version into the typed plugin compatibility form.
pub(crate) fn current_gpy_version() -> Result<GpyVersion> {
    GpyVersion::new(VERSION)
}

fn plugin_segment_file(root: &Path, segment: &crate::plugin::SegmentName) -> PathBuf {
    root.join("segments").join(format!("{segment}.fish"))
}

/// Error for a manifest whose `entry_type` this runtime cannot load.
fn unsupported_entry_type(entry_type: &EntryType) -> Error {
    Error::invalid(format!(
        "Unsupported entry_type '{}': only 'file' and 'script' plugins are loadable by this runtime",
        entry_type.as_str()
    ))
}

/// Validate that a plugin root contains the files required by its manifest
/// and that this GPY satisfies its `min_gpy_version`.
fn validate_plugin_root(root: &Path, manifest: &PluginManifest) -> Result<ScriptSegments> {
    // Fail at validation rather than at prompt init.
    if !manifest.entry_type.is_runtime_supported() {
        return Err(unsupported_entry_type(&manifest.entry_type));
    }
    manifest.check_min_gpy_version(&current_gpy_version()?)?;

    if !root.is_dir() {
        return Err(Error::invalid(format!(
            "Plugin root '{}' is not a directory",
            root.display()
        )));
    }

    let manifest_path = root.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(Error::invalid(format!(
            "Missing plugin manifest: {}",
            manifest_path.display()
        )));
    }

    match manifest.entry_type {
        EntryType::File => {
            for segment in &manifest.provided_segments {
                let segment_file = plugin_segment_file(root, segment);
                if !segment_file.is_file() {
                    return Err(Error::invalid(format!(
                        "Missing segment file for '{segment}': {}",
                        segment_file.display()
                    )));
                }
            }
        }
        EntryType::Script => {
            for segment in &manifest.provided_segments {
                let segment_manifest = script_segment_manifest_path(root, segment);
                if !segment_manifest.is_file() {
                    return Err(Error::invalid(format!(
                        "Missing script segment manifest for '{segment}': {}",
                        segment_manifest.display()
                    )));
                }
            }
        }
        EntryType::Command => return Err(unsupported_entry_type(&manifest.entry_type)),
    }

    DiscoveredPlugin::load_script_segments(manifest, root)
}

/// Resolve a plugin path target into manifest and root paths.
fn resolve_manifest_target(path: &Path) -> Result<(PathBuf, PathBuf)> {
    let plugin_root = if path.is_dir() {
        path.to_path_buf()
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| Error::invalid("Path has no parent directory"))?;
        // A bare filename like `plugin.toml` has an empty parent; it means the cwd.
        if parent.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            parent.to_path_buf()
        }
    };
    let manifest_path = if plugin_root.ends_with("plugin.toml") {
        plugin_root
    } else {
        plugin_root.join("plugin.toml")
    };
    let root = manifest_path
        .parent()
        .ok_or_else(|| Error::invalid("Plugin manifest has no parent directory"))?
        .to_path_buf();
    Ok((manifest_path, root))
}

/// Load and parse a plugin manifest from a filesystem path target.
fn load_manifest_from_path(path: &Path) -> Result<(PluginManifest, PathBuf, ScriptSegments)> {
    let (manifest_path, root) = resolve_manifest_target(path)?;
    let content = std::fs::read_to_string(&manifest_path).map_err(|e| {
        Error::invalid(format!(
            "Failed to read plugin manifest {}: {e}",
            manifest_path.display()
        ))
    })?;
    let manifest = crate::plugin::PluginManifest::parse(&content)?;
    let script_segments = validate_plugin_root(&root, &manifest)?;
    Ok((manifest, root, script_segments))
}

/// One-line scope note printed by `gpy plugin new`, `validate` and `list`.
///
/// Applies only to v1 `.fish` plugins (`entry_type = "file"`); `script` plugins run in every shell.
pub(crate) const FISH_ONLY_NOTE: &str =
    "Plugin segments from v1 `.fish` files load in Fish only; Bash and Zsh prompts skip them.";

/// Whether `plugin` is a user v1 `.fish` plugin; built-in plugins load in every shell.
const fn is_fish_only_plugin(plugin: &DiscoveredPlugin) -> bool {
    !matches!(plugin.source, PluginSource::FirstParty)
        && matches!(plugin.manifest.entry_type, EntryType::File)
}

/// Print the Fish-only note when `entry_type` is a v1 `.fish` file plugin.
fn print_fish_only_note(entry_type: &EntryType) {
    if matches!(entry_type, EntryType::File) {
        println!("   note: {FISH_ONLY_NOTE}");
    }
}

fn print_manifest_validation(manifest: &PluginManifest, root: &Path, segments: &ScriptSegments) {
    crate::commands::output::ok(format_args!(
        "Valid plugin: {} {} (api {})",
        manifest.id, manifest.version, manifest.api_version
    ));
    println!("   root: {}", root.display());
    println!("   entry: {}", manifest.entry_type.as_str());
    println!("   segments: {}", {
        let list = manifest
            .provided_segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if list.is_empty() {
            "(none)".to_owned()
        } else {
            list.join(", ")
        }
    });
    if matches!(manifest.entry_type, EntryType::File) {
        println!("   public helpers: {}", PUBLIC_PLUGIN_HELPERS.join(", "));
    }
    print_min_version(manifest.compatibility_constraints.as_ref());
    print_fish_only_note(&manifest.entry_type);
    print_script_segments(segments);
}

/// Joined `items`, or `-` when empty.
fn joined<T>(items: &[T], show: impl Fn(&T) -> &str) -> String {
    if items.is_empty() {
        "-".to_owned()
    } else {
        items.iter().map(show).collect::<Vec<_>>().join(", ")
    }
}

/// One block per script segment: what runs, with what, how often and when.
fn print_script_segments(segments: &ScriptSegments) {
    print!("{}", script_segments_report(segments));
}

/// The text [`print_script_segments`] prints. The description is the only
/// free manifest text here (the other fields reject controls), so it is
/// stripped of control characters before it reaches the terminal.
fn script_segments_report(segments: &ScriptSegments) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    // Writing to a String cannot fail.
    for (name, segment) in segments {
        let _ = writeln!(out, "   segment {name}:");
        if let Some(description) = &segment.description {
            let _ = writeln!(
                out,
                "     description: {}",
                crate::template::sanitize_control_chars(description)
            );
        }
        let _ = writeln!(out, "     command: {}", segment.command.as_str());
        let _ = writeln!(
            out,
            "     inputs: {}",
            joined(&segment.inputs, |i| i.as_str())
        );
        let _ = writeln!(out, "     env: {}", joined(&segment.env, |n| n.as_str()));
        let _ = writeln!(
            out,
            "     cache: {} scope, ttl_secs {}, timeout_ms {}",
            segment.cache_scope().as_str(),
            segment.ttl_secs,
            segment.timeout_ms
        );
        for (key, value) in [
            (
                "require_files",
                joined(&segment.require_files, |n| n.as_str()),
            ),
            (
                "require_folders",
                joined(&segment.require_folders, |n| n.as_str()),
            ),
            (
                "require_extensions",
                joined(&segment.require_extensions, |e| e.as_str()),
            ),
            ("require_env", joined(&segment.require_env, |n| n.as_str())),
        ] {
            if value != "-" {
                let _ = writeln!(out, "     {key}: {value}");
            }
        }
        if let Some(require_command) = &segment.require_command {
            let _ = writeln!(out, "     require_command: {}", require_command.as_str());
        }
    }
    out
}

fn print_discovered_plugin_validation(plugin: &crate::plugin::DiscoveredPlugin) {
    crate::commands::output::ok(format_args!(
        "Valid discovered plugin: {} {} [{}]",
        plugin.manifest.id,
        plugin.manifest.version,
        render_source(plugin.source)
    ));
    println!("   root: {}", plugin.root.display());
    println!(
        "   segments: {}",
        plugin
            .manifest
            .provided_segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    print_min_version(plugin.manifest.compatibility_constraints.as_ref());
    print_fish_only_note(&plugin.manifest.entry_type);
    print_script_segments(&plugin.script_segments);
}

/// Print the declared minimum; validation already rejected an unmet one.
fn print_min_version(compatibility_constraints: Option<&crate::plugin::CompatibilityConstraints>) {
    if let Some(min_version) =
        compatibility_constraints.and_then(|constraints| constraints.min_gpy_version.as_ref())
    {
        println!("   min gpy version: {min_version}");
    }
}

fn scaffold_manifest(plugin_id: &crate::plugin::PluginId, segment: &SegmentName) -> String {
    format!(
        r#"# GPY plugin manifest
id = "{plugin_id}"
name = "{plugin_id}"
version = "0.1.0"
api_version = "{api_version}"
entry_type = "file"
provided_segments = ["{segment}"]
description = "Describe what this plugin shows"

[compatibility_constraints]
min_gpy_version = "{min_gpy_version}"
"#,
        api_version = PluginApiVersion::V1,
        min_gpy_version = env!("CARGO_PKG_VERSION"),
    )
}

fn scaffold_segment(segment: &SegmentName) -> String {
    let segment_token = segment.variable_token();
    format!(
        r#"# SPDX-License-Identifier: GPL-3.0-or-later

function segment_{segment}_detect
    return 0
end

function segment_{segment}_render --argument-names is_last
    set -l bg_color (set -q __gpy_segment_{segment_token}_bg_color; and echo $__gpy_segment_{segment_token}_bg_color; or echo blue)
    set -l fg_color (set -q __gpy_segment_{segment_token}_text_color; and echo $__gpy_segment_{segment_token}_text_color; or echo white)
    set -l icon (set -q __gpy_segment_{segment_token}_icon; and echo $__gpy_segment_{segment_token}_icon; or echo "*")

    gpy_section_standalone $bg_color $fg_color "$icon {segment}" $is_last
end
"#
    )
}

/// List all discovered plugins and diagnostics.
pub fn list() {
    let discovery = discover_plugins();
    let user_root = user_plugins_dir();

    println!("Plugins:");
    println!("========\n");
    println!("User plugin root: {}", user_root.display());
    if discovery.plugins.iter().any(is_fish_only_plugin) {
        println!("{FISH_ONLY_NOTE}");
    }

    if discovery.plugins.is_empty() {
        println!("(none discovered)");
    } else {
        for plugin in discovery.plugins {
            println!(
                "\n- {} {} [{}] (ready)",
                plugin.manifest.id,
                plugin.manifest.version,
                render_source(plugin.source)
            );
            println!("  api: {}", plugin.manifest.api_version);
            println!("  entry: {}", plugin.manifest.entry_type.as_str());
            println!("  root: {}", plugin.root.display());
            println!(
                "  segments: {}",
                plugin
                    .manifest
                    .provided_segments
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    if !discovery.diagnostics.is_empty() {
        println!("\nDiagnostics:");
        for diagnostic in discovery.diagnostics {
            println!(
                "- [{}] {}: {}",
                render_source(diagnostic.source),
                diagnostic.plugin_dir.display(),
                diagnostic.message
            );
        }
        println!("\nHint: run `gpy plugin validate <path-or-id>` for a specific plugin.");
    }
}

/// Validate a plugin by filesystem path (directory containing `plugin.toml`) or plugin ID.
///
/// # Errors
/// Returns an error if the target plugin is invalid or cannot be found.
pub fn validate(path_or_id: &str) -> Result<()> {
    let path = Path::new(path_or_id);
    if path.exists() {
        let (manifest, root, segments) = load_manifest_from_path(path)?;
        print_manifest_validation(&manifest, &root, &segments);
        return Ok(());
    }

    let discovery = discover_plugins();
    if let Some(plugin) = discovery
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.id.as_str() == path_or_id)
    {
        if matches!(plugin.source, PluginSource::FirstParty) {
            // First-party metadata has no filesystem root; never show `<first-party>` as one.
            validate_first_party_metadata(&plugin.manifest)?;
            print_first_party_validation(plugin);
        } else {
            validate_plugin_root(&plugin.root, &plugin.manifest)?;
            print_discovered_plugin_validation(plugin);
        }
        return Ok(());
    }

    Err(Error::invalid(format!(
        "No discovered plugin with id '{path_or_id}', and path does not exist"
    )))
}

/// Validate first-party (built-in) plugin metadata, which has no filesystem root.
fn validate_first_party_metadata(manifest: &PluginManifest) -> Result<()> {
    if !manifest.entry_type.is_runtime_supported() {
        return Err(unsupported_entry_type(&manifest.entry_type));
    }
    if manifest.provided_segments.is_empty() {
        return Err(Error::invalid(
            "First-party plugin declares no segments".to_owned(),
        ));
    }
    Ok(())
}

fn print_first_party_validation(plugin: &crate::plugin::DiscoveredPlugin) {
    crate::commands::output::ok(format_args!(
        "Valid built-in plugin: {} {} (api {})",
        plugin.manifest.id, plugin.manifest.version, plugin.manifest.api_version
    ));
    println!("   source: first-party (built-in, no filesystem root)");
    println!(
        "   segments: {}",
        plugin
            .manifest
            .provided_segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    print_min_version(plugin.manifest.compatibility_constraints.as_ref());
}

/// Create a new plugin scaffold in the user plugin root.
///
/// # Errors
/// Returns an error if the ID is invalid, the target exists, or writing fails.
pub fn new(id: &str, segment_name: Option<&str>) -> Result<()> {
    let plugin_id = crate::plugin::PluginId::new(id)?;
    let segment = SegmentName::new(segment_name.unwrap_or(plugin_id.as_str()))?;
    let plugin_root = user_plugins_dir().join(plugin_id.as_str());

    if plugin_root.exists() {
        return Err(Error::invalid(format!(
            "Plugin directory already exists: {}",
            plugin_root.display()
        )));
    }

    fs::create_dir_all(plugin_root.join("segments")).map_err(|e| {
        Error::invalid(format!(
            "Failed to create plugin directory {}: {e}",
            plugin_root.display()
        ))
    })?;

    let manifest_path = plugin_root.join("plugin.toml");
    let segment_path = plugin_segment_file(&plugin_root, &segment);

    fs::write(&manifest_path, scaffold_manifest(&plugin_id, &segment)).map_err(|e| {
        Error::invalid(format!(
            "Failed to write plugin manifest {}: {e}",
            manifest_path.display()
        ))
    })?;
    fs::write(&segment_path, scaffold_segment(&segment)).map_err(|e| {
        Error::invalid(format!(
            "Failed to write plugin segment {}: {e}",
            segment_path.display()
        ))
    })?;

    println!("Created plugin scaffold");
    println!("   plugin: {}", plugin_id.as_str());
    println!("   root: {}", plugin_root.display());
    println!("   manifest: {}", manifest_path.display());
    println!("   segment file: {}", segment_path.display());
    println!("\nNext steps:");
    println!("  1. Edit the generated files");
    println!("  2. Run `gpy plugin validate {}`", plugin_root.display());
    println!("  3. Run `gpy enable {segment}`");
    println!("\nNote: {FISH_ONLY_NOTE}");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn validate_rejects_command_entry_type_by_path() {
        let temp = TempDir::new().expect("tempdir");
        let manifest = r#"
id = "cmd-plugin"
name = "Cmd Plugin"
version = "0.1.0"
api_version = "v1"
provided_segments = ["cmd_demo"]
entry_type = "command"
"#;
        fs::write(temp.path().join("plugin.toml"), manifest).expect("write manifest");

        let err = validate(temp.path().to_str().expect("utf8 path"))
            .expect_err("command plugin must fail validation");
        assert!(
            err.to_string().contains("Unsupported entry_type"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_first_party_gpy_core_succeeds() {
        validate("gpy-core").expect("first-party gpy-core should validate");
    }

    fn write_script_plugin(root: &Path, min_version: &str) {
        fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
id = "script-plugin"
name = "Script Plugin"
version = "0.1.0"
api_version = "v1"
provided_segments = ["demo"]
entry_type = "script"

[compatibility_constraints]
min_gpy_version = "{min_version}"
"#
            ),
        )
        .expect("write manifest");
    }

    #[test]
    fn validate_script_plugin_needs_no_fish_file() {
        let temp = TempDir::new().expect("tempdir");
        write_script_plugin(temp.path(), "0.0.1");
        let path = temp.path().to_str().expect("utf8 path");

        let err = validate(path).expect_err("segment manifest is required");
        assert!(
            err.to_string()
                .contains("Missing script segment manifest for 'demo'"),
            "unexpected error: {err}"
        );

        fs::create_dir_all(temp.path().join("segments")).expect("segments dir");
        fs::write(
            temp.path().join("segments/demo.toml"),
            "command = \"bin/demo.sh\"\n",
        )
        .expect("write segment manifest");
        fs::create_dir_all(temp.path().join("bin")).expect("bin dir");
        fs::write(temp.path().join("bin/demo.sh"), "echo demo\n").expect("write script");
        validate(path).expect("script plugin without a .fish file validates");
    }

    #[test]
    fn validate_rejects_unmet_min_gpy_version() {
        let temp = TempDir::new().expect("tempdir");
        write_script_plugin(temp.path(), "999.0.0");
        let err = validate(temp.path().to_str().expect("utf8 path"))
            .expect_err("a newer min_gpy_version must fail validation");
        assert!(
            err.to_string().contains("requires GPY 999.0.0"),
            "unexpected error: {err}"
        );
    }

    /// `gpy plugin validate` prints the segment description: ESC and CSI
    /// (U+009B) from the manifest must not reach the terminal.
    #[test]
    fn script_segment_report_strips_controls_from_the_description() {
        let manifest = ScriptSegmentManifest::parse(
            "command = \"bin/demo.sh\"\ndescription = \"Shows \\u001b]0;pwned\\u0007\\u009b31mdemo\"\n",
        )
        .expect("segment manifest");
        let segments = BTreeMap::from([(SegmentName::new("demo").expect("name"), manifest)]);
        let report = script_segments_report(&segments);
        assert!(
            !report.chars().any(|c| c.is_control() && c != '\n'),
            "{report:?}"
        );
        assert!(
            report.contains("     description: Shows ]0;pwned31mdemo\n"),
            "{report}"
        );
    }
}
