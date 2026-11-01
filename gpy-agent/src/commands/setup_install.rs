//! Staged, alias-safe file installation behind `gpy setup`.
//!
//! A replacement is built in a hidden sibling of the live directory (same
//! filesystem) and moved into place only once it is complete: the copied
//! scripts, the generated completions and the version stamp. Any failure
//! before the swap leaves the live tree untouched, and a failure during it is
//! rolled back. Before anything is created or removed, every target is
//! resolved through symlinks and refused when it is, contains or lies inside
//! the source, so setup can never delete the files it installs from.

use super::output;
use super::setup::{VERSION_STAMP, check_sourceable, shell_dir_for};
use crate::shell::Shell;
use crate::{Error, Result};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Directory beside the shell trees holding the script plugins GPY ships
/// (`username`, `hostname`), installed as the bundled plugin root.
pub(super) const BUNDLED_PLUGINS_SOURCE: &str = "plugins";

/// Wrap an I/O error with the action and path that failed.
fn io_at<'a>(action: &'a str, path: &'a Path) -> impl FnOnce(io::Error) -> io::Error + 'a {
    move |err| {
        io::Error::new(
            err.kind(),
            format!("cannot {action} {}: {err}", path.display()),
        )
    }
}

/// The runner in a source tree, and where Fish and Zsh installs keep a copy.
const RUNNER_SOURCE: &str = "bash/core/g_runner.bash";
const RUNNER_INSTALLED: &str = "core/g_runner.bash";

/// Write `bytes` to `path`, creating its parent directories.
fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_at("create", parent))?;
    }
    fs::write(path, bytes).map_err(io_at("write", path))
}

/// Remove a directory tree (or file); a missing path is fine.
fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(io_at("remove", path)(err)),
        _ => Ok(()),
    }
}

/// Remove leftovers of a finished attempt; a failure is only a warning, as
/// the install itself already succeeded or was rolled back.
fn discard(path: &Path) {
    if let Err(err) = remove_tree(path) {
        output::warn(format_args!("{err}; delete it by hand when convenient"));
    }
}

/// Resolve every symlink in `path`. A path that does not exist yet resolves
/// through its nearest existing ancestor, so a symlinked parent is followed.
fn resolve(path: &Path) -> Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(real) => return Ok(real),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(io_at("resolve", path)(err).into()),
    }
    if fs::symlink_metadata(path).is_ok() {
        return Err(Error::invalid(format!(
            "{} is a symlink to a missing location; repair or remove it first",
            path.display()
        )));
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => Ok(resolve(parent)?.join(name)),
        _ => Err(Error::invalid(format!("cannot resolve {}", path.display()))),
    }
}

/// Resolve `target` and refuse it when it is, contains or lies inside
/// `source_root` (compared after resolving both, so a symlink alias cannot
/// hide the overlap). Returns the resolved target.
fn guard(source_root: &Path, target: &Path) -> Result<PathBuf> {
    let source = resolve(source_root)?;
    let real = resolve(target)?;
    if real.starts_with(&source) || source.starts_with(&real) {
        return Err(Error::invalid(format!(
            "refusing to install into {} (resolves to {}): it is the same directory as, \
             contains, or lies inside the source {} (resolves to {}), so setup would \
             overwrite or delete the source files. Install from a separate copy of the \
             files, or point the install location somewhere else",
            target.display(),
            real.display(),
            source_root.display(),
            source.display()
        )));
    }
    Ok(real)
}

/// [`guard`] for a target that is replaced wholesale: the target itself must
/// not be a symlink, as its contents would be deleted.
fn guard_replace(source_root: &Path, target: &Path) -> Result<PathBuf> {
    if fs::symlink_metadata(target).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(Error::invalid(format!(
            "{} is a symlink; refusing to replace it with a directory from {}. \
             Remove the link first",
            target.display(),
            source_root.display()
        )));
    }
    guard(source_root, target)
}

/// Where Fish autoloads the merged `gpy.fish` completion file from.
pub(super) fn fish_completions_dir(config_home: &Path) -> PathBuf {
    config_home.join("fish").join("completions")
}

/// Where the bundled plugin root is installed.
pub(super) fn plugins_target(config_home: &Path) -> PathBuf {
    config_home
        .join("gpy")
        .join(crate::plugin::BUNDLED_PLUGINS_DIR_NAME)
}

