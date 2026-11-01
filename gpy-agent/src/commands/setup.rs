//! `gpy setup`: install the per-shell scripts and wire them into the rc file.
//!
//! Lays files out exactly as `install-oneline.sh` does (Fish:
//! `$XDG_CONFIG_HOME/fish/gpy`; Zsh/Bash: `$XDG_CONFIG_HOME/gpy/<shell>`) so
//! both installers share one uninstall path. The `gpy-init` marker lines (see
//! `rc_block`) are the contract `scripts/uninstall.*` remove by. Every step
//! is idempotent.

use super::rc_block::{BlockChange, append_block, ensure_block, has_block};
use super::setup_install::{
    BUNDLED_PLUGINS_SOURCE, install_bundled_plugins, install_files, preflight, replace_tree,
};
use crate::commands::init::{self, InitOptions};
use crate::paths::{self, Os};
use crate::shell::Shell;
use crate::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Comment on the line after the block's start marker that says what the block is for.
const RC_BLOCK_HEADER: &str = "GPY shell integration";
/// File in each installed shell tree naming the `gpy` version that wrote it,
/// so `gpy doctor` can spot a tree an upgrade left behind.
pub(super) const VERSION_STAMP: &str = ".gpy-version";
/// Every shell `gpy setup` can install.
const SHELLS: [Shell; 3] = [Shell::Fish, Shell::Zsh, Shell::Bash];

/// Options for [`run`], mirroring the `gpy setup` CLI flags.
#[derive(Debug, Clone, Default)]
pub struct SetupOptions {
    /// Shell to set up; detected from the environment when `None`.
    pub shell: Option<Shell>,
    /// Directory holding the `fish/`, `zsh/` and `bash/` trees; searched for
    /// when `None`.
    pub from: Option<PathBuf>,
    /// Skip the interactive icon prompt of the first-run config bootstrap.
    pub non_interactive: bool,
}

/// The process environment `gpy setup` reads, gathered once for testability.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// Absolute home directory.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, when set.
    pub xdg_config_home: Option<PathBuf>,
    /// `ZDOTDIR`, when set and non-empty.
    pub zdotdir: Option<PathBuf>,
    /// `$SHELL`, the login shell.
    pub login_shell: Option<String>,
    /// `GPY_SHELL`, the explicit override shared with the curl installer.
    pub gpy_shell: Option<String>,
    /// `GPY_SHARE_DIR`, an explicit location of the shell trees.
    pub share_dir: Option<PathBuf>,
    /// The running binary, with symlinks resolved.
    pub exe: Option<PathBuf>,
}

impl Environment {
    /// Read the environment of the current process.
    #[must_use]
    pub fn from_process() -> Self {
        let non_empty = |key: &str| std::env::var_os(key).filter(|value| !value.is_empty());
        Self {
            home: paths::home_path(),
            xdg_config_home: paths::xdg_config_home(),
            zdotdir: non_empty("ZDOTDIR").map(PathBuf::from),
            login_shell: std::env::var("SHELL").ok(),
            gpy_shell: std::env::var("GPY_SHELL").ok(),
            share_dir: non_empty("GPY_SHARE_DIR").map(PathBuf::from),
            exe: std::env::current_exe()
                .ok()
                .map(|exe| fs::canonicalize(&exe).unwrap_or(exe)),
        }
    }

    /// `$XDG_CONFIG_HOME`, or `$HOME/.config`; fails without an absolute home.
    fn config_home(&self) -> Result<PathBuf> {
        let home = self.home.as_deref();
        if home.is_none() {
            return Err(Error::invalid(
                "cannot find your home directory (HOME is unset or not an absolute path)",
            ));
        }
        let root = paths::config_root_for(self.xdg_config_home.as_deref(), home, Os::Unix);
        // `config_root_for` returns `<config home>/gpy`.
        root.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| Error::invalid("cannot resolve the config directory"))
    }

    /// The home directory.
    fn home(&self) -> Result<&Path> {
        self.home
            .as_deref()
            .ok_or_else(|| Error::invalid("cannot find your home directory"))
    }
}

