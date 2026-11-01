//! `gpy plugin install` and `gpy plugin remove`.
//!
//! Install checks the source for content and layout only. The script trust
//! checks are not run on it: a checkout made under `umask 002`, or inside a
//! group-writable directory, is exactly what this command exists to fix. The
//! tree is copied with modes the trust check accepts into a staging directory,
//! swapped into the plugins root, and the copy is then put through full
//! discovery. Remove moves a user plugin to a backup and takes its segments out
//! of `ui.enabled_segments`.
//!
//! Staging and backups live outside the plugins root because discovery treats
//! any subdirectory holding `plugin.toml` as a plugin.

use super::backup_stamp::backup_stamp;
use super::output;
use super::plugin::{plugin_segment_file, unsupported_entry_type};
use super::utils::{active_config_path, load_active_config, reload_agent_and_notify};
use crate::config::loader::save_config;
use crate::paths::config_root;
use crate::plugin::{
    EntryType, PluginId, PluginManifest, PluginSource, ScriptSegmentManifest, discover_plugins,
    script_segment_manifest_path, user_plugins_dir,
};
use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// What a plugin source tree holds, relative to its root.
#[derive(Debug, Clone, Copy)]
enum EntryKind {
    Dir,
    File { executable: bool },
}

/// One file or directory of a plugin source tree.
struct TreeEntry {
    relative: PathBuf,
    kind: EntryKind,
}

/// What an install will do before it touches the filesystem.
struct Plan {
    /// `<user plugins dir>/<id>`.
    target: PathBuf,
    /// Directories to move to `plugins.bak` first: the installed plugin with
    /// this id, then whatever else occupies `target`.
    to_move: Vec<PathBuf>,
    /// The bundled or first-party plugin a user copy shadows, if any.
    shadowed: Option<PluginSource>,
}

/// Install the plugin in `source` as a user plugin.
///
/// # Errors
/// Returns an error when the source is not a valid plugin tree, the plugin is
/// already installed (or shadows a bundled one) without `force`, or the copy
/// does not load; a failed install leaves the plugins directory as it was.
pub fn install(source: &Path, force: bool) -> Result<()> {
    let entries = collect_tree(source)?;
    let manifest = check_source(source)?;
    let plan = plan_install(&manifest.id, force)?;
    let backups = backup_targets(&manifest.id, &plan.to_move)?;

    let stage = stage_copy(source, &entries, &manifest.id)?;
    swap_in(&stage, &plan.target, &backups, &manifest.id)?;

    output::plain(format_args!(
        "Installed plugin {} {} into {}",
        manifest.id,
        manifest.version,
        plan.target.display()
    ));
    for (from, backup) in &backups {
        output::plain(format_args!(
            "Moved the previous {} to {}",
            from.display(),
            backup.display()
        ));
    }
    if let Some(shadowed) = plan.shadowed {
        output::plain(format_args!(
            "It shadows the {shadowed} plugin {}.",
            manifest.id
        ));
    }
    for segment in &manifest.provided_segments {
        output::plain(format_args!("Enable its segments: gpy enable {segment}"));
    }
    Ok(())
}

/// Remove the user plugin `id`: move it to `plugins.bak` and disable its
/// segments.
///
/// # Errors
/// Returns an error when `id` is malformed, names no user plugin, or the
/// directory or the config cannot be updated.
pub fn remove(id: &str) -> Result<()> {
    let plugin_id = PluginId::new(id)?;
    let discovery = discover_plugins();
    let Some(found) = discovery
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.id == plugin_id)
    else {
        return Err(unknown_user_plugin(&plugin_id));
    };
    if !matches!(found.source, PluginSource::User) {
        let commands: Vec<String> = found
            .manifest
            .provided_segments
            .iter()
            .map(|segment| format!("gpy disable {segment}"))
            .collect();
        return Err(Error::invalid(format!(
            "{plugin_id} ships with GPY; disable its segments with: {}",
            commands.join(", ")
        )));
    }
    let root = found.root.clone().ok_or_else(|| {
        Error::invalid(format!("plugin '{plugin_id}' has no directory to remove"))
    })?;
    let segments: Vec<String> = found
        .manifest
        .provided_segments
        .iter()
        .map(ToString::to_string)
        .collect();

    let moves = backup_targets(&plugin_id, std::slice::from_ref(&root))?;
    let backup = moves
        .first()
        .map(|(_, backup)| backup.clone())
        .ok_or_else(|| Error::invalid("no backup location"))?;
    let config_path = active_config_path()?;
    let original = load_active_config(&config_path)?;

    // The directory goes first, so a failure after this point leaves the
    // config untouched and the plugin recoverable from its backup.
    move_aside(&moves)?;

    let mut updated = original.clone();
    updated
        .ui
        .enabled_segments
        .retain(|enabled| !segments.contains(enabled));
    let disabled: Vec<&String> = segments
        .iter()
        .filter(|segment| original.ui.enabled_segments.contains(segment))
        .collect();
    if !disabled.is_empty() {
        if let Err(err) = save_config(&config_path, &original, &updated, &["ui.enabled_segments"]) {
            let finish: Vec<String> = segments
                .iter()
                .map(|segment| format!("gpy disable {segment}"))
                .collect();
            return Err(Error::invalid(format!(
                "{err}; the plugin is already in {}; finish with: {}",
                backup.display(),
                finish.join(", ")
            )));
        }
        for segment in disabled {
            output::plain(format_args!("Disabled {segment}"));
        }
    }
    output::plain(format_args!(
        "Removed plugin {plugin_id}; its files are in {}",
        backup.display()
    ));
    reload_agent_and_notify();
    Ok(())
}

