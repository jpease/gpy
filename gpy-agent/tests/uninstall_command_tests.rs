//! `gpy uninstall`: rc files restored byte for byte, the configuration kept
//! unless purged, no removal without consent, the fish prompt restored only
//! when it is GPY's, binaries removed only from the installers' directory, and
//! a recorded supervisor PID signalled only when it runs the supervisor loop.
//! Everything runs against an isolated HOME/XDG tree.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code may panic on failure"
)]

mod common;
#[path = "common/setup_harness.rs"]
mod setup_harness;

use common::child_guard::{KillOnDrop, PtyChild};
use common::cli_harness::{CliCommandResult, CliTestEnv};
use common::pty::{
    EXIT_DEADLINE, OUTPUT_DEADLINE, open_pty, spawn_reader_thread, wait_for_exit, wait_for_output,
};
use portable_pty::CommandBuilder;
use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::Command;

/// An inherited `ZDOTDIR` would send the uninstaller to the developer's real rc file.
const SANDBOX_ENV: [(&str, &str); 1] = [("ZDOTDIR", "")];

/// The 2025-08 prompt GPY copied in as a regular file; the current one is read
/// from the checkout.
const LEGACY_GPY_PROMPT: &str = "# MAIN PROMPT FUNCTION
function fish_prompt
    echo
    for segment in $__enabled_segments
        if functions -q segment_{$segment}_detect; and segment_{$segment}_detect
            if functions -q segment_{$segment}_render
                segment_{$segment}_render
            end
        end
    end
    set_color normal
    printf \"\\n%s \" \"$__icon_prompt\"
end
";

/// A user's prompt that names gpy; no trailing newline so a rewrite would show.
const USER_PROMPT: &str = "# Custom prompt; evaluated gpy but chose this instead
# for segment in $__enabled_segments  (gpy did it this way)
function fish_prompt
    set -l gpy_like \"not gpy\"
    printf \"mine> \"
end";

const OLD_BACKUP: &str = "function fish_prompt; echo restored_old; end\n";
const BACKUP_NAME: &str = "fish_prompt.fish.backup.20260101_000000";

fn uninstall(env: &CliTestEnv, args: &[&str]) -> CliCommandResult {
    let mut full = vec!["uninstall"];
    full.extend_from_slice(args);
    env.run_gpy_with_env(&full, &SANDBOX_ENV).unwrap()
}

fn setup_shell(env: &CliTestEnv, shell: &str) {
    let output = setup_harness::setup(env.root(), &["--shell", shell]);
    setup_harness::assert_ok(&output);
}

fn is_absent(path: &Path) -> bool {
    path.symlink_metadata().is_err()
}

/// `~/.local/share/gpy/bash`, the Bash tree `gpy setup` installs.
fn bash_dir(env: &CliTestEnv) -> PathBuf {
    env.root().join(".local/share/gpy/bash")
}

