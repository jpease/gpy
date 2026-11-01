//! `gpy uninstall`: remove what `gpy setup` and the installers create.
//!
//! Uses the path functions and the `rc_block` module `gpy setup` writes with,
//! so the two cannot drift apart. The configuration is kept unless asked for.

use super::lifecycle;
use super::output;
use super::rc_block::{has_start, remove_blocks};
use super::setup::{
    DataRoot, Environment, SHELLS, VERSION_STAMP, legacy_shell_dir_for, shell_dir_for,
};
use super::setup_install::{fish_completions_dir, plugins_target};
use crate::shell::Shell;
use crate::{Error, Result, paths};
use std::fs;
use std::io::{ErrorKind, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Marker in the command line of the shell supervisor loop.
#[cfg(unix)]
const SUPERVISOR_MARKER: &str = "__gpy_agent_supervisor_loop";

/// Options for [`run`], mirroring the `gpy uninstall` CLI flags.
#[derive(Debug, Clone, Default)]
pub struct UninstallOptions {
    /// Remove without asking; the only non-interactive path.
    pub yes: bool,
    /// Also delete the configuration directory.
    pub purge_config: bool,
}

/// What the running binary's location says about who owns the binaries.
enum Binaries {
    /// `~/.local/bin`, the one directory the installers write: ours to delete.
    Installer {
        bin_dir: PathBuf,
        share_dir: PathBuf,
    },
    /// Anywhere else: a package manager (or the user) owns them.
    Foreign {
        cellar: bool,
        /// A `share/gpy` beside the binary that `gpy setup` may have created.
        stray_share: Option<PathBuf>,
    },
}

/// Everything an uninstall will touch, gathered before anything is removed.
struct Plan {
    config_home: PathBuf,
    config_dir: PathBuf,
    data_root: DataRoot,
    /// Shell trees, completions, the fish conf.d entry and bundled plugins.
    integration: Vec<PathBuf>,
    /// rc files holding a GPY block.
    rc_files: Vec<PathBuf>,
    /// rc files that could not be read.
    unreadable: Vec<PathBuf>,
    binaries: Binaries,
    cache: Option<PathBuf>,
    runtime: PathBuf,
}

/// Paths that could not be handled, for the closing summary.
struct Tally {
    skipped_rc: Vec<PathBuf>,
    failed: Vec<PathBuf>,
}

impl Tally {
    /// Remove `path` (file, link or tree) and say so; remember a failure.
    fn remove(&mut self, path: &Path) {
        match remove_path(path) {
            Ok(true) => output::plain(format_args!("Removed {}", path.display())),
            Ok(false) => {}
            Err(err) => {
                output::warn_stderr(format_args!("could not remove {}: {err}", path.display()));
                self.failed.push(path.to_path_buf());
            }
        }
    }
}

/// Run `gpy uninstall`.
///
/// Exit code 0 when everything was removed; 1 when the user declined or some
/// rc file or path could not be handled.
///
/// # Errors
/// Fails without a home directory, when stdin is not a terminal and `--yes`
/// was not given, or when an rc file cannot be rewritten.
pub fn run(options: &UninstallOptions) -> Result<ExitCode> {
    if cfg!(not(unix)) {
        return Err(Error::invalid(
            "shell integration needs a Unix shell; on Windows, use GPY under WSL",
        ));
    }
    let env = Environment::from_process();
    let plan = build_plan(&env)?;
    print_plan(&plan, options.purge_config);
    if !confirm(options.yes)? {
        output::plain("Nothing was removed.");
        return Ok(ExitCode::FAILURE);
    }
    let mut tally = Tally {
        skipped_rc: plan.unreadable.clone(),
        failed: Vec::new(),
    };

    output::blank();
    #[cfg(unix)]
    stop_supervisor(&plan.runtime);
    if let Err(err) = lifecycle::stop() {
        output::warn(format_args!(
            "could not stop the agent ({}); continuing",
            crate::error::user_message(&err)
        ));
    }

    clean_rc_files(&plan.rc_files, &mut tally)?;
    for path in &plan.integration {
        tally.remove(path);
    }
    // The data root itself goes only when nothing else of the user's is in it.
    if fs::remove_dir(&plan.data_root.0).is_ok() {
        output::plain(format_args!("Removed empty {}", plan.data_root.0.display()));
    }
    restore_fish_prompt(&plan.config_home, &plan.data_root, &mut tally);
    remove_binaries(&plan.binaries, &mut tally);
    remove_config(&plan.config_dir, options.purge_config, &mut tally);
    if let Some(cache) = &plan.cache {
        tally.remove(cache);
    }
    if plan.cache.as_deref() != Some(plan.runtime.as_path()) {
        tally.remove(&plan.runtime);
    }
    Ok(finish(&tally))
}

/// The closing report and exit code.
fn finish(tally: &Tally) -> ExitCode {
    if tally.skipped_rc.is_empty() && tally.failed.is_empty() {
        output::ok("GPY uninstalled");
        output::plain("Restart your shell.");
        return ExitCode::SUCCESS;
    }
    if !tally.skipped_rc.is_empty() {
        output::plain(format_args!(
            "GPY uninstalled, but these rc files could not be checked or cleaned: {}. \
             If one holds a GPY block, delete the lines from '# >>> gpy-init >>>' to \
             '# <<< gpy-init <<<' by hand.",
            join_paths(&tally.skipped_rc)
        ));
    }
    if !tally.failed.is_empty() {
        output::plain(format_args!(
            "GPY uninstalled, but these paths could not be removed: {}.",
            join_paths(&tally.failed)
        ));
    }
    ExitCode::FAILURE
}

fn join_paths(list: &[PathBuf]) -> String {
    list.iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Gather what exists. An rc file counts only when it holds a GPY start
/// marker, whatever its encoding.
fn build_plan(env: &Environment) -> Result<Plan> {
    let home = env.home()?.to_path_buf();
    let config_home = env.config_home()?;
    let fish_dir = config_home.join("fish");

    let data_root = env.data_root()?;
    let mut integration: Vec<PathBuf> = SHELLS
        .into_iter()
        .map(|shell| shell_dir_for(&data_root, shell))
        .collect();
    // Deprecated: the pre-data-root locations, removed only when GPY stamped them.
    integration.extend(
        SHELLS
            .into_iter()
            .map(|shell| legacy_shell_dir_for(&config_home, shell))
            .filter(|dir| dir.join(VERSION_STAMP).is_file()),
    );
    let completions = fish_completions_dir(&config_home);
    integration.push(completions.join("gpy.fish"));
    integration.push(completions.join("gpy-dynamic.fish"));
    integration.push(fish_dir.join("conf.d").join("gpy_init.fish"));
    integration.push(plugins_target(&data_root));
    // Deprecated: the pre-data-root bundled plugin root, never stamped.
    integration.push(
        config_home
            .join("gpy")
            .join(crate::plugin::BUNDLED_PLUGINS_DIR_NAME),
    );

    let mut candidates = vec![
        home.join(".bashrc"),
        home.join(".bash_profile"),
        home.join(".bash_login"),
        home.join(".profile"),
    ];
    if let Some(zdotdir) = env.zdotdir.as_deref().filter(|dir| *dir != home) {
        candidates.push(zdotdir.join(".zshrc"));
    }
    candidates.push(home.join(".zshrc"));
    candidates.push(fish_dir.join("config.fish"));

    let mut rc_files = Vec::new();
    let mut unreadable = Vec::new();
    for rc in candidates {
        match fs::read(&rc) {
            Ok(bytes) if has_start(&bytes) => rc_files.push(rc),
            Ok(_) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                output::warn_stderr(format_args!("skipped {}: {err}", rc.display()));
                unreadable.push(rc);
            }
        }
    }

    Ok(Plan {
        config_dir: config_home.join("gpy"),
        config_home,
        data_root,
        integration,
        rc_files,
        unreadable,
        binaries: binaries_of(env, &home),
        cache: paths::cache_root(),
        runtime: paths::runtime_dir(),
    })
}

/// Decide who owns the binaries from where the running `gpy` lives.
fn binaries_of(env: &Environment, home: &Path) -> Binaries {
    let bin_dir = home.join(".local").join("bin");
    let exe_dir = env.exe.as_deref().and_then(Path::parent);
    if exe_dir.is_some_and(|dir| fs::canonicalize(&bin_dir).is_ok_and(|bin| bin == dir)) {
        return Binaries::Installer {
            bin_dir,
            share_dir: home.join(".local").join("share").join("gpy"),
        };
    }
    let cellar = env
        .exe
        .as_deref()
        .is_some_and(|exe| exe.to_string_lossy().contains("/Cellar/"));
    let stray_share = exe_dir
        .and_then(Path::parent)
        .map(|prefix| prefix.join("share").join("gpy"))
        .filter(|dir| dir.exists() && !cellar);
    Binaries::Foreign {
        cellar,
        stray_share,
    }
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// Tell the user what is about to go.
fn print_plan(plan: &Plan, purge_config: bool) {
    output::plain("GPY will be uninstalled for the current user:");
    for path in plan.integration.iter().filter(|path| exists(path)) {
        output::plain(format_args!("  - {}", path.display()));
    }
    for rc in &plan.rc_files {
        output::plain(format_args!("  - the GPY block in {}", rc.display()));
    }
    output::plain(format_args!(
        "  - the fish prompt link in {} (restoring your previous prompt)",
        plan.config_home.join("fish").join("functions").display()
    ));
    match &plan.binaries {
        Binaries::Installer { bin_dir, share_dir } => {
            output::plain(format_args!(
                "  - gpy and gpy-agent in {} (and their backups), and {}",
                bin_dir.display(),
                share_dir.display()
            ));
        }
        Binaries::Foreign { .. } => {
            output::plain("  - no binaries: this gpy is not in ~/.local/bin");
        }
    }
    if purge_config {
        output::plain(format_args!(
            "  - configuration: {} (--purge-config)",
            plan.config_dir.display()
        ));
    } else {
        output::plain(format_args!(
            "  - configuration: {} is kept; pass --purge-config to delete it",
            plan.config_dir.display()
        ));
    }
    if let Some(cache) = &plan.cache {
        output::plain(format_args!("  - cache: {}", cache.display()));
    }
    output::plain(format_args!(
        "  - runtime files: {}",
        plan.runtime.display()
    ));
    output::plain("  - the running agent and supervisor");
}

/// Ask for consent. EOF or a non-terminal stdin is not consent.
fn confirm(yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(Error::invalid(
            "no confirmation received; nothing was removed. Re-run with --yes",
        ));
    }
    let mut stdout = std::io::stdout();
    stdout.write_all(b"Uninstall GPY? [y/N] ")?;
    stdout.flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    let reply = answer.trim();
    Ok(reply.eq_ignore_ascii_case("y") || reply.eq_ignore_ascii_case("yes"))
}

/// Remove a file, link or tree. `Ok(false)` when it was already gone.
fn remove_path(path: &Path) -> std::io::Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    if meta.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(true)
}