/// Everything `setup` did, for the closing report.
struct Outcome {
    shell_dir: PathBuf,
    rc_file: PathBuf,
    rc_block_added: bool,
    /// Where the bundled plugins went; `None` for a source without them.
    plugins_dir: Option<PathBuf>,
}

/// Run `gpy setup`. `completions` renders the structural completion script,
/// which only the binary can generate.
///
/// Shells set up earlier get their files refreshed from the same source too,
/// so an upgrade run for one shell does not leave the others on old scripts.
///
/// # Errors
/// Fails when no shell can be determined, the scripts cannot be found, or a
/// file cannot be written. The rc file is touched last.
pub fn run(options: &SetupOptions, completions: impl Fn(Shell) -> Result<Vec<u8>>) -> Result<()> {
    if cfg!(not(unix)) {
        return Err(Error::invalid(
            "shell integration needs a Unix shell; on Windows, use GPY under WSL",
        ));
    }
    let env = Environment::from_process();
    let shell = detect_shell(
        options.shell,
        env.gpy_shell.as_deref(),
        env.login_shell.as_deref(),
    )?;
    let source_root = find_source_root(options.from.as_deref(), &env, shell)?;
    crate::commands::output::plain(format_args!(
        "Setting up GPY for {shell} from {}",
        source_root.display()
    ));

    let outcome = install(shell, &source_root, &env, &completions)?;

    crate::commands::output::plain(format_args!(
        "Installed {shell} files to {}",
        outcome.shell_dir.display()
    ));
    if let Some(dir) = &outcome.plugins_dir {
        crate::commands::output::plain(format_args!(
            "Installed bundled plugins to {}",
            dir.display()
        ));
    }
    if outcome.rc_block_added {
        crate::commands::output::plain(format_args!(
            "Added GPY initialization to {}",
            outcome.rc_file.display()
        ));
    } else {
        crate::commands::output::plain(format_args!(
            "GPY already configured in {}",
            outcome.rc_file.display()
        ));
    }
    refresh_other_shells(shell, &source_root, &env, &completions);
    if options.from.is_some() || env.share_dir.is_some() {
        keep_shell_trees(&source_root, &env);
    }
    super::setup_themes::retire_unmodified_themes();
    if let Err(err) = init::run(InitOptions {
        non_interactive: options.non_interactive,
        redetect: false,
    }) {
        crate::commands::output::warn(format_args!(
            "Icon setup skipped ({err}); using built-in defaults"
        ));
    }
    print_next_steps(shell, &outcome.rc_file, env.login_shell.as_deref());
    Ok(())
}

/// Copy the shell trees and the bundled plugins from an explicit `source_root`
/// to `share/gpy` beside the binary (`~/.local/share/gpy` for the installers),
/// where a later bare `gpy setup` looks: the installers pass a temporary
/// `--from`. Skipped when the binary lives in the source tree (a checkout's
/// build) or the trees are already there; a failure is reported and setup
/// still succeeds.
fn keep_shell_trees(source_root: &Path, env: &Environment) {
    let Some(prefix) = env
        .exe
        .as_deref()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return;
    };
    let source = fs::canonicalize(source_root).unwrap_or_else(|_| source_root.to_path_buf());
    let target = prefix.join("share").join("gpy");
    if env
        .exe
        .as_deref()
        .is_some_and(|exe| exe.starts_with(&source))
        || fs::canonicalize(&target).is_ok_and(|dir| dir == source)
    {
        return;
    }
    let copy = || -> Result<()> {
        for shell in SHELLS {
            if !has_shell_tree(&source, shell) {
                continue;
            }
            replace_tree(
                &source.join(shell.as_str()),
                &target.join(shell.as_str()),
                &source,
            )?;
        }
        let plugins = source.join(BUNDLED_PLUGINS_SOURCE);
        if plugins.is_dir() {
            replace_tree(&plugins, &target.join(BUNDLED_PLUGINS_SOURCE), &source)?;
        }
        Ok(())
    };
    match copy() {
        Ok(()) => crate::commands::output::plain(format_args!(
            "Kept the shell scripts in {} for later `gpy setup` runs",
            target.display()
        )),
        Err(err) => crate::commands::output::warn(format_args!(
            "Could not keep the shell scripts in {} ({}); a later `gpy setup` will need --from",
            target.display(),
            crate::error::user_message(&err)
        )),
    }
}

