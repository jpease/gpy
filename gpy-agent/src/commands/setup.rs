//! `gpy setup`: install the per-shell scripts and wire them into the rc file.
//!
//! Installs the release layout (`fish/`, `bash/`, `zsh/`, `plugins/`) under
//! the data root, `$XDG_DATA_HOME/gpy`, and wires only the chosen shell's rc
//! file. The `gpy-init` marker lines (see `rc_block`) are the contract
//! `gpy uninstall` removes by. Every step is idempotent.

use super::rc_block::{BlockChange, BlockState, block_state, ensure_block, has_block};
use super::setup_fish::{leftover_trees, link_fish_prompt, report_leftovers, retire_fish_loader};
use super::setup_install::{InstallMode, install_bundled_plugins, install_files, preflight};
use crate::commands::exit_codes::SETUP_INCOMPLETE;
use crate::commands::init::{self, InitOptions};
use crate::paths::{self, Os};
use crate::shell::Shell;
use crate::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Comment on the line after the block's start marker that says what the block is for.
pub(super) const RC_BLOCK_HEADER: &str = "GPY shell integration";
/// File in each installed shell tree naming the `gpy` version that wrote it,
/// so `gpy doctor` can spot a tree an upgrade left behind.
pub(super) const VERSION_STAMP: &str = ".gpy-version";
/// Every shell `gpy setup` can install.
pub(super) const SHELLS: [Shell; 3] = [Shell::Fish, Shell::Zsh, Shell::Bash];

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
    /// Validate the source tree and install path, then stop without writing.
    pub check: bool,
}

/// The process environment `gpy setup` reads, gathered once for testability.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// Absolute home directory.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, when set.
    pub xdg_config_home: Option<PathBuf>,
    /// `XDG_DATA_HOME`, when set.
    pub xdg_data_home: Option<PathBuf>,
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
            xdg_data_home: paths::xdg_data_home(),
            zdotdir: non_empty("ZDOTDIR").map(PathBuf::from),
            login_shell: std::env::var("SHELL").ok(),
            gpy_shell: crate::env_config::SHELL.text(),
            share_dir: crate::env_config::SHARE_DIR.path(),
            exe: std::env::current_exe()
                .ok()
                .map(|exe| fs::canonicalize(&exe).unwrap_or(exe)),
        }
    }

    /// `$XDG_CONFIG_HOME`, or `$HOME/.config`; fails without an absolute home.
    pub(super) fn config_home(&self) -> Result<PathBuf> {
        let home = self.home.as_deref().ok_or_else(no_home)?;
        // `config_root_for` returns `<config home>/gpy`.
        paths::config_root_for(self.xdg_config_home.as_deref(), Some(home), Os::Unix)
            .and_then(|root| root.parent().map(Path::to_path_buf))
            .ok_or_else(|| Error::invalid("cannot resolve the config directory"))
    }

    /// The data root, `$XDG_DATA_HOME/gpy` or `~/.local/share/gpy`, where the
    /// shell trees and bundled plugins are installed.
    pub(super) fn data_root(&self) -> Result<DataRoot> {
        let (xdg, home) = (self.xdg_data_home.as_deref(), self.home.as_deref());
        paths::data_root_for(xdg, home, None, Os::Unix)
            .map(DataRoot)
            .ok_or_else(no_home)
    }

    /// The home directory.
    pub(super) fn home(&self) -> Result<&Path> {
        self.home
            .as_deref()
            .ok_or_else(|| Error::invalid("cannot find your home directory"))
    }
}

/// The error for a missing or relative `HOME`.
fn no_home() -> Error {
    Error::invalid("cannot find your home directory (HOME is unset or not an absolute path)")
}

/// The directory `gpy setup` installs the shell trees and bundled plugins into.
#[derive(Debug, Clone)]
pub(super) struct DataRoot(pub(super) PathBuf);

/// Everything `setup` did, for the closing report.
struct Outcome {
    shell_dir: PathBuf,
    rc_file: PathBuf,
    rc_change: BlockChange,
    /// Where the bundled plugins went; `None` for a source without them.
    plugins_dir: Option<PathBuf>,
    /// Directories of the pre-0.9 layout still on disk.
    leftovers: Vec<PathBuf>,
}