/// Stop the supervisor recorded in `<runtime>/supervisor.pid`, but only when
/// that PID's command line is the supervisor loop: a recycled PID belongs to
/// someone else. The pidfile is removed either way.
#[cfg(unix)]
fn stop_supervisor(runtime: &Path) {
    let pidfile = runtime.join("supervisor.pid");
    let text = match fs::read_to_string(&pidfile) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => return,
        Err(_) => {
            drop(fs::remove_file(&pidfile));
            return;
        }
    };
    if let Some(pid) = text.trim().parse::<i32>().ok().filter(|pid| *pid > 1_i32) {
        stop_if_supervisor(pid, &pidfile);
    }
    drop(fs::remove_file(&pidfile));
}

/// SIGTERM `pid` (SIGKILL after two seconds) when it runs the supervisor loop.
#[cfg(unix)]
fn stop_if_supervisor(raw_pid: i32, pidfile: &Path) {
    use nix::sys::signal::{Signal, kill};
    use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

    let Ok(unsigned) = u32::try_from(raw_pid) else {
        return;
    };
    let pid = Pid::from_u32(unsigned);
    let mut system = System::new();
    // The command line is not collected unless asked for.
    let refresh = |sys: &mut System| {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
        );
    };
    // A zombie has exited; only its parent has yet to collect it.
    let running = |sys: &System| {
        sys.process(pid)
            .is_some_and(|process| process.status() != ProcessStatus::Zombie)
    };

    refresh(&mut system);
    let is_supervisor = system.process(pid).is_some_and(|process| {
        process
            .cmd()
            .iter()
            .any(|arg| arg.to_string_lossy().contains(SUPERVISOR_MARKER))
    });
    if !is_supervisor {
        if running(&system) {
            output::plain(format_args!(
                "PID {raw_pid} in {} is not a GPY supervisor; leaving it alone",
                pidfile.display()
            ));
        }
        return;
    }
    let target = nix::unistd::Pid::from_raw(raw_pid);
    if kill(target, Signal::SIGTERM).is_err() {
        return;
    }
    for _ in 0_u8..20_u8 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        refresh(&mut system);
        if !running(&system) {
            break;
        }
    }
    if running(&system) {
        kill(target, Signal::SIGKILL).ok();
    }
    output::plain(format_args!("Stopped supervisor process {raw_pid}"));
}

