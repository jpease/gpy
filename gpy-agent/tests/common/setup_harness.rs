//! Helpers for the `gpy setup` integration tests: run the real binary against
//! an isolated HOME with `--from` at the repo's shell trees.

#![allow(
    dead_code,
    reason = "included by several test binaries, each using a subset"
)]
#![allow(clippy::expect_used, reason = "test code may panic on failure")]
#![allow(clippy::unwrap_used, reason = "test code may panic on failure")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate lives inside the repository")
        .to_path_buf()
}

pub fn setup(home: &Path, args: &[&str]) -> Output {
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

pub fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "gpy setup failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn block_count(rc: &Path) -> usize {
    fs::read_to_string(rc)
        .unwrap()
        .matches("# >>> gpy-init >>>")
        .count()
}
