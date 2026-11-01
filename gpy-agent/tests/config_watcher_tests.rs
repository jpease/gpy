//! `ConfigWatcher` notices on-disk config edits.

use gpy_agent::config::manager::ConfigManager;
use serial_test::serial;
use std::{fs, time::Duration};
use tempfile::TempDir;

#[path = "common/wait.rs"]
mod wait;
use wait::wait_until_async;

// Concurrent FSEvents bootstraps across nextest processes can starve one
// stream of its first event; `file_serial` on the shared group serializes
// against every FSEvents-bootstrapping test in the crate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
async fn config_watcher_detects_language_toggle() {
    let temp_home = TempDir::new().expect("temp home");
    let config_path = temp_home.path().join("config.toml");

    fs::write(
        &config_path,
        r#"[language]
enabled = true
show_versions = true

[ui]
show_icons = false
theme = "default"
directory.max_length = 80
enabled_segments = ["clock","directory","language","git"]
"#,
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("manager");
    manager
        .start_watching(Duration::from_millis(250))
        .expect("start watching");
    assert!(manager.get().language.show_versions);

    fs::write(
        &config_path,
        r#"[language]
enabled = true
show_versions = false

[ui]
show_icons = false
theme = "default"
directory.max_length = 80
enabled_segments = ["clock","directory","language","git"]
"#,
    )
    .expect("write updated config");

    let changed = wait_until_async(Duration::from_secs(6), Duration::from_millis(50), || {
        !manager.get().language.show_versions
    })
    .await;

    assert!(
        changed,
        "config watcher failed to observe language.show_versions toggle"
    );

    manager.stop_watching();
    drop(manager);
}