/// Take the GPY block out of every listed rc file. A file that can no longer
/// be read is skipped; one that cannot be rewritten stops the run before any
/// shell tree is removed.
fn clean_rc_files(rc_files: &[PathBuf], tally: &mut Tally) -> Result<()> {
    for rc in rc_files {
        let outcome = match remove_blocks(rc) {
            Ok(outcome) => outcome,
            Err(err) => {
                if let Err(read_err) = fs::read(rc) {
                    output::warn_stderr(format_args!("skipped {}: {read_err}", rc.display()));
                    tally.skipped_rc.push(rc.clone());
                    continue;
                }
                return Err(err);
            }
        };
        if outcome.deleted_file {
            output::plain(format_args!(
                "Removed {} (created by GPY, now empty)",
                rc.display()
            ));
        } else if outcome.removed {
            output::plain(format_args!("Removed the GPY block from {}", rc.display()));
        }
        if outcome.unmatched_start {
            output::warn_stderr(format_args!(
                "{} has a '# >>> gpy-init >>>' line with no matching end; left in place",
                rc.display()
            ));
        }
        if outcome.unmatched_end {
            output::warn_stderr(format_args!(
                "{} has a '# <<< gpy-init <<<' line with no matching start; left in place",
                rc.display()
            ));
        }
    }
    Ok(())
}

