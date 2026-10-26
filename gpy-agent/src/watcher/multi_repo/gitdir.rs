//! Git directory resolution: `.git` pointer files and linked-worktree common directories.

use crate::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Single-line pointer file read caps.
const GITDIR_POINTER_READ_CAP: usize = 4_096;
const COMMONDIR_POINTER_READ_CAP: usize = 4_096;

/// Common git directory of a linked worktree; `None` for ordinary repos and
/// submodules (no `commondir` pointer).
pub(super) fn resolve_common_dir(git_dir: &Path) -> Option<PathBuf> {
    let common =
        crate::fs_util::read_small_file(&git_dir.join("commondir"), COMMONDIR_POINTER_READ_CAP)
            .ok()
            .and_then(|contents| join_commondir_pointer(git_dir, &contents))
            .or_else(|| common_dir_from_worktrees_layout(git_dir))?;
    (common != git_dir).then_some(common)
}

/// Parse `commondir` contents, joining relative paths onto `git_dir`.
#[must_use]
pub(super) fn parse_commondir_pointer(git_dir: &Path, contents: &str) -> Option<PathBuf> {
    let target = Path::new(contents.trim());
    if target.as_os_str().is_empty() {
        return None;
    }
    Some(crate::git::resolve_relative_to(git_dir, target))
}

/// Resolve a `commondir` pointer (typically `../..`) to a canonical path.
fn join_commondir_pointer(git_dir: &Path, contents: &str) -> Option<PathBuf> {
    let resolved = parse_commondir_pointer(git_dir, contents)?;
    fs::canonicalize(&resolved).ok()
}

/// Fallback when `commondir` is unreadable: git's (unpromised) layout is
/// `<common>/worktrees/<name>`.
fn common_dir_from_worktrees_layout(git_dir: &Path) -> Option<PathBuf> {
    let parent = git_dir.parent()?;
    if parent.file_name()? != "worktrees" {
        return None;
    }
    fs::canonicalize(parent.parent()?).ok()
}

/// Resolve the actual git directory for the supplied repository.
///
/// # Errors
/// Returns an error when the `.git` entry cannot be read or parsed.
pub(super) fn resolve_gitdir(git_root: &Path) -> Result<PathBuf> {
    let git_entry = git_root.join(".git");

    if git_entry.is_dir() {
        return Ok(fs::canonicalize(&git_entry).unwrap_or(git_entry));
    }

    if git_entry.is_file() {
        let contents = crate::fs_util::read_small_file(&git_entry, GITDIR_POINTER_READ_CAP)
            .map_err(|e| {
                Error::watcher(format!(
                    "Failed to read gitdir pointer at {}: {e}",
                    git_entry.display()
                ))
            })?;

        let resolved = crate::git::parse_gitdir_link(&contents, git_root).ok_or_else(|| {
            Error::watcher(format!(
                "Invalid gitdir pointer contents at {}",
                git_entry.display()
            ))
        })?;

        return Ok(fs::canonicalize(&resolved).unwrap_or(resolved));
    }

    Err(Error::watcher(format!(
        "Could not locate git metadata at {}",
        git_entry.display()
    )))
}
