//! Bounded ancestor walks shared by tool-config, `.ruby-version` and project-root lookups.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Ancestor-walk depth bound outside `$HOME` (guards pathological paths).
pub(crate) const MAX_ANCESTOR_DEPTH: usize = 64;

/// The user's home directory, canonicalized once: callers walk canonical
/// paths (`find_worktree_root`, `non_git_root`), so a `$HOME` reached through
/// a symlink would otherwise never match an ancestor and the walk would run
/// past it.
pub(crate) fn home_dir() -> Option<PathBuf> {
    static HOME: LazyLock<Option<PathBuf>> =
        LazyLock::new(|| crate::paths::home_path().map(|home| home.canonicalize().unwrap_or(home)));
    HOME.clone()
}

/// `path` and its ancestors, nearest first, ending at `home` (inclusive) or
/// after `limit` levels, whichever comes first.
pub(crate) fn bounded_ancestors<'a>(
    path: &'a Path,
    home: Option<&'a Path>,
    limit: usize,
) -> impl Iterator<Item = &'a Path> {
    let mut past_home = false;
    path.ancestors()
        .take_while(move |ancestor| {
            let within = !past_home;
            past_home = home == Some(*ancestor);
            within
        })
        .take(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_after_home_inclusive() {
        let walked: Vec<&Path> =
            bounded_ancestors(Path::new("/h/u/a/b"), Some(Path::new("/h/u")), 64).collect();
        assert_eq!(
            walked,
            [
                Path::new("/h/u/a/b"),
                Path::new("/h/u/a"),
                Path::new("/h/u")
            ]
        );
    }

    #[test]
    fn stops_after_limit() {
        assert_eq!(bounded_ancestors(Path::new("/a/b/c/d"), None, 2).count(), 2);
    }

    #[test]
    fn walks_to_root_without_home() {
        assert_eq!(bounded_ancestors(Path::new("/a/b"), None, 64).count(), 3);
    }
}
