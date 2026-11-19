//! Integration tests for the `gpy` CLI binary.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

#[path = "../common/cli_harness.rs"]
mod cli_harness;
use cli_harness::CliTestEnv;
#[cfg(unix)]
#[allow(
    dead_code,
    reason = "this binary uses only the silent-socket helper from the shared fixtures"
)]
#[path = "../common/fixtures.rs"]
mod fixtures;

mod completions;
mod config;
mod doctor;
mod plugin;
mod segments;
mod theme_diff_reset_migrate;
mod theme_new_save_validate;
mod theme_use;

pub(crate) fn run_gpy_command(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let result = CliTestEnv::new()?.run_gpy(args)?;
    result.assert_success(&format!("gpy {}", args.join(" ")));
    Ok(result.stdout)
}

#[test]
fn test_gpy_version() {
    let output = run_gpy_command(&["--version"]).expect("Failed to run gpy --version");
    assert!(
        output.contains(env!("CARGO_PKG_VERSION")),
        "Version output should contain the crate version {}",
        env!("CARGO_PKG_VERSION")
    );
}

/// The segments `gpy segments` marks enabled (`[x] <name> …`), in listed order.
fn enabled_segments(env: &CliTestEnv) -> Vec<String> {
    let listing = env
        .run_gpy(&["segments"])
        .expect("Failed to run gpy segments");
    listing.assert_success("gpy segments");
    listing
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("[x] "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

// `ThemeManager::new` falls back to the default theme for unknown names;
// validation must not.

const NONEXISTENT_THEME_NAME: &str = "this-theme-does-not-exist-459";
