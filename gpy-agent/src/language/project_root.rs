//! Project-root resolution for language detection (#727).
//!
//! Inside git, the repository root is authoritative. Outside git there is no
//! registered root, so the nearest ancestor holding a project marker file
//! stands in for one: without it, a subdirectory with no sources of its own
//! (`docs/`, `assets/`) detects nothing while the shell pre-filters, which walk
//! up to find a marker, still ask for the segment. The IPC language handler
//! and `oneshot lang` both resolve their root here, so they always agree.

use crate::cache::ttl_map::TtlMap;
use crate::language::metadata::marker_file_names;
use crate::language::version::MAX_TOOL_VERSION_ANCESTORS;
use crate::watcher::multi_repo::MultiRepoWatcher;
use std::path::{Path, PathBuf};

/// The directory language detection for a request path is anchored at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectRoot {
    /// Root of the enclosing git repository.
    Git(PathBuf),
    /// Not inside git: the nearest ancestor below `$HOME` that holds a marker
    /// file, or the (canonical) request directory itself when none does.
    NonGit(PathBuf),
}

impl ProjectRoot {
    /// Whether the root is a git repository root.
    #[must_use]
    pub const fn is_git(&self) -> bool {
        matches!(self, Self::Git(_))
    }

    /// The root directory itself.
    #[must_use]
    pub fn into_path(self) -> PathBuf {
        match self {
            Self::Git(root) | Self::NonGit(root) => root,
        }
    }
}

/// Maximum request directories tracked by the non-git root memo.
const NON_GIT_ROOT_CACHE_CAPACITY: usize = 512;

/// Memoizes the non-git ancestor walk per canonical request directory, since
/// it runs on every non-git `lang` request. Shares the version cache's TTL knob
/// via [`TtlMap`].
static NON_GIT_ROOT_CACHE: TtlMap<PathBuf, PathBuf> = TtlMap::new(NON_GIT_ROOT_CACHE_CAPACITY);

/// Resolve the project root language detection for `path` should use.
///
/// The git root when `path` is inside a repository (unchanged behaviour);
/// otherwise the nearest ancestor holding a project marker file, else `path`.
/// The non-git walk only lists directory entries (never scans content, #390),
/// never treats or searches `$HOME` or anything above it, and stops after
/// [`MAX_TOOL_VERSION_ANCESTORS`] levels.
#[must_use]
pub fn project_root(path: &Path) -> ProjectRoot {
    MultiRepoWatcher::find_git_root(path)
        .map_or_else(|| ProjectRoot::NonGit(non_git_root(path)), ProjectRoot::Git)
}

/// Memoized non-git half of [`project_root`].
fn non_git_root(path: &Path) -> PathBuf {
    let dir = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(cached) = NON_GIT_ROOT_CACHE.get(&dir) {
        return cached;
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home_dir| home_dir.canonicalize().unwrap_or(home_dir));
    let root = resolve_non_git_root(&dir, home.as_deref());
    NON_GIT_ROOT_CACHE.insert(dir, root.clone());
    root
}

/// Nearest marker ancestor of `dir`, else `dir` itself (today's behaviour for a
/// directory outside any project).
fn resolve_non_git_root(dir: &Path, home: Option<&Path>) -> PathBuf {
    nearest_marker_ancestor(dir, home).unwrap_or_else(|| dir.to_path_buf())
}

/// Nearest ancestor of `path` (itself included) holding a marker file.
///
/// `$HOME` is the boundary, not a candidate: a stray `~/package.json` must not
/// turn every non-project directory under home into a node project. Outside
/// `$HOME`, the walk stops after [`MAX_TOOL_VERSION_ANCESTORS`] levels.
fn nearest_marker_ancestor(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut checked = 0_usize;
    for ancestor in path.ancestors() {
        if home.is_some_and(|home_dir| ancestor == home_dir) {
            break;
        }
        if has_marker_file(ancestor) {
            return Some(ancestor.to_path_buf());
        }

        checked = checked.saturating_add(1);
        if checked >= MAX_TOOL_VERSION_ANCESTORS {
            break;
        }
    }
    None
}

/// Whether `dir` directly contains a marker file, compared case-insensitively
/// like the detector's own marker detection. One directory read per call.
fn has_marker_file(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let markers = marker_file_names();
    entries.flatten().any(|entry| {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        markers
            .iter()
            .any(|marker| marker.eq_ignore_ascii_case(name.as_ref()))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;

    fn canonical_temp_dir() -> (tempfile::TempDir, PathBuf) {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let root = temp_dir.path().canonicalize().expect("canonicalize");
        (temp_dir, root)
    }

    #[test]
    fn nearest_marker_ancestor_wins() {
        let (_temp_dir, home) = canonical_temp_dir();
        let outer = home.join("outer");
        let inner = outer.join("inner");
        let docs = inner.join("docs");
        std::fs::create_dir_all(&docs).expect("docs dir");
        std::fs::write(outer.join("Cargo.toml"), "").expect("outer marker");
        // Case-insensitive, like the detector's marker matching.
        std::fs::write(inner.join("PACKAGE.JSON"), "{}").expect("inner marker");

        assert_eq!(
            nearest_marker_ancestor(&docs, Some(&home)),
            Some(inner.clone()),
            "the nearest marker ancestor must win over a farther one"
        );
        assert_eq!(
            nearest_marker_ancestor(&inner, Some(&home)),
            Some(inner),
            "a directory holding a marker is its own root"
        );
    }

    #[test]
    fn walk_stops_at_home_without_using_it() {
        let (_temp_dir, outside) = canonical_temp_dir();
        let home = outside.join("home");
        let docs = home.join("notes").join("docs");
        std::fs::create_dir_all(&docs).expect("docs dir");
        std::fs::write(outside.join("package.json"), "{}").expect("marker above home");
        std::fs::write(home.join("package.json"), "{}").expect("marker in home");

        assert_eq!(
            nearest_marker_ancestor(&docs, Some(&home)),
            None,
            "neither $HOME nor anything above it may become the project root"
        );
    }

    #[test]
    fn walk_is_bounded_outside_home() {
        let (_temp_dir, top) = canonical_temp_dir();
        std::fs::write(top.join("package.json"), "{}").expect("marker");
        let deep =
            (0_usize..MAX_TOOL_VERSION_ANCESTORS).fold(top.clone(), |dir, _level| dir.join("d"));
        std::fs::create_dir_all(&deep).expect("deep dir");

        assert_eq!(
            nearest_marker_ancestor(&deep, None),
            None,
            "the walk must give up after MAX_TOOL_VERSION_ANCESTORS levels"
        );
        let within_bound = deep.parent().expect("parent");
        assert_eq!(
            nearest_marker_ancestor(within_bound, None),
            Some(top),
            "a marker exactly MAX_TOOL_VERSION_ANCESTORS levels up is still found"
        );
    }

    #[test]
    fn no_marker_resolves_to_the_path_itself() {
        let (_temp_dir, home) = canonical_temp_dir();
        let scratch = home.join("scratch");
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        std::fs::write(scratch.join("notes.txt"), "x").expect("non-marker file");

        assert_eq!(
            resolve_non_git_root(&scratch, Some(&home)),
            scratch,
            "with no marker ancestor the request directory stays the root"
        );
    }
}
