//! Integration tests for `gpy setup`: the per-shell install-and-wire step that
//! Homebrew, source builds and release archives share with the curl installer.
//!
//! Drives the real `gpy` binary against an isolated HOME with `--from` at the
//! repo's shell trees, so the shipped layout is under test.

#![cfg(unix)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate lives inside the repository")
        .to_path_buf()
}

fn setup(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gpy"))
        .arg("setup")
        .args(["--non-interactive", "--from"])
        .arg(repo_root())
        .args(args)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GPY_NERD_FONT", "0")
        .output()
        .expect("failed to spawn gpy setup")
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "gpy setup failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn block_count(rc: &Path) -> usize {
    fs::read_to_string(rc)
        .unwrap()
        .matches("# >>> gpy-init >>>")
        .count()
}

#[test]
fn zsh_setup_installs_files_and_wires_zshrc_once() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    let dir = home.path().join(".config/gpy/zsh");
    assert!(dir.join("gpy.zsh").is_file());
    assert!(dir.join("core/init.zsh").is_file());
    assert!(dir.join("completions/_gpy-dynamic").is_file());
    let structural = fs::read_to_string(dir.join("completions/_gpy")).unwrap();
    assert!(structural.contains("#compdef gpy"), "{structural}");

    let rc = home.path().join(".zshrc");
    assert_eq!(block_count(&rc), 1, "re-running must not stack blocks");
    let text = fs::read_to_string(&rc).unwrap();
    assert!(text.contains(&format!("source \"{}\"", dir.join("gpy.zsh").display())));
}

#[test]
fn bash_setup_wires_bashrc_and_a_login_loader() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    let dir = home.path().join(".config/gpy/bash");
    assert!(dir.join("gpy.bash").is_file());
    assert!(dir.join("completions/gpy.bash").is_file());
    assert_eq!(block_count(&home.path().join(".bashrc")), 1);
    let profile = fs::read_to_string(home.path().join(".profile")).unwrap();
    assert!(profile.contains(". \"$HOME/.bashrc\""), "{profile}");
}

/// A `.bashrc` that counts its runs, then stops (as most do) in a
/// non-interactive shell, before GPY's block.
const COUNTING_BASHRC: &str = "GPY_TEST_BASHRC_RUNS=$((${GPY_TEST_BASHRC_RUNS:-0} + 1))\ncase $- in *i*) ;; *) return ;; esac\n";

/// How many times a real Bash login shell in `home` ran `~/.bashrc`.
fn bashrc_runs_in_login_shell(home: &Path) -> String {
    let output = Command::new("bash")
        .args(["--login", "-c", "printf %s \"${GPY_TEST_BASHRC_RUNS:-0}\""])
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("failed to spawn bash");
    assert!(
        output.status.success(),
        "bash --login failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn bash_setup_wires_a_login_file_that_only_mentions_bashrc_in_comments() {
    for (name, text) in [
        (
            ".profile",
            "# .bashrc is intentionally not sourced here yet\n",
        ),
        (".bash_profile", "# [ -f ~/.bashrc ] && . ~/.bashrc\n"),
    ] {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".bashrc"), COUNTING_BASHRC).unwrap();
        let login = home.path().join(name);
        fs::write(&login, text).unwrap();

        assert_ok(&setup(home.path(), &["--shell", "bash"]));
        assert_ok(&setup(home.path(), &["--shell", "bash"]));

        assert!(fs::read_to_string(&login).unwrap().starts_with(text));
        assert_eq!(
            block_count(&login),
            1,
            "{name}: re-running must not stack loaders"
        );
        assert_eq!(bashrc_runs_in_login_shell(home.path()), "1", "{name}");
    }
}

#[test]
fn bash_setup_wires_the_login_file_bash_reads_first() {
    let home = TempDir::new().unwrap();
    fs::write(home.path().join(".bashrc"), COUNTING_BASHRC).unwrap();
    let bash_login = home.path().join(".bash_login");
    fs::write(&bash_login, "# [ -f ~/.bashrc ] && . ~/.bashrc\n").unwrap();
    let profile = home.path().join(".profile");
    fs::write(&profile, "[ -f ~/.bashrc ] && . ~/.bashrc\n").unwrap();

    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    assert_eq!(block_count(&bash_login), 1);
    assert_eq!(
        fs::read_to_string(&profile).unwrap(),
        "[ -f ~/.bashrc ] && . ~/.bashrc\n"
    );
    assert_eq!(bashrc_runs_in_login_shell(home.path()), "1");
}

#[test]
fn bash_setup_leaves_a_login_file_that_already_loads_bashrc() {
    for loader in [
        "[ -r ~/.bashrc ] && . ~/.bashrc\n",
        "if [ -f \"$HOME/.bashrc\" ]; then\n    . \"$HOME/.bashrc\"\nfi\n",
        "# Load the interactive config.\ntest -f ~/.bashrc && source ~/.bashrc\n",
    ] {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".bashrc"), COUNTING_BASHRC).unwrap();
        let login = home.path().join(".bash_profile");
        fs::write(&login, loader).unwrap();

        assert_ok(&setup(home.path(), &["--shell", "bash"]));

        assert_eq!(fs::read_to_string(&login).unwrap(), loader);
        assert!(!home.path().join(".profile").exists());
        assert_eq!(bashrc_runs_in_login_shell(home.path()), "1", "{loader}");
    }
}

