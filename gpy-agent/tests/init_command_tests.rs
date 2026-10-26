//! Icon-style bootstrap: `gpy setup --redetect-icons` and the first-install
//! path of `gpy setup`. `GPY_NERD_FONT` pins the detected capability.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::doc_markdown)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

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

    fn gpy(&self, args: &[&str], nerd_font: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_gpy"))
            .args(args)
            .current_dir(&self.home)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GPY_CONFIG_PATH", &self.config_path)
            .env("GPY_NERD_FONT", nerd_font)
            .output()
            .expect("failed to spawn gpy")
    }

    /// `gpy setup --redetect-icons --non-interactive`.
    fn redetect(&self, nerd_font: &str) -> Output {
        self.gpy(
            &["setup", "--redetect-icons", "--non-interactive"],
            nerd_font,
        )
    }
}

#[test]
fn no_nerd_font_defaults_to_ascii() {
    let env = InitEnv::new();
    let output = env.redetect("none");

    assert!(
        output.status.success(),
        "redetect should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(show_icons_value(&env.config_path), Some(false));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("show_icons true"),
        "ASCII default must surface the opt-in command, got: {stdout}"
    );
}

#[test]
fn unknown_capability_defaults_to_ascii() {
    let env = InitEnv::new();
    assert!(env.redetect("unknown").status.success());
    assert_eq!(show_icons_value(&env.config_path), Some(false));
}

#[test]
fn nerd_font_keeps_glyph_default() {
    let env = InitEnv::new();
    assert!(env.redetect("nerd").status.success());
    assert_eq!(show_icons_value(&env.config_path), Some(true));
}

#[test]
fn redetect_overwrites_only_show_icons() {
    let env = InitEnv::new();
    fs::write(
        &env.config_path,
        "# my hand-edited config\n[ui]\nshow_icons = true\ntheme = \"starship\"\n",
    )
    .expect("seed config");

    let output = env.redetect("none");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = fs::read_to_string(&env.config_path).expect("read config");
    assert_eq!(show_icons_value(&env.config_path), Some(false), "{after}");
    assert!(after.contains("# my hand-edited config"), "{after}");
    assert!(after.contains("theme = \"starship\""), "{after}");
}

#[test]
fn redetect_conflicts_with_shell_and_from() {
    let env = InitEnv::new();
    for extra in [["--shell", "zsh"], ["--from", "."]] {
        let mut args = vec!["setup", "--redetect-icons"];
        args.extend(extra);
        let output = env.gpy(&args, "none");
        assert_eq!(output.status.code(), Some(2_i32), "{output:?}");
    }
}

#[cfg(unix)]
#[test]
fn setup_leaves_an_existing_config_untouched() {
    let env = InitEnv::new();
    let sentinel = "# my hand-edited config\n[ui]\nshow_icons = true\n";
    fs::write(&env.config_path, sentinel).expect("seed config");

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate lives inside the repository")
        .to_string_lossy()
        .into_owned();
    let output = env.gpy(
        &[
            "setup",
            "--non-interactive",
            "--shell",
            "zsh",
            "--from",
            &repo_root,
        ],
        "none",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = fs::read_to_string(&env.config_path).expect("read config");
    assert_eq!(after, sentinel, "an existing config must be left unchanged");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("already exists"), "{stdout}");
}
