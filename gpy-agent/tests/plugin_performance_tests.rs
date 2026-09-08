#![allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]

use gpy_agent::plugin::discover_plugins;
use serial_test::serial;
use std::fs;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn create_plugins(root: &std::path::Path, count: usize) {
    for i in 0..count {
        let id = format!("demo-{i:02}");
        let plugin_dir = root.join(&id);
        fs::create_dir_all(plugin_dir.join("segments")).expect("create plugin dir");
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"id = "{id}"
name = "{id}"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["seg_{i}"]
"#
            ),
        )
        .expect("write manifest");
    }
}

fn run_discovery_with_plugins(plugin_count: usize) -> (Duration, usize, Vec<String>) {
    let xdg = TempDir::new().expect("xdg tempdir");
    let plugins_root = xdg.path().join("gpy").join("plugins");
    fs::create_dir_all(&plugins_root).expect("create plugins root");
    create_plugins(&plugins_root, plugin_count);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", xdg.path());
        std::env::remove_var("GPY_BUNDLED_PLUGIN_DIR");
    }

    let start = Instant::now();
    let discovery = discover_plugins();
    let elapsed = start.elapsed();

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    let diagnostics = discovery
        .diagnostics
        .into_iter()
        .map(|d| d.message)
        .collect::<Vec<_>>();
    (elapsed, discovery.plugins.len(), diagnostics)
}

#[test]
#[serial]
fn plugin_discovery_budget_zero_plugins() {
    let (elapsed, discovered, diagnostics) = run_discovery_with_plugins(0);
    // Includes one first-party metadata plugin.
    assert_eq!(discovered, 1, "diagnostics: {diagnostics:?}");
    assert!(
        elapsed < Duration::from_millis(50),
        "0-plugin discovery should stay below 50ms, got {elapsed:?}"
    );
}

#[test]
#[serial]
fn plugin_discovery_budget_ten_plugins() {
    let (elapsed, discovered, diagnostics) = run_discovery_with_plugins(10);
    // 10 user plugins + first-party metadata plugin.
    assert_eq!(discovered, 11, "diagnostics: {diagnostics:?}");
    assert!(
        elapsed < Duration::from_millis(120),
        "10-plugin discovery should stay below 120ms, got {elapsed:?}"
    );
}

#[test]
#[serial]
fn plugin_discovery_budget_fifty_plugins() {
    let (elapsed, discovered, diagnostics) = run_discovery_with_plugins(50);
    // 50 user plugins + first-party metadata plugin.
    assert_eq!(discovered, 51, "diagnostics: {diagnostics:?}");
    assert!(
        elapsed < Duration::from_millis(300),
        "50-plugin discovery should stay below 300ms, got {elapsed:?}"
    );
}