/// Run `gpy setup`. `completions` renders the structural completion script,
/// which only the binary can generate.
///
/// Every shell tree in the source is installed, so an upgrade run for one
/// shell leaves the others on current scripts too.
///
/// Returns [`SETUP_INCOMPLETE`] when the rc file holds an incomplete GPY
/// block: everything else is done, but the shell does not load GPY.
///
/// # Errors
/// Fails when no shell can be determined, the scripts cannot be found, or a
/// file cannot be written. The rc file is touched last.
pub fn run(
    options: &SetupOptions,
    completions: impl Fn(Shell) -> Result<Vec<u8>>,
) -> Result<ExitCode> {
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
    if options.check {
        let shell_dir = shell_dir_for(&env.data_root()?, shell);
        check_sourceable(&shell_dir)?;
        crate::commands::output::plain(format_args!(
            "ok: {shell} files in {} can be installed to {}",
            source_root.display(),
            shell_dir.display()
        ));
        return Ok(ExitCode::SUCCESS);
    }
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
    match outcome.rc_change {
        BlockChange::Added => crate::commands::output::plain(format_args!(
            "Added GPY initialization to {}",
            outcome.rc_file.display()
        )),
        // The warning for an incomplete block was already printed.
        BlockChange::Incomplete => {}
        BlockChange::Unchanged | BlockChange::Upgraded | BlockChange::KeptEdited => {
            crate::commands::output::plain(format_args!(
                "GPY already configured in {}",
                outcome.rc_file.display()
            ));
        }
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
    report_leftovers(&outcome.leftovers);
    if outcome.rc_change == BlockChange::Incomplete {
        return Ok(ExitCode::from(SETUP_INCOMPLETE));
    }
    Ok(ExitCode::SUCCESS)
}

/// One installed shell integration, as `gpy doctor` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledIntegration {
    /// The shell.
    pub shell: Shell,
    /// Where its scripts are installed: the tree the rc block sources, or the
    /// current data root's when the block names none.
    pub dir: PathBuf,
    /// The `gpy` version that installed them; `None` for a tree written
    /// before setup stamped versions.
    pub version: Option<String>,
    /// The rc file holding the GPY block.
    pub rc: PathBuf,
    /// The file the rc block sources; `None` when the block is incomplete or
    /// names no file.
    pub entry: Option<PathBuf>,
    /// The rc file has a start marker but no complete block, so the shell is
    /// not wired.
    pub incomplete: bool,
}

/// The shell integrations `gpy setup` has wired for the current user: the
/// shells whose rc file holds the GPY block.
#[must_use]
pub fn installed_integrations() -> Vec<InstalledIntegration> {
    installed_integrations_in(&Environment::from_process())
}

/// [`installed_integrations`] under an explicit environment.
fn installed_integrations_in(env: &Environment) -> Vec<InstalledIntegration> {
    let Ok(root) = env.data_root() else {
        return Vec::new();
    };
    SHELLS
        .into_iter()
        .filter_map(|shell| {
            let rc = rc_file_for(env, shell).ok()?;
            let mut incomplete = false;
            let entry = match block_state(&rc) {
                BlockState::Missing => return None,
                BlockState::Incomplete => {
                    incomplete = true;
                    None
                }
                BlockState::Present(body) => sourced_entry(&body),
            };
            let dir = entry.as_deref().map_or_else(
                || shell_dir_for(&root, shell),
                |sourced| tree_dir_of(shell, sourced),
            );
            let version = fs::read_to_string(dir.join(VERSION_STAMP))
                .ok()
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty());
            Some(InstalledIntegration {
                shell,
                dir,
                version,
                rc,
                entry,
                incomplete,
            })
        })
        .collect()
}

/// The file an rc block body sources: the first `source "<path>"`.
fn sourced_entry(body: &str) -> Option<PathBuf> {
    let (_, after) = body.split_once("source \"")?;
    let (path, _) = after.split_once('"')?;
    Some(PathBuf::from(path))
}

/// The shell tree a sourced `entry` file belongs to: Fish's entry sits in
/// `conf.d/`, Zsh's and Bash's at the top.
fn tree_dir_of(shell: Shell, entry: &Path) -> PathBuf {
    let parent = entry.parent().unwrap_or(entry);
    if shell == Shell::Fish {
        parent.parent().unwrap_or(parent).to_path_buf()
    } else {
        parent.to_path_buf()
    }
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

/// Every file `root/<shell>/` must hold for `gpy setup` to install `shell`.
/// The one list `has_shell_tree` and `gpy setup --check` both use.
const fn required_files(shell: Shell) -> &'static [&'static str] {
    match shell {
        Shell::Fish => &[
            "conf.d/gpy_init.fish",
            "functions/fish_prompt.fish",
            "core/init.fish",
            "core/flags.fish",
            "completions/gpy-dynamic.fish",
        ],
        Shell::Zsh => &["gpy.zsh", "core/init.zsh"],
        Shell::Bash => &["gpy.bash", "core/init.bash"],
    }
}