/// Validate every location `setup` for `shell` will write against
/// `source_root`, before any of them is touched.
///
/// # Errors
/// Fails when an install location aliases or overlaps the source.
pub(super) fn preflight(shell: Shell, source_root: &Path, config_home: &Path) -> Result<()> {
    guard(source_root, &shell_dir_for(config_home, shell))?;
    if shell == Shell::Fish {
        guard(source_root, &fish_completions_dir(config_home))?;
    }
    if source_root.join(BUNDLED_PLUGINS_SOURCE).is_dir() {
        guard_replace(source_root, &plugins_target(config_home))?;
    }
    Ok(())
}

/// A hidden sibling of `live` for the staged or replaced files.
fn sibling(live: &Path, tag: &str) -> Result<PathBuf> {
    let (Some(parent), Some(name)) = (live.parent(), live.file_name()) else {
        return Err(Error::invalid(format!(
            "cannot stage beside {}",
            live.display()
        )));
    };
    let mut leaf = OsString::from(".");
    leaf.push(name);
    leaf.push(format!(".gpy-{tag}-{}", std::process::id()));
    Ok(parent.join(leaf))
}

/// Recursively copy `src` to `dst`, skipping top-level entries named in
/// `skip` and any symlink. Errors name the path and the operation.
#[allow(
    clippy::filetype_is_file,
    reason = "only regular files are copied; symlinks, sockets and FIFOs are skipped on purpose"
)]
fn copy_tree(src: &Path, dst: &Path, skip: &[&str]) -> io::Result<()> {
    fs::create_dir_all(dst).map_err(io_at("create", dst))?;
    for dirent in fs::read_dir(src).map_err(io_at("list", src))? {
        let entry = dirent.map_err(io_at("list", src))?;
        let name = entry.file_name();
        if skip.iter().any(|skipped| name == *skipped) {
            continue;
        }
        let from = entry.path();
        let target = dst.join(&name);
        let kind = entry.file_type().map_err(io_at("inspect", &from))?;
        if kind.is_dir() {
            copy_tree(&from, &target, &[])?;
        } else if kind.is_file() {
            let bytes = fs::read(&from).map_err(io_at("read", &from))?;
            write_file(&target, &bytes)?;
        }
    }
    Ok(())
}

/// One entry moved from the staging directory into the live tree.
struct Step {
    /// Final location.
    target: PathBuf,
    /// Staged replacement.
    new: PathBuf,
    /// Where the old `target` is parked until the swap is done.
    aside: PathBuf,
    moved_aside: bool,
    installed: bool,
}

impl Step {
    const fn new(target: PathBuf, new: PathBuf, aside: PathBuf) -> Self {
        Self {
            target,
            new,
            aside,
            moved_aside: false,
            installed: false,
        }
    }

    /// Park the old entry (if any), then move the staged one into place.
    fn apply(&mut self) -> io::Result<()> {
        if fs::symlink_metadata(&self.target).is_ok() {
            fs::rename(&self.target, &self.aside).map_err(io_at("move aside", &self.target))?;
            self.moved_aside = true;
        }
        fs::rename(&self.new, &self.target).map_err(io_at("install", &self.target))?;
        self.installed = true;
        Ok(())
    }

    /// Put the old entry back.
    fn undo(&self) -> io::Result<()> {
        if self.installed {
            fs::rename(&self.target, &self.new).map_err(io_at("remove", &self.target))?;
        }
        if self.moved_aside {
            fs::rename(&self.aside, &self.target).map_err(io_at("restore", &self.target))?;
        }
        Ok(())
    }
}

/// Apply every step in order; on a failure undo them in reverse.
fn commit(mut steps: Vec<Step>, backup: &Path) -> io::Result<()> {
    let mut failure = None;
    for step in &mut steps {
        if let Err(err) = step.apply() {
            failure = Some(err);
            break;
        }
    }
    let Some(err) = failure else {
        return Ok(());
    };
    let stuck: Vec<String> = steps
        .iter()
        .rev()
        .filter_map(|step| step.undo().err().map(|undo_err| format!("{undo_err}")))
        .collect();
    if stuck.is_empty() {
        discard(backup);
        Err(err)
    } else {
        Err(io::Error::new(
            err.kind(),
            format!(
                "{err}; rollback incomplete ({}); the previous files are in {}",
                stuck.join("; "),
                backup.display()
            ),
        ))
    }
}

/// How the staged files replace the live directory.
#[derive(Clone, Copy)]
enum Swap {
    /// Replace only the entries the stage holds; other files in the live
    /// directory stay.
    Entries,
    /// Replace the whole directory.
    Whole,
}