/// Refresh the files of every other shell already set up (its tree is
/// installed) from `source_root`. Its rc file is left alone. A failure is
/// reported and skipped: the shell asked for is already done.
fn refresh_other_shells(
    done: Shell,
    source_root: &Path,
    env: &Environment,
    completions: &impl Fn(Shell) -> Result<Vec<u8>>,
) {
    let Ok(config_home) = env.config_home() else {
        return;
    };
    for shell in SHELLS {
        let shell_dir = shell_dir_for(&config_home, shell);
        if shell == done
            || !shell_dir.join(entry_point(shell)).is_file()
            || !has_shell_tree(source_root, shell)
        {
            continue;
        }
        match install_files(shell, source_root, &config_home, completions) {
            Ok(dir) => crate::commands::output::plain(format_args!(
                "Refreshed {shell} files in {} (set up earlier)",
                dir.display()
            )),
            Err(err) => crate::commands::output::warn(format_args!(
                "Could not refresh the {shell} files in {} ({}); run `gpy setup --shell {shell}`",
                shell_dir.display(),
                crate::error::user_message(&err)
            )),
        }
    }
}

/// One installed shell integration, as `gpy doctor` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledIntegration {
    /// The shell.
    pub shell: Shell,
    /// Where its scripts are installed.
    pub dir: PathBuf,
    /// The `gpy` version that installed them; `None` for a tree written
    /// before setup stamped versions.
    pub version: Option<String>,
}

/// The shell integrations `gpy setup` has installed for the current user.
#[must_use]
pub fn installed_integrations() -> Vec<InstalledIntegration> {
    let Ok(config_home) = Environment::from_process().config_home() else {
        return Vec::new();
    };
    installed_integrations_in(&config_home)
}

/// [`installed_integrations`] under an explicit config home.
fn installed_integrations_in(config_home: &Path) -> Vec<InstalledIntegration> {
    SHELLS
        .into_iter()
        .filter_map(|shell| {
            let dir = shell_dir_for(config_home, shell);
            dir.join(entry_point(shell)).is_file().then(|| {
                let version = fs::read_to_string(dir.join(VERSION_STAMP))
                    .ok()
                    .map(|text| text.trim().to_owned())
                    .filter(|text| !text.is_empty());
                InstalledIntegration {
                    shell,
                    dir,
                    version,
                }
            })
        })
        .collect()
}

/// Choose the shell: the flag wins, then `GPY_SHELL`, then `$SHELL`.
///
/// # Errors
/// Fails on an unsupported value, or when nothing identifies a shell.
#[allow(
    clippy::map_err_ignore,
    reason = "Error::Invalid has no source field and the message already quotes the rejected shell name"
)]
pub fn detect_shell(
    flag: Option<Shell>,
    gpy_shell: Option<&str>,
    login_shell: Option<&str>,
) -> Result<Shell> {
    if let Some(shell) = flag {
        return Ok(shell);
    }
    if let Some(name) = gpy_shell.filter(|name| !name.is_empty()) {
        return name.parse().map_err(|_| {
            Error::invalid(format!(
                "GPY_SHELL is '{name}', which is not fish, zsh or bash"
            ))
        });
    }
    let basename = login_shell
        .and_then(|path| path.rsplit('/').next())
        .filter(|name| !name.is_empty());
    basename.map_or_else(
        || {
            Err(Error::invalid(
                "cannot tell which shell to set up ($SHELL is unset); pass --shell fish, zsh or bash",
            ))
        },
        |name| {
            name.parse().map_err(|_| {
                Error::invalid(format!(
                    "your login shell ($SHELL) is '{name}', which GPY does not support; \
                     pass --shell fish, zsh or bash to set up a shell you use anyway"
                ))
            })
        },
    )
}

/// The file whose presence marks `root/<shell>` as a usable script tree.
const fn entry_point(shell: Shell) -> &'static str {
    match shell {
        Shell::Fish => "conf.d/gpy_init.fish",
        Shell::Zsh => "gpy.zsh",
        Shell::Bash => "gpy.bash",
    }
}