/// The script runner Fish and Zsh find beside their tree, relative to the root.
const RUNNER_SIBLING: &str = "bash/core/g_runner.bash";

/// The first file `root` lacks for `shell`, relative to `root`; `None` when
/// the tree is complete.
fn first_missing_file(root: &Path, shell: Shell) -> Option<String> {
    let shell_name = shell.as_str();
    let own = required_files(shell)
        .iter()
        .map(|file| format!("{shell_name}/{file}"));
    let runner = matches!(shell, Shell::Fish | Shell::Zsh).then(|| RUNNER_SIBLING.to_owned());
    own.chain(runner)
        .find(|relative| !root.join(relative).is_file())
}

fn has_shell_tree(root: &Path, shell: Shell) -> bool {
    first_missing_file(root, shell).is_none()
}

/// Validate an explicitly requested source root.
fn explicit_root(origin: &str, dir: &Path, shell: Shell) -> Result<PathBuf> {
    first_missing_file(dir, shell).map_or_else(
        || Ok(dir.to_path_buf()),
        |missing| {
            Err(Error::invalid(format!(
                "{origin} {} has no {missing}",
                dir.display()
            )))
        },
    )
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
        let Some(dir) = explicit else {
            continue;
        };
        return explicit_root(origin, dir, shell);
    }

    let mut candidates = Vec::new();
    if let Some(exe_dir) = env.exe.as_deref().and_then(Path::parent) {
        // Homebrew and release archives: <prefix>/bin/gpy beside <prefix>/share/gpy.
        candidates.push(exe_dir.join("../share/gpy"));
        // A source checkout: .../target/<profile>/gpy inside the repository.
        candidates.extend(exe_dir.ancestors().map(Path::to_path_buf));
    }
    // What an earlier `gpy setup` installed: last, so a checkout build never
    // prefers a stale copy.
    candidates.extend(env.data_root().ok().map(|root| root.0));
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
             sourced safely from a shell rc file; use an XDG_DATA_HOME without those characters"
        )));
    }
    Ok(())
}

/// Install the trees and plugins into the data root, then wire the rc file
/// of `shell` and collect what an older layout left behind.
///
/// Every tree the source ships is installed; completions are generated for
/// `shell` and for each other shell whose rc file already has the GPY block.
/// A failure on another shell is a warning.
fn install(
    shell: Shell,
    source_root: &Path,
    env: &Environment,
    completions: &impl Fn(Shell) -> Result<Vec<u8>>,
) -> Result<Outcome> {
    let config_home = env.config_home()?;
    let root = env.data_root()?;
    let mode = if is_in_place(source_root, &root) {
        InstallMode::InPlace
    } else {
        InstallMode::Copy
    };
    let wired =
        |other: Shell| other == shell || rc_file_for(env, other).is_ok_and(|rc| has_block(&rc));
    let others: Vec<Shell> = SHELLS
        .into_iter()
        .filter(|&other| other != shell && source_root.join(other.as_str()).is_dir())
        .collect();
    // Only the chosen shell's locations gate setup; the others are guarded as
    // each is installed, and a refusal there is a warning.
    if mode == InstallMode::Copy {
        preflight(
            &[shell],
            shell == Shell::Fish,
            source_root,
            &root,
            &config_home,
        )?;
    }
    let shell_dir = install_files(
        shell,
        source_root,
        &root,
        &config_home,
        mode,
        true,
        completions,
    )?;
    for other in others {
        if let Err(err) = install_files(
            other,
            source_root,
            &root,
            &config_home,
            mode,
            wired(other),
            completions,
        ) {
            crate::commands::output::warn(format_args!(
                "Could not install the {other} files ({}); run `gpy setup --shell {other}`",
                crate::error::user_message(&err)
            ));
        }
    }
    let plugins_dir = install_bundled_plugins(source_root, &root, mode)?;

    let (rc_file, rc_change) = match shell {
        Shell::Fish => wire_fish(env, &config_home, &shell_dir)?,
        Shell::Zsh => wire_zsh(env, &shell_dir)?,
        Shell::Bash => wire_bash(env, &shell_dir)?,
    };
    Ok(Outcome {
        shell_dir,
        rc_file,
        rc_change,
        plugins_dir,
        leftovers: leftover_trees(&config_home, &root.0),
    })
}

