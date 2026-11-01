//! `gpy setup --shell fish` run in place from the data root must still merge
//! the Fish completions: the glue ships in the data root's Fish tree.

#![cfg(unix)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

#[path = "common/setup_harness.rs"]
mod setup_harness;

use setup_harness::{assert_ok, setup};
use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn setup_in_place_fish_merges_completions_from_the_data_root() {
    let home = TempDir::new().unwrap();
    assert_ok(&setup(home.path(), &["--shell", "fish"]));
    let data = home.path().join(".local/share/gpy");
    let merged = home.path().join(".config/fish/completions/gpy.fish");
    assert!(data.join("fish/completions/gpy-dynamic.fish").is_file());
    fs::remove_file(&merged).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_gpy"))
        .args(["setup", "--non-interactive", "--shell", "fish", "--from"])
        .arg(&data)
        .current_dir(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GPY_NERD_FONT", "0")
        .output()
        .expect("failed to spawn gpy setup");

    assert_ok(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("cannot read completion glue"), "{stderr}");
    let text = fs::read_to_string(&merged).unwrap();
    assert!(text.contains("complete -c gpy"), "{text}");
    assert!(text.contains("__gpy_complete_cached"), "{text}");
}
