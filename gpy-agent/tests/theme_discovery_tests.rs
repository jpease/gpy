//! `ThemeManager::discover_available_themes`: builtin, user and plugin themes
//! and their precedence.
//!
//! These tests point the process-global `XDG_CONFIG_HOME` (and related keys)
//! at temp directories through [`EnvGuard`], and run `#[serial]`.
#![allow(clippy::unwrap_used, reason = "test setup fails loudly")]
#![allow(clippy::expect_used, reason = "test setup fails loudly")]

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::{config::discovery::Source, theme::ThemeManager};
use serial_test::serial;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const THEME: &str = "[segments.clock]\nbg_color = \"blue\"\ntext_color = \"white\"\n";

fn write_user_theme(config_dir: &Path, name: &str) {
    let themes_dir = config_dir.join("themes");
    fs::create_dir_all(&themes_dir).unwrap();
    fs::write(themes_dir.join(format!("{name}.toml")), THEME).unwrap();
}

#[test]
#[serial]
fn test_theme_discovery_includes_starship_builtin() {
    let temp_dir = TempDir::new().unwrap();

    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", temp_dir.path());
    env.remove("GPY_BUNDLED_PLUGIN_DIR");

    let themes = ThemeManager::discover_available_themes();
    let starship = themes
        .iter()
        .find(|theme| theme.name == "starship")
        .expect("starship preset present as builtin");
    assert_eq!(starship.source, Source::Builtin);

    // The other builtins remain discoverable.
    assert!(themes.iter().any(|t| t.name == "default"));
    assert!(themes.iter().any(|t| t.name == "text"));
}

#[test]
#[serial]
fn test_theme_discovery_includes_user_themes() {
    let temp_dir = TempDir::new().unwrap();
    write_user_theme(&temp_dir.path().join("gpy"), "community");

    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", temp_dir.path());
    env.remove("GPY_BUNDLED_PLUGIN_DIR");

    let themes = ThemeManager::discover_available_themes();
    let community = themes
        .iter()
        .find(|theme| theme.name == "community")
        .expect("community theme present");
    assert_eq!(community.source, Source::User);
    assert!(community.path.is_some());
}

#[test]
#[serial]
fn test_theme_discovery_prefers_user_over_plugin() {
    let user_temp = TempDir::new().unwrap();
    let bundled_temp = TempDir::new().unwrap();

    // User theme
    write_user_theme(&user_temp.path().join("gpy"), "shared");

    // Bundled plugin with same theme name
    let plugin_dir = bundled_temp.path().join("demo");
    let themes_dir = plugin_dir.join("themes");
    fs::create_dir_all(&themes_dir).unwrap();
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "demo"
name = "Demo"
version = "0.1.0"
api_version = "v1"
entry_type = "script"
provided_segments = ["demo"]
"#,
    )
    .unwrap();
    fs::create_dir_all(plugin_dir.join("segments")).unwrap();
    fs::write(
        plugin_dir.join("segments/demo.toml"),
        "command = \"run.sh\"\n",
    )
    .unwrap();
    fs::write(plugin_dir.join("run.sh"), "echo demo\n").unwrap();
    fs::write(themes_dir.join("shared.toml"), THEME).unwrap();

    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", user_temp.path());
    env.set("GPY_BUNDLED_PLUGIN_DIR", bundled_temp.path());

    let themes = ThemeManager::discover_available_themes();
    let shared = themes
        .iter()
        .find(|theme| theme.name == "shared")
        .expect("shared theme present");
    assert_eq!(shared.source, Source::User);
}