/// Move the stage into `live`. The version stamp goes in last, so an
/// interrupted swap is not stamped current.
fn swap(mode: Swap, stage: &Path, live: &Path, backup: &Path) -> io::Result<()> {
    match fs::symlink_metadata(live) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return fs::rename(stage, live).map_err(io_at("install", live));
        }
        Err(err) => return Err(io_at("inspect", live)(err)),
        Ok(meta) if !meta.is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} exists and is not a directory", live.display()),
            ));
        }
        Ok(_) => {}
    }
    let steps = match mode {
        Swap::Whole => vec![Step::new(
            live.to_path_buf(),
            stage.to_path_buf(),
            backup.to_path_buf(),
        )],
        Swap::Entries => {
            let mut names = Vec::new();
            for dirent in fs::read_dir(stage).map_err(io_at("list", stage))? {
                names.push(dirent.map_err(io_at("list", stage))?.file_name());
            }
            names.sort_by_key(|name| name == VERSION_STAMP);
            fs::create_dir_all(backup).map_err(io_at("create", backup))?;
            names
                .iter()
                .map(|name| Step::new(live.join(name), stage.join(name), backup.join(name)))
                .collect()
        }
    };
    commit(steps, backup)
}

/// Build the replacement with `fill` in a staging directory beside `live`,
/// then swap it in. `live` is untouched when `fill` or the swap fails; the
/// old files removed after a good swap are cleaned up with a warning at worst.
fn stage_and_swap(
    live: &Path,
    mode: Swap,
    fill: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<()> {
    let stage = sibling(live, "staging")?;
    let backup = sibling(live, "old")?;
    if let Some(parent) = live.parent() {
        fs::create_dir_all(parent).map_err(io_at("create", parent))?;
    }
    remove_tree(&stage)?;
    remove_tree(&backup)?;
    fs::create_dir_all(&stage).map_err(io_at("create", &stage))?;
    let result = fill(&stage).and_then(|()| swap(mode, &stage, live, &backup));
    discard(&stage);
    if result.is_ok() {
        discard(&backup);
    }
    result.map_err(Error::from)
}

/// Fish's merged completion file, written beside its final name during
/// preparation and renamed into place right after the tree swap.
struct PendingCompletion {
    temp: PathBuf,
    dest: PathBuf,
    /// The glue file older versions left in the autoload directory.
    legacy: PathBuf,
}

impl PendingCompletion {
    /// Merge the generated script with the source's glue; nothing live
    /// changes until [`PendingCompletion::commit`].
    fn prepare(
        source_root: &Path,
        source: &Path,
        config_home: &Path,
        generated: Vec<u8>,
    ) -> Result<Self> {
        let dir = fish_completions_dir(config_home);
        guard(source_root, &dir)?;
        let glue_path = source.join("completions/gpy-dynamic.fish");
        let glue = fs::read(&glue_path).map_err(|err| {
            Error::invalid(format!(
                "cannot read completion glue {}: {err}",
                glue_path.display()
            ))
        })?;
        let mut script = generated;
        script.extend_from_slice(&glue);
        let temp = dir.join(format!(".gpy.fish.gpy-new-{}", std::process::id()));
        write_file(&temp, &script)?;
        Ok(Self {
            temp,
            dest: dir.join("gpy.fish"),
            legacy: dir.join("gpy-dynamic.fish"),
        })
    }

    /// Replace `gpy.fish` and drop the legacy glue file (never autoloaded
    /// as its own file).
    fn commit(&self) -> io::Result<()> {
        match fs::remove_file(&self.legacy) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => {
                return Err(io_at("remove", &self.legacy)(err));
            }
            _ => {}
        }
        fs::rename(&self.temp, &self.dest).map_err(io_at("install", &self.dest))
    }

    /// Delete the prepared file after a failed install.
    fn discard(&self) {
        if let Err(err) = fs::remove_file(&self.temp) {
            output::warn(format_args!(
                "{}; delete it by hand when convenient",
                io_at("remove", &self.temp)(err)
            ));
        }
    }
}

