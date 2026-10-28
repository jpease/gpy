//! Resolving theme names to chain links (ADR-0012 section 3).
//!
//! An `extends` names a theme, never a path, and is resolved only through
//! theme discovery: the user themes directory, plugin `themes/` directories
//! and the embedded builtins, matching file stems exactly. A file never
//! resolves to itself, and plugin themes may extend only builtins.

use super::ChainLink;
use crate::Result;
use crate::config::discovery::{Source, toml_stems};
use crate::plugin::discover_plugins;
use crate::theme::ThemeManager;
use crate::theme::user_copy::{is_builtin, user_copy_path};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Where the theme `name` resolves as the requested theme (`ui.theme`,
/// `gpy theme use`): the user file `<config>/themes/<name>.toml` if it exists,
/// else the discovered plugin or user winner, else the embedded builtin.
/// `None` when nothing matches.
///
/// The user file is found by joining `name`, so on a case-insensitive
/// filesystem `Starship` opens `starship.toml`; an overlay's own name is then
/// taken from the directory listing when it loads.
#[must_use]
pub fn resolve_top(name: &str) -> Option<ChainLink> {
    let user_path = user_copy_path(name);
    if user_path.exists() {
        return Some(ChainLink {
            name: name.to_owned(),
            source: Source::User,
            path: user_path,
            on_disk: true,
        });
    }
    if let Some(found) = ThemeManager::discover_available_themes()
        .into_iter()
        .find(|theme| theme.name == name)
        && let Some(path) = found.path
    {
        let on_disk = path.exists();
        return Some(ChainLink {
            name: name.to_owned(),
            source: found.source,
            path,
            on_disk,
        });
    }
    is_builtin(name).then(|| ChainLink::embedded(name))
}

/// The link of a theme file loaded by path: a user theme named after its
/// stem, unless it lies in a plugin's `themes/` directory (whose parent holds
/// `plugin.toml`). A path with no file is the embedded builtin of its stem.
pub(super) fn link_for_path(path: &Path) -> ChainLink {
    let name = path
        .file_stem()
        .and_then(OsStr::to_str)
        .map_or_else(|| path.display().to_string(), str::to_owned);
    let on_disk = path.exists();
    let source = if on_disk {
        plugin_of(path).map_or(Source::User, |plugin_id| Source::Plugin { plugin_id })
    } else {
        Source::Builtin
    };
    ChainLink {
        name,
        source,
        path: path.to_path_buf(),
        on_disk,
    }
}

/// The id of the plugin whose `themes/` directory holds `path`, if one does.
/// A plugin directory discovery does not list is named after the directory.
fn plugin_of(path: &Path) -> Option<String> {
    let themes = path.parent()?;
    if themes.file_name() != Some(OsStr::new("themes")) {
        return None;
    }
    let root = themes.parent()?;
    if !root.join("plugin.toml").is_file() {
        return None;
    }
    let canonical = |dir: &Path| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let wanted = canonical(root);
    let listed = discover_plugins().plugins.into_iter().find(|plugin| {
        plugin
            .root
            .as_deref()
            .is_some_and(|plugin_root| canonical(plugin_root) == wanted)
    });
    Some(listed.map_or_else(
        || {
            root.file_name()
                .map_or_else(String::new, |dir| dir.to_string_lossy().into_owned())
        },
        |plugin| plugin.manifest.id.as_str().to_owned(),
    ))
}

/// How the requested file's own name is found when it is an overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnName {
    /// Requested by name: a user file's own name is its on-disk stem, taken
    /// from the directory listing.
    Listed,
    /// Requested by path: the path's stem.
    Stem,
}

/// `top` with its own name and path taken from its directory listing, when
/// it is a user file requested by name: the entry whose stem equals the
/// requested name, else the single entry equal to it ignoring case (by
/// `str::to_lowercase`).
///
/// # Errors
/// Returns an error if no entry, or more than one, matches.
pub(super) fn own_link(mut top: ChainLink, own: OwnName) -> Result<ChainLink> {
    if own == OwnName::Stem || top.source != Source::User || !top.on_disk {
        return Ok(top);
    }
    let entries: Vec<(String, PathBuf)> = top
        .path
        .parent()
        .map(|dir| toml_stems(dir).collect())
        .unwrap_or_default();
    let Some((stem, path)) = own_entry(entries, &top.name) else {
        return Err(super::overlay_error(
            &top.path.display().to_string(),
            &format!(
                "theme '{}' opened {}, whose file name differs; set ui.theme to the file's name",
                top.name,
                top.path.display()
            ),
            "Set `ui.theme` (or the name passed to `gpy theme`) to the theme file's exact name.",
        ));
    };
    top.name = stem;
    top.path = path;
    Ok(top)
}

/// The directory entry `requested` names: an exact stem match, else the single
/// stem equal to it ignoring case. No Unicode normalisation is applied.
pub(super) fn own_entry(
    entries: Vec<(String, PathBuf)>,
    requested: &str,
) -> Option<(String, PathBuf)> {
    let lowered = requested.to_lowercase();
    let mut folded = Vec::new();
    for (stem, path) in entries {
        if stem == requested {
            return Some((stem, path));
        }
        if stem.to_lowercase() == lowered {
            folded.push((stem, path));
        }
    }
    let mut matches = folded.into_iter();
    let single = matches.next();
    if matches.next().is_some() {
        return None;
    }
    single
}

