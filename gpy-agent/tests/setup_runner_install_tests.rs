//! `gpy setup` installs the one committed runner (`bash/core/g_runner.bash`)
//! once, in the Bash tree; Fish and Zsh find it beside their own trees.

#![cfg(unix)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

#[path = "common/setup_harness.rs"]
mod setup_harness;

use setup_harness::{assert_ok, repo_root, setup};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const RUNNER: &str = "bash/core/g_runner.bash";

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for item in fs::read_dir(from).unwrap() {
        let entry = item.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// `gpy setup --shell <shell> --from <source>` in an isolated HOME.
fn setup_from(home: &Path, source: &Path, shell: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--non-interactive", "--shell", shell, "--from"])
        .arg(source)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GPY_NERD_FONT", "0")
        .output()
        .expect("failed to spawn gpy setup")
}

/// A copy of the shell trees without the runner.
fn source_without_runner() -> TempDir {
    let source = TempDir::new().unwrap();
    for tree in ["fish", "zsh", "bash", "plugins"] {
        copy_dir(&repo_root().join(tree), &source.path().join(tree));
    }
    fs::remove_file(source.path().join(RUNNER)).unwrap();
    source
}

/// Names of the files below `dir` (relative, sorted), including hidden ones.
fn tree_listing(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            out.push(path.strip_prefix(root).unwrap().display().to_string());
            if path.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn fish_and_zsh_setup_leave_one_runner_in_the_bash_tree() {
    let source_runner = fs::read(repo_root().join(RUNNER)).unwrap();
    for shell in ["fish", "zsh"] {
        let home = TempDir::new().unwrap();
        assert_ok(&setup(home.path(), &["--shell", shell]));
        let data = home.path().join(".local/share/gpy");
        assert_eq!(
            fs::read(data.join(RUNNER)).unwrap(),
            source_runner,
            "{shell}: installed runner differs from {RUNNER}"
        );
        for tree in ["fish", "zsh"] {
            assert!(
                !data.join(tree).join("core/g_runner.bash").exists(),
                "{shell}: the {tree} tree must not carry its own runner"
            );
        }
    }
}

#[test]
fn a_source_without_the_runner_is_refused_before_anything_is_written() {
    for shell in ["fish", "zsh"] {
        let home = TempDir::new().unwrap();
        assert_ok(&setup(home.path(), &["--shell", shell]));
        let before = tree_listing(home.path());
        let installed = home.path().join(".local/share/gpy").join(RUNNER);
        let runner_before = fs::read(&installed).unwrap();

        let source = source_without_runner();
        let output = setup_from(home.path(), source.path(), shell);

        assert!(!output.status.success(), "{shell}: setup must fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("has no bash/core/g_runner.bash"),
            "{shell}: {stderr}"
        );
        assert_eq!(fs::read(&installed).unwrap(), runner_before);
        assert_eq!(
            tree_listing(home.path()),
            before,
            "{shell}: files left behind"
        );
    }
}