fn functions_dir(env: &CliTestEnv) -> PathBuf {
    let dir = env.xdg_config_home().join("fish").join("functions");
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo_prompt() -> String {
    fs::read_to_string(setup_harness::repo_root().join("fish/functions/fish_prompt.fish")).unwrap()
}

fn combined(result: &CliCommandResult) -> String {
    format!("{}{}", result.stdout, result.stderr)
}

#[test]
fn uninstall_restores_rc_files_byte_for_byte() {
    let env = CliTestEnv::new().unwrap();
    let home = env.root();
    let real_zshrc = home.join("dotfiles-zshrc");
    fs::write(&real_zshrc, "alias z=1\n").unwrap();
    symlink(&real_zshrc, home.join(".zshrc")).unwrap();
    fs::write(home.join(".bashrc"), "export A=1").unwrap();
    for shell in ["fish", "zsh", "bash"] {
        setup_shell(&env, shell);
    }
    assert_eq!(setup_harness::block_count(&home.join(".bashrc")), 1);

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert_eq!(fs::read(home.join(".bashrc")).unwrap(), b"export A=1");
    assert!(home.join(".zshrc").symlink_metadata().unwrap().is_symlink());
    assert_eq!(fs::read(&real_zshrc).unwrap(), b"alias z=1\n");
    for gone in [
        ".profile",
        ".config/fish/config.fish",
        ".local/share/gpy/fish",
        ".config/fish/functions/fish_prompt.fish",
        ".config/fish/conf.d/gpy_init.fish",
        ".config/fish/completions/gpy.fish",
        ".config/fish/completions/gpy-dynamic.fish",
        ".local/share/gpy/zsh",
        ".local/share/gpy/bash",
        ".local/share/gpy/plugins",
    ] {
        assert!(is_absent(&home.join(gone)), "{gone} should be gone");
    }
}

#[test]
fn uninstall_keeps_config_unless_purged() {
    let env = CliTestEnv::new().unwrap();
    setup_shell(&env, "bash");
    let config = env.config_dir();
    // `gpy setup` retires the unedited builtin copy (`default.toml`), so the
    // fixture needs a theme the user wrote.
    fs::write(env.themes_dir().join("mine.toml"), "x = 1\n").unwrap();
    fs::create_dir_all(config.join("palettes")).unwrap();
    fs::write(config.join("palettes/mine.toml"), "x = 1\n").unwrap();
    fs::write(env.plugins_dir().join("mine.txt"), "plugin").unwrap();

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert!(env.config_path().is_file());
    assert!(env.themes_dir().join("mine.toml").is_file());
    assert!(config.join("palettes/mine.toml").is_file());
    assert!(env.plugins_dir().join("mine.txt").is_file());

    uninstall(&env, &["--yes", "--purge-config"]).assert_success("gpy uninstall --purge-config");

    assert!(is_absent(&config));
}

#[test]
fn uninstall_without_yes_and_without_a_terminal_removes_nothing() {
    let env = CliTestEnv::new().unwrap();
    setup_shell(&env, "bash");
    let bashrc = env.root().join(".bashrc");
    let before = fs::read(&bashrc).unwrap();

    let result = uninstall(&env, &[]);

    assert_eq!(result.exit_code, 1_i32, "{}", combined(&result));
    assert!(
        result.stderr.contains("no confirmation received"),
        "{}",
        result.stderr
    );
    assert_eq!(fs::read(&bashrc).unwrap(), before);
    assert!(bash_dir(&env).is_dir());
}

#[test]
fn uninstall_removes_gpy_prompt_and_restores_newest_backup() {
    let env = CliTestEnv::new().unwrap();
    setup_shell(&env, "fish");
    let functions = functions_dir(&env);
    fs::write(functions.join(BACKUP_NAME), OLD_BACKUP).unwrap();
    fs::write(
        functions.join("fish_prompt.fish.backup.20260202_000000"),
        "function fish_prompt; echo newest; end\n",
    )
    .unwrap();

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert_eq!(
        fs::read_to_string(functions.join("fish_prompt.fish")).unwrap(),
        "function fish_prompt; echo newest; end\n"
    );
    assert!(functions.join(BACKUP_NAME).is_file());
    assert!(is_absent(
        &functions.join("fish_prompt.fish.backup.20260202_000000")
    ));
}

#[test]
fn uninstall_keeps_a_user_prompt_that_mentions_gpy() {
    let env = CliTestEnv::new().unwrap();
    let functions = functions_dir(&env);
    fs::write(functions.join("fish_prompt.fish"), USER_PROMPT).unwrap();
    fs::write(functions.join(BACKUP_NAME), OLD_BACKUP).unwrap();

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert_eq!(
        fs::read(functions.join("fish_prompt.fish")).unwrap(),
        USER_PROMPT.as_bytes()
    );
    assert_eq!(
        fs::read(functions.join(BACKUP_NAME)).unwrap(),
        OLD_BACKUP.as_bytes()
    );
}

#[test]
fn uninstall_keeps_an_unrelated_prompt_symlink() {
    let env = CliTestEnv::new().unwrap();
    let functions = functions_dir(&env);
    let dotfile = env.root().join("dotfiles_prompt.fish");
    fs::write(&dotfile, USER_PROMPT).unwrap();
    symlink(&dotfile, functions.join("fish_prompt.fish")).unwrap();
    fs::write(functions.join(BACKUP_NAME), OLD_BACKUP).unwrap();

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert_eq!(
        fs::read_link(functions.join("fish_prompt.fish")).unwrap(),
        dotfile
    );
    assert_eq!(fs::read(&dotfile).unwrap(), USER_PROMPT.as_bytes());
    assert_eq!(
        fs::read(functions.join(BACKUP_NAME)).unwrap(),
        OLD_BACKUP.as_bytes()
    );
}

#[test]
fn uninstall_removes_gpy_prompt_symlink_and_restores_backup() {
    let env = CliTestEnv::new().unwrap();
    let functions = functions_dir(&env);
    let own_dir = env.root().join(".local/share/gpy/fish/functions");
    fs::create_dir_all(&own_dir).unwrap();
    fs::write(own_dir.join("fish_prompt.fish"), repo_prompt()).unwrap();
    symlink(
        own_dir.join("fish_prompt.fish"),
        functions.join("fish_prompt.fish"),
    )
    .unwrap();
    fs::write(functions.join(BACKUP_NAME), OLD_BACKUP).unwrap();

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    let restored = functions.join("fish_prompt.fish");
    assert!(!restored.symlink_metadata().unwrap().is_symlink());
    assert_eq!(fs::read(&restored).unwrap(), OLD_BACKUP.as_bytes());
    assert!(is_absent(&functions.join(BACKUP_NAME)));
}

#[test]
fn uninstall_removes_a_regular_file_gpy_prompt_and_restores_backup() {
    for gpy_copy in [repo_prompt(), LEGACY_GPY_PROMPT.to_owned()] {
        let env = CliTestEnv::new().unwrap();
        let functions = functions_dir(&env);
        fs::write(functions.join("fish_prompt.fish"), &gpy_copy).unwrap();
        fs::write(functions.join(BACKUP_NAME), OLD_BACKUP).unwrap();

        uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

        assert_eq!(
            fs::read(functions.join("fish_prompt.fish")).unwrap(),
            OLD_BACKUP.as_bytes()
        );
        assert!(is_absent(&functions.join(BACKUP_NAME)));
    }
}

#[test]
fn uninstall_removes_a_regular_file_gpy_prompt_without_backup() {
    for gpy_copy in [repo_prompt(), LEGACY_GPY_PROMPT.to_owned()] {
        let env = CliTestEnv::new().unwrap();
        let functions = functions_dir(&env);
        fs::write(functions.join("fish_prompt.fish"), &gpy_copy).unwrap();

        uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

        assert!(is_absent(&functions.join("fish_prompt.fish")));
    }
}

#[test]
fn uninstall_removes_an_empty_config_dir_without_purge() {
    let env = CliTestEnv::new().unwrap();
    let config = env.config_dir();
    fs::remove_dir_all(&config).unwrap();
    fs::create_dir_all(&config).unwrap();

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert!(is_absent(&config));
}

#[test]
fn uninstall_removes_installer_binaries_and_backups() {
    let env = CliTestEnv::new().unwrap();
    let bin = env.root().join(".local/bin");
    let share = env.root().join(".local/share/gpy");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(share.join("fish")).unwrap();
    let gpy = bin.join("gpy");
    fs::copy(env!("CARGO_BIN_EXE_gpy"), &gpy).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_gpy-agent"), bin.join("gpy-agent")).unwrap();
    fs::write(bin.join("gpy.backup.20260101_000000"), "old").unwrap();
    fs::write(bin.join("gpy-agent.backup.20260101_000000"), "old").unwrap();
    fs::write(bin.join("other-tool"), "keep").unwrap();

    env.run_program_with_env(&gpy, &["uninstall", "--yes"], &SANDBOX_ENV)
        .unwrap()
        .assert_success("installed gpy uninstall --yes");

    for gone in [
        "gpy",
        "gpy-agent",
        "gpy.backup.20260101_000000",
        "gpy-agent.backup.20260101_000000",
    ] {
        assert!(is_absent(&bin.join(gone)), "{gone} should be gone");
    }
    assert!(bin.join("other-tool").is_file());
    assert!(is_absent(&share));
}

#[test]
fn uninstall_leaves_package_manager_binaries_and_names_brew() {
    let env = CliTestEnv::new().unwrap();
    let bin = env.root().join("Cellar/gpy/1.0/bin");
    fs::create_dir_all(&bin).unwrap();
    let gpy = bin.join("gpy");
    fs::copy(env!("CARGO_BIN_EXE_gpy"), &gpy).unwrap();

    let result = env
        .run_program_with_env(&gpy, &["uninstall", "--yes"], &SANDBOX_ENV)
        .unwrap();

    result.assert_success("Cellar gpy uninstall --yes");
    assert!(gpy.is_file(), "a package manager's binary must stay");
    assert!(
        result.stdout.contains("brew uninstall gpy"),
        "{}",
        result.stdout
    );
    assert!(
        result
            .stdout
            .contains("Remove the binaries with the tool that installed them"),
        "{}",
        result.stdout
    );
}

fn write_supervisor_pid(env: &CliTestEnv, pid: u32) -> PathBuf {
    let runtime = env.xdg_runtime_dir().join("gpy");
    fs::create_dir_all(&runtime).unwrap();
    let pidfile = runtime.join("supervisor.pid");
    fs::write(&pidfile, format!("{pid}\n")).unwrap();
    pidfile
}

#[test]
fn uninstall_does_not_signal_a_foreign_pid_in_the_supervisor_pidfile() {
    let env = CliTestEnv::new().unwrap();
    let mut child = KillOnDrop::new(Command::new("sleep").arg("30").spawn().unwrap());
    let pidfile = write_supervisor_pid(&env, child.id());

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert!(
        child.try_wait().unwrap().is_none(),
        "an unrelated process must not be signalled"
    );
    assert!(!pidfile.exists(), "the pidfile is removed either way");
}

/// Kills the whole process group of a spawned supervisor stand-in on drop, so
/// the orphaned `sleep` does not outlive the test.
struct GroupKill(i32);

impl Drop for GroupKill {
    fn drop(&mut self) {
        nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(self.0),
            nix::sys::signal::Signal::SIGKILL,
        )
        .ok();
    }
}

