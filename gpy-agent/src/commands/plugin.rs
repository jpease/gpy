//! `gpy plugin` command handlers.

use super::script_lint::{Lint, Undeclared, lint_script, undeclared_vars};
use crate::commands::output::{self, Status};
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

/// Cap on the script text read for the `$VAR` scan.
const SCRIPT_READ_CAP: usize = 1_048_576;

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
        "unsupported entry_type '{}': only 'file' and 'script' plugins are loadable by this runtime",
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
            "plugin root '{}' is not a directory",
            root.display()
        )));
    }

    let manifest_path = root.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(Error::invalid(format!(
            "missing plugin manifest: {}",
            manifest_path.display()
        )));
    }

    match manifest.entry_type {
        EntryType::File => {
            for segment in &manifest.provided_segments {
                let segment_file = plugin_segment_file(root, segment);
                if !segment_file.is_file() {
                    return Err(Error::invalid(format!(
                        "missing segment file for '{segment}': {}",
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
                        "missing script segment manifest for '{segment}': {}",
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
            .ok_or_else(|| Error::invalid("path has no parent directory"))?;
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
        .ok_or_else(|| Error::invalid("plugin manifest has no parent directory"))?
        .to_path_buf();
    Ok((manifest_path, root))
}

/// Load and parse a plugin manifest from a filesystem path target.
fn load_manifest_from_path(path: &Path) -> Result<(PluginManifest, PathBuf, ScriptSegments)> {
    let (manifest_path, root) = resolve_manifest_target(path)?;
    let content = std::fs::read_to_string(&manifest_path).map_err(|e| {
        Error::io_context(
            format_args!("failed to read plugin manifest {}", manifest_path.display()),
            &e,
        )
    })?;
    let manifest = crate::plugin::PluginManifest::parse(&content)?;
    let script_segments = validate_plugin_root(&root, &manifest)?;
    Ok((manifest, root, script_segments))
}

/// One-line scope note printed by `gpy plugin new`, `validate` and `list`.
///
/// Applies only to v1 `.fish` plugins (`entry_type = "file"`); `script` plugins are not run by any shell yet.
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
        output::detail(format_args!("note: {FISH_ONLY_NOTE}"));
    }
}

/// Comma-separated segment names, or `(none)`.
fn segment_list(segments: &[SegmentName]) -> String {
    if segments.is_empty() {
        "(none)".to_owned()
    } else {
        segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn print_manifest_validation(manifest: &PluginManifest, root: &Path, segments: &ScriptSegments) {
    output::ok(format_args!(
        "Valid plugin: {} {} (api {})",
        manifest.id, manifest.version, manifest.api_version
    ));
    output::detail(format_args!("root: {}", root.display()));
    output::detail(format_args!("entry: {}", manifest.entry_type.as_str()));
    output::detail(format_args!(
        "segments: {}",
        segment_list(&manifest.provided_segments)
    ));
    if matches!(manifest.entry_type, EntryType::File) {
        output::detail(format_args!(
            "public helpers: {}",
            PUBLIC_PLUGIN_HELPERS.join(", ")
        ));
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
    for text in script_segments_report(segments).lines() {
        output::plain(text);
    }
}

/// The text [`print_script_segments`] prints. The description is the only
/// free manifest text here (the other fields reject controls), so it is
/// stripped of control characters before it reaches the terminal.
fn script_segments_report(segments: &ScriptSegments) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let mut line = |depth: usize, text: &dyn std::fmt::Display| {
        // Writing to a String cannot fail.
        let _ = writeln!(out, "{}{text}", output::indent(depth));
    };
    for (name, segment) in segments {
        line(1, &format_args!("segment {name}:"));
        if let Some(description) = &segment.description {
            line(
                2,
                &format_args!(
                    "description: {}",
                    crate::template::sanitize_control_chars(description)
                ),
            );
        }
        line(2, &format_args!("command: {}", segment.command.as_str()));
        line(
            2,
            &format_args!("inputs: {}", joined(&segment.inputs, |i| i.as_str())),
        );
        line(
            2,
            &format_args!("env: {}", joined(&segment.env, |n| n.as_str())),
        );
        line(
            2,
            &format_args!(
                "cache: {} scope, ttl_secs {}, timeout_ms {}",
                segment.cache_scope().as_str(),
                segment.ttl_secs,
                segment.timeout_ms
            ),
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
                line(2, &format_args!("{key}: {value}"));
            }
        }
        if let Some(require_command) = &segment.require_command {
            line(
                2,
                &format_args!("require_command: {}", require_command.as_str()),
            );
        }
    }
    out
}

fn print_discovered_plugin_validation(plugin: &crate::plugin::DiscoveredPlugin, root: &Path) {
    output::ok(format_args!(
        "Valid discovered plugin: {} {} [{}]",
        plugin.manifest.id, plugin.manifest.version, plugin.source
    ));
    output::detail(format_args!("root: {}", root.display()));
    output::detail(format_args!(
        "segments: {}",
        segment_list(&plugin.manifest.provided_segments)
    ));
    print_min_version(plugin.manifest.compatibility_constraints.as_ref());
    print_fish_only_note(&plugin.manifest.entry_type);
    print_script_segments(&plugin.script_segments);
}

/// Print the declared minimum; validation already rejected an unmet one.
fn print_min_version(compatibility_constraints: Option<&crate::plugin::CompatibilityConstraints>) {
    if let Some(min_version) =
        compatibility_constraints.and_then(|constraints| constraints.min_gpy_version.as_ref())
    {
        output::detail(format_args!("min gpy version: {min_version}"));
    }
}

/// One line for `gpy plugin list`: cache behaviour and the description.
fn script_summary(segment: &ScriptSegmentManifest) -> String {
    let mut summary = format!(
        "{} scope, ttl_secs {}, timeout_ms {}",
        segment.cache_scope().as_str(),
        segment.ttl_secs,
        segment.timeout_ms
    );
    if let Some(description) = &segment.description {
        summary.push_str(" - ");
        summary.push_str(&crate::template::sanitize_control_chars(description));
    }
    summary
}

/// What `validate` found in one script of a segment.
struct ScriptCheck {
    /// The script as written in the segment manifest.
    script: String,
    lint: Lint,
    undeclared: Vec<Undeclared>,
}

/// The checks of every script (`command`, `require_command`) of one segment.
struct SegmentCheck {
    segment: SegmentName,
    scripts: Vec<ScriptCheck>,
}

/// Lint each script and scan it for `$VAR` uses the manifest does not provide.
fn check_scripts(root: &Path, segments: &ScriptSegments) -> Vec<SegmentCheck> {
    segments
        .iter()
        .map(|(segment, manifest)| SegmentCheck {
            segment: segment.clone(),
            scripts: std::iter::once(&manifest.command)
                .chain(manifest.require_command.as_ref())
                .filter_map(|script| {
                    // Containment was checked when the manifest loaded.
                    let resolved = script.resolve(root).ok()?;
                    let text = crate::fs_util::read_small_file(&resolved, SCRIPT_READ_CAP)
                        .unwrap_or_default();
                    Some(ScriptCheck {
                        script: script.as_str().to_owned(),
                        lint: lint_script(&resolved),
                        undeclared: undeclared_vars(&text, manifest),
                    })
                })
                .collect(),
        })
        .collect()
}

/// Print every lint failure and fail; nothing is printed when all scripts pass.
fn ensure_lints_pass(checks: &[SegmentCheck]) -> Result<()> {
    let mut failed = 0_usize;
    for check in checks {
        for script in &check.scripts {
            if let Lint::Failed { tool, report } = &script.lint {
                failed = failed.saturating_add(1);
                output::fail(format_args!(
                    "segment {}: {} failed {tool}",
                    check.segment, script.script
                ));
                for line in report.lines() {
                    output::detail(crate::template::sanitize_control_chars(line));
                }
            }
        }
    }
    if failed == 0 {
        Ok(())
    } else {
        Err(Error::invalid(format!(
            "{failed} script(s) failed the shell lint; fix the findings above"
        )))
    }
}

/// Print the lint verdict and the `$VAR` warnings of each script.
fn print_script_checks(checks: &[SegmentCheck]) {
    for check in checks {
        for script in &check.scripts {
            let prefix = format!("segment {}: {}", check.segment, script.script);
            match &script.lint {
                Lint::Clean(tool) => {
                    output::detail_status(
                        1,
                        Status::Ok,
                        format_args!("{prefix}: {tool} found no problems"),
                    );
                }
                Lint::Unavailable => output::detail_status(
                    1,
                    Status::Warn,
                    format_args!("{prefix}: not linted; install shellcheck or dash"),
                ),
                Lint::Failed { .. } => {}
            }
            for var in &script.undeclared {
                output::detail_status(
                    1,
                    Status::Warn,
                    format_args!(
                        "{prefix}: uses ${}, which is not provided: {}",
                        var.name, var.fix
                    ),
                );
            }
        }
    }
}

/// Plugin-relative path of the scaffold's script.
fn scaffold_script_path(segment: &SegmentName) -> String {
    format!("bin/{segment}.sh")
}

fn scaffold_manifest(plugin_id: &crate::plugin::PluginId, segment: &SegmentName) -> String {
    format!(
        r#"# GPY plugin manifest
id = "{plugin_id}"
name = "{plugin_id}"
version = "0.1.0"
api_version = "{api_version}"
entry_type = "script"
provided_segments = ["{segment}"]
description = "Describe what this plugin shows"

[compatibility_constraints]
min_gpy_version = "{min_gpy_version}"
"#,
        api_version = PluginApiVersion::V1,
        min_gpy_version = env!("CARGO_PKG_VERSION"),
    )
}

fn scaffold_segment_manifest(segment: &SegmentName) -> String {
    format!(
        r#"# Script segment manifest; schema: gpy-agent/schemas/script-segment-manifest.json
description = "Describe what this segment shows"
command = "{script}"

# Public shell values the script reads as GPY_<NAME>: pwd, is_root, is_sudo,
# is_ssh, cmd_duration, prompt_icons. Declaring "pwd" runs the script in the
# shell's directory and caches its result per folder.
inputs = []

# Extra environment variables the script may read. They also key the cache.
env = []

# Seconds a result is reused; keep it short. 0 runs on every due prompt.
ttl_secs = 5
# The script is killed after this many milliseconds (at most 500).
timeout_ms = 300
"#,
        script = scaffold_script_path(segment),
    )
}

fn scaffold_script(segment: &SegmentName) -> String {
    format!(
        r#"#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Line 1 is the segment text (empty output hides the segment). Line 2 may name
# a colour variant. Rules: POSIX sh only; at most 4 KiB of output; exit fast;
# no background jobs. The environment is clean: only PATH, HOME, USER, LOGNAME,
# LANG, LC_*, TERM, TZ, TMPDIR, the names in `env` and the GPY_* values for the
# declared `inputs` reach the script. Check it with `gpy plugin validate`.

printf '%s\n' "{segment}"
"#
    )
}

/// List all discovered plugins and diagnostics.
pub fn list() {
    let discovery = discover_plugins();
    let user_root = user_plugins_dir();

    output::heading("Plugins:");
    output::plain(format_args!("User plugin root: {}", user_root.display()));
    if discovery.plugins.iter().any(is_fish_only_plugin) {
        output::plain(FISH_ONLY_NOTE);
    }

    if discovery.plugins.is_empty() {
        output::plain("(none discovered)");
    } else {
        for plugin in discovery.plugins {
            output::plain(format_args!(
                "\n- {} {} [{}] (ready)",
                plugin.manifest.id, plugin.manifest.version, plugin.source
            ));
            output::detail(format_args!("api: {}", plugin.manifest.api_version));
            output::detail(format_args!(
                "entry: {}",
                plugin.manifest.entry_type.as_str()
            ));
            if let Some(root) = &plugin.root {
                output::detail(format_args!("root: {}", root.display()));
            }
            output::detail(format_args!(
                "segments: {}",
                segment_list(&plugin.manifest.provided_segments)
            ));
            for (name, segment) in &plugin.script_segments {
                output::detail(format_args!("segment {name}: {}", script_summary(segment)));
            }
        }
    }

    if !discovery.diagnostics.is_empty() {
        output::plain("\nDiagnostics:");
        for diagnostic in discovery.diagnostics {
            output::plain(format_args!(
                "- [{}] {}: {}",
                diagnostic.source,
                diagnostic.plugin_dir.display(),
                diagnostic.message
            ));
        }
        output::plain("\nHint: run `gpy plugin validate <path-or-id>` for a specific plugin.");
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
        let checks = check_scripts(&root, &segments);
        ensure_lints_pass(&checks)?;
        let themes = super::plugin_themes::validate_plugin_themes(&root)?;
        print_manifest_validation(&manifest, &root, &segments);
        print_script_checks(&checks);
        super::plugin_themes::print_plugin_themes(&themes);
        return Ok(());
    }

    let discovery = discover_plugins();
    if let Some(plugin) = discovery
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.id.as_str() == path_or_id)
    {
        if let Some(root) = &plugin.root {
            let segments = validate_plugin_root(root, &plugin.manifest)?;
            let checks = check_scripts(root, &segments);
            ensure_lints_pass(&checks)?;
            let themes = super::plugin_themes::validate_plugin_themes(root)?;
            print_discovered_plugin_validation(plugin, root);
            print_script_checks(&checks);
            super::plugin_themes::print_plugin_themes(&themes);
        } else {
            validate_first_party_metadata(&plugin.manifest)?;
            print_first_party_validation(plugin);
        }
        return Ok(());
    }

    Err(Error::invalid(format!(
        "no discovered plugin with id '{path_or_id}', and path does not exist"
    )))
}

/// Validate first-party (built-in) plugin metadata, which has no filesystem root.
fn validate_first_party_metadata(manifest: &PluginManifest) -> Result<()> {
    if !manifest.entry_type.is_runtime_supported() {
        return Err(unsupported_entry_type(&manifest.entry_type));
    }
    if manifest.provided_segments.is_empty() {
        return Err(Error::invalid(
            "first-party plugin declares no segments".to_owned(),
        ));
    }
    Ok(())
}

fn print_first_party_validation(plugin: &crate::plugin::DiscoveredPlugin) {
    output::ok(format_args!(
        "Valid built-in plugin: {} {} (api {})",
        plugin.manifest.id, plugin.manifest.version, plugin.manifest.api_version
    ));
    output::detail("source: first-party (built-in, no filesystem root)");
    output::detail(format_args!(
        "segments: {}",
        segment_list(&plugin.manifest.provided_segments)
    ));
    print_min_version(plugin.manifest.compatibility_constraints.as_ref());
}

/// Create a new plugin scaffold in the user plugin root.
///
/// # Errors
/// Returns an error if the ID is invalid, the target exists, or writing fails.
#[allow(
    clippy::create_dir,
    reason = "create_dir fails with AlreadyExists, which is how a concurrent or repeated `plugin new` is refused"
)]
pub fn new(id: &str, segment_name: Option<&str>) -> Result<()> {
    let plugin_id = crate::plugin::PluginId::new(id)?;
    let segment = SegmentName::new(segment_name.unwrap_or(plugin_id.as_str()))?;
    let plugin_root = user_plugins_dir().join(plugin_id.as_str());

    let already_exists = || {
        Error::invalid(format!(
            "plugin directory already exists: {}",
            plugin_root.display()
        ))
    };
    let create_failed = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            already_exists()
        } else {
            Error::io_context(
                format_args!(
                    "failed to create plugin directory {}",
                    plugin_root.display()
                ),
                &e,
            )
        }
    };
    if let Some(parent) = plugin_root.parent() {
        fs::create_dir_all(parent).map_err(create_failed)?;
    }
    // Modes are set at creation (narrowed by the umask, never widened), so a
    // `umask 0002` cannot leave group-writable paths the script policy rejects.
    for dir in [
        plugin_root.clone(),
        plugin_root.join("segments"),
        plugin_root.join("bin"),
    ] {
        crate::fs_util::create_dir_unshared(&dir).map_err(create_failed)?;
    }

    let manifest_path = plugin_root.join("plugin.toml");
    let segment_manifest_path = script_segment_manifest_path(&plugin_root, &segment);
    let script_path = plugin_root.join(scaffold_script_path(&segment));

    let write_new = |path: &Path, contents: String, what: &str| {
        crate::fs_util::create_new_unshared(path, contents.as_bytes()).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                already_exists()
            } else {
                Error::io_context(
                    format_args!("failed to write plugin {what} {}", path.display()),
                    &e,
                )
            }
        })
    };
    write_new(
        &manifest_path,
        scaffold_manifest(&plugin_id, &segment),
        "manifest",
    )?;
    write_new(
        &segment_manifest_path,
        scaffold_segment_manifest(&segment),
        "segment manifest",
    )?;
    write_new(&script_path, scaffold_script(&segment), "script")?;

    output::plain("Created plugin scaffold");
    output::detail(format_args!("plugin: {}", plugin_id.as_str()));
    output::detail(format_args!("root: {}", plugin_root.display()));
    output::detail(format_args!("manifest: {}", manifest_path.display()));
    output::detail(format_args!(
        "segment manifest: {}",
        segment_manifest_path.display()
    ));
    output::detail(format_args!("script: {}", script_path.display()));
    output::plain("\nNext steps:");
    output::detail("1. Edit the generated files");
    output::detail(format_args!(
        "2. Keep {} and its directories owned by you and not writable by group or others; GPY skips a script segment that fails this check (`gpy plugin validate` says why).",
        scaffold_script_path(&segment)
    ));
    output::detail(format_args!(
        "3. Run `gpy plugin validate {}`",
        plugin_root.display()
    ));
    output::detail(format_args!("4. Run `gpy enable {segment}`"));
    Ok(())
}

#[cfg(test)]
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
            err.to_string().contains("unsupported entry_type"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_first_party_gpy_core_succeeds() {
        validate(crate::plugin::registry::FIRST_PARTY_ID)
            .expect("first-party plugin should validate");
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
                .contains("missing script segment manifest for 'demo'"),
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
            report.contains("      description: Shows ]0;pwned31mdemo\n"),
            "{report}"
        );
    }
}
