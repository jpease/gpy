//! Integration tests for `gpy-agent init` (first-run icon/config bootstrap, #411).
//!
//! These drive the real binary end-to-end, injecting the detected font
//! capability with `GPY_NERD_FONT` so the detection→default-selection logic is
//! covered deterministically without depending on the host's installed fonts.

#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::doc_markdown)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

/// Run `gpy-agent init --non-interactive` against an isolated, empty HOME.
///
/// `GPY_NERD_FONT` forces the detected capability. `config_path` is where the command should
/// write (via `GPY_CONFIG_PATH`); it does not exist yet.
fn run_init(home: &std::path::Path, config_path: &std::path::Path, nerd_font: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gpy-agent"))
        .args(["init", "--non-interactive"])
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GPY_CONFIG_PATH", config_path)
        .env("GPY_NERD_FONT", nerd_font)
        .output()
        .expect("failed to spawn gpy-agent init")
}

fn show_icons_value(config_path: &std::path::Path) -> Option<bool> {
    let contents = fs::read_to_string(config_path).ok()?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("show_icons") {
            let value = rest.trim_start_matches([' ', '=']).trim();
            return match value {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
        }
    }
    None
}

struct InitEnv {
    _tmp: TempDir,
    home: PathBuf,
    config_path: PathBuf,
}

impl InitEnv {
    fn new() -> Self {
        let tmp = TempDir::new().expect("temp dir");
        let home = tmp.path().to_path_buf();
        let config_path = home.join("gpy-config.toml");
        Self {
            _tmp: tmp,
            home,
            config_path,
        }
    }

    fn run(&self, nerd_font: &str) -> Output {
        run_init(&self.home, &self.config_path, nerd_font)
    }
}

#[test]
fn fresh_install_without_nerd_font_defaults_to_ascii() {
    let env = InitEnv::new();
    let output = env.run("none");

    assert!(
        output.status.success(),
        "init should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        show_icons_value(&env.config_path),
        Some(false),
        "a machine with no Nerd Font must default to ASCII so the first prompt is not tofu"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("show_icons true"),
        "ASCII default must surface the opt-in command, got: {stdout}"
    );
}

#[test]
fn fresh_install_with_unknown_capability_defaults_to_ascii() {
    let env = InitEnv::new();
    let output = env.run("unknown");

    assert!(output.status.success());
    assert_eq!(
        show_icons_value(&env.config_path),
        Some(false),
        "an undetectable machine must take the safe ASCII default"
    );
}

#[test]
fn fresh_install_with_nerd_font_keeps_glyph_default() {
    let env = InitEnv::new();
    let output = env.run("nerd");

    assert!(output.status.success());
    assert_eq!(
        show_icons_value(&env.config_path),
        Some(true),
        "a detected Nerd Font must keep the glyph default"
    );
}

#[test]
fn existing_config_is_left_untouched() {
    let env = InitEnv::new();

    // Simulate an existing user whose config already has icons ON.
    let sentinel = "# my hand-edited config\n[ui]\nshow_icons = true\n";
    fs::write(&env.config_path, sentinel).expect("seed config");

    // Even forcing "no Nerd Font", the existing file must not be rewritten.
    let output = env.run("none");
    assert!(output.status.success());

    let after = fs::read_to_string(&env.config_path).expect("read config");
    assert_eq!(
        after, sentinel,
        "an existing config must be left byte-for-byte unchanged"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("already exists"),
        "init should report it left the existing config alone, got: {stdout}"
    );
}

#[test]
fn force_overwrites_existing_config() {
    let env = InitEnv::new();
    fs::write(&env.config_path, "[ui]\nshow_icons = true\n").expect("seed config");

    let output = Command::new(env!("CARGO_BIN_EXE_gpy-agent"))
        .args(["init", "--non-interactive", "--force"])
        .current_dir(&env.home)
        .env("HOME", &env.home)
        .env("XDG_CONFIG_HOME", env.home.join(".config"))
        .env("GPY_CONFIG_PATH", &env.config_path)
        .env("GPY_NERD_FONT", "none")
        .output()
        .expect("failed to spawn gpy-agent init --force");

    assert!(output.status.success());
    assert_eq!(
        show_icons_value(&env.config_path),
        Some(false),
        "--force must rewrite the config with the detected default"
    );
}
