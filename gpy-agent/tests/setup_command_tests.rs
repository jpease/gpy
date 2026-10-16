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

#[test]
fn bash_setup_leaves_a_login_file_that_already_loads_bashrc() {
    let home = TempDir::new().unwrap();
    let login = home.path().join(".bash_profile");
    fs::write(&login, "[ -r ~/.bashrc ] && . ~/.bashrc\n").unwrap();

    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    assert_eq!(
        fs::read_to_string(&login).unwrap(),
        "[ -r ~/.bashrc ] && . ~/.bashrc\n"
    );
    assert!(!home.path().join(".profile").exists());
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