/// The error for an id with no user plugin: either nothing by that name, or a
/// directory discovery could not load.
fn unknown_user_plugin(id: &PluginId) -> Error {
    let direct = user_plugins_dir().join(id.as_str());
    if std::fs::symlink_metadata(&direct).is_ok() {
        Error::invalid(format!(
            "{path} exists but is not a loadable plugin; run gpy plugin validate {path} to see why, or move it away yourself",
            path = direct.display()
        ))
    } else {
        Error::invalid(format!("no user plugin '{id}'; see gpy plugin list"))
    }
}

/// Walk `root` without following symlinks. Only regular files and directories
/// are allowed; a symlink, device, FIFO or socket refuses the whole tree.
fn collect_tree(root: &Path) -> Result<Vec<TreeEntry>> {
    let root_meta = std::fs::symlink_metadata(root)
        .map_err(|e| Error::io_context(format_args!("failed to read {}", root.display()), &e))?;
    if !root_meta.is_dir() {
        return Err(Error::invalid(format!(
            "{} is not a plugin directory (it must be a real directory holding plugin.toml)",
            root.display()
        )));
    }

    let mut entries = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let dir = root.join(&relative);
        let listing = std::fs::read_dir(&dir)
            .map_err(|e| Error::io_context(format_args!("failed to read {}", dir.display()), &e))?;
        for listed in listing {
            let item = listed.map_err(|e| {
                Error::io_context(format_args!("failed to read {}", dir.display()), &e)
            })?;
            let child = relative.join(item.file_name());
            let path = root.join(&child);
            let meta = std::fs::symlink_metadata(&path).map_err(|e| {
                Error::io_context(format_args!("failed to read {}", path.display()), &e)
            })?;
            if meta.is_dir() {
                pending.push(child.clone());
                entries.push(TreeEntry {
                    relative: child,
                    kind: EntryKind::Dir,
                });
            } else if meta.is_file() {
                entries.push(TreeEntry {
                    relative: child,
                    kind: EntryKind::File {
                        executable: is_owner_executable(&meta),
                    },
                });
            } else {
                let what = if meta.file_type().is_symlink() {
                    "a symlink"
                } else {
                    "not a regular file or directory"
                };
                return Err(Error::invalid(format!(
                    "{} is {what}; a plugin may hold only regular files and directories",
                    path.display()
                )));
            }
        }
    }
    // Parents sort before their children, so creating in this order works.
    entries.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(entries)
}

/// Parse the manifests of the source and check that the files they name
/// exist, without any trust check.
fn check_source(source: &Path) -> Result<PluginManifest> {
    let manifest_path = source.join("plugin.toml");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        Error::io_context(
            format_args!("failed to read plugin manifest {}", manifest_path.display()),
            &e,
        )
    })?;
    let manifest = PluginManifest::parse(&text)?;

    match manifest.entry_type {
        EntryType::Script => {
            for segment in &manifest.provided_segments {
                let path = script_segment_manifest_path(source, segment);
                let segment_text = std::fs::read_to_string(&path).map_err(|e| {
                    Error::io_context(
                        format_args!("failed to read segment manifest {}", path.display()),
                        &e,
                    )
                })?;
                let segment_manifest = ScriptSegmentManifest::parse(&segment_text)
                    .map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
                require_regular_file(source, segment_manifest.command.as_str(), "command")?;
                if let Some(require) = &segment_manifest.require_command {
                    require_regular_file(source, require.as_str(), "require_command")?;
                }
            }
        }
        EntryType::File => {
            for segment in &manifest.provided_segments {
                let path = plugin_segment_file(source, segment);
                match std::fs::symlink_metadata(&path) {
                    Ok(meta) if meta.is_file() => {}
                    _ => {
                        return Err(Error::invalid(format!(
                            "missing segment file for '{segment}': {}",
                            path.display()
                        )));
                    }
                }
            }
        }
        EntryType::Command => return Err(unsupported_entry_type(&manifest.entry_type)),
    }
    Ok(manifest)
}

