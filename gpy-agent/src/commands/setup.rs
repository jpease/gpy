//! `gpy setup`: install the per-shell scripts and wire them into the rc file.
//!
//! Lays files out exactly as `install-oneline.sh` does (Fish:
//! `$XDG_CONFIG_HOME/fish/gpy`; Zsh/Bash: `$XDG_CONFIG_HOME/gpy/<shell>`) so
//! both installers share one uninstall path. The `gpy-init` marker lines are
//! the contract `scripts/uninstall.*` remove by. Every step is idempotent.

use crate::commands::init::{self, InitOptions};
use crate::paths::{self, Os};
use crate::shell::Shell;
use crate::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// First line of the rc-file block; `scripts/uninstall.*` match on it.
const BLOCK_START: &str = "# >>> gpy-init >>>";
/// Last line of the rc-file block.
const BLOCK_END: &str = "# <<< gpy-init <<<";
/// Tag in the block header for an rc file `gpy setup` itself created, so the
/// uninstaller may delete it again.
const CREATED_NOTE: &str = " (gpy-created-file)";
/// File in each installed shell tree naming the `gpy` version that wrote it,
/// so `gpy doctor` can spot a tree an upgrade left behind.
const VERSION_STAMP: &str = ".gpy-version";
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
    pub xdg_config_home: Option<String>,
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
            home: paths::home_dir().map(PathBuf::from),
            xdg_config_home: paths::root_var("XDG_CONFIG_HOME"),
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
        let home = self.home.as_deref().and_then(Path::to_str);
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
    println!("Setting up GPY for {shell} from {}", source_root.display());

    let outcome = install(shell, &source_root, &env, &completions)?;

    println!("Installed {shell} files to {}", outcome.shell_dir.display());
    if outcome.rc_block_added {
        println!("Added GPY initialization to {}", outcome.rc_file.display());
    } else {
        println!("GPY already configured in {}", outcome.rc_file.display());
    }
    refresh_other_shells(shell, &source_root, &env, &completions);
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
            Ok(dir) => println!(
                "Refreshed {shell} files in {} (set up earlier)",
                dir.display()
            ),
            Err(err) => crate::commands::output::warn(format_args!(
                "Could not refresh the {shell} files in {} ({err}); run `gpy setup --shell {shell}`",
                shell_dir.display()
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
                "could not find GPY's {shell} files next to this binary. \
                 Point setup at a checkout of the repository with `gpy setup --from <dir>` \
                 (the directory containing fish/, zsh/ and bash/), \
                 or use the curl installer, which downloads them."
            ))
        })
}

/// Reject a path that could break out of the rc block's double quotes. One
/// check for all three shells is simpler than escaping three ways.
fn check_sourceable(path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    if text.contains(['"', '$', '`', '\\', '\n']) {
        return Err(Error::invalid(format!(
            "{text} contains a quote, $, backtick, backslash or newline, which cannot be \
             sourced safely from a shell rc file; use an XDG_CONFIG_HOME without those characters"
        )));
    }
    Ok(())
}

/// Copy the shell tree and completions, then wire the rc file.
fn install(
    shell: Shell,
    source_root: &Path,
    env: &Environment,
    completions: &impl Fn(Shell) -> Result<Vec<u8>>,
) -> Result<Outcome> {
    let config_home = env.config_home()?;
    let shell_dir = install_files(shell, source_root, &config_home, completions)?;

    let (rc_file, rc_block_added) = match shell {
        Shell::Fish => wire_fish(&config_home, &shell_dir)?,
        Shell::Zsh => wire_zsh(env, &shell_dir)?,
        Shell::Bash => wire_bash(env, &shell_dir)?,
    };
    Ok(Outcome {
        shell_dir,
        rc_file,
        rc_block_added,
    })
}

/// Where `gpy setup` installs `shell`'s scripts under `config_home`.
fn shell_dir_for(config_home: &Path, shell: Shell) -> PathBuf {
    match shell {
        Shell::Fish => config_home.join("fish").join("gpy"),
        Shell::Zsh | Shell::Bash => config_home.join("gpy").join(shell.as_str()),
    }
}

