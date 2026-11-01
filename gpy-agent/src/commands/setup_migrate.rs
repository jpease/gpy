//! Migration from the layout releases before the data-root move installed
//! (`$XDG_CONFIG_HOME/fish/gpy`, `$XDG_CONFIG_HOME/gpy/{bash,zsh}`).
//!
//! Deprecated: removed together with `legacy_shell_dir_for` after one minor
//! release. Nothing is deleted here: open shells still source files from the
//! old trees lazily, and an edited rc block may still point at them.

use super::output;
use super::rc_block::{BlockChange, ensure_block, has_block};
use super::setup::{
    DataRoot, Environment, RC_BLOCK_HEADER, SHELLS, VERSION_STAMP, bash_rc_body, fish_rc_body,
    legacy_bash_rc_bodies, legacy_shell_dir_for, link_fish_prompt, links_into, rc_file_for,
    shell_dir_for, zsh_rc_body,
};
use crate::shell::Shell;
use std::path::{Path, PathBuf};

/// The rc block bodies earlier releases wrote for `shell`, sourcing the old tree.
pub(super) fn old_rc_bodies(config_home: &Path, shell: Shell) -> Vec<String> {
    let old = legacy_shell_dir_for(config_home, shell);
    match shell {
        Shell::Fish => vec![fish_rc_body(&old)],
        Shell::Zsh => vec![zsh_rc_body(&old)],
        Shell::Bash => {
            let gpy_bash = old.join("gpy.bash");
            let mut bodies = legacy_bash_rc_bodies(&gpy_bash);
            bodies.push(bash_rc_body(&gpy_bash));
            bodies
        }
    }
}

/// The file the rc block of `shell` sources inside the tree `dir`.
fn entry_file(dir: &Path, shell: Shell) -> PathBuf {
    match shell {
        Shell::Fish => dir.join("conf.d/gpy_init.fish"),
        Shell::Zsh => dir.join("gpy.zsh"),
        Shell::Bash => dir.join("gpy.bash"),
    }
}

/// The rc block body for `shell` under the data-root tree `dir`.
fn new_rc_body(dir: &Path, shell: Shell) -> String {
    match shell {
        Shell::Fish => fish_rc_body(dir),
        Shell::Zsh => zsh_rc_body(dir),
        Shell::Bash => bash_rc_body(&dir.join("gpy.bash")),
    }
}

/// Move every shell with a stamped old tree over to the data root: rewrite
/// its rc block, re-point a Fish prompt link, and say what to remove.
/// Failures are warnings; the old setup keeps working.
pub(super) fn migrate_old_layout(env: &Environment, config_home: &Path, root: &DataRoot) {
    for shell in SHELLS {
        let old = legacy_shell_dir_for(config_home, shell);
        if !old.join(VERSION_STAMP).is_file() {
            continue;
        }
        let new_dir = shell_dir_for(root, shell);
        let mut kept_edited = None;
        if let Ok(rc) = rc_file_for(env, shell)
            && has_block(&rc)
        {
            match ensure_block(
                &rc,
                RC_BLOCK_HEADER,
                &new_rc_body(&new_dir, shell),
                &old_rc_bodies(config_home, shell),
            ) {
                Ok(BlockChange::KeptEdited) => kept_edited = Some(rc),
                Ok(_) => {}
                Err(err) => output::warn(format_args!(
                    "could not update the GPY block in {}: {err}; it still loads {}",
                    rc.display(),
                    old.display()
                )),
            }
        }
        if shell == Shell::Fish {
            repoint_fish_prompt(config_home, &new_dir, &old);
        }
        match kept_edited {
            Some(rc) => output::plain(format_args!(
                "Your edited GPY block in {} still loads {}; change it to {}",
                rc.display(),
                old.display(),
                entry_file(&new_dir, shell).display()
            )),
            None => output::plain(format_args!(
                "The GPY files in {dir} are no longer used; remove them after restarting your open shells: rm -rf '{dir}'",
                dir = old.display()
            )),
        }
    }
}

/// Re-point a `fish_prompt.fish` link that goes into the old Fish tree.
fn repoint_fish_prompt(config_home: &Path, new_dir: &Path, old: &Path) {
    let target = config_home.join("fish/functions/fish_prompt.fish");
    let source = new_dir.join("functions/fish_prompt.fish");
    if links_into(&target, old)
        && source.is_file()
        && let Err(err) = link_fish_prompt(&target, &source, old)
    {
        output::warn(format_args!(
            "could not re-point {}: {}",
            target.display(),
            crate::error::user_message(&err)
        ));
    }
}
