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

fn fish_enabled_segments(env: &CliTestEnv) -> String {
    let export = env
        .run_gpy_agent(&["theme", "export", "--shell", "fish"])
        .expect("Failed to run gpy-agent theme export");
    export.assert_success("gpy-agent theme export --shell fish");
    let Some(line) = export
        .stdout
        .lines()
        .find(|candidate| candidate.starts_with("set -g __enabled_segments"))
    else {
        panic!("no __enabled_segments line: {export:?}");
    };
    line.to_owned()
}

// `ThemeManager::new` falls back to the default theme for unknown names;
// validation must not.

const NONEXISTENT_THEME_NAME: &str = "this-theme-does-not-exist-459";
