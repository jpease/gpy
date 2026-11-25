//! Doctor diagnosis for bundled plugins that an upgrade left uninstalled.

use crate::plugin::BUNDLED_PLUGIN_NAMES;
use std::path::Path;

/// Whether `segment` names a bundled plugin whose manifest is absent from
/// `bundled_dir`, i.e. `gpy setup` has not installed it. With no bundled
/// root (no data directory) every bundled plugin is missing.
pub(super) fn is_uninstalled_bundled(segment: &str, bundled_dir: Option<&Path>) -> bool {
    BUNDLED_PLUGIN_NAMES.contains(&segment)
        && bundled_dir.is_none_or(|dir| !dir.join(segment).join("plugin.toml").is_file())
}

/// Remediation shown when unrecognized segments are uninstalled bundled
/// plugins. `overridden` is the `GPY_BUNDLED_PLUGIN_DIR` directory when it
/// is set: `gpy setup` installs only into the data directory, so it cannot
/// fill that one.
pub(super) fn uninstalled_bundled_remediation(overridden: Option<&Path>) -> String {
    overridden.map_or_else(
        || "Remediation: Run `gpy setup`. These are bundled plugins that were never installed or are outdated after an upgrade.".to_owned(),
        |dir| {
            format!(
                "Remediation: GPY_BUNDLED_PLUGIN_DIR points at {}, which lacks these bundled plugins; `gpy setup` installs them only into the data directory. Point the variable at a directory that holds them, or unset it and run `gpy setup`.",
                dir.display()
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::is_uninstalled_bundled;
    use tempfile::TempDir;

    #[test]
    fn bundled_name_with_empty_dir_is_uninstalled() {
        let dir = TempDir::new().expect("tempdir");
        assert!(is_uninstalled_bundled("username", Some(dir.path())));
        assert!(is_uninstalled_bundled("hostname", Some(dir.path())));
    }

    #[test]
    fn installed_bundled_plugin_is_not_flagged() {
        let dir = TempDir::new().expect("tempdir");
        let plugin = dir.path().join("username");
        std::fs::create_dir_all(&plugin).expect("mkdir");
        std::fs::write(plugin.join("plugin.toml"), "id = \"username\"\n").expect("write");
        assert!(!is_uninstalled_bundled("username", Some(dir.path())));
    }

    #[test]
    fn non_bundled_name_is_never_flagged() {
        let dir = TempDir::new().expect("tempdir");
        assert!(!is_uninstalled_bundled("usrname", Some(dir.path())));
        assert!(!is_uninstalled_bundled("usrname", None));
    }

    /// No data directory: there is no bundled root, so every bundled plugin
    /// is missing rather than looked up under a relative `.local/share/gpy`.
    #[test]
    fn no_bundled_root_means_every_bundled_plugin_is_missing() {
        assert!(is_uninstalled_bundled("username", None));
        assert!(is_uninstalled_bundled("hostname", None));
    }
}