fn has_shell_tree(root: &Path, shell: Shell) -> bool {
    root.join(shell.as_str()).join(entry_point(shell)).is_file()
}

/// Locate the directory holding the per-shell script trees. An explicit
/// `--from` or `GPY_SHARE_DIR` without the tree is an error, not a reason to
/// search elsewhere.
///
/// # Errors
/// Fails when no candidate holds the tree.
pub fn find_source_root(from: Option<&Path>, env: &Environment, shell: Shell) -> Result<PathBuf> {
    for (origin, explicit) in [
        ("--from", from),
        ("GPY_SHARE_DIR", env.share_dir.as_deref()),
    ] {
        if let Some(dir) = explicit {
            return if has_shell_tree(dir, shell) {
                Ok(dir.to_path_buf())
            } else {
                Err(Error::invalid(format!(
                    "{origin} {} has no {shell} tree (expected {}/{shell}/{})",
                    dir.display(),
                    dir.display(),
                    entry_point(shell)
                )))
            };
        }
    }

    let mut candidates = Vec::new();
    if let Some(exe_dir) = env.exe.as_deref().and_then(Path::parent) {
        // Homebrew and release archives: <prefix>/bin/gpy beside <prefix>/share/gpy.
        candidates.push(exe_dir.join("../share/gpy"));
        // A source checkout: .../target/<profile>/gpy inside the repository.
        candidates.extend(exe_dir.ancestors().map(Path::to_path_buf));
    }
    candidates
        .iter()
        .find(|dir| has_shell_tree(dir, shell))
        .map(|dir| fs::canonicalize(dir).unwrap_or_else(|_| dir.clone()))
        .ok_or_else(|| {
            Error::invalid(format!(
                "cannot find GPY's {shell} files next to this binary. \
                 Point setup at a checkout of the repository with `gpy setup --from <dir>` \
                 (the directory containing fish/, zsh/ and bash/), \
                 or use the curl installer, which downloads them."
            ))
        })
}

/// Reject a path that could break out of the rc block's double quotes. One
/// check for all three shells is simpler than escaping three ways.
pub(super) fn check_sourceable(path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    if text.contains(['"', '$', '`', '\\', '\n']) {
        return Err(Error::invalid(format!(
            "{text} contains a quote, $, backtick, backslash or newline, which cannot be \
             sourced safely from a shell rc file; use an XDG_CONFIG_HOME without those characters"
        )));
    }
    Ok(())
}

/// Check the install locations, copy the shell tree and completions, then
/// wire the rc file.
fn install(
    shell: Shell,
    source_root: &Path,
    env: &Environment,
    completions: &impl Fn(Shell) -> Result<Vec<u8>>,
) -> Result<Outcome> {
    let config_home = env.config_home()?;
    preflight(shell, source_root, &config_home)?;
    let shell_dir = install_files(shell, source_root, &config_home, completions)?;
    let plugins_dir = install_bundled_plugins(source_root, &config_home)?;

    let (rc_file, rc_block_added) = match shell {
        Shell::Fish => wire_fish(&config_home, &shell_dir)?,
        Shell::Zsh => wire_zsh(env, &shell_dir)?,
        Shell::Bash => wire_bash(env, &shell_dir)?,
    };
    Ok(Outcome {
        shell_dir,
        rc_file,
        rc_block_added,
        plugins_dir,
    })
}

/// Where `gpy setup` installs `shell`'s scripts under `config_home`.
pub(super) fn shell_dir_for(config_home: &Path, shell: Shell) -> PathBuf {
    match shell {
        Shell::Fish => config_home.join("fish").join("gpy"),
        Shell::Zsh | Shell::Bash => config_home.join("gpy").join(shell.as_str()),
    }
}

/// Wire Fish: rc block in `config.fish` plus the `fish_prompt` symlink.
fn wire_fish(config_home: &Path, shell_dir: &Path) -> Result<(PathBuf, bool)> {
    let fish_dir = config_home.join("fish");
    let config = fish_dir.join("config.fish");
    let body = format!(
        "if status is-interactive\n    source \"{}\"\nend",
        shell_dir.join("conf.d/gpy_init.fish").display()
    );
    let added = append_block(&config, RC_BLOCK_HEADER, &body)?;
    link_fish_prompt(
        &fish_dir.join("functions/fish_prompt.fish"),
        &shell_dir.join("functions/fish_prompt.fish"),
    )?;
    Ok((config, added))
}

