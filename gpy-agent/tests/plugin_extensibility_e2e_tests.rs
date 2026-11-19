//! End-to-end plugin tests: discovery, theme values, and segment token collisions.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns panic on failure"
)]

use gpy_agent::config::Config;
use gpy_agent::config::discovery::Source;
use gpy_agent::theme::ThemeManager;
use serial_test::serial;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use std::assert_matches;

fn write_config(root: &Path, theme: &str, segments: &[&str]) {
    let cfg_dir = root.join("gpy");
    fs::create_dir_all(&cfg_dir).expect("create config dir");
    let list = segments
        .iter()
        .map(|segment| format!("\"{segment}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let content = format!(
        r#"[agent]
enabled = true

[git]

[language]

[ui]
theme = "{theme}"
enabled_segments = [{list}]
"#
    );
    fs::write(cfg_dir.join("config.toml"), content).expect("write config");
}

/// Install script plugin `id` providing `segment` under `<xdg>/gpy/plugins`;
/// returns its root.
fn install_script_plugin(xdg: &Path, id: &str, segment: &str) -> std::path::PathBuf {
    let root = xdg.join("gpy").join("plugins").join(id);
    fs::create_dir_all(root.join("segments")).expect("create plugin dirs");
    fs::write(
        root.join("plugin.toml"),
        format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
             entry_type = \"script\"\nprovided_segments = [\"{segment}\"]\n"
        ),
    )
    .expect("write manifest");
    fs::write(
        root.join("segments").join(format!("{segment}.toml")),
        "command = \"run.sh\"\n",
    )
    .expect("write segment manifest");
    fs::write(root.join("run.sh"), "echo PLUGIN_SEGMENT\n").expect("write script");
    root
}

#[test]
#[serial]
fn e2e_plugin_segment_and_theme_discovery_and_values() {
    let xdg = TempDir::new().expect("xdg tempdir");
    let plugin_dir = install_script_plugin(xdg.path(), "k8s-tools", "k8s_tools");
    fs::create_dir_all(plugin_dir.join("themes")).expect("create plugin themes dir");
    fs::write(
        plugin_dir.join("themes").join("community.toml"),
        r##"[ui]

[segments.character]
success_symbol = ">"

[segments.clock]
bg_color = "black"
text_color = "white"

[segments.k8s_tools]
bg_color = "#073642"
text_color = "white"
icon = "⎈"
"##,
    )
    .expect("write plugin theme");
    write_config(xdg.path(), "community", &["clock", "k8s_tools"]);

    let mut env = EnvGuard::set_one("XDG_CONFIG_HOME", xdg.path());
    env.set("XDG_DATA_HOME", xdg.path());
    env.remove("GPY_BUNDLED_PLUGIN_DIR");

    let themes = ThemeManager::discover_available_themes();
    let community = themes
        .iter()
        .find(|theme| theme.name == "community")
        .expect("plugin theme should be discovered");
    assert_matches!(community.source, Source::Plugin { .. });
    let manager = ThemeManager::new("community").expect("load plugin theme");
    let values = gpy_agent::theme::export::prompt_shell_values(
        &manager.get(),
        &Config::default(),
        &gpy_agent::template::Palette::default(),
    );

    assert_eq!(
        values
            .get("__gpy_segment_k8s_tools_bg_color")
            .map(String::as_str),
        Some("#073642")
    );
}

/// Run the real `gpy` CLI with `HOME` and the XDG roots inside `root`.
fn gpy(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gpy"))
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_RUNTIME_DIR", root.join("runtime"))
        .env_remove("GPY_BUNDLED_PLUGIN_DIR")
        .args(args)
        .output()
        .expect("run gpy")
}

#[test]
fn e2e_colliding_segment_tokens_are_rejected_not_redirected() {
    let root = TempDir::new().expect("sandbox tempdir");
    let xdg = root.path().join("config");
    install_script_plugin(&xdg, "ctx-a", "k8s-ctx");
    install_script_plugin(&xdg, "ctx-b", "k8s_ctx");

    let list = gpy(root.path(), &["plugin", "list"]);
    assert!(
        list.status.success(),
        "gpy plugin list failed\nstderr:\n{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let listing = String::from_utf8_lossy(&list.stdout);
    assert!(
        listing.contains("- ctx-a") && !listing.contains("- ctx-b"),
        "the later colliding plugin must not be registered:\n{listing}"
    );
    assert!(
        listing.contains("k8s-ctx") && listing.contains("k8s_ctx"),
        "a diagnostic must name both colliding segments:\n{listing}"
    );

    write_config(&xdg, "default", &["k8s-ctx", "k8s_ctx"]);
    let get = gpy(root.path(), &["config", "get", "ui.enabled_segments"]);
    assert!(
        !get.status.success(),
        "config enabling both colliding names must not load:\nstdout:\n{}",
        String::from_utf8_lossy(&get.stdout)
    );
    let stderr = String::from_utf8_lossy(&get.stderr);
    assert!(
        stderr.contains("k8s-ctx") && stderr.contains("k8s_ctx"),
        "error must name both segments:\n{stderr}"
    );
}
