//! What sits at a `gpy plugin install` target directory.

use crate::plugin::{PluginId, PluginManifest};
use std::path::Path;

/// What sits at the install target.
pub(super) enum Occupant {
    Absent,
    /// A plugin with the id being installed.
    SamePlugin,
    /// Anything else, described for the refusal message.
    Other(String),
}

/// Read `target`'s own `plugin.toml`: discovery omits a directory it cannot
/// load, so the id is taken from the manifest, not from the directory name.
pub(super) fn occupant(target: &Path, id: &PluginId) -> Occupant {
    if std::fs::symlink_metadata(target).is_err() {
        return Occupant::Absent;
    }
    let Ok(text) = std::fs::read_to_string(target.join("plugin.toml")) else {
        return Occupant::Other("no readable plugin.toml".to_owned());
    };
    match PluginManifest::parse(&text) {
        Err(_) => Occupant::Other("an invalid plugin.toml".to_owned()),
        Ok(manifest) if manifest.id == *id => Occupant::SamePlugin,
        Ok(manifest) => Occupant::Other(format!("plugin '{}'", manifest.id)),
    }
}

/// Whether `a` and `b` name one directory. A case-variant spelling on a
/// case-insensitive volume (`Foo`, `foo`) does, though the paths differ.
#[cfg(unix)]
pub(super) fn same_directory(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(first), Ok(second)) => first.dev() == second.dev() && first.ino() == second.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub(super) fn same_directory(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(first), Ok(second)) => first == second,
        _ => false,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::same_directory;

    #[test]
    fn a_symlink_and_its_target_are_the_same_directory() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let real = temp.path().join("real");
        let other = temp.path().join("other");
        let alias = temp.path().join("alias");
        std::fs::create_dir_all(&real).expect("create real");
        std::fs::create_dir_all(&other).expect("create other");
        std::os::unix::fs::symlink(&real, &alias).expect("symlink");

        assert!(same_directory(&real, &alias));
        assert!(!same_directory(&real, &other));
        assert!(!same_directory(&real, &temp.path().join("missing")));
    }
}