/// Remove GPY's `fish_prompt.fish` (its symlink, or a regular-file copy of its
/// prompt) and, when nothing is left there, restore the newest
/// `fish_prompt.fish.backup.<stamp>`. A prompt that is not GPY's stays.
fn restore_fish_prompt(config_home: &Path, root: &DataRoot, tally: &mut Tally) {
    let prompt = config_home
        .join("fish")
        .join("functions")
        .join("fish_prompt.fish");
    // GPY's prompt link goes into the current Fish tree, or the pre-data-root one.
    let own_dirs = [
        shell_dir_for(root, Shell::Fish),
        legacy_shell_dir_for(config_home, Shell::Fish),
    ];

    if let Ok(meta) = prompt.symlink_metadata() {
        if meta.file_type().is_symlink() {
            if fs::read_link(&prompt).is_ok_and(|target| {
                own_dirs
                    .iter()
                    .any(|dir| target == dir.join("functions").join("fish_prompt.fish"))
            }) {
                tally.remove(&prompt);
            } else {
                output::plain("fish_prompt.fish is a symlink GPY did not create; leaving it alone");
            }
        } else if meta.is_file() && is_gpy_prompt_copy(&prompt) {
            tally.remove(&prompt);
        }
    }
    if exists(&prompt) {
        return;
    }
    let Some(backup) = newest_backup(&prompt, &own_dirs) else {
        return;
    };
    match fs::rename(&backup, &prompt) {
        Ok(()) => output::plain(format_args!(
            "Restored your fish prompt from {}",
            backup.display()
        )),
        Err(err) => {
            output::warn_stderr(format_args!(
                "could not restore {}: {err}",
                backup.display()
            ));
            tally.failed.push(backup);
        }
    }
}