#[test]
fn fish_setup_links_the_prompt_and_backs_up_an_existing_one() {
    let home = TempDir::new().unwrap();
    let functions = home.path().join(".config/fish/functions");
    fs::create_dir_all(&functions).unwrap();
    fs::write(
        functions.join("fish_prompt.fish"),
        "function fish_prompt; end\n",
    )
    .unwrap();

    assert_ok(&setup(home.path(), &["--shell", "fish"]));
    assert_ok(&setup(home.path(), &["--shell", "fish"]));

    let dir = home.path().join(".config/fish/gpy");
    assert_eq!(
        fs::read_link(functions.join("fish_prompt.fish")).unwrap(),
        dir.join("functions/fish_prompt.fish")
    );
    let backups: Vec<_> = fs::read_dir(&functions)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".backup."))
        .collect();
    assert_eq!(backups.len(), 1, "only the foreign prompt is backed up");
    // The uninstallers restore only `.backup.YYYYMMDD_HHMMSS`.
    let name = backups
        .first()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .into_owned();
    let stamp = name.strip_prefix("fish_prompt.fish.backup.").unwrap();
    let (date, time) = stamp.split_once('_').unwrap();
    assert!(
        date.len() == 8 && time.len() == 6,
        "unexpected backup stamp in {name}"
    );
    assert!(stamp.chars().all(|c| c.is_ascii_digit() || c == '_'));

    assert!(
        !home.path().join(".config/fish/gpy/completions").exists(),
        "fish completions live in fish/completions, not the gpy tree"
    );
    let completions =
        fs::read_to_string(home.path().join(".config/fish/completions/gpy.fish")).unwrap();
    assert!(completions.contains("complete -c gpy"), "{completions}");
    assert_eq!(
        block_count(&home.path().join(".config/fish/config.fish")),
        1
    );
}

#[test]
fn shell_comes_from_the_login_shell_when_no_flag_is_given() {
    let home = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--non-interactive", "--from"])
        .arg(repo_root())
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("SHELL", "/usr/local/bin/zsh")
        .env("GPY_NERD_FONT", "0")
        .output()
        .unwrap();
    assert_ok(&output);
    assert!(home.path().join(".config/gpy/zsh/gpy.zsh").is_file());
}

#[test]
fn unsupported_login_shell_fails_with_a_pointer_to_the_flag() {
    let home = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--non-interactive", "--from"])
        .arg(repo_root())
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("SHELL", "/bin/tcsh")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--shell"));
    assert!(!home.path().join(".config").exists(), "nothing was written");
}

#[test]
fn missing_script_tree_fails_before_touching_any_rc_file() {
    let home = TempDir::new().unwrap();
    let empty = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--shell", "zsh", "--non-interactive", "--from"])
        .arg(empty.path())
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!home.path().join(".zshrc").exists());
}

#[test]
fn re_running_replaces_files_a_package_manager_left_read_only() {
    use std::os::unix::fs::PermissionsExt;

    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    let script = home.path().join(".config/gpy/zsh/gpy.zsh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o444)).unwrap();

    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
}

fn doctor(home: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .arg("doctor")
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("failed to spawn gpy doctor");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// An upgrade runs setup for one shell. A shell set up earlier must get the
/// new scripts too, without its rc file being touched, and `gpy doctor` must
/// name a shell an older gpy left behind.
#[test]
fn setup_refreshes_shells_set_up_earlier_and_doctor_flags_stale_ones() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "bash"]));
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    // What an older gpy left: different scripts and its own version stamp.
    let bash_dir = home.path().join(".config/gpy/bash");
    let core = bash_dir.join("core/ipc.bash");
    fs::write(&core, "# scripts from an older gpy\n").unwrap();
    fs::write(bash_dir.join(".gpy-version"), "0.0.1\n").unwrap();
    let bashrc = fs::read_to_string(home.path().join(".bashrc")).unwrap();
    let report = doctor(home.path());
    assert!(
        report.contains("Checking bash integration... ")
            && report.contains("installed by gpy 0.0.1"),
        "{report}"
    );
    assert!(report.contains("gpy setup --shell bash"), "{report}");

    let output = setup(home.path(), &["--shell", "zsh"]);
    assert_ok(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Refreshed bash files"), "{stdout}");
    assert_eq!(
        fs::read_to_string(&core).unwrap(),
        fs::read_to_string(repo_root().join("bash/core/ipc.bash")).unwrap(),
        "bash scripts refreshed"
    );
    assert_eq!(
        fs::read_to_string(bash_dir.join(".gpy-version"))
            .unwrap()
            .trim(),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        fs::read_to_string(home.path().join(".bashrc")).unwrap(),
        bashrc,
        "the other shell's rc file is left alone"
    );
    assert!(
        !home.path().join(".config/fish").exists(),
        "a shell never set up is not installed"
    );
    let report = doctor(home.path());
    assert!(!report.contains("installed by"), "{report}");
}
