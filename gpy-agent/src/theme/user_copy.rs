//! Shared helpers for user theme files that shadow a builtin theme.
//!
//! A file at `<config>/themes/<name>.toml` fully shadows the embedded builtin of
//! the same name, so it is a fork that stops receiving upstream changes. These
//! helpers are the single place for the "is this a builtin override" test, the
//! recovery hint shown in diagnostics, and moving a copy aside to `.bak`.

use super::manager::{BUILTIN_THEMES, ThemeManager};
use crate::{Error, Result};
use std::path::{Path, PathBuf};

/// Whether `name` is an embedded builtin theme.
#[must_use]
pub fn is_builtin(name: &str) -> bool {
    BUILTIN_THEMES.iter().any(|(builtin, _)| *builtin == name)
}

/// The embedded TOML of builtin `name`, if it is one.
#[must_use]
pub fn builtin_content(name: &str) -> Option<&'static str> {
    BUILTIN_THEMES
        .iter()
        .find(|(builtin, _)| *builtin == name)
        .map(|(_, content)| *content)
}

/// The names of the embedded builtin themes.
pub fn builtin_names() -> impl Iterator<Item = &'static str> {
    BUILTIN_THEMES.iter().map(|(name, _)| *name)
}

/// Path of the user theme file for `name` (it may not exist).
#[must_use]
pub fn user_copy_path(name: &str) -> PathBuf {
    ThemeManager::user_themes_dir().join(format!("{name}.toml"))
}

/// The user file for `name`, when it exists and shadows an embedded builtin.
/// It may be a full copy or an overlay; see [`shadowing_full_copy`].
#[must_use]
pub fn shadowing_copy(name: &str) -> Option<PathBuf> {
    if !is_builtin(name) {
        return None;
    }
    let path = user_copy_path(name);
    path.is_file().then_some(path)
}

/// Whether the theme file at `path` is an overlay: its top-level table has an
/// `extends` key. A file that can't be read or parsed is not.
#[must_use]
pub fn is_overlay_file(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .is_some_and(|table| table.contains_key("extends"))
}

/// The user file for `name` when it shadows an embedded builtin as a full
/// copy, a fork that stops receiving upstream changes. An overlay is not one.
#[must_use]
pub fn shadowing_full_copy(name: &str) -> Option<PathBuf> {
    shadowing_copy(name).filter(|path| !is_overlay_file(path))
}

/// Recovery text appended to diagnostics about a user file that overrides builtin `name`.
/// Names `gpy theme reset|diff|migrate|rebase` and the upgrade notes.
#[must_use]
pub fn recovery_hint(name: &str) -> String {
    format!(
        "{path} overrides the builtin theme '{name}' and may have been written by an older GPY.\n  \
         Recovery: `gpy theme diff {name}` shows how it differs from the builtin, \
         `gpy theme migrate {name}` rewrites keys removed by breaking changes, \
         `gpy theme rebase {name}` keeps only your changes as an overlay of the builtin, \
         `gpy theme reset {name}` moves it to {name}.toml.bak so the builtin takes over. \
         See the upgrade notes in CHANGELOG.md.",
        path = user_copy_path(name).display(),
    )
}

/// Append [`recovery_hint`] to a load `error` when `path` is the existing user
/// full copy of an embedded builtin; any other error passes through unchanged.
/// An overlay's error is its chain error and gets no hint.
///
/// Every theme load path (`theme::load::load_theme_file`,
/// `theme::load::load_named_theme` and `ThemeManager`'s load) applies this
/// exactly once, so callers must not add the hint again.
#[must_use]
pub fn annotate_load_error(path: &Path, error: Error) -> Error {
    let builtin = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .filter(|name| is_builtin(name))
        .filter(|name| path.is_file() && path == user_copy_path(name) && !is_overlay_file(path));
    match (builtin, error) {
        (Some(name), Error::Config { message, source }) => Error::Config {
            message: format!("{message}\n  {}", recovery_hint(name)),
            source,
        },
        (_, other) => other,
    }
}

/// Move `path` aside to `<file>.bak` in the same directory and return the new path.
/// An existing `.bak` is never overwritten: later moves get `.bak.1`, `.bak.2`, ...
///
/// # Errors
/// Returns an error when `path` has no file name or the rename fails.
pub fn move_aside(path: &Path) -> Result<PathBuf> {
    let target = free_backup_path(path)?;
    std::fs::rename(path, &target).map_err(|e| {
        Error::config(format!(
            "Failed to move {} to {}: {e}",
            path.display(),
            target.display()
        ))
    })?;
    Ok(target)
}

/// Copy `path` to the first free `<file>.bak`, `<file>.bak.1`, ... in the same
/// directory, leaving `path` in place, and return the backup path.
///
/// # Errors
/// Returns an error when `path` has no file name or the copy fails.
pub fn copy_aside(path: &Path) -> Result<PathBuf> {
    let target = free_backup_path(path)?;
    std::fs::copy(path, &target).map_err(|e| {
        Error::config(format!(
            "Failed to back up {} to {}: {e}",
            path.display(),
            target.display()
        ))
    })?;
    Ok(target)
}

/// The first of `<file>.bak`, `<file>.bak.1`, ... beside `path` that does not exist.
fn free_backup_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| Error::config(format!("Invalid file path {}", path.display())))?;
    let mut target = path.with_file_name(format!("{file_name}.bak"));
    let mut n: u32 = 0;
    while target.exists() {
        n = n.saturating_add(1);
        target = path.with_file_name(format!("{file_name}.bak.{n}"));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::{is_builtin, move_aside};

    #[test]
    fn builtin_names_are_recognised() {
        assert!(is_builtin("starship"));
        assert!(!is_builtin("no-such-theme"));
    }

    #[test]
    fn move_aside_never_overwrites_an_existing_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let theme = dir.path().join("x.toml");
        std::fs::write(dir.path().join("x.toml.bak"), "old").expect("seed bak");
        std::fs::write(&theme, "new").expect("seed theme");

        let moved = move_aside(&theme).expect("move");

        assert_eq!(moved, dir.path().join("x.toml.bak.1"));
        assert!(!theme.exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.toml.bak")).expect("read"),
            "old"
        );
    }
}