#[test]
fn uninstall_stops_a_process_whose_command_line_names_the_supervisor_loop() {
    let env = CliTestEnv::new().unwrap();
    // The trailing `:` stops `sh` from exec-ing `sleep`, so the command line
    // keeps the marker.
    let mut child = Command::new("sh")
        .args(["-c", "sleep 60; :", "__gpy_agent_supervisor_loop"])
        .process_group(0)
        .spawn()
        .unwrap();
    let _group = GroupKill(i32::try_from(child.id()).unwrap());
    let pidfile = write_supervisor_pid(&env, child.id());

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    // Reaping first matters: an unreaped child is a zombie `kill(pid, 0)` finds.
    let status = child.wait().unwrap();
    assert_eq!(
        status.signal(),
        Some(15_i32),
        "the supervisor must get SIGTERM"
    );
    assert!(!pidfile.exists());
}

#[test]
fn uninstall_cleans_non_utf8_rc_files() {
    let env = CliTestEnv::new().unwrap();
    let home = env.root();
    fs::write(home.join(".bashrc"), "export B=2\n").unwrap();
    setup_shell(&env, "bash");
    fs::write(home.join(".bash_login"), b"# caf\xe9\n").unwrap();
    let fish_config = env.xdg_config_home().join("fish/config.fish");
    fs::create_dir_all(fish_config.parent().unwrap()).unwrap();
    let mut bytes = b"# caf\xe9\n".to_vec();
    bytes.extend_from_slice(
        b"\n# >>> gpy-init >>>\n# GPY shell integration\nif status is-interactive\n    source \"/x\"\nend\n# <<< gpy-init <<<\n",
    );
    fs::write(&fish_config, bytes).unwrap();

    let result = uninstall(&env, &["--yes"]);

    result.assert_success("gpy uninstall --yes");
    assert_eq!(fs::read(home.join(".bashrc")).unwrap(), b"export B=2\n");
    assert_eq!(fs::read(&fish_config).unwrap(), b"# caf\xe9\n");
    assert_eq!(fs::read(home.join(".bash_login")).unwrap(), b"# caf\xe9\n");
    assert!(
        !combined(&result)
            .lines()
            .any(|line| line.contains(".bash_login")),
        "{}",
        combined(&result)
    );
}