/// Copy `shell`'s tree and completions from `source_root` and stamp the
/// version; returns the installed tree. Leaves the rc file alone.
fn install_files(
    shell: Shell,
    source_root: &Path,
    config_home: &Path,
    completions: &impl Fn(Shell) -> Result<Vec<u8>>,
) -> Result<PathBuf> {
    let shell_dir = shell_dir_for(config_home, shell);
    check_sourceable(&shell_dir)?;

    let source = source_root.join(shell.as_str());
    // Fish's completion glue is merged into the generated file below.
    let skip: &[&str] = if shell == Shell::Fish {
        &["completions"]
    } else {
        &[]
    };
    copy_tree(&source, &shell_dir, skip)?;

    let generated = completions(shell)?;
    match shell {
        Shell::Fish => {
            let dir = config_home.join("fish").join("completions");
            fs::create_dir_all(&dir)?;
            // Never autoloaded as its own file.
            let _ = fs::remove_file(dir.join("gpy-dynamic.fish"));
            let mut script = generated;
            if let Ok(glue) = fs::read(source.join("completions/gpy-dynamic.fish")) {
                script.extend_from_slice(&glue);
            }
            write_replacing(&dir.join("gpy.fish"), &script)?;
        }
        Shell::Zsh => write_replacing(&shell_dir.join("completions/_gpy"), &generated)?,
        Shell::Bash => write_replacing(&shell_dir.join("completions/gpy.bash"), &generated)?,
    }
    write_replacing(
        &shell_dir.join(VERSION_STAMP),
        format!("{}\n", crate::VERSION).as_bytes(),
    )?;
    Ok(shell_dir)
}

/// Recursively copy `src` to `dst`, skipping top-level entries named in
/// `skip` and any symlink.
fn copy_tree(src: &Path, dst: &Path, skip: &[&str]) -> Result<()> {
    fs::create_dir_all(dst)?;
    for dirent in fs::read_dir(src)? {
        let entry = dirent?;
        let name = entry.file_name();
        if skip.iter().any(|skipped| name == *skipped) {
            continue;
        }
        let target = dst.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, &[])?;
        } else if kind.is_file() {
            write_replacing(&target, &fs::read(entry.path())?)?;
        }
    }
    Ok(())
}

/// Write `contents` to `path`, removing it first so a re-run can replace
/// files a package manager left read-only.
fn write_replacing(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    fs::write(path, contents)?;
    Ok(())
}

/// Append a `gpy-init` block to `rc`, creating it when missing; `false` when
/// a block is already there. A file this call creates gets [`CREATED_NOTE`].
fn append_block(rc: &Path, header: &str, body: &str) -> Result<bool> {
    use std::io::Write;

    let existing = match fs::read_to_string(rc) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };
    if existing
        .as_deref()
        .is_some_and(|text| text.lines().any(|line| line == BLOCK_START))
    {
        return Ok(false);
    }
    if let Some(parent) = rc.parent() {
        fs::create_dir_all(parent)?;
    }
    let note = if existing.is_none() { CREATED_NOTE } else { "" };
    let mut file = fs::OpenOptions::new().create(true).append(true).open(rc)?;
    write!(
        file,
        "\n{BLOCK_START}\n# {header}{note}\n{body}\n{BLOCK_END}\n"
    )?;
    Ok(true)
}

/// Wire Fish: rc block in `config.fish` plus the `fish_prompt` symlink.
fn wire_fish(config_home: &Path, shell_dir: &Path) -> Result<(PathBuf, bool)> {
    let fish_dir = config_home.join("fish");
    let config = fish_dir.join("config.fish");
    let body = format!(
        "if status is-interactive\n    source \"{}\"\nend",
        shell_dir.join("conf.d/gpy_init.fish").display()
    );
    let added = append_block(&config, "GPY Prompt Enhancement", &body)?;
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
            backup.push(format!(".backup.{}", backup_stamp(now)));
            fs::rename(target, &backup)?;
            println!(
                "Moved your existing fish_prompt to {}",
                PathBuf::from(backup).display()
            );
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

/// `YYYYMMDD_HHMMSS` (UTC), the suffix the installers write and
/// `scripts/uninstall.*` restore. Civil-from-days after Howard Hinnant.
#[cfg(unix)]
fn backup_stamp(epoch_secs: u64) -> String {
    let days = epoch_secs / 86_400;
    let secs_of_day = epoch_secs % 86_400;

    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);

    format!(
        "{year:04}{month:02}{day:02}_{:02}{:02}{:02}",
        secs_of_day / 3_600,
        secs_of_day % 3_600 / 60,
        secs_of_day % 60
    )
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
    let added = append_block(&rc, "GPY Prompt Enhancement", &body)?;
    Ok((rc, added))
}