/// Whether the source is the data root itself, so nothing needs copying.
fn is_in_place(source_root: &Path, root: &DataRoot) -> bool {
    root.0.exists() && fs::canonicalize(source_root).ok() == fs::canonicalize(&root.0).ok()
}

/// Where `gpy setup` installs `shell`'s scripts under the data root.
pub(super) fn shell_dir_for(root: &DataRoot, shell: Shell) -> PathBuf {
    root.0.join(shell.as_str())
}

/// The rc file `gpy setup` wires for `shell`.
pub(super) fn rc_file_for(env: &Environment, shell: Shell) -> Result<PathBuf> {
    Ok(match shell {
        Shell::Fish => env.config_home()?.join("fish").join("config.fish"),
        Shell::Zsh => env
            .zdotdir
            .as_deref()
            .map_or_else(|| env.home(), Ok)?
            .join(".zshrc"),
        Shell::Bash => env.home()?.join(".bashrc"),
    })
}

/// The Fish rc block body sourcing the tree at `shell_dir`.
pub(super) fn fish_rc_body(shell_dir: &Path) -> String {
    format!(
        "if status is-interactive\n    source \"{}\"\nend",
        shell_dir.join("conf.d/gpy_init.fish").display()
    )
}

/// `config.fish` block bodies releases before 0.9 wrote, which source the tree
/// in `~/.config/fish/gpy` that `gpy setup` no longer keeps. Some carried a
/// `# GPY Prompt Enhancement` comment as their first body line.
pub(super) fn legacy_fish_rc_bodies(config_home: &Path) -> Vec<String> {
    let old_init = config_home.join("fish/gpy/conf.d/gpy_init.fish");
    let body = format!(
        "if status is-interactive\n    source \"{}\"\nend",
        old_init.display()
    );
    vec![format!("# GPY Prompt Enhancement\n{body}"), body]
}

/// The Zsh rc block body sourcing the tree at `shell_dir`.
pub(super) fn zsh_rc_body(shell_dir: &Path) -> String {
    format!("source \"{}\"", shell_dir.join("gpy.zsh").display())
}

/// Wire Fish: rc block in `config.fish`, the `fish_prompt` symlink, and
/// retire the autoloaded loader of the pre-0.9 layout.
fn wire_fish(
    env: &Environment,
    config_home: &Path,
    shell_dir: &Path,
) -> Result<(PathBuf, BlockChange)> {
    let config = rc_file_for(env, Shell::Fish)?;
    let change = ensure_and_report(
        &config,
        RC_BLOCK_HEADER,
        &fish_rc_body(shell_dir),
        &legacy_fish_rc_bodies(config_home),
        "so it loads the files in the data directory",
    )?;
    let fish_dir = config_home.join("fish");
    link_fish_prompt(
        &fish_dir.join("functions/fish_prompt.fish"),
        &shell_dir.join("functions/fish_prompt.fish"),
    )?;
    if change != BlockChange::Incomplete {
        retire_fish_loader(config_home)?;
    }
    Ok((config, change))
}

/// Wire Zsh: rc block in `$ZDOTDIR/.zshrc` or `~/.zshrc`.
fn wire_zsh(env: &Environment, shell_dir: &Path) -> Result<(PathBuf, BlockChange)> {
    let rc = rc_file_for(env, Shell::Zsh)?;
    let change = ensure_and_report(
        &rc,
        RC_BLOCK_HEADER,
        &zsh_rc_body(shell_dir),
        &[],
        "so it loads the files in the data directory",
    )?;
    Ok((rc, change))
}

/// The `.bashrc` block body: GPY loads only in interactive shells.
pub(super) fn bash_rc_body(gpy_bash: &Path) -> String {
    format!(
        "case $- in\n    *i*) source \"{}\" ;;\nesac",
        gpy_bash.display()
    )
}

/// `.bashrc` block bodies earlier releases wrote for this install.
pub(super) fn legacy_bash_rc_bodies(gpy_bash: &Path) -> Vec<String> {
    vec![format!("source \"{}\"", gpy_bash.display())]
}