/// Install `shell`'s tree and completions from `source_root`, with the
/// version stamp, and return the install directory. Leaves the rc file alone.
///
/// The files other than the shipped ones in the install directory are kept;
/// each shipped subtree is replaced whole, so scripts a newer release dropped
/// are gone. Everything is prepared in a staging directory first, so a source
/// that cannot be read, a missing Fish completion glue or a failed completion
/// generation leaves the previous install exactly as it was.
///
/// # Errors
/// Fails when the install location aliases or overlaps the source, or a file
/// cannot be prepared or swapped in.
pub(super) fn install_files(
    shell: Shell,
    source_root: &Path,
    config_home: &Path,
    completions: &impl Fn(Shell) -> Result<Vec<u8>>,
) -> Result<PathBuf> {
    let shell_dir = shell_dir_for(config_home, shell);
    check_sourceable(&shell_dir)?;
    let live = guard(source_root, &shell_dir)?;

    let source = source_root.join(shell.as_str());
    let generated = completions(shell)?;
    // Fish's completion glue is merged into the generated file instead.
    let skip: &[&str] = if shell == Shell::Fish {
        &["completions"]
    } else {
        &[]
    };
    let (staged_completion, pending) = match shell {
        Shell::Fish => (
            None,
            Some(PendingCompletion::prepare(
                source_root,
                &source,
                config_home,
                generated,
            )?),
        ),
        Shell::Zsh => (Some(("completions/_gpy", generated)), None),
        Shell::Bash => (Some(("completions/gpy.bash", generated)), None),
    };

    // The one committed runner lives in the Bash tree; Fish and Zsh run it in
    // a Bash child, so their installed trees carry a copy.
    let runner = if shell == Shell::Bash {
        None
    } else {
        let runner_path = source_root.join(RUNNER_SOURCE);
        let Ok(bytes) = fs::read(&runner_path) else {
            if let Some(completion) = &pending {
                completion.discard();
            }
            return Err(Error::invalid(format!(
                "{} has no {RUNNER_SOURCE}; the script tree is incomplete",
                source_root.display()
            )));
        };
        Some(bytes)
    };

    let swapped = stage_and_swap(&live, Swap::Entries, |stage| {
        copy_tree(&source, stage, skip)?;
        if let Some(bytes) = &runner {
            write_file(&stage.join(RUNNER_INSTALLED), bytes)?;
        }
        if let Some((relative, bytes)) = &staged_completion {
            write_file(&stage.join(relative), bytes)?;
        }
        write_file(
            &stage.join(VERSION_STAMP),
            format!("{}\n", crate::VERSION).as_bytes(),
        )
    });
    match (swapped, pending) {
        (Ok(()), Some(completion)) => completion.commit()?,
        (Ok(()), None) => {}
        (Err(err), Some(completion)) => {
            completion.discard();
            return Err(err);
        }
        (Err(err), None) => return Err(err),
    }
    Ok(shell_dir)
}

/// Replace `dst` with a copy of `src`, staged beside it first so a failed
/// copy leaves `dst` as it was. `source_root` is the directory `src` belongs
/// to; `dst` must not alias or overlap it.
///
/// # Errors
/// Fails when `dst` aliases or overlaps the source, or is a symlink, or the
/// copy or swap fails.
pub(super) fn replace_tree(src: &Path, dst: &Path, source_root: &Path) -> Result<()> {
    replace_staged(src, dst, source_root, |_| Ok(()))
}

/// [`replace_tree`] with `prepare` run on the staged copy before the swap.
fn replace_staged(
    src: &Path,
    dst: &Path,
    source_root: &Path,
    prepare: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<()> {
    let live = guard_replace(source_root, dst)?;
    stage_and_swap(&live, Swap::Whole, |stage| {
        copy_tree(src, stage, &[])?;
        prepare(stage)
    })
}

/// Replace the bundled plugin root (`<config home>/gpy/bundled-plugins`) with
/// `source_root/plugins`, so a plugin dropped by an upgrade is gone too. A
/// source without that tree (an older archive) leaves the installed root alone.
/// Group and world write bits are cleared before the swap, as export refuses
/// a shared script.
pub(super) fn install_bundled_plugins(
    source_root: &Path,
    config_home: &Path,
) -> Result<Option<PathBuf>> {
    let source = source_root.join(BUNDLED_PLUGINS_SOURCE);
    if !source.is_dir() {
        return Ok(None);
    }
    let target = plugins_target(config_home);
    replace_staged(&source, &target, source_root, clear_shared_write)?;
    Ok(Some(target))
}

/// Clear the group and world write bits of `path` and everything below it.
#[cfg(unix)]
fn clear_shared_write(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::symlink_metadata(path).map_err(io_at("inspect", path))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mode = metadata.permissions().mode();
    if mode & 0o022 != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(mode & !0o022))
            .map_err(io_at("change permissions of", path))?;
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(io_at("list", path))? {
            clear_shared_write(&entry.map_err(io_at("list", path))?.path())?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn clear_shared_write(_path: &Path) -> io::Result<()> {
    Ok(())
}