/// `relative` (a manifest's `command` or `require_command`) must name a
/// regular file under `root`.
fn require_regular_file(root: &Path, relative: &str, key: &str) -> Result<()> {
    let path = root.join(relative);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => Ok(()),
        _ => Err(Error::invalid(format!(
            "{key} '{relative}' is not a regular file: {}",
            path.display()
        ))),
    }
}

/// Decide what an install replaces, refusing what needs `--force`.
fn plan_install(id: &PluginId, force: bool) -> Result<Plan> {
    let discovery = discover_plugins();
    let existing = discovery
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.id == *id && matches!(plugin.source, PluginSource::User))
        .and_then(|plugin| plugin.root.clone());
    let shadowed = discovery
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.id == *id && !matches!(plugin.source, PluginSource::User))
        .map(|plugin| plugin.source);

    let target = user_plugins_dir().join(id.as_str());
    let mut to_move: Vec<PathBuf> = existing.iter().cloned().collect();
    // Discovery omits a directory that is broken or has no `plugin.toml`, and
    // renaming onto a non-empty directory fails, so look at the path itself.
    let blocked = existing.as_ref() != Some(&target) && std::fs::symlink_metadata(&target).is_ok();
    if blocked {
        to_move.push(target.clone());
    }

    if !force {
        if let Some(root) = &existing {
            return Err(Error::invalid(format!(
                "plugin '{id}' is already installed at {}; pass --force to replace it (the old copy moves to plugins.bak)",
                root.display()
            )));
        }
        if blocked {
            return Err(Error::invalid(format!(
                "{} exists but is not a loadable plugin; move it away or pass --force",
                target.display()
            )));
        }
        if let Some(source) = shadowed {
            return Err(Error::invalid(format!(
                "plugin '{id}' would shadow the {source} plugin '{id}'; pass --force to install it anyway"
            )));
        }
    }
    Ok(Plan {
        target,
        to_move,
        shadowed,
    })
}

/// `plugins.bak/<id>.<stamp>` for each directory in `to_move`; fails before
/// anything moves when one of those paths is taken.
fn backup_targets(id: &PluginId, to_move: &[PathBuf]) -> Result<Vec<(PathBuf, PathBuf)>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let stamp = backup_stamp(now);
    let backup_dir = config_root().join("plugins.bak");

    let mut backups = Vec::new();
    for (index, from) in to_move.iter().enumerate() {
        let name = if index == 0 {
            format!("{id}.{stamp}")
        } else {
            format!("{id}.{stamp}.blocking")
        };
        let to = backup_dir.join(name);
        if std::fs::symlink_metadata(&to).is_ok() {
            return Err(Error::invalid(format!(
                "backup {} already exists; retry",
                to.display()
            )));
        }
        backups.push((from.clone(), to));
    }
    Ok(backups)
}

/// Copy `source` into `<config root>/plugins.staging/<id>-<pid>` with modes the
/// trust check accepts. Returns the staging directory.
fn stage_copy(source: &Path, entries: &[TreeEntry], id: &PluginId) -> Result<PathBuf> {
    let staging_root = config_root().join("plugins.staging");
    ensure_dir_chain(&staging_root)?;
    ensure_dir_chain(&user_plugins_dir())?;

    let stage = staging_root.join(format!("{id}-{}", std::process::id()));
    match std::fs::remove_dir_all(&stage) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(Error::io_context(
                format_args!("failed to clear {}", stage.display()),
                &e,
            ));
        }
    }
    copy_tree(source, entries, &stage).inspect_err(|_| {
        drop(std::fs::remove_dir_all(&stage));
    })?;
    Ok(stage)
}

