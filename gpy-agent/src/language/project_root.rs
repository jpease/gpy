//! Project-root resolution for language detection.
//!
//! Outside git, the nearest marker-holding ancestor stands in for the repo root
//! so marker-less subdirectories (`docs/`) still match the shell pre-filters.

use crate::cache::ttl_map::TtlMap;
use crate::language::ancestors::{MAX_ANCESTOR_DEPTH, bounded_ancestors, home_dir};
use crate::language::detection_cache::LANGUAGE_REFRESH_INTERVAL;
use crate::language::metadata::marker_file_names;

use crate::watcher::multi_repo::MultiRepoWatcher;
use std::path::{Path, PathBuf};

/// The directory language detection for a request path is anchored at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectRoot {
    /// Root of the enclosing git repository.
    Git(PathBuf),
    /// Nearest marker ancestor below `$HOME`, else the request directory.
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

/// Memoizes the non-git ancestor walk per canonical request directory. It
/// expires with the language refresh interval, so a marker created or removed
/// above a request directory is honored once a stale detection would be
/// re-run anyway; non-git trees have no watcher to invalidate it sooner.
static NON_GIT_ROOT_CACHE: TtlMap<PathBuf, PathBuf> =
    TtlMap::with_ttl(NON_GIT_ROOT_CACHE_CAPACITY, LANGUAGE_REFRESH_INTERVAL);

/// Resolve the language-detection root for `path`: git root, else nearest
/// marker ancestor below `$HOME` (bounded by [`MAX_ANCESTOR_DEPTH`]), else `path`.
#[must_use]
pub fn project_root(path: &Path) -> ProjectRoot {
    MultiRepoWatcher::find_git_root(path)
        .map_or_else(|| ProjectRoot::NonGit(non_git_root(path)), ProjectRoot::Git)
}

/// Memoized non-git half of [`project_root`].
fn non_git_root(path: &Path) -> PathBuf {
    memoized_non_git_root(path, || {
        home_dir().map(|home| home.canonicalize().unwrap_or(home))
    })
}

/// [`resolve_non_git_root`] for canonical `path` through [`NON_GIT_ROOT_CACHE`];
/// `home` is only consulted on a miss.
fn memoized_non_git_root(path: &Path, home: impl FnOnce() -> Option<PathBuf>) -> PathBuf {
    let dir = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(cached) = NON_GIT_ROOT_CACHE.get(&dir) {
        return cached;
    }
    let root = resolve_non_git_root(&dir, home().as_deref());
    NON_GIT_ROOT_CACHE.insert(dir, root.clone());
    root
}

/// Nearest marker ancestor of `dir`, else `dir` itself.
fn resolve_non_git_root(dir: &Path, home: Option<&Path>) -> PathBuf {
    nearest_marker_ancestor(dir, home).unwrap_or_else(|| dir.to_path_buf())
}

/// Nearest ancestor of `path` (inclusive) holding a marker file.
///
/// `$HOME` is a boundary, not a candidate: a stray `~/package.json` must not
/// make every directory under home a node project.
fn nearest_marker_ancestor(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    bounded_ancestors(path, home, MAX_ANCESTOR_DEPTH)
        .take_while(|ancestor| home != Some(*ancestor))
        .find(|ancestor| has_marker_file(ancestor))
        .map(Path::to_path_buf)
}

/// Most directory entries [`has_marker_file`] reads for a case-insensitive
/// match. A temp or downloads directory can hold a million entries, and an
/// ancestor walk on the request path must not list them all; such a
/// directory is not a project root anyway.
const MARKER_SCAN_LIMIT: usize = 1024;

/// Whether `dir` directly contains a marker file (case-insensitive). The
/// exact names are checked first (each one lookup, and every spelling on a
/// case-insensitive filesystem); other spellings only within the first
/// [`MARKER_SCAN_LIMIT`] entries.
fn has_marker_file(dir: &Path) -> bool {
    let markers = marker_file_names();
    if markers
        .iter()
        .any(|marker| dir.join(marker).symlink_metadata().is_ok())
    {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.take(MARKER_SCAN_LIMIT).flatten().any(|entry| {
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

    impl ProjectRoot {
        /// Backdate `path`'s memoized non-git root by `age`.
        pub(crate) fn backdate_non_git_memo(path: &Path, age: std::time::Duration) {
            let dir = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            if let Some(root) = NON_GIT_ROOT_CACHE.get(&dir) {
                let backdated = std::time::SystemTime::now()
                    .checked_sub(age)
                    .expect("test age must not predate the clock's range");
                NON_GIT_ROOT_CACHE.insert_at(dir, root, backdated);
            }
        }
    }

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
        let deep = (0_usize..MAX_ANCESTOR_DEPTH).fold(top.clone(), |dir, _level| dir.join("d"));
        std::fs::create_dir_all(&deep).expect("deep dir");

        assert_eq!(
            nearest_marker_ancestor(&deep, None),
            None,
            "the walk must give up after MAX_ANCESTOR_DEPTH levels"
        );
        let within_bound = deep.parent().expect("parent");
        assert_eq!(
            nearest_marker_ancestor(within_bound, None),
            Some(top),
            "a marker exactly MAX_ANCESTOR_DEPTH levels up is still found"
        );
    }

    #[test]
    fn markers_are_found_by_name_in_a_large_dir_and_in_any_case_in_a_small_one() {
        let (_temp_dir, root) = canonical_temp_dir();
        let large = root.join("large");
        std::fs::create_dir_all(&large).expect("large dir");
        for i in 0..MARKER_SCAN_LIMIT + 10 {
            std::fs::write(large.join(format!("f{i}")), "").expect("filler");
        }
        std::fs::write(large.join("package.json"), "{}").expect("marker");
        assert!(
            has_marker_file(&large),
            "an exact name is found past the scan limit"
        );

        let small = root.join("small");
        std::fs::create_dir_all(&small).expect("small dir");
        std::fs::write(small.join("PACKAGE.JSON"), "{}").expect("odd-case marker");
        assert!(
            has_marker_file(&small),
            "another spelling is found within the limit"
        );

        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).expect("plain dir");
        std::fs::write(plain.join("notes.txt"), "x").expect("non-marker");
        assert!(!has_marker_file(&plain));
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

    /// A marker appearing or vanishing above a memoized directory is honored
    /// once the language refresh interval passes; `$HOME` stays a boundary.
    #[test]
    fn memo_follows_ancestor_marker_changes_within_the_refresh_interval() {
        let (_temp_dir, home) = canonical_temp_dir();
        let project = home.join("project");
        let docs = project.join("docs");
        std::fs::create_dir_all(&docs).expect("docs dir");
        std::fs::write(home.join("package.json"), "{}").expect("marker in home");
        let resolve = || memoized_non_git_root(&docs, || Some(home.clone()));
        let expire = || ProjectRoot::backdate_non_git_memo(&docs, LANGUAGE_REFRESH_INTERVAL);

        assert_eq!(
            resolve(),
            docs,
            "no marker below $HOME: docs is its own root"
        );

        std::fs::write(project.join("Cargo.toml"), "").expect("project marker");
        expire();
        assert_eq!(resolve(), project, "a new ancestor marker is picked up");

        std::fs::remove_file(project.join("Cargo.toml")).expect("remove marker");
        expire();
        assert_eq!(resolve(), docs, "a removed ancestor marker is dropped");
    }
}