/// Login-file loader body; inert outside Bash.
const BASH_LOGIN_LOADER: &str =
    "[ -n \"${BASH_VERSION:-}\" ] && [ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"";
/// Login-file loader body earlier releases wrote.
const LEGACY_BASH_LOGIN_LOADER: &str = "[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"";
/// Header line of the login-file loader block.
const BASH_LOGIN_HEADER: &str = "GPY: load ~/.bashrc in login shells";

/// Run [`ensure_block`] on an rc file and report the outcome. Failing to
/// rewrite an existing block is a warning, since the old block keeps working;
/// an incomplete block is a warning too, since the shell is not wired.
pub(super) fn ensure_and_report(
    rc: &Path,
    header: &str,
    body: &str,
    legacy_bodies: &[String],
    upgraded_note: &str,
) -> Result<BlockChange> {
    let had_block = has_block(rc);
    let change = match ensure_block(rc, header, body, legacy_bodies) {
        Ok(change) => change,
        Err(err) if had_block => {
            eprintln!(
                "warn: could not update the GPY block in {}: {err}; it still works with the old behaviour",
                rc.display()
            );
            return Ok(BlockChange::Unchanged);
        }
        Err(err) => return Err(err),
    };
    match change {
        BlockChange::Added | BlockChange::Unchanged => {}
        BlockChange::Upgraded => {
            crate::commands::output::plain(format_args!(
                "Updated the GPY block in {} {upgraded_note}",
                rc.display()
            ));
        }
        BlockChange::KeptEdited => {
            crate::commands::output::plain(format_args!(
                "GPY block in {} differs from the current text; left unchanged. See docs/INSTALL.md#gpy-setup",
                rc.display()
            ));
        }
        BlockChange::Incomplete => {
            crate::commands::output::warn_stderr(format_args!(
                "the GPY block in {} is incomplete (its '# <<< gpy-init <<<' end line or '# ' \
                 header line is missing), so GPY is not wired. Delete the lines from \
                 '# >>> gpy-init >>>' on that GPY added, then run `gpy setup` again",
                rc.display()
            ));
        }
    }
    Ok(change)
}

/// Wire Bash: rc block in `~/.bashrc`, plus a login-shell loader.
fn wire_bash(env: &Environment, shell_dir: &Path) -> Result<(PathBuf, BlockChange)> {
    let home = env.home()?;
    let rc = rc_file_for(env, Shell::Bash)?;
    let gpy_bash = shell_dir.join("gpy.bash");
    let legacy_bodies = legacy_bash_rc_bodies(&gpy_bash);
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

/// Whether this is Apple's frozen `/bin/bash` 3.2 (decided from the path alone).
fn is_stock_macos_bash(shell: Shell, login_shell: Option<&str>, macos: bool) -> bool {
    macos && shell == Shell::Bash && login_shell == Some("/bin/bash")
}

fn print_next_steps(shell: Shell, rc_file: &Path, login_shell: Option<&str>) {
    crate::commands::output::blank();
    crate::commands::output::plain(format_args!(
        "Next: restart your shell with `exec {shell}`, or `source {}`.",
        super::utils::shell_quote(rc_file)
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
    #[test]
    fn installed_integrations_follow_the_path_the_rc_block_sources() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = home.path().join("elsewhere/zsh");
        fs::write(
            home.path().join(".zshrc"),
            format!(
                "# >>> gpy-init >>>\n# {RC_BLOCK_HEADER}\n{}\n# <<< gpy-init <<<\n",
                zsh_rc_body(&elsewhere)
            ),
        )
        .unwrap();
        fs::write(
            home.path().join(".bashrc"),
            "# >>> gpy-init >>>\n# GPY shell integration\n",
        )
        .unwrap();
        let env = Environment {
            home: Some(home.path().to_path_buf()),
            ..Environment::default()
        };

        let found = installed_integrations_in(&env);

        let zsh = found.iter().find(|item| item.shell == Shell::Zsh).unwrap();
        assert_eq!(
            zsh.entry.as_deref(),
            Some(elsewhere.join("gpy.zsh").as_path())
        );
        assert_eq!(zsh.dir, elsewhere);
        assert!(!zsh.incomplete);
        let bash = found.iter().find(|item| item.shell == Shell::Bash).unwrap();
        assert!(bash.incomplete && bash.entry.is_none());
        assert!(found.iter().all(|item| item.shell != Shell::Fish));
    }
}
