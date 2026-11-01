//! Integration tests for `gpy setup --check`: it validates the source tree and
//! the install path, and writes nothing.

#![cfg(unix)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

#[path = "common/setup_harness.rs"]
mod setup_harness;

use setup_harness::{assert_ok, repo_root, setup};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for dirent in fs::read_dir(from).unwrap() {
        let entry = dirent.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A source root holding the three shell trees and nothing else.
fn complete_tree() -> TempDir {
    let source = TempDir::new().unwrap();
    for shell in ["fish", "zsh", "bash"] {
        copy_dir(&repo_root().join(shell), &source.path().join(shell));
    }
    source
}

fn check_from(source: &Path, home: &Path, shell: &str, xdg_data_home: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["setup", "--check", "--shell", shell, "--from"])
        .arg(source)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default());
    // `env_clear` drops the profile path cargo-llvm-cov exports; without it an
    // instrumented child writes `default_*.profraw` into its cwd (the temp HOME).
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        cmd.env("LLVM_PROFILE_FILE", profile);
    }
    if let Some(dir) = xdg_data_home {
        cmd.env("XDG_DATA_HOME", dir);
    }
    cmd.output().expect("failed to spawn gpy setup --check")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn snapshot(root: &Path) -> BTreeSet<PathBuf> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for dirent in fs::read_dir(&dir).unwrap() {
            let path = dirent.unwrap().path();
            if path.is_dir() {
                pending.push(path.clone());
            }
            found.insert(path);
        }
    }
    found
}

#[test]
fn check_passes_for_a_complete_tree() {
    let home = TempDir::new().unwrap();
    let source = complete_tree();
    for shell in ["fish", "zsh", "bash"] {
        let output = check_from(source.path(), home.path(), shell, None);
        assert_ok(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("ok: "), "{stdout}");
        assert!(stdout.contains(shell), "{stdout}");
    }
}

#[test]
fn check_names_the_first_missing_file() {
    let home = TempDir::new().unwrap();
    let cases = [
        ("fish", "fish/core/flags.fish"),
        ("fish", "fish/completions/gpy-dynamic.fish"),
        ("fish", "bash/core/g_runner.bash"),
        ("zsh", "zsh/core/init.zsh"),
        ("bash", "bash/gpy.bash"),
    ];
    for (shell, missing) in cases {
        let source = complete_tree();
        fs::remove_file(source.path().join(missing)).unwrap();

        let output = check_from(source.path(), home.path(), shell, None);

        assert!(!output.status.success(), "{shell} without {missing}");
        let err = stderr(&output);
        assert!(err.contains(&format!("has no {missing}")), "{err}");
        assert!(err.contains("--from"), "{err}");
    }
}

#[test]
fn check_refuses_an_unsourceable_install_path() {
    let home = TempDir::new().unwrap();
    let source = complete_tree();
    for hostile in ["quo\"te", "dol$lar", "back`tick", "back\\slash"] {
        let data_home = home.path().join(hostile);

        let output = check_from(source.path(), home.path(), "bash", Some(&data_home));

        assert!(!output.status.success(), "{hostile} must be refused");
        assert!(stderr(&output).contains("cannot be sourced"), "{hostile}");
    }
}

#[test]
fn check_writes_nothing() {
    let home = TempDir::new().unwrap();
    let source = complete_tree();
    let before = snapshot(home.path());
    let source_before = snapshot(source.path());

    assert_ok(&check_from(source.path(), home.path(), "fish", None));
    assert_ok(&setup(home.path(), &["--check", "--shell", "zsh"]));

    assert_eq!(snapshot(home.path()), before);
    assert_eq!(snapshot(source.path()), source_before);
}
