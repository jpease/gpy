//! Enabled script segments (`entry_type = "script"`) resolved for the prompt:
//! the interpreter, the canonical script paths, triggers, inputs, `env` names,
//! resolved settings, limits and the static part of the cache key.

use crate::config::Config;
use crate::config::plugin_segments::{self, PluginSegmentConfig};
use crate::debug_log;
use crate::plugin::{
    BuiltinSegment, DiscoveredPlugin, EntryType, InputName, PluginDiscovery, ScriptSegmentManifest,
    SegmentName, check_owned_and_not_shared,
};
use crate::{Error, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The interpreter scripts run with unless the macOS launcher can be skipped.
const DEFAULT_SH: &str = "/bin/sh";

/// On macOS `/bin/sh` is a launcher that runs the shell this link names; running
/// that shell directly, as `sh`, saves about 0.8 ms per script run.
const MACOS_SH_SELECTOR: &str = "/private/var/select/sh";

/// One enabled script segment, resolved for the prompt. Paths are canonical and
/// absolute; every string is UTF-8 without control characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptSegmentExport {
    /// The segment's variable token (`[a-z0-9_]+`).
    pub token: String,
    /// The segment name (`[a-z0-9_-]+`).
    pub name: String,
    /// The plugin root, the script's working directory unless `pwd` is an input.
    pub root: String,
    /// The `command` script.
    pub command: String,
    /// The `require_command` script, or empty.
    pub require_command: String,
    /// Declared inputs, manifest spelling, in manifest order.
    pub inputs: Vec<&'static str>,
    /// Declared `env` names, in manifest order.
    pub env: Vec<String>,
    /// `GPY_OPT_<key>=<value>` for every declared setting, sorted by key.
    pub opts: Vec<String>,
    /// Effective `ttl_secs`.
    pub ttl_secs: u32,
    /// Effective `timeout_ms`.
    pub timeout_ms: u32,
    /// `session` or `folder`.
    pub scope: &'static str,
    /// `require_files`, in manifest order.
    pub require_files: Vec<String>,
    /// `require_folders`, in manifest order.
    pub require_folders: Vec<String>,
    /// `require_extensions`, without the dot, in manifest order.
    pub require_extensions: Vec<String>,
    /// `require_env`, in manifest order.
    pub require_env: Vec<String>,
    /// The static part of the cache key; see [`static_cache_key`].
    pub key: String,
}

/// The enabled script segments and the interpreter they run with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptSegmentExports {
    /// The interpreter path.
    pub sh: String,
    /// Enabled script segments in `ui.enabled_segments` order.
    pub segments: Vec<ScriptSegmentExport>,
}

impl Default for ScriptSegmentExports {
    fn default() -> Self {
        Self {
            sh: DEFAULT_SH.to_owned(),
            segments: Vec::new(),
        }
    }
}

/// Resolve every enabled script segment, in `ui.enabled_segments`
/// order. Built-in names, names that are not script segments and repeated
/// tokens are skipped; a segment that fails containment, the ownership check
/// or its `[plugin.<segment>]` overrides is skipped with a debug log line.
#[must_use]
pub(crate) fn collect_script_segments(
    discovery: &PluginDiscovery,
    config: &Config,
) -> ScriptSegmentExports {
    let mut tokens = HashSet::new();
    let mut segments = Vec::new();
    for raw_name in &config.ui.enabled_segments {
        let Ok(name) = SegmentName::new(raw_name) else {
            continue;
        };
        if BuiltinSegment::try_from(name.as_str()).is_ok() || !tokens.insert(name.variable_token())
        {
            continue;
        }
        let Some((plugin, manifest)) = find_script_segment(discovery, &name) else {
            continue;
        };
        match export_segment(plugin, &name, manifest, config.plugin.get(&name)) {
            Ok(segment) => segments.push(segment),
            Err(e) => debug_log!("theme", "Skipped script segment {name}: {e}"),
        }
    }
    ScriptSegmentExports {
        sh: resolve_sh(Path::new(MACOS_SH_SELECTOR)),
        segments,
    }
}

/// The discovered script plugin providing `name`, with the segment's manifest.
fn find_script_segment<'a>(
    discovery: &'a PluginDiscovery,
    name: &SegmentName,
) -> Option<(&'a DiscoveredPlugin, &'a ScriptSegmentManifest)> {
    discovery
        .plugins
        .iter()
        .filter(|plugin| matches!(plugin.manifest.entry_type, EntryType::Script))
        .find_map(|plugin| Some((plugin, plugin.script_segments.get(name)?)))
}

