//! Integration tests for the rc block `gpy setup --shell bash` writes: it loads
//! GPY only in interactive Bash, the login loader is inert outside Bash, and
//! blocks written by earlier releases are upgraded in place.
//!
//! Drives the real `gpy` binary against an isolated HOME, then starts real
//! `bash` and `dash` login shells in it.

#![cfg(unix)]
#![allow(clippy::expect_used, reason = "test code may panic on failure")]
#![allow(clippy::unwrap_used, reason = "test code may panic on failure")]

#[path = "common/setup_harness.rs"]
mod setup_harness;
#[path = "common/skip.rs"]
mod skip;

use setup_harness::{assert_ok, block_count, setup};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const LEGACY_LOADER: &str = "[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"";
const GUARDED_LOADER: &str =
    "[ -n \"${BASH_VERSION:-}\" ] && [ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"";

fn gpy_bash(home: &Path) -> String {
    home.join(".config/gpy/bash/gpy.bash").display().to_string()
}

/// A GPY block as an earlier release wrote it.
fn block(header: &str, body: &str) -> String {
    format!("\n# >>> gpy-init >>>\n# {header}\n{body}\n# <<< gpy-init <<<\n")
}

fn guarded_rc_body(home: &Path) -> String {
    format!(
        "case $- in\n    *i*) source \"{}\" ;;\nesac",
        gpy_bash(home)
    )
}

fn bash_login(home: &Path, interactive: bool) -> Output {
    let mut command = Command::new("bash");
    command.arg("--login");
    if interactive {
        command.arg("-i");
    }
    command
        .args([
            "-c",
            "declare -F __gpy_init >/dev/null && echo loaded || echo absent",
        ])
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GPY_AGENT_ENABLED", "0")
        .env("GPY_AGENT_SUPERVISOR_ENABLED", "0")
        .output()
        .expect("failed to spawn bash")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn dash_login(home: &Path) -> Output {
    Command::new("dash")
        .args(["-l", "-c", "echo ok"])
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("failed to spawn dash")
}

fn dash_is_missing() -> bool {
    Command::new("dash").args(["-c", "true"]).output().is_err()
}

fn assert_silent_ok(output: &Output) {
    assert_eq!(stdout_of(output), "ok");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn bash_setup_does_not_load_gpy_in_a_non_interactive_login_shell() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    assert_eq!(stdout_of(&bash_login(home.path(), false)), "absent");
}

#[test]
fn bash_setup_loads_gpy_in_an_interactive_login_shell() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    assert_eq!(stdout_of(&bash_login(home.path(), true)), "loaded");
}

#[test]
fn bash_login_loader_is_inert_in_sh() {
    if dash_is_missing() {
        skip::skip_test("dash not found in PATH");
        return;
    }
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    assert_silent_ok(&dash_login(home.path()));
}

#[test]
fn setup_upgrades_an_unguarded_bash_block_in_place() {
    let home = TempDir::new().unwrap();
    let legacy = block(
        "GPY shell integration",
        &format!("source \"{}\"", gpy_bash(home.path())),
    );
    let rc = home.path().join(".bashrc");
    fs::write(&rc, format!("export A=1\n{legacy}alias b=2\n")).unwrap();

    let output = setup(home.path(), &["--shell", "bash"]);
    assert_ok(&output);

    let upgraded = block("GPY shell integration", &guarded_rc_body(home.path()));
    assert_eq!(
        fs::read_to_string(&rc).unwrap(),
        format!("export A=1\n{upgraded}alias b=2\n")
    );
    assert_eq!(block_count(&rc), 1);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Updated the GPY block in"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_ok(&setup(home.path(), &["--shell", "bash"]));
    assert_eq!(block_count(&rc), 1);
}

#[test]
fn setup_upgrades_an_unguarded_login_loader_in_place() {
    let home = TempDir::new().unwrap();
    let legacy = block("GPY: load ~/.bashrc in login shells", LEGACY_LOADER);
    let profile = home.path().join(".profile");
    fs::write(&profile, format!("export A=1\n{legacy}echo done\n")).unwrap();

    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    let upgraded = block("GPY: load ~/.bashrc in login shells", GUARDED_LOADER);
    assert_eq!(
        fs::read_to_string(&profile).unwrap(),
        format!("export A=1\n{upgraded}echo done\n")
    );
    if dash_is_missing() {
        skip::skip_test("dash not found in PATH");
        return;
    }
    let output = dash_login(home.path());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn non_interactive_bash_login_shell_still_runs_bashrc_through_the_loader() {
    let home = TempDir::new().unwrap();
    fs::write(
        home.path().join(".bashrc"),
        "GPY_TEST_BASHRC_RUNS=$((${GPY_TEST_BASHRC_RUNS:-0} + 1))\n",
    )
    .unwrap();
    assert_ok(&setup(home.path(), &["--shell", "bash"]));

    let output = Command::new("bash")
        .args(["--login", "-c", "printf %s \"${GPY_TEST_BASHRC_RUNS:-0}\""])
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("failed to spawn bash");

    assert_eq!(stdout_of(&output), "1");
}

#[test]
fn setup_leaves_an_edited_gpy_block_alone() {
    let home = TempDir::new().unwrap();
    let edited = block(
        "GPY shell integration",
        &format!("source \"{}\" # mine", gpy_bash(home.path())),
    );
    let rc = home.path().join(".bashrc");
    fs::write(&rc, format!("export A=1\n{edited}")).unwrap();
    let before = fs::read(&rc).unwrap();

    let output = setup(home.path(), &["--shell", "bash"]);
    assert_ok(&output);

    assert_eq!(fs::read(&rc).unwrap(), before);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("left unchanged"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn setup_exits_zero_and_warns_when_a_legacy_block_cannot_be_upgraded() {
    if nix::unistd::Uid::effective().is_root() {
        skip::skip_test("root ignores directory modes");
        return;
    }
    let home = TempDir::new().unwrap();
    // The first run leaves the guarded loader in ~/.profile and ~/.config in place.
    assert_ok(&setup(home.path(), &["--shell", "bash"]));
    fs::remove_file(home.path().join(".bashrc")).unwrap();

    let away = TempDir::new().unwrap();
    let target = away.path().join("bashrc");
    let legacy = block(
        "GPY shell integration",
        &format!("source \"{}\"", gpy_bash(home.path())),
    );
    fs::write(&target, &legacy).unwrap();
    std::os::unix::fs::symlink(&target, home.path().join(".bashrc")).unwrap();
    fs::set_permissions(away.path(), fs::Permissions::from_mode(0o555)).unwrap();

    let output = setup(home.path(), &["--shell", "bash"]);
    let after = fs::read_to_string(&target).unwrap();
    fs::set_permissions(away.path(), fs::Permissions::from_mode(0o755)).unwrap();

    assert_ok(&output);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("warn: could not update the GPY block in"),
        "{combined}"
    );
    assert_eq!(after, legacy);
}
