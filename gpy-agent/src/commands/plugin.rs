//! `gpy plugin` command handlers.
//!
//! Plugin commands present discovered manifests, diagnostics, and exported
//! shell integration in CLI-friendly form. The durable plugin model and search
//! paths live in [`crate::plugin`]; this module is responsible for command
//! output, validation display, and integration with user workflows.

use crate::plugin::{
    EntryType, GpyVersion, PluginApiVersion, PluginManifest, PluginSource, SegmentName,
    discover_plugins, user_plugins_dir,
};
use crate::{Error, Result, VERSION};
use std::path::Path;
use std::{fs, path::PathBuf};

pub(crate) const fn render_source(source: PluginSource) -> &'static str {
    match source {
        PluginSource::FirstParty => "first-party",
        PluginSource::Bundled => "bundled",
        PluginSource::User => "user",
    }
}

const fn render_status(status: crate::plugin::PluginLoadStatus) -> &'static str {
    match status {
        crate::plugin::PluginLoadStatus::Ready => "ready",
    }
}

const PUBLIC_PLUGIN_HELPERS: &[&str] = &[
    "gpy_section_start",
    "gpy_section_append",
    "gpy_section_end",
    "gpy_section_standalone",
];

/// Parse the current GPY version into the typed plugin compatibility form.
///
/// # Errors
///
/// Returns an error if the current crate version does not match the plugin
/// manifest semver format expected by `GpyVersion`.
pub(crate) fn current_gpy_version() -> Result<GpyVersion> {
    GpyVersion::new(VERSION)
}

const fn render_entry_type(entry_type: &EntryType) -> &'static str {
    match entry_type {
        EntryType::File => "file",
        EntryType::Command => "command",
    }
}

fn plugin_segment_file(root: &Path, segment: &crate::plugin::SegmentName) -> PathBuf {
    root.join("segments").join(format!("{segment}.fish"))
}

