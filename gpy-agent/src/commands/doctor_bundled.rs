//! Doctor diagnosis for bundled plugins that an upgrade left uninstalled.

use std::path::Path;

/// Segments provided by the plugins `gpy setup` installs under the bundled
/// plugin root (`plugins/` in the release archive). Before 0.7 these were
/// builtin segments, so an upgrade without `gpy setup` silently drops them.
const BUNDLED_PLUGIN_NAMES: [&str; 2] = ["username", "hostname"];

/// Whether `segment` names a bundled plugin whose manifest is absent from
/// `bundled_dir`, i.e. `gpy setup` has not installed it.
pub(super) fn is_uninstalled_bundled(segment: &str, bundled_dir: &Path) -> bool {
    BUNDLED_PLUGIN_NAMES.contains(&segment)
        && !bundled_dir.join(segment).join("plugin.toml").is_file()
}

/// Remediation shown when unrecognized segments are uninstalled bundled plugins.
pub(super) const SETUP_REMEDIATION: &str = "Remediation: Run `gpy setup`. These are bundled plugins that were never installed or are outdated after an upgrade.";

#[cfg(test)]
mod tests {
    use super::is_uninstalled_bundled;
    use tempfile::TempDir;

    #[test]
    fn bundled_name_with_empty_dir_is_uninstalled() {
        let dir = TempDir::new().expect("tempdir");
        assert!(is_uninstalled_bundled("username", dir.path()));
        assert!(is_uninstalled_bundled("hostname", dir.path()));
    }

    #[test]
    fn installed_bundled_plugin_is_not_flagged() {
        let dir = TempDir::new().expect("tempdir");
        let plugin = dir.path().join("username");
        std::fs::create_dir_all(&plugin).expect("mkdir");
        std::fs::write(plugin.join("plugin.toml"), "id = \"username\"\n").expect("write");
        assert!(!is_uninstalled_bundled("username", dir.path()));
    }

    #[test]
    fn non_bundled_name_is_never_flagged() {
        let dir = TempDir::new().expect("tempdir");
        assert!(!is_uninstalled_bundled("usrname", dir.path()));
    }
}