/// What a chain link is for cycle detection: a file's canonical path, or an
/// embedded builtin's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Identity {
    /// An on-disk file.
    File(PathBuf),
    /// An embedded builtin.
    Builtin(String),
}

impl Identity {
    /// The identity of `link`.
    pub(super) fn of(link: &ChainLink) -> Self {
        if link.on_disk {
            Self::File(std::fs::canonicalize(&link.path).unwrap_or_else(|_| link.path.clone()))
        } else {
            Self::Builtin(link.name.clone())
        }
    }
}

/// A theme file a plugin provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PluginTheme {
    /// The providing plugin's id.
    pub(super) plugin_id: String,
    /// The file stem, the theme's name.
    pub(super) stem: String,
    /// The file.
    pub(super) path: PathBuf,
}

/// Every theme candidate by source, listed at most once per load and shared
/// by every hop. Each source is listed only when a hop needs it, so a full
/// theme lists nothing.
#[derive(Debug)]
pub(super) struct Candidates {
    /// The user themes directory; `None` until a hop needs it.
    user_dir: Option<PathBuf>,
    user: Option<Vec<(String, PathBuf)>>,
    /// In plugin id order, so the last match is the tie winner.
    plugins: Option<Vec<PluginTheme>>,
}

impl Candidates {
    /// Candidates from the real user themes directory and plugin discovery.
    pub(super) const fn discover() -> Self {
        Self {
            user_dir: None,
            user: None,
            plugins: None,
        }
    }

    /// Candidates from `user_dir` and a fixed plugin list (in plugin id order).
    #[cfg(test)]
    pub(super) const fn fixed(user_dir: PathBuf, plugins: Vec<PluginTheme>) -> Self {
        Self {
            user_dir: Some(user_dir),
            user: None,
            plugins: Some(plugins),
        }
    }

    /// The user theme file whose stem is exactly `name`.
    fn user(&mut self, name: &str) -> Option<PathBuf> {
        let user_dir = self
            .user_dir
            .get_or_insert_with(ThemeManager::user_themes_dir);
        self.user
            .get_or_insert_with(|| toml_stems(user_dir).collect())
            .iter()
            .find(|(stem, _)| stem == name)
            .map(|(_, path)| path.clone())
    }

    /// The winning plugin theme file whose stem is exactly `name`: among
    /// several plugins, the lexically greatest id, as discovery's tie rule.
    fn plugin(&mut self, name: &str) -> Option<PluginTheme> {
        self.plugins
            .get_or_insert_with(list_plugin_themes)
            .iter()
            .rev()
            .find(|theme| theme.stem == name)
            .cloned()
    }
}

/// Every plugin theme file, in plugin id order.
fn list_plugin_themes() -> Vec<PluginTheme> {
    let mut themes = Vec::new();
    for plugin in discover_plugins().plugins {
        let Some(root) = plugin.root else {
            continue;
        };
        let dir = root.join("themes");
        let plugin_id = plugin.manifest.id.as_str().to_owned();
        let stems: Vec<(String, PathBuf)> = toml_stems(&dir).collect();
        themes.extend(stems.into_iter().map(|(stem, path)| PluginTheme {
            plugin_id: plugin_id.clone(),
            stem,
            path,
        }));
    }
    themes
}

/// Resolve the base `name` that the file `from` extends.
///
/// - From a plugin file (or a builtin): the embedded builtin `name`, skipping
///   user and plugin files with that name.
/// - From a user file: user, then plugin, then builtin, except that a file
///   never resolves to itself, so its own name starts below the user source.
///
/// # Errors
/// Returns the diagnostic when `name` resolves to nothing allowed.
pub(super) fn resolve_base(
    name: &str,
    from: &ChainLink,
    candidates: &mut Candidates,
) -> std::result::Result<ChainLink, String> {
    if from.source != Source::User {
        return if is_builtin(name) {
            Ok(ChainLink::embedded(name))
        } else {
            Err(format!(
                "extends '{name}', which is not a builtin theme; a plugin theme may extend only a builtin theme"
            ))
        };
    }
    let own = name == from.name;
    if !own && let Some(path) = candidates.user(name) {
        return Ok(ChainLink {
            name: name.to_owned(),
            source: Source::User,
            path,
            on_disk: true,
        });
    }
    if let Some(theme) = candidates.plugin(name) {
        return Ok(ChainLink {
            name: theme.stem,
            source: Source::Plugin {
                plugin_id: theme.plugin_id,
            },
            path: theme.path,
            on_disk: true,
        });
    }
    if is_builtin(name) {
        return Ok(ChainLink::embedded(name));
    }
    Err(if own {
        format!("extends '{name}' names this theme and no plugin or builtin theme '{name}' exists")
    } else {
        format!("extends '{name}', which matches no user, plugin or builtin theme")
    })
}
