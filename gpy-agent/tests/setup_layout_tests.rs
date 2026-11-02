//! Integration tests for where `gpy setup` installs: the release layout
//! (`fish/`, `bash/`, `zsh/`, `plugins/`) under `$XDG_DATA_HOME/gpy`, found
//! again as a source by a bare `gpy setup`.

#![cfg(unix)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

#[path = "common/setup_harness.rs"]
mod setup_harness;

use setup_harness::{assert_ok, block_count, repo_root, setup};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn data(home: &Path) -> PathBuf {
    home.join(".local/share/gpy")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A `gpy` copied into `<prefix>/bin`, so the build checkout is not one of
/// its source candidates.
fn installed_gpy(prefix: &Path) -> PathBuf {
    let bin = prefix.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let gpy = bin.join("gpy");
    fs::copy(env!("CARGO_BIN_EXE_gpy"), &gpy).unwrap();
    gpy
}

/// `gpy setup --non-interactive` from `program` in an isolated HOME with
/// `extra_env` added.
fn run_setup(
    program: &Path,
    home: &Path,
    args: &[&std::ffi::OsStr],
    extra_env: &[(&str, &Path)],
) -> Output {
    let mut cmd = Command::new(program);
    cmd.args(["setup", "--non-interactive"])
        .args(args)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GPY_NERD_FONT", "0");
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    cmd.output().expect("failed to spawn gpy setup")
}

#[test]
fn fresh_setup_uses_the_data_root() {
    let home = TempDir::new().unwrap();

    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    let root = data(home.path());
    for tree in ["fish", "bash", "zsh", "plugins"] {
        assert!(
            root.join(tree).is_dir(),
            "{tree} is installed in the data root"
        );
    }
    for old in ["bash", "zsh", "bundled-plugins"] {
        assert!(
            !home.path().join(".config/gpy").join(old).exists(),
            "nothing named {old} under the config root"
        );
    }
    assert!(!home.path().join(".config/fish/gpy").exists());
    assert!(
        !home
            .path()
            .join(".config/fish/completions/gpy.fish")
            .exists(),
        "no Fish completions for a Zsh setup"
    );
}

#[test]
fn custom_xdg_data_home_is_found_by_a_bare_setup() {
    let home = TempDir::new().unwrap();
    let prefix = TempDir::new().unwrap();
    let gpy = installed_gpy(prefix.path());
    let xdg_data = home.path().join("data");
    let env = [("XDG_DATA_HOME", xdg_data.as_path())];

    assert_ok(&run_setup(
        &gpy,
        home.path(),
        &[
            "--shell".as_ref(),
            "zsh".as_ref(),
            "--from".as_ref(),
            repo_root().as_os_str(),
        ],
        &env,
    ));
    assert!(xdg_data.join("gpy/zsh/gpy.zsh").is_file());
    assert!(!data(home.path()).exists());

    assert_ok(&run_setup(
        &gpy,
        home.path(),
        &["--shell".as_ref(), "bash".as_ref()],
        &env,
    ));
    assert!(xdg_data.join("gpy/bash/gpy.bash").is_file());
    assert!(xdg_data.join("gpy/bash/completions/gpy.bash").is_file());
}

#[test]
fn a_relative_xdg_data_home_is_ignored() {
    let home = TempDir::new().unwrap();
    let relative = Path::new("relative-data");
    let output = run_setup(
        Path::new(env!("CARGO_BIN_EXE_gpy")),
        home.path(),
        &[
            "--shell".as_ref(),
            "zsh".as_ref(),
            "--from".as_ref(),
            repo_root().as_os_str(),
        ],
        &[("XDG_DATA_HOME", relative)],
    );
    assert_ok(&output);
    assert!(data(home.path()).join("zsh/gpy.zsh").is_file());
    assert!(!home.path().join("relative-data").exists());
}

#[test]
fn setup_in_place_from_the_data_root_copies_nothing() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    let root = data(home.path());
    let entry = root.join("zsh/gpy.zsh");
    let mut edited = fs::read_to_string(&entry).unwrap();
    edited.push_str("# edited in place\n");
    fs::write(&entry, &edited).unwrap();
    fs::remove_file(root.join("zsh/.gpy-version")).unwrap();
    fs::remove_file(root.join("zsh/completions/_gpy")).unwrap();

    let output = run_setup(
        Path::new(env!("CARGO_BIN_EXE_gpy")),
        home.path(),
        &[
            "--shell".as_ref(),
            "zsh".as_ref(),
            "--from".as_ref(),
            root.as_os_str(),
        ],
        &[],
    );

    assert_ok(&output);
    assert_eq!(
        fs::read_to_string(&entry).unwrap(),
        edited,
        "nothing was copied"
    );
    assert!(root.join("zsh/.gpy-version").is_file());
    assert!(root.join("zsh/completions/_gpy").is_file());
    assert_eq!(block_count(&home.path().join(".zshrc")), 1);
}

#[test]
fn checkout_build_prefers_the_checkout_over_the_data_root() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));
    let entry = data(home.path()).join("zsh/gpy.zsh");
    fs::write(&entry, "# stale copy\n").unwrap();

    let output = run_setup(
        Path::new(env!("CARGO_BIN_EXE_gpy")),
        home.path(),
        &["--shell".as_ref(), "zsh".as_ref()],
        &[],
    );

    assert_ok(&output);
    assert_eq!(
        fs::read(&entry).unwrap(),
        fs::read(repo_root().join("zsh/gpy.zsh")).unwrap(),
        "the checkout's files replaced the stale data root copy"
    );
}

#[test]
fn completions_only_for_wired_shells() {
    let home = TempDir::new().unwrap();
    let h = home.path();
    assert_ok(&setup(h, &["--shell", "zsh"]));
    let root = data(h);
    assert!(root.join("zsh/completions/_gpy").is_file());
    assert!(!root.join("bash/completions/gpy.bash").exists());
    assert!(!h.join(".config/fish/completions/gpy.fish").exists());

    assert_ok(&setup(h, &["--shell", "bash"]));
    fs::remove_file(root.join("bash/completions/gpy.bash")).unwrap();
    assert_ok(&setup(h, &["--shell", "zsh"]));

    assert!(
        root.join("bash/completions/gpy.bash").is_file(),
        "Bash is wired now, so a Zsh run refreshes its completions"
    );
    assert!(!h.join(".config/fish/completions/gpy.fish").exists());
}

#[test]
fn doctor_lists_shells_by_rc_block() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "zsh"]));

    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .arg("doctor")
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("failed to spawn gpy doctor");

    let report = stdout(&output);
    assert!(report.contains("Checking zsh integration"), "{report}");
    assert!(!report.contains("Checking bash integration"), "{report}");
    assert!(!report.contains("Checking fish integration"), "{report}");
}
