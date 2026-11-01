//! Theme overlays (ADR-0012) through every load entry point: the daemon's
//! `ThemeManager`, the `oneshot` loader, and the CLI (`gpy theme validate`,
//! `gpy theme use`, `gpy-agent oneshot`).
//!
//! The in-process tests point `XDG_CONFIG_HOME` at a temp dir through
//! [`EnvGuard`] and run `#[serial]`.
#![allow(
    clippy::unwrap_used,
    reason = "test code: a failed unwrap is the failure"
)]
#![allow(
    clippy::expect_used,
    reason = "test code: a failed expect is the failure"
)]

#[path = "common/cli_harness.rs"]
mod cli_harness;
#[path = "common/env_guard.rs"]
mod env_guard;

use cli_harness::CliTestEnv;
use env_guard::EnvGuard;
use gpy_agent::config::defaults::STARSHIP_THEME_CONTENT;
use gpy_agent::config::loader;
use gpy_agent::theme::ThemeManager;
use serial_test::serial;
use std::fs;
use tempfile::TempDir;

/// A hand-written self-named overlay of the `starship` builtin.
const OVERLAY: &str = "\
# Mine: Starship with a marked directory.
extends = \"starship\"

[segments.directory]
format = \"[OVL $path](bold cyan) \"
";

const OVERLAY_FORMAT: &str = "[OVL $path](bold cyan) ";

/// A temp config root holding `themes/starship.toml` = [`OVERLAY`], and the
/// guard pointing `XDG_CONFIG_HOME` at it with no bundled plugins.
fn overlay_config() -> (TempDir, EnvGuard) {
    let temp = TempDir::new().unwrap();
    let themes = temp.path().join("gpy").join("themes");
    fs::create_dir_all(&themes).unwrap();
    fs::write(themes.join("starship.toml"), OVERLAY).unwrap();
    let plugins = temp.path().join("no-plugins");
    fs::create_dir_all(&plugins).unwrap();
    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", temp.path());
    env.set("GPY_BUNDLED_PLUGIN_DIR", &plugins);
    (temp, env)
}

#[test]
#[serial]
fn daemon_theme_manager_loads_a_self_named_overlay() {
    let (_temp, _env) = overlay_config();
    let builtin = gpy_agent::theme::parse(STARSHIP_THEME_CONTENT, "starship").unwrap();

    let manager = ThemeManager::new("starship").expect("overlay loads in the daemon");
    let theme = manager.get();

    assert_eq!(
        theme.segments.directory.format.as_deref(),
        Some(OVERLAY_FORMAT)
    );
    assert_eq!(theme.segments.git.format, builtin.segments.git.format);
    manager.reload().expect("reloading re-reads the chain");
}

#[test]
#[serial]
fn oneshot_loader_loads_a_self_named_overlay() {
    let (_temp, _env) = overlay_config();

    let loaded = loader::load_named_theme("starship").expect("overlay loads for oneshot");

    assert_eq!(
        loaded.chain.to_string(),
        "starship (user) → starship (builtin)"
    );
    assert_eq!(
        loaded.theme.segments.directory.format.as_deref(),
        Some(OVERLAY_FORMAT)
    );
    let theme = loader::load_theme("starship").expect("load_theme");
    assert_eq!(
        theme.segments.directory.format.as_deref(),
        Some(OVERLAY_FORMAT)
    );
}

#[test]
#[serial]
fn builtin_constructor_ignores_a_user_overlay() {
    let (_temp, _env) = overlay_config();

    let manager = ThemeManager::builtin("starship").expect("builtin loads");

    assert_ne!(
        manager.get().segments.directory.format.as_deref(),
        Some(OVERLAY_FORMAT)
    );
}

/// An isolated CLI env whose active theme is the `starship` overlay.
fn cli_env() -> CliTestEnv {
    let env =
        CliTestEnv::with_config("[ui]\ntheme = \"starship\"\nenabled_segments = [\"directory\"]\n")
            .expect("create CLI env");
    fs::create_dir_all(env.themes_dir()).expect("themes dir");
    fs::write(env.themes_dir().join("starship.toml"), OVERLAY).expect("write overlay");
    env
}

#[test]
fn cli_validates_uses_and_renders_a_self_named_overlay() {
    let env = cli_env();

    env.run_gpy(&["theme", "validate", "starship"])
        .expect("run gpy theme validate")
        .assert_success("gpy theme validate starship");
    let path = env.themes_dir().join("starship.toml");
    env.run_gpy(&["theme", "validate", path.to_str().expect("utf-8 path")])
        .expect("run gpy theme validate <path>")
        .assert_success("gpy theme validate <path>");
    env.run_gpy(&["theme", "use", "starship"])
        .expect("run gpy theme use")
        .assert_success("gpy theme use starship");

    let cwd = env.root().join("fixture");
    fs::create_dir_all(&cwd).expect("fixture dir");
    let rendered = env
        .run_gpy_agent(&[
            "oneshot",
            "directory",
            "--cwd",
            cwd.to_str().expect("utf-8 path"),
            "--format",
            "ansi",
        ])
        .expect("run gpy-agent oneshot");
    rendered.assert_success("gpy-agent oneshot directory");
    assert!(
        rendered.stdout.contains("OVL") && rendered.stdout.contains("fixture"),
        "the overlay's directory format renders: {:?}",
        rendered.stdout
    );
}

#[test]
fn cli_names_the_overlay_line_of_a_bad_key() {
    let env = cli_env();
    let path = env.themes_dir().join("mine.toml");
    fs::write(
        &path,
        "extends = \"starship\"\n\n[segments.username]\nformat = \"$user\"\n",
    )
    .expect("write broken overlay");

    let result = env
        .run_gpy(&["theme", "validate", "mine"])
        .expect("run gpy theme validate");
    let output = format!("{}{}", result.stdout, result.stderr);

    assert_ne!(result.exit_code, 0_i32, "{output}");
    assert!(
        output.contains("segments.username.format")
            && output.contains(&format!("{}:4:1", path.display())),
        "names the key and the overlay line: {output}"
    );
    // `mine` extends the env's user `starship` overlay, which extends the builtin.
    assert!(
        output.contains("(chain: mine (user) → starship (user) → starship (builtin))"),
        "names the chain: {output}"
    );
}

#[test]
fn cli_names_the_overlay_line_of_a_bad_template() {
    let env = cli_env();
    let path = env.themes_dir().join("mine.toml");
    fs::write(
        &path,
        "extends = \"starship\"\n\n[segments.git]\nformat = \"[$branch](fg:nonsuch)\"\n",
    )
    .expect("write overlay with a bad template");

    for args in [["theme", "validate", "mine"], ["theme", "use", "mine"]] {
        let result = env.run_gpy(&args).expect("run gpy");
        let output = format!("{}{}", result.stdout, result.stderr);

        assert_ne!(result.exit_code, 0_i32, "{args:?}: {output}");
        assert!(
            output.contains("segments.git.format")
                && output.contains(&format!("{}:4:1", path.display())),
            "{args:?}: names the key and the overlay line: {output}"
        );
    }
}