#[cfg(unix)]
fn is_gpy_prompt_copy(path: &Path) -> bool {
    super::setup::is_gpy_prompt_copy(path)
}

#[cfg(not(unix))]
fn is_gpy_prompt_copy(_path: &Path) -> bool {
    false
}

/// Whether `stamp` is `YYYYMMDD_HHMMSS`.
fn is_backup_stamp(stamp: &str) -> bool {
    stamp.split_once('_').is_some_and(|(date, time)| {
        date.len() == 8
            && time.len() == 6
            && date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit())
    })
}

/// The newest `<prompt>.backup.<stamp>` beside `prompt`. A symlink into
/// GPY's own shell tree never qualifies.
fn newest_backup(prompt: &Path, own_dirs: &[PathBuf]) -> Option<PathBuf> {
    let dir = prompt.parent()?;
    let prefix = format!("{}.backup.", prompt.file_name()?.to_string_lossy());
    let mut best: Option<(String, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stamp) = name.strip_prefix(&prefix).filter(|s| is_backup_stamp(s)) else {
            continue;
        };
        let path = entry.path();
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        if meta.file_type().is_symlink() {
            if fs::read_link(&path)
                .is_ok_and(|target| own_dirs.iter().any(|own| target.starts_with(own)))
            {
                continue;
            }
        } else if !meta.is_file() {
            continue;
        }
        let key = format!("{stamp}\t{name}");
        if best.as_ref().is_none_or(|(current, _)| key > *current) {
            best = Some((key, path));
        }
    }
    best.map(|(_, path)| path)
}

/// Delete the installers' binaries, or point at whoever owns them.
fn remove_binaries(binaries: &Binaries, tally: &mut Tally) {
    match binaries {
        Binaries::Installer { bin_dir, share_dir } => {
            for name in ["gpy", "gpy-agent"] {
                tally.remove(&bin_dir.join(name));
            }
            let backups: Vec<_> = fs::read_dir(bin_dir)
                .into_iter()
                .flatten()
                .flatten()
                .collect();
            for entry in backups {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("gpy.backup.") || name.starts_with("gpy-agent.backup.") {
                    tally.remove(&entry.path());
                }
            }
            tally.remove(share_dir);
        }
        Binaries::Foreign {
            cellar,
            stray_share,
        } => {
            output::plain("Remove the binaries with the tool that installed them");
            if *cellar {
                output::plain("  brew uninstall gpy");
            }
            if let Some(share) = stray_share {
                output::plain(format_args!(
                    "The shell scripts `gpy setup` kept are not removed; delete them with: rm -rf '{}'",
                    share.display()
                ));
            }
        }
    }
}

/// Delete the configuration directory with `--purge-config`; otherwise only an
/// empty one.
fn remove_config(config_dir: &Path, purge: bool, tally: &mut Tally) {
    if purge {
        tally.remove(config_dir);
    } else if exists(config_dir) {
        if fs::remove_dir(config_dir).is_ok() {
            output::plain(format_args!("Removed empty {}", config_dir.display()));
        } else {
            output::plain(format_args!(
                "Kept your configuration in {}; delete it with: rm -rf '{}'",
                config_dir.display(),
                config_dir.display()
            ));
        }
    }
}
