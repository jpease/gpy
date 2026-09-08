#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::default_numeric_fallback)]

use gpy_agent::config::manager::ConfigManager;
use serial_test::serial;
use std::{fs, time::Duration};
use tempfile::TempDir;

// gpy-agent#385: this test bootstraps a real `notify` FSEvents stream via
// `ConfigManager::start_watching`. nextest runs each test as its own OS
// process, so when this test and another FSEvents-bootstrapping test (either
// `test_config_watcher_triggers_reload_on_git_toggle` in
// config_hot_reload_tests.rs, or the #384-fixed tests in watcher_tests.rs)
// happen to start at nearly the same wall-clock instant, one stream can go
// its entire wait budget without ever delivering a first event (confirmed:
// 12/15 concurrent runs of just this test alongside
// `test_config_watcher_triggers_reload_on_git_toggle` failed). Plain
// `#[serial]` only guards same-process interleaving, so it doesn't help here;
// `file_serial` provides the cross-process lock. Reusing #384's
// `watcher_fsevents_bootstrap` group name (rather than a config-scoped name)
// is deliberate: it serializes this test against every other
// FSEvents-bootstrapping test in the crate, not just its two config-watcher
// siblings, which is what the measured failure rate calls for.
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

    let mut changed = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if !manager.get().language.show_versions {
            changed = true;
            break;
        }
    }

    assert!(
        changed,
        "config watcher failed to observe language.show_versions toggle"
    );

    manager.stop_watching();
    drop(manager);
}
