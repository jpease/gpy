//! Bash wiring for `gpy setup`: the `.bashrc` block and the login-file loader
//! that makes login shells read `~/.bashrc`.

use super::rc_block::{BlockChange, ensure_block, has_block};
use super::setup::{Environment, RC_BLOCK_HEADER, ensure_and_report, rc_file_for};
use crate::Result;
use crate::shell::Shell;
use std::fs;
use std::path::{Path, PathBuf};

/// The `.bashrc` block body: GPY loads only in interactive shells.
pub(super) fn bash_rc_body(gpy_bash: &Path) -> String {
    format!(
        "case $- in\n    *i*) source \"{}\" ;;\nesac",
        gpy_bash.display()
    )
}

/// `.bashrc` block bodies earlier releases wrote: an unguarded `source` of this
/// install, and the one releases before 0.9 wrote for the tree in
/// `~/.config/gpy/bash` that `gpy setup` no longer keeps.
pub(super) fn legacy_bash_rc_bodies(gpy_bash: &Path, config_home: &Path) -> Vec<String> {
    let old_gpy_bash = config_home.join("gpy/bash/gpy.bash");
    [gpy_bash, old_gpy_bash.as_path()]
        .map(|sourced| format!("source \"{}\"", sourced.display()))
        .into()
}

/// Login-file loader body; inert outside Bash.
const BASH_LOGIN_LOADER: &str =
    "[ -n \"${BASH_VERSION:-}\" ] && [ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"";
/// Login-file loader body earlier releases wrote.
const LEGACY_BASH_LOGIN_LOADER: &str = "[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"";
/// Header line of the login-file loader block.
const BASH_LOGIN_HEADER: &str = "GPY: load ~/.bashrc in login shells";

/// Wire Bash: rc block in `~/.bashrc`, plus a login-shell loader.
pub(super) fn wire_bash(
    env: &Environment,
    config_home: &Path,
    shell_dir: &Path,
) -> Result<(PathBuf, BlockChange)> {
    let home = env.home()?;
    let rc = rc_file_for(env, Shell::Bash)?;
    let gpy_bash = shell_dir.join("gpy.bash");
    let legacy_bodies = legacy_bash_rc_bodies(&gpy_bash, config_home);
    let change = ensure_and_report(
        &rc,
        RC_BLOCK_HEADER,
        &bash_rc_body(&gpy_bash),
        &legacy_bodies,
        "so it loads the files in the data directory",
    )?;

    // Upgrade loaders earlier releases wrote before deciding whether one is
    // needed: `sources_bashrc` accepts the old loader as a working one.
    let login_files = [".bash_profile", ".bash_login", ".profile"].map(|name| home.join(name));
    for path in &login_files {
        if has_block(path) {
            ensure_and_report(
                path,
                BASH_LOGIN_HEADER,
                BASH_LOGIN_LOADER,
                &[LEGACY_BASH_LOGIN_LOADER.to_owned()],
                "so `sh` login shells skip it",
            )?;
        }
    }

    // A login shell reads only the first of these. With none present, create
    // ~/.profile (~/.bash_profile would shadow a later ~/.profile).
    let login = login_files
        .iter()
        .find(|path| path.exists())
        .cloned()
        .unwrap_or_else(|| home.join(".profile"));
    let loads_bashrc =
        fs::read(&login).is_ok_and(|bytes| sources_bashrc(&String::from_utf8_lossy(&bytes)));
    if !loads_bashrc
        && ensure_block(&login, BASH_LOGIN_HEADER, BASH_LOGIN_LOADER, &[])? == BlockChange::Added
    {
        crate::commands::output::plain(format_args!(
            "Added ~/.bashrc loader to {}",
            login.display()
        ));
    }
    Ok((rc, change))
}

/// Whether a startup file runs `~/.bashrc`: a `.`/`source` command, outside a
/// comment, whose argument is a `.bashrc` path. A mention alone (a comment, a
/// commented-out loader, `echo ~/.bashrc`) does not load it.
fn sources_bashrc(text: &str) -> bool {
    text.lines().any(|line| {
        let words = shell_words(line);
        words
            .array_windows::<2>()
            .enumerate()
            .any(|(index, &[command, argument])| {
                let in_command_position = index
                    .checked_sub(1)
                    .and_then(|previous| words.get(previous))
                    .is_none_or(|word| {
                        matches!(
                            *word,
                            ";" | "&" | "|" | "(" | ")" | "{" | "then" | "else" | "do"
                        )
                    });
                let path = argument.trim_matches(['"', '\'']);
                in_command_position
                    && matches!(command, "." | "source")
                    && (path == ".bashrc" || path.ends_with("/.bashrc"))
            })
    })
}

/// Splits one shell line into words, with each of `;&|()` as its own word,
/// and drops a trailing `#` comment. Quoting is not interpreted.
fn shell_words(line: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut word_start = None;
    for (index, ch) in line.char_indices() {
        let separator = matches!(ch, ';' | '&' | '|' | '(' | ')');
        if ch.is_whitespace() || separator {
            if let Some(start) = word_start.take() {
                words.push(&line[start..index]);
            }
            if separator {
                words.push(&line[index..=index]);
            }
        } else if word_start.is_none() {
            if ch == '#' {
                return words;
            }
            word_start = Some(index);
        }
    }
    if let Some(start) = word_start {
        words.push(&line[start..]);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_uncommented_source_command_counts_as_a_bashrc_loader() {
        for loader in [
            "[ -f ~/.bashrc ] && . ~/.bashrc",
            "[ -r \"$HOME/.bashrc\" ] && source \"$HOME/.bashrc\" # login",
            "if [ -f \"$HOME/.bashrc\" ]; then . \"$HOME/.bashrc\"; fi",
            "if [ -f ~/.bashrc ]; then\n    . ~/.bashrc\nfi",
            "test -f ${HOME}/.bashrc&&. ${HOME}/.bashrc",
            "case $- in *i*) . ~/.bashrc;; esac",
        ] {
            assert!(sources_bashrc(loader), "{loader}");
        }
        for mention in [
            "# .bashrc is intentionally not sourced here yet",
            "# [ -f ~/.bashrc ] && . ~/.bashrc",
            "    #. \"$HOME/.bashrc\"",
            "export PATH=$PATH:~/bin # then . ~/.bashrc",
            "echo . ~/.bashrc",
            "[ -f ~/.bashrc ] && . ~/.bashrc.local",
        ] {
            assert!(!sources_bashrc(mention), "{mention}");
        }
    }
}
