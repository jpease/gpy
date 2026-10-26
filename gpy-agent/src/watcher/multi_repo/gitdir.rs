//! Git directory resolution: `.git` pointer files and linked-worktree common directories.

use std::fs;
use std::path::{Path, PathBuf};

/// Common git directory of a linked worktree; `None` for ordinary repos and
/// submodules (no `commondir` pointer).
pub(super) fn resolve_common_dir(git_dir: &Path) -> Option<PathBuf> {
    let common =
        crate::fs_util::read_small_file(&git_dir.join("commondir"), crate::git::POINTER_READ_CAP)
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