/// Wire Bash: rc block in `~/.bashrc`, plus a login-shell loader.
fn wire_bash(env: &Environment, shell_dir: &Path) -> Result<(PathBuf, bool)> {
    let home = env.home()?;
    let rc = home.join(".bashrc");
    let body = format!("source \"{}\"", shell_dir.join("gpy.bash").display());
    let added = append_block(&rc, "GPY Prompt Enhancement", &body)?;

    // A login shell reads only the first of these. With none present, create
    // ~/.profile (~/.bash_profile would shadow a later ~/.profile).
    let login = [".bash_profile", ".bash_login", ".profile"]
        .iter()
        .map(|name| home.join(name))
        .find(|path| path.exists())
        .unwrap_or_else(|| home.join(".profile"));
    let loads_bashrc = fs::read_to_string(&login).is_ok_and(|text| sources_bashrc(&text));
    if !loads_bashrc
        && append_block(
            &login,
            "GPY: load ~/.bashrc in login shells",
            "[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"",
        )?
    {
        println!("Added ~/.bashrc loader to {}", login.display());
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
            let in_command_position = index == 0
                || matches!(
                    words[index - 1],
                    ";" | "&" | "|" | "(" | ")" | "{" | "then" | "else" | "do"
                );
            let path = pair[1].trim_matches(['"', '\'']);
            in_command_position
                && matches!(pair[0], "." | "source")
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
    println!();
    println!(
        "Next: restart your shell with `exec {shell}`, or `source {}`.",
        rc_file.display()
    );
    println!("Check the result any time with `gpy doctor`.");
    if is_stock_macos_bash(shell, login_shell, cfg!(target_os = "macos")) {
        println!();
        println!("Note: your login shell is macOS's /bin/bash, version 3.2 (2007).");
        println!("GPY works best on Bash 5: `brew install bash`.");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

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
    fn append_block_is_idempotent_and_tags_created_files() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");

        assert!(append_block(&rc, "hdr", "body").unwrap());
        assert!(!append_block(&rc, "hdr", "body").unwrap());

        let text = fs::read_to_string(&rc).unwrap();
        assert_eq!(text.matches(BLOCK_START).count(), 1);
        assert!(text.contains("# hdr (gpy-created-file)"), "{text}");
    }

    #[test]
    fn append_block_keeps_existing_content_and_skips_the_created_tag() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join("rc");
        fs::write(&rc, "export A=1\n").unwrap();

        assert!(append_block(&rc, "hdr", "body").unwrap());

        let text = fs::read_to_string(&rc).unwrap();
        assert!(text.starts_with("export A=1\n"));
        assert!(!text.contains("gpy-created-file"));
    }

    #[cfg(unix)]
    #[test]
    fn backup_stamp_matches_the_shell_installers_format() {
        // `date -u -r N +%Y%m%d_%H%M%S` for each of these.
        assert_eq!(backup_stamp(0), "19700101_000000");
        assert_eq!(backup_stamp(951_782_400), "20000229_000000");
        assert_eq!(backup_stamp(1_760_485_533), "20251014_234533");
        assert_eq!(backup_stamp(4_102_444_799), "20991231_235959");
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
        let name = backups[0]
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
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), original);
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
        assert_eq!(fs::read_link(&backups[0]).unwrap(), dotfiles);
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
        assert_eq!(fs::read(&backups[0]).unwrap(), original.as_bytes());
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