/// Create `stage` and copy `entries` of `source` into it: directories 0755,
/// files 0644 (0755 when the source file has the owner execute bit), both
/// narrowed by the umask and never group- or world-writable.
fn copy_tree(source: &Path, entries: &[TreeEntry], stage: &Path) -> Result<()> {
    let create_dir = |path: &Path| {
        crate::fs_util::create_dir_unshared(path)
            .map_err(|e| Error::io_context(format_args!("failed to create {}", path.display()), &e))
    };
    create_dir(stage)?;
    for entry in entries {
        let from = source.join(&entry.relative);
        let to = stage.join(&entry.relative);
        match entry.kind {
            EntryKind::Dir => create_dir(&to)?,
            EntryKind::File { executable } => {
                let contents = std::fs::read(&from).map_err(|e| {
                    Error::io_context(format_args!("failed to read {}", from.display()), &e)
                })?;
                crate::fs_util::create_new_unshared(&to, &contents).map_err(|e| {
                    Error::io_context(format_args!("failed to write {}", to.display()), &e)
                })?;
                if executable {
                    mark_executable(&to).map_err(|e| {
                        Error::io_context(
                            format_args!("failed to set the mode of {}", to.display()),
                            &e,
                        )
                    })?;
                }
            }
        }
    }
    Ok(())
}

/// Create `path` and every missing ancestor, one component at a time, parent
/// first, each directory 0755 narrowed by the umask. `create_dir_all` honors
/// the umask for the mode, so under `umask 002` it would leave group-writable
/// directories that the script trust check can reject.
fn ensure_dir_chain(path: &Path) -> Result<()> {
    let missing: Vec<&Path> = path
        .ancestors()
        .take_while(|dir| !dir.as_os_str().is_empty() && !dir.exists())
        .collect();
    for dir in missing.into_iter().rev() {
        match crate::fs_util::create_dir_unshared(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(Error::io_context(
                    format_args!("failed to create {}", dir.display()),
                    &e,
                ));
            }
        }
    }
    Ok(())
}

/// Move each `(from, to)` pair, creating the parent of `to`.
fn move_aside(moves: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (from, to) in moves {
        if let Some(parent) = to.parent() {
            ensure_dir_chain(parent)?;
        }
        std::fs::rename(from, to).map_err(|e| {
            Error::io_context(
                format_args!("failed to move {} to {}", from.display(), to.display()),
                &e,
            )
        })?;
    }
    Ok(())
}

/// Back up what `backups` names, rename `stage` to `target`, and check that
/// the copy loads. Any failure puts everything back.
fn swap_in(
    stage: &Path,
    target: &Path,
    backups: &[(PathBuf, PathBuf)],
    id: &PluginId,
) -> Result<()> {
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for backup in backups {
        if let Err(err) = move_aside(std::slice::from_ref(backup)) {
            return Err(unwind(err, &[stage], &moved));
        }
        moved.push(backup.clone());
    }
    if let Err(e) = std::fs::rename(stage, target) {
        let err = Error::io_context(
            format_args!(
                "failed to move {} into place at {}",
                stage.display(),
                target.display()
            ),
            &e,
        );
        return Err(unwind(err, &[stage], &moved));
    }
    verify_installed(target, id).map_err(|err| unwind(err, &[target], &moved))
}

/// The copy at `target` must be discovered as a user plugin, with no
/// diagnostic of its own; other plugins' diagnostics are not this install's.
fn verify_installed(target: &Path, id: &PluginId) -> Result<()> {
    let discovery = discover_plugins();
    if let Some(diagnostic) = discovery
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.plugin_dir == target)
    {
        return Err(Error::invalid(diagnostic.message.clone()));
    }
    let loaded = discovery.plugins.iter().any(|plugin| {
        matches!(plugin.source, PluginSource::User) && plugin.root.as_deref() == Some(target)
    });
    if loaded {
        Ok(())
    } else {
        Err(Error::invalid(format!(
            "plugin '{id}' did not load from {} after the copy",
            target.display()
        )))
    }
}

/// Remove `created`, put `moved` back, and return `err`, noting anything that
/// could not be restored.
fn unwind(err: Error, created: &[&Path], moved: &[(PathBuf, PathBuf)]) -> Error {
    for path in created {
        drop(std::fs::remove_dir_all(path));
    }
    let stuck: Vec<String> = moved
        .iter()
        .rev()
        .filter_map(|(from, backup)| {
            std::fs::rename(backup, from).err().map(|e| {
                format!(
                    "could not restore {} from {}: {e}",
                    from.display(),
                    backup.display()
                )
            })
        })
        .collect();
    if stuck.is_empty() {
        err
    } else {
        Error::invalid(format!("{err}; {}", stuck.join("; ")))
    }
}

#[cfg(unix)]
fn is_owner_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    meta.permissions().mode() & 0o100 != 0
}

#[cfg(not(unix))]
fn is_owner_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