/// A path as UTF-8 text.
fn path_text(path: PathBuf) -> Result<String> {
    path.into_os_string().into_string().map_err(|raw| {
        Error::invalid(format!(
            "'{}' is not UTF-8",
            raw.to_string_lossy().escape_debug()
        ))
    })
}

/// Resolve one segment. Containment and ownership are checked again here, not
/// only at discovery: the files may have changed since.
fn export_segment(
    plugin: &DiscoveredPlugin,
    name: &SegmentName,
    manifest: &ScriptSegmentManifest,
    overrides: Option<&PluginSegmentConfig>,
) -> Result<ScriptSegmentExport> {
    let root = plugin
        .root
        .as_deref()
        .ok_or_else(|| Error::invalid("the plugin has no root directory"))?;
    let command = path_text(manifest.command.resolve(root)?)?;
    let require_command = manifest
        .require_command
        .as_ref()
        .map(|script| script.resolve(root).and_then(path_text))
        .transpose()?
        .unwrap_or_default();
    // `resolve` checked that the scripts' canonical paths, which start with
    // this one, are UTF-8 without control characters.
    let canonical_root = path_text(root.canonicalize().map_err(|e| {
        Error::invalid(format!(
            "cannot resolve plugin root '{}': {e}",
            root.to_string_lossy().escape_debug()
        ))
    })?)?;
    let resolved = plugin_segments::resolve(name, manifest, overrides)?;
    let opts: Vec<String> = resolved
        .settings
        .iter()
        .map(|(key, value)| format!("{}={value}", key.env_var()))
        .collect();
    let version = plugin.manifest.version.as_str();
    if let Some(text) = std::iter::once(version)
        .chain(opts.iter().map(String::as_str))
        .find(|text| text.chars().any(char::is_control))
    {
        return Err(Error::invalid(format!(
            "'{}' holds a control character",
            text.escape_debug()
        )));
    }
    let key = static_cache_key(
        [
            plugin.manifest.id.as_str(),
            version,
            name.as_str(),
            command.as_str(),
        ]
        .into_iter()
        .chain(opts.iter().map(String::as_str)),
    );
    Ok(ScriptSegmentExport {
        token: name.variable_token(),
        name: name.as_str().to_owned(),
        root: canonical_root,
        command,
        require_command,
        inputs: manifest
            .inputs
            .iter()
            .copied()
            .map(InputName::as_str)
            .collect(),
        env: strings(&manifest.env),
        opts,
        ttl_secs: resolved.ttl_secs.get(),
        timeout_ms: resolved.timeout_ms.get(),
        scope: manifest.cache_scope().as_str(),
        require_files: strings(&manifest.require_files),
        require_folders: strings(&manifest.require_folders),
        require_extensions: strings(&manifest.require_extensions),
        require_env: strings(&manifest.require_env),
        key,
    })
}

/// Manifest names as owned text (their `Display` is the name itself).
fn strings<T: ToString>(items: &[T]) -> Vec<String> {
    items.iter().map(ToString::to_string).collect()
}

/// Each field as `<UTF-8 byte length>:<field>`, concatenated: no field can
/// shift into its neighbour, whatever characters it holds.
fn static_cache_key<'a>(fields: impl IntoIterator<Item = &'a str>) -> String {
    let mut key = String::new();
    for field in fields {
        key.push_str(&field.len().to_string());
        key.push(':');
        key.push_str(field);
    }
    key
}

/// The interpreter path: on macOS the shell the `/bin/sh` launcher would run
/// (see [`launcher_target`]), else `/bin/sh`.
fn resolve_sh(selector: &Path) -> String {
    if cfg!(target_os = "macos")
        && let Some(target) = launcher_target(selector)
    {
        return target;
    }
    DEFAULT_SH.to_owned()
}

/// The file `selector` resolves to, if it is an executable regular file at a
/// plain UTF-8 path that only the user or root can change.
fn launcher_target(selector: &Path) -> Option<String> {
    let target = selector.canonicalize().ok()?;
    let meta = target.metadata().ok()?;
    if !meta.is_file() || !is_executable(&meta) {
        return None;
    }
    check_owned_and_not_shared(&target).ok()?;
    path_text(target)
        .ok()
        .filter(|text| !text.chars().any(char::is_control))
}

fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(test)]
#[path = "script_segments_tests.rs"]
mod tests;