/// Validate that a plugin root contains the files required by its manifest.
///
/// # Errors
///
/// Returns an error if the plugin root or any required segment file is missing.
fn validate_plugin_root(root: &Path, manifest: &PluginManifest) -> Result<()> {
    // Reject entry types the runtime cannot load before touching the filesystem,
    // so unsupported plugins fail validation instead of failing at prompt init (#175).
    if !manifest.entry_type.is_runtime_supported() {
        return Err(Error::invalid(format!(
            "Unsupported entry_type '{}': only 'file' plugins are loadable by this runtime",
            manifest.entry_type.as_str()
        )));
    }

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

    if matches!(manifest.entry_type, EntryType::File) {
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

    Ok(())
}

/// Resolve a plugin path target into manifest and root paths.
///
/// # Errors
///
/// Returns an error if the path cannot be mapped to a plugin root and manifest path.
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
///
/// # Errors
///
/// Returns an error if the manifest cannot be resolved, read, parsed, or if the plugin root is incomplete.
fn load_manifest_from_path(path: &Path) -> Result<(PluginManifest, PathBuf)> {
    let (manifest_path, root) = resolve_manifest_target(path)?;
    let content = std::fs::read_to_string(&manifest_path).map_err(|e| {
        Error::invalid(format!(
            "Failed to read plugin manifest {}: {e}",
            manifest_path.display()
        ))
    })?;
    let manifest = crate::plugin::PluginManifest::parse(&content)?;
    validate_plugin_root(&root, &manifest)?;
    Ok((manifest, root))
}

fn print_manifest_validation(manifest: &PluginManifest, root: &Path) {
    println!(
        "✅ Valid plugin: {} {} (api {})",
        manifest.id, manifest.version, manifest.api_version
    );
    println!("   root: {}", root.display());
    println!("   entry: {}", render_entry_type(&manifest.entry_type));
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
    println!("   public helpers: {}", PUBLIC_PLUGIN_HELPERS.join(", "));
    print_min_version_warning(manifest.compatibility_constraints.as_ref());
}

fn print_discovered_plugin_validation(plugin: &crate::plugin::DiscoveredPlugin) {
    println!(
        "✅ Valid discovered plugin: {} {} [{}]",
        plugin.manifest.id,
        plugin.manifest.version,
        render_source(plugin.source)
    );
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
    print_min_version_warning(plugin.manifest.compatibility_constraints.as_ref());
}

fn print_min_version_warning(
    compatibility_constraints: Option<&crate::plugin::CompatibilityConstraints>,
) {
    if let Some(constraints) = compatibility_constraints
        && let Some(min_version) = &constraints.min_gpy_version
    {
        println!("   min gpy version: {min_version}");
        if let Ok(current) = current_gpy_version()
            && min_version.is_greater_than(&current)
        {
            println!(
                "   compatibility warning: this plugin requires GPY {min_version}+, current is {current}"
            );
        }
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
    let segment_token = segment.as_str().replace('-', "_");
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
///
/// # Errors
///
/// Returns an error if plugin discovery fails unexpectedly.
pub fn list() {
    let discovery = discover_plugins();
    let user_root = user_plugins_dir();

    println!("Plugins:");
    println!("========\n");
    println!("User plugin root: {}", user_root.display());

    if discovery.plugins.is_empty() {
        println!("(none discovered)");
    } else {
        for plugin in discovery.plugins {
            println!(
                "\n- {} {} [{}] ({})",
                plugin.manifest.id,
                plugin.manifest.version,
                render_source(plugin.source),
                render_status(plugin.status)
            );
            println!("  api: {}", plugin.manifest.api_version);
            println!(
                "  entry: {}",
                render_entry_type(&plugin.manifest.entry_type)
            );
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
///
/// Returns an error if the target plugin is invalid or cannot be found.
pub fn validate(path_or_id: &str) -> Result<()> {
    let path = Path::new(path_or_id);
    if path.exists() {
        let (manifest, root) = load_manifest_from_path(path)?;
        print_manifest_validation(&manifest, &root);
        return Ok(());
    }

    let discovery = discover_plugins();
    if let Some(plugin) = discovery
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.id.as_str() == path_or_id)
    {
        if matches!(plugin.source, PluginSource::FirstParty) {
            // First-party metadata is synthesized in-process and has no filesystem
            // root; validate its manifest invariants rather than a directory, and
            // never present the synthetic `<first-party>` root as real (#176).
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
///
/// # Errors
///
/// Returns an error if the synthesized metadata is internally inconsistent.
fn validate_first_party_metadata(manifest: &PluginManifest) -> Result<()> {
    if !manifest.entry_type.is_runtime_supported() {
        return Err(Error::invalid(format!(
            "Unsupported entry_type '{}': only 'file' plugins are loadable by this runtime",
            manifest.entry_type.as_str()
        )));
    }
    if manifest.provided_segments.is_empty() {
        return Err(Error::invalid(
            "First-party plugin declares no segments".to_owned(),
        ));
    }
    Ok(())
}

fn print_first_party_validation(plugin: &crate::plugin::DiscoveredPlugin) {
    println!(
        "✅ Valid built-in plugin: {} {} (api {})",
        plugin.manifest.id, plugin.manifest.version, plugin.manifest.api_version
    );
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
    print_min_version_warning(plugin.manifest.compatibility_constraints.as_ref());
}

/// Create a new plugin scaffold in the user plugin root.
///
/// # Errors
///
/// Returns an error if the ID is invalid, the target already exists, or files cannot be written.
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

    println!("✅ Created plugin scaffold");
    println!("   plugin: {}", plugin_id.as_str());
    println!("   root: {}", plugin_root.display());
    println!("   manifest: {}", manifest_path.display());
    println!("   segment file: {}", segment_path.display());
    println!("\nNext steps:");
    println!("  1. Edit the generated files");
    println!("  2. Run `gpy plugin validate {}`", plugin_root.display());
    println!("  3. Run `gpy enable {segment}`");
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
        // gpy-core is synthesized first-party metadata with no filesystem root;
        // validation must succeed via metadata rather than a directory check (#176).
        validate("gpy-core").expect("first-party gpy-core should validate");
    }
}
