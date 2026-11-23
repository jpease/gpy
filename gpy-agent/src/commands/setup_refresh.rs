//! `gpy setup --refresh-stale`: after an upgrade, reinstall the shell trees a
//! previous `gpy setup` installed for an older version. `gpy prompt` starts it
//! detached when the stamp of its shell's tree differs from its own version,
//! so a package manager that never runs `gpy setup` (Homebrew) still moves the
//! shells onto the new files. No rc file is read for writing or changed; open
//! shells keep the code they sourced until they start again.

use super::setup::{
    Environment, VERSION_STAMP, find_source_root, installed_integrations_in, is_in_place,
    shell_dir_for,
};
use super::setup_install::{InstallMode, install_bundled_plugins, install_files, install_root};
use crate::Result;
use crate::shell::Shell;
use std::fs;
use std::path::Path;

/// The version a stamped tree at `dir` was installed for; `None` when the tree
/// carries no stamp, which only `gpy setup` writes.
#[must_use]
pub fn stamped_version(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(VERSION_STAMP))
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// Whether `shell`'s tree under the data root was installed by `gpy setup` for
/// another version than this binary's. Reads one small file.
#[must_use]
pub fn tree_is_stale(shell: Shell) -> bool {
    let Ok(root) = Environment::from_process().data_root() else {
        return false;
    };
    stamped_version(&shell_dir_for(&root, shell)).is_some_and(|v| v != crate::VERSION)
}

/// Reinstall every stale tree `gpy setup` wired: the shell's rc block sources
/// the tree under the data root, and its stamp names another version. The
/// bundled plugins are reinstalled with them. Failures are skipped: the next
/// prompt that finds a tree still stale tries again.
pub fn refresh_stale(completions: impl Fn(Shell) -> Result<Vec<u8>>) {
    let env = Environment::from_process();
    let (Ok(root), Ok(config_home)) = (install_root(&env), env.config_home()) else {
        return;
    };
    let mut plugins_source = None;
    for integration in installed_integrations_in(&env) {
        let shell = integration.shell;
        // A tree elsewhere was not installed by setup into the data root; one
        // without a stamp predates stamps and is left to `gpy setup`.
        if integration.dir != shell_dir_for(&root, shell)
            || integration.version.is_none_or(|v| v == crate::VERSION)
        {
            continue;
        }
        let Ok(source_root) = find_source_root(None, &env, shell) else {
            continue;
        };
        // Only the stale tree itself was found: nothing newer to install.
        if is_in_place(&source_root, &root) {
            continue;
        }
        if install_files(
            shell,
            &source_root,
            &root,
            &config_home,
            InstallMode::Copy,
            true,
            &completions,
        )
        .is_ok()
        {
            plugins_source.get_or_insert(source_root);
        }
    }
    if let Some(source_root) = plugins_source {
        drop(install_bundled_plugins(
            &source_root,
            &root,
            InstallMode::Copy,
        ));
    }
}