#[test]
fn uninstall_reports_an_unreadable_rc_file_and_exits_1() {
    use std::os::unix::fs::PermissionsExt;

    if nix::unistd::Uid::effective().is_root() {
        common::skip::skip_test("root reads a mode 0000 file");
        return;
    }
    let env = CliTestEnv::new().unwrap();
    setup_shell(&env, "bash");
    setup_shell(&env, "zsh");
    let zshrc = env.root().join(".zshrc");
    fs::set_permissions(&zshrc, fs::Permissions::from_mode(0o000)).unwrap();

    let result = uninstall(&env, &["--yes"]);

    fs::set_permissions(&zshrc, fs::Permissions::from_mode(0o600)).unwrap();
    let output = combined(&result);
    assert_eq!(result.exit_code, 1_i32, "{output}");
    assert!(output.contains("warn: skipped"), "{output}");
    assert!(
        output.contains("could not be checked or cleaned"),
        "{output}"
    );
    assert!(output.contains(".zshrc"), "{output}");
    assert!(
        is_absent(&env.root().join(".bashrc")),
        "the other rc files are still cleaned"
    );
    assert!(is_absent(&bash_dir(&env)));
}

#[test]
fn uninstall_after_new_layout_keeps_the_fish_config_dir() {
    let env = CliTestEnv::new().unwrap();
    let fish = env.xdg_config_home().join("fish");
    fs::create_dir_all(fish.join("conf.d")).unwrap();
    fs::write(fish.join("config.fish"), "set -gx KEEP 1\n").unwrap();
    fs::write(fish.join("conf.d/user.fish"), "set -gx USER_CONF 1\n").unwrap();
    setup_shell(&env, "fish");
    assert!(env.root().join(".local/share/gpy/fish").is_dir());

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert_eq!(
        fs::read(fish.join("config.fish")).unwrap(),
        b"set -gx KEEP 1\n"
    );
    assert_eq!(
        fs::read(fish.join("conf.d/user.fish")).unwrap(),
        b"set -gx USER_CONF 1\n"
    );
    assert!(is_absent(&env.root().join(".local/share/gpy/fish")));
}