/// Make `target` a symlink to `source`. An old regular-file copy of GPY's prompt is replaced; anything else is moved aside first.
#[cfg(unix)]
fn link_fish_prompt(target: &Path, source: &Path) -> Result<()> {
    if fs::read_link(target).is_ok_and(|current| current == source) {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Ok(meta) = target.symlink_metadata() {
        if meta.is_file() && is_gpy_prompt_copy(target) {
            fs::remove_file(target)?;
        } else {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            let mut backup = target.as_os_str().to_owned();
            backup.push(format!(
                ".backup.{}",
                super::backup_stamp::backup_stamp(now)
            ));
            fs::rename(target, &backup)?;
            crate::commands::output::plain(format_args!(
                "Moved your existing fish_prompt to {}",
                PathBuf::from(backup).display()
            ));
        }
    }
    std::os::unix::fs::symlink(source, target)?;
    Ok(())
}

/// A regular-file copy of GPY's own prompt: both segment-dispatch lines every
/// GPY prompt shipped with (the `scripts/uninstall.*` rule).
#[cfg(unix)]
fn is_gpy_prompt_copy(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| {
        text.lines()
            .any(|l| l.trim_ascii() == "for segment in $__enabled_segments")
            && text
                .lines()
                .any(|l| l.trim_ascii() == "if functions -q segment_{$segment}_render")
    })
}

#[cfg(not(unix))]
fn link_fish_prompt(_target: &Path, _source: &Path) -> Result<()> {
    Err(Error::invalid("shell integration needs a Unix shell"))
}

/// Wire Zsh: rc block in `$ZDOTDIR/.zshrc` or `~/.zshrc`.
fn wire_zsh(env: &Environment, shell_dir: &Path) -> Result<(PathBuf, bool)> {
    let rc = env
        .zdotdir
        .as_deref()
        .map_or_else(|| env.home(), Ok)?
        .join(".zshrc");
    let body = format!("source \"{}\"", shell_dir.join("gpy.zsh").display());
    let added = append_block(&rc, RC_BLOCK_HEADER, &body)?;
    Ok((rc, added))
}

/// The `.bashrc` block body: GPY loads only in interactive shells.
fn bash_rc_body(gpy_bash: &Path) -> String {
    format!(
        "case $- in\n    *i*) source \"{}\" ;;\nesac",
        gpy_bash.display()
    )
}

/// `.bashrc` block bodies earlier releases wrote for this install.
fn legacy_bash_rc_bodies(gpy_bash: &Path) -> Vec<String> {
    vec![format!("source \"{}\"", gpy_bash.display())]
}

/// Login-file loader body; inert outside Bash.
const BASH_LOGIN_LOADER: &str =
    "[ -n \"${BASH_VERSION:-}\" ] && [ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"";
/// Login-file loader body earlier releases wrote.
const LEGACY_BASH_LOGIN_LOADER: &str = "[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"";
/// Header line of the login-file loader block.
const BASH_LOGIN_HEADER: &str = "GPY: load ~/.bashrc in login shells";

/// Run [`ensure_block`] on an rc file and report the outcome; `true` only when
/// a block was added. Failing to rewrite an existing block is a warning, since
/// the old block keeps working.
fn ensure_and_report(
    rc: &Path,
    header: &str,
    body: &str,
    legacy_bodies: &[String],
    upgraded_note: &str,
) -> Result<bool> {
    let had_block = has_block(rc);
    match ensure_block(rc, header, body, legacy_bodies) {
        Ok(BlockChange::Added) => Ok(true),
        Ok(BlockChange::Unchanged) => Ok(false),
        Ok(BlockChange::Upgraded) => {
            crate::commands::output::plain(format_args!(
                "Updated the GPY block in {} {upgraded_note}",
                rc.display()
            ));
            Ok(false)
        }
        Ok(BlockChange::KeptEdited) => {
            crate::commands::output::plain(format_args!(
                "GPY block in {} differs from the current text; left unchanged. See docs/INSTALL.md#gpy-setup",
                rc.display()
            ));
            Ok(false)
        }
        Err(err) if had_block => {
            eprintln!(
                "warn: could not update the GPY block in {}: {err}; it still works with the old behaviour",
                rc.display()
            );
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

/// Wire Bash: rc block in `~/.bashrc`, plus a login-shell loader.
fn wire_bash(env: &Environment, shell_dir: &Path) -> Result<(PathBuf, bool)> {
    let home = env.home()?;
    let rc = home.join(".bashrc");
    let gpy_bash = shell_dir.join("gpy.bash");
    let added = ensure_and_report(
        &rc,
        RC_BLOCK_HEADER,
        &bash_rc_body(&gpy_bash),
        &legacy_bash_rc_bodies(&gpy_bash),
        "so it loads only in interactive shells",
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
    let loads_bashrc = fs::read_to_string(&login).is_ok_and(|text| sources_bashrc(&text));
    if !loads_bashrc
        && ensure_block(&login, BASH_LOGIN_HEADER, BASH_LOGIN_LOADER, &[])? == BlockChange::Added
    {
        crate::commands::output::plain(format_args!(
            "Added ~/.bashrc loader to {}",
            login.display()
        ));
    }
    Ok((rc, added))
}

/// Whether a startup file runs `~/.bashrc`: a `.`/`source` command, outside a
/// comment, whose argument is a `.bashrc` path. A mention alone (a comment, a
/// commented-out loader, `echo ~/.bashrc`) does not load it.
fn sources_bashrc(text: &str) -> bool {
    text.lines().any(|line| {
        let words = shell_words(line);
        words.windows(2).enumerate().any(|(index, pair)| {
            let in_command_position = index
                .checked_sub(1)
                .and_then(|previous| words.get(previous))
                .is_none_or(|word| {
                    matches!(
                        *word,
                        ";" | "&" | "|" | "(" | ")" | "{" | "then" | "else" | "do"
                    )
                });
            let &[command, argument] = pair else {
                return false;
            };
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

/// Whether this is Apple's frozen `/bin/bash` 3.2 (decided from the path alone).
fn is_stock_macos_bash(shell: Shell, login_shell: Option<&str>, macos: bool) -> bool {
    macos && shell == Shell::Bash && login_shell == Some("/bin/bash")
}

fn print_next_steps(shell: Shell, rc_file: &Path, login_shell: Option<&str>) {
    crate::commands::output::blank();
    crate::commands::output::plain(format_args!(
        "Next: restart your shell with `exec {shell}`, or `source {}`.",
        rc_file.display()
    ));
    crate::commands::output::plain("Check the result any time with `gpy doctor`.");
    if is_stock_macos_bash(shell, login_shell, cfg!(target_os = "macos")) {
        crate::commands::output::blank();
        crate::commands::output::plain(
            "Note: your login shell is macOS's /bin/bash, version 3.2 (2007).",
        );
        crate::commands::output::plain("GPY works best on Bash 5: `brew install bash`.");
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn flag_beats_environment() {
        let shell = detect_shell(Some(Shell::Fish), Some("zsh"), Some("/bin/bash")).unwrap();
        assert_eq!(shell, Shell::Fish);
    }

    #[test]
    fn gpy_shell_beats_login_shell() {
        let shell = detect_shell(None, Some("zsh"), Some("/usr/bin/fish")).unwrap();
        assert_eq!(shell, Shell::Zsh);
    }

    #[test]
    fn login_shell_uses_basename() {
        let shell = detect_shell(None, None, Some("/opt/homebrew/bin/fish")).unwrap();
        assert_eq!(shell, Shell::Fish);
    }

    #[test]
    fn empty_gpy_shell_falls_through() {
        let shell = detect_shell(None, Some(""), Some("/bin/zsh")).unwrap();
        assert_eq!(shell, Shell::Zsh);
    }

    #[test]
    fn unsupported_login_shell_asks_for_flag() {
        let err = detect_shell(None, None, Some("/bin/tcsh")).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("tcsh") && message.contains("--shell"),
            "{message}"
        );
    }

    #[test]
    fn bad_gpy_shell_is_an_error_not_a_guess() {
        let err = detect_shell(None, Some("nushell"), Some("/bin/zsh")).unwrap_err();
        assert!(err.to_string().contains("nushell"));
    }

    #[test]
    fn only_the_stock_macos_bash_gets_the_old_bash_note() {
        assert!(is_stock_macos_bash(Shell::Bash, Some("/bin/bash"), true));
        assert!(!is_stock_macos_bash(
            Shell::Bash,
            Some("/opt/homebrew/bin/bash"),
            true
        ));
        assert!(!is_stock_macos_bash(Shell::Bash, Some("/bin/zsh"), true));
        assert!(!is_stock_macos_bash(Shell::Zsh, Some("/bin/bash"), true));
        assert!(!is_stock_macos_bash(Shell::Bash, Some("/bin/bash"), false));
    }

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

    #[test]
    fn explicit_source_without_the_tree_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment::default();
        let err = find_source_root(Some(dir.path()), &env, Shell::Zsh).unwrap_err();
        assert!(err.to_string().contains("--from"));
    }

    #[test]
    fn unsafe_config_paths_are_refused() {
        assert!(check_sourceable(Path::new("/opt/a b/.config")).is_ok());
        assert!(check_sourceable(Path::new("/opt/$USER/.config")).is_err());
        assert!(check_sourceable(Path::new("/opt/a\"b")).is_err());
    }

    #[cfg(unix)]
    fn prompt_fixture(dir: &Path) -> (PathBuf, PathBuf) {
        let target = dir.join("fish/functions/fish_prompt.fish");
        let source = dir.join("gpy/functions/fish_prompt.fish");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, "function fish_prompt; end\n").unwrap();
        (target, source)
    }

    #[cfg(unix)]
    fn prompt_backups(target: &Path) -> Vec<PathBuf> {
        fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("fish_prompt.fish.backup")
            })
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn link_fish_prompt_backup_uses_timestamped_name() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        let original = "function fish_prompt; echo mine; end\n";
        fs::write(&target, original).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        let backups = prompt_backups(&target);
        assert_eq!(backups.len(), 1);
        let name = backups
            .first()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let suffix = name.split(".backup.").nth(1).unwrap();
        assert_eq!(suffix.len(), 15);
        for (i, c) in suffix.chars().enumerate() {
            if i == 8 {
                assert_eq!(c, '_');
            } else {
                assert!(c.is_ascii_digit(), "{suffix}");
            }
        }
        assert_eq!(
            fs::read_to_string(backups.first().unwrap()).unwrap(),
            original
        );
    }

    #[cfg(unix)]
    #[test]
    fn link_fish_prompt_moves_foreign_symlink_aside() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        let dotfiles = dir.path().join("dotfiles/fish_prompt.fish");
        fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
        fs::write(&dotfiles, "function fish_prompt; end\n").unwrap();
        std::os::unix::fs::symlink(&dotfiles, &target).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        let backups = prompt_backups(&target);
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_link(backups.first().unwrap()).unwrap(), dotfiles);
        assert!(dotfiles.exists());
    }

    #[cfg(unix)]
    #[test]
    fn link_fish_prompt_backs_up_prompt_that_only_mentions_gpy() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        let original = "# Tried GPY once; kept my own. See __enabled_segments.\nfunction fish_prompt\n    printf \"mine> \"\nend\n";
        fs::write(&target, original).unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        let backups = prompt_backups(&target);
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read(backups.first().unwrap()).unwrap(),
            original.as_bytes()
        );
    }

    #[cfg(unix)]
    #[test]
    fn link_fish_prompt_replaces_old_gpy_copy_without_backup() {
        let dir = tempfile::tempdir().unwrap();
        let (target, source) = prompt_fixture(dir.path());
        fs::write(
            &target,
            "function fish_prompt\n    for segment in $__enabled_segments\n        if functions -q segment_{$segment}_render\n            segment_{$segment}_render\n        end\n    end\nend\n",
        )
        .unwrap();
        link_fish_prompt(&target, &source).unwrap();
        assert_eq!(fs::read_link(&target).unwrap(), source);
        assert_eq!(prompt_backups(&target), Vec::<PathBuf>::new());
    }
}