#[test]
fn uninstall_removes_stamped_old_trees() {
    let env = CliTestEnv::new().unwrap();
    let config = env.xdg_config_home();
    let old_fish = config.join("fish/gpy");
    let old_zsh = config.join("gpy/zsh");
    let old_bash = config.join("gpy/bash");
    let old_plugins = config.join("gpy/bundled-plugins");
    for dir in [&old_fish, &old_zsh, &old_bash, &old_plugins] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(old_fish.join(".gpy-version"), "0.8.0\n").unwrap();
    fs::write(old_zsh.join(".gpy-version"), "0.8.0\n").unwrap();
    fs::write(old_bash.join("notes.txt"), "not stamped\n").unwrap();
    fs::write(old_plugins.join("keep.txt"), "removed anyway\n").unwrap();
    setup_shell(&env, "zsh");

    uninstall(&env, &["--yes"]).assert_success("gpy uninstall --yes");

    assert!(is_absent(&old_fish));
    assert!(is_absent(&old_zsh));
    assert!(is_absent(&old_plugins));
    assert!(
        old_bash.join("notes.txt").is_file(),
        "an unstamped dir is not GPY's"
    );
    assert!(is_absent(&env.root().join(".local/share/gpy/zsh")));
}

/// `gpy uninstall` on a pty, answering the prompt with `answer`; returns the
/// exit code.
fn uninstall_on_pty(env: &CliTestEnv, answer: &[u8]) -> u32 {
    let (pair, reader) = open_pty();
    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.arg("uninstall");
    cmd.cwd(env.root());
    cmd.env("HOME", env.root());
    cmd.env("XDG_CONFIG_HOME", env.xdg_config_home());
    cmd.env("XDG_DATA_HOME", env.root().join(".local/share"));
    cmd.env("XDG_CACHE_HOME", env.xdg_cache_home());
    cmd.env("XDG_RUNTIME_DIR", env.xdg_runtime_dir());
    cmd.env("ZDOTDIR", "");
    let mut child = PtyChild::new(pair.slave.spawn_command(cmd).expect("spawn gpy uninstall"));
    drop(pair.slave);

    wait_for_output(&captured, b"Uninstall GPY? [y/N]", OUTPUT_DEADLINE);
    writer.write_all(answer).expect("write answer");
    writer.flush().expect("flush answer");
    wait_for_exit(&mut *child, EXIT_DEADLINE).exit_code()
}

#[test]
fn uninstall_asks_and_honours_the_answer() {
    let env = CliTestEnv::new().unwrap();
    setup_shell(&env, "bash");
    let bash_dir = bash_dir(&env);

    assert_eq!(uninstall_on_pty(&env, b"n\n"), 1);
    assert!(bash_dir.is_dir(), "answering n must remove nothing");

    assert_eq!(uninstall_on_pty(&env, b"y\n"), 0);
    assert!(is_absent(&bash_dir), "answering y must uninstall");
}
