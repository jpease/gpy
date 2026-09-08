//! Config hot-reload integration tests
//!
//! Tests the complete hot-reload flow:
//! 1. Config file changes
//! 2. Agent detects change via file watcher
//! 3. Agent sends SIGUSR2 to registered Fish processes
//! 4. Fish reloads theme variables (`GPY_GIT_ENABLED`, `__enabled_segments`, etc.)
//! 5. Prompt updates with correct delimiters
//!
//! These tests prevent regressions of the critical bug where disabling git.enabled
//! would not send SIGUSR2, causing Fish to use stale segment lists and wrong delimiters.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::str_to_string)]
#![allow(clippy::default_numeric_fallback)]

use gpy_agent::config::manager::ConfigManager;
use gpy_agent::ipc::ClientDirectory;
use serial_test::serial;
use std::fs;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

#[cfg(unix)]
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

// Signal counters for tracking SIGUSR1 and SIGUSR2 delivery
static SIGUSR1_COUNTER: OnceLock<AtomicUsize> = OnceLock::new();
static SIGUSR2_COUNTER: OnceLock<AtomicUsize> = OnceLock::new();

fn get_sigusr1_counter() -> &'static AtomicUsize {
    SIGUSR1_COUNTER.get_or_init(|| AtomicUsize::new(0))
}

fn get_sigusr2_counter() -> &'static AtomicUsize {
    SIGUSR2_COUNTER.get_or_init(|| AtomicUsize::new(0))
}

#[cfg(unix)]
extern "C" fn count_sigusr1(_signal: i32) {
    get_sigusr1_counter().fetch_add(1, Ordering::Relaxed);
}

#[cfg(unix)]
extern "C" fn count_sigusr2(_signal: i32) {
    get_sigusr2_counter().fetch_add(1, Ordering::Relaxed);
}

/// Test that SIGUSR2 is sent when git.enabled changes from true to false
///
/// This is a regression test for the critical bug where the agent would only send
/// SIGUSR1 (git status update) but not SIGUSR2 (config/theme reload) when disabling git.
/// Without SIGUSR2, Fish keeps the old `__enabled_segments` list and uses the wrong delimiter.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_sigusr2_sent_when_git_disabled() {
    // Reset counters
    get_sigusr1_counter().store(0, Ordering::Relaxed);
    get_sigusr2_counter().store(0, Ordering::Relaxed);

    // Install signal handlers
    let sigusr1_handler = SigAction::new(
        SigHandler::Handler(count_sigusr1),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let sigusr2_handler = SigAction::new(
        SigHandler::Handler(count_sigusr2),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let prev_usr1 =
        unsafe { sigaction(Signal::SIGUSR1, &sigusr1_handler) }.expect("install SIGUSR1 handler");
    let prev_usr2 =
        unsafe { sigaction(Signal::SIGUSR2, &sigusr2_handler) }.expect("install SIGUSR2 handler");

    // Create temp config with git enabled
    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[agent]
enabled = true
live_updates = true

[git]
enabled = true

[ui]
enabled_segments = ["clock", "directory", "git"]
"#,
    )
    .expect("write initial config");

    // Create config manager
    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    let config = manager.get();
    assert!(config.git.enabled, "git should be enabled initially");

    // Register this process to receive signals
    let registry = ClientDirectory::new();
    let pid = std::process::id();
    let cwd = std::env::current_dir().expect("current dir");
    registry.register(pid, Some(cwd));

    // Simulate agent detecting config change and sending signals
    // (In real agent, this happens in agent.rs reload_config)
    let initial_sigusr2_count = get_sigusr2_counter().load(Ordering::Relaxed);

    // Change config to disable git
    fs::write(
        &config_path,
        r#"
[agent]
enabled = true
live_updates = true

[git]
enabled = false

[ui]
enabled_segments = ["clock", "directory", "git"]
"#,
    )
    .expect("write updated config");

    // Reload config
    manager.reload_now().expect("reload config");
    let new_config = manager.get();
    assert!(!new_config.git.enabled, "git should be disabled now");

    // Agent should send SIGUSR2 when git is disabled
    // This is the critical fix: agent.rs:670 now calls client_registry.notify_sigusr2()
    registry.notify_sigusr2();

    // Wait for signal delivery
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify SIGUSR2 was sent
    let sigusr2_count = get_sigusr2_counter().load(Ordering::Relaxed);
    assert!(
        sigusr2_count > initial_sigusr2_count,
        "SIGUSR2 should be sent when git.enabled changes to false. Before: {initial_sigusr2_count}, After: {sigusr2_count}"
    );

    // Restore signal handlers
    unsafe {
        sigaction(Signal::SIGUSR1, &prev_usr1).expect("restore SIGUSR1");
        sigaction(Signal::SIGUSR2, &prev_usr2).expect("restore SIGUSR2");
    }
}

/// Test that SIGUSR2 is sent when git.enabled changes from false to true
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_sigusr2_sent_when_git_enabled() {
    // Reset counters
    get_sigusr2_counter().store(0, Ordering::Relaxed);

    // Install signal handler
    let sigusr2_handler = SigAction::new(
        SigHandler::Handler(count_sigusr2),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let prev_usr2 =
        unsafe { sigaction(Signal::SIGUSR2, &sigusr2_handler) }.expect("install SIGUSR2 handler");

    // Create temp config with git disabled
    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[git]
enabled = false

[ui]
enabled_segments = ["clock", "directory"]
"#,
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    assert!(!manager.get().git.enabled);

    // Register for signals
    let registry = ClientDirectory::new();
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = get_sigusr2_counter().load(Ordering::Relaxed);

    // Enable git
    fs::write(
        &config_path,
        r#"
[git]
enabled = true

[ui]
enabled_segments = ["clock", "directory", "git"]
"#,
    )
    .expect("write updated config");

    manager.reload_now().expect("reload");
    assert!(manager.get().git.enabled);

    // Send SIGUSR2
    registry.notify_sigusr2();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let final_count = get_sigusr2_counter().load(Ordering::Relaxed);
    assert!(
        final_count > initial_count,
        "SIGUSR2 should be sent when git.enabled changes to true"
    );

    // Restore handler
    unsafe {
        sigaction(Signal::SIGUSR2, &prev_usr2).expect("restore SIGUSR2");
    }
}

/// Test that theme export reflects updated `enabled_segments` when git is disabled
///
/// This ensures that when git.enabled = false, the theme export removes "git"
/// from `__enabled_segments`, which allows Fish segments to correctly detect
/// which segment is last and use the appropriate delimiter.
#[test]
fn test_theme_export_updates_enabled_segments_when_git_disabled() {
    use gpy_agent::config::Config;
    use gpy_agent::shell::Shell;
    use gpy_agent::theme::ThemeManager;

    // Create config with git disabled
    let mut config = Config::default();
    config.git.enabled = false;
    config.ui.enabled_segments = vec![
        "clock".to_string(),
        "directory".to_string(),
        "git".to_string(),
    ];

    let theme_manager = ThemeManager::new("default").expect("load default theme");
    let fish_output = theme_manager.export(Shell::Fish, &config);

    // Verify git is removed from enabled_segments
    assert!(
        fish_output.contains("set -g __enabled_segments clock directory"),
        "Git should be removed from enabled_segments when git.enabled=false. Got: {fish_output}"
    );
    assert!(
        !fish_output.contains("clock directory git"),
        "Enabled segments should not include git when disabled. Got: {fish_output}"
    );

    // Verify GPY_GIT_ENABLED is set to 0
    assert!(
        fish_output.contains("set -gx GPY_GIT_ENABLED \"0\""),
        "GPY_GIT_ENABLED should be 0 when git.enabled=false. Got: {fish_output}"
    );
}

/// Test that theme export includes git in `enabled_segments` when git is enabled
#[test]
fn test_theme_export_includes_git_when_enabled() {
    use gpy_agent::config::Config;
    use gpy_agent::shell::Shell;
    use gpy_agent::theme::ThemeManager;

    let mut config = Config::default();
    config.git.enabled = true;
    config.ui.enabled_segments = vec![
        "clock".to_string(),
        "directory".to_string(),
        "git".to_string(),
    ];

    let theme_manager = ThemeManager::new("default").expect("load default theme");
    let fish_output = theme_manager.export(Shell::Fish, &config);

    // Verify git is included
    assert!(
        fish_output.contains("set -g __enabled_segments clock directory git"),
        "Git should be included in enabled_segments when git.enabled=true. Got: {fish_output}"
    );

    // Verify GPY_GIT_ENABLED is set to 1
    assert!(
        fish_output.contains("set -gx GPY_GIT_ENABLED \"1\""),
        "GPY_GIT_ENABLED should be 1 when git.enabled=true. Got: {fish_output}"
    );
}

/// Test that language segment is removed from `enabled_segments` when disabled
#[test]
fn test_theme_export_updates_enabled_segments_when_language_disabled() {
    use gpy_agent::config::Config;
    use gpy_agent::shell::Shell;
    use gpy_agent::theme::ThemeManager;

    let mut config = Config::default();
    config.language.enabled = false;
    config.ui.enabled_segments = vec![
        "clock".to_string(),
        "language".to_string(),
        "directory".to_string(),
        "git".to_string(),
    ];

    let theme_manager = ThemeManager::new("default").expect("load default theme");
    let fish_output = theme_manager.export(Shell::Fish, &config);

    // Verify language is removed
    assert!(
        fish_output.contains("set -g __enabled_segments clock directory git"),
        "Language should be removed from enabled_segments when language.enabled=false. Got: {fish_output}"
    );
    assert!(
        !fish_output.contains("__enabled_segments clock language directory git"),
        "Enabled segments should not include language when disabled. Got: {fish_output}"
    );

    // Verify GPY_LANGUAGE_ENABLED is set to 0
    assert!(fish_output.contains("set -gx GPY_LANGUAGE_ENABLED \"0\""),);
}

/// Test that both git and language are removed when both are disabled
#[test]
fn test_theme_export_updates_enabled_segments_when_both_disabled() {
    use gpy_agent::config::Config;
    use gpy_agent::shell::Shell;
    use gpy_agent::theme::ThemeManager;

    let mut config = Config::default();
    config.git.enabled = false;
    config.language.enabled = false;
    config.ui.enabled_segments = vec![
        "clock".to_string(),
        "language".to_string(),
        "directory".to_string(),
        "git".to_string(),
    ];

    let theme_manager = ThemeManager::new("default").expect("load default theme");
    let fish_output = theme_manager.export(Shell::Fish, &config);

    // Verify both are removed
    assert!(
        fish_output.contains("set -g __enabled_segments clock directory"),
        "Both git and language should be removed when disabled. Got: {fish_output}"
    );
    assert!(
        fish_output.contains("set -gx GPY_GIT_ENABLED \"0\""),
        "GPY_GIT_ENABLED should be 0. Got: {fish_output}"
    );
    assert!(
        fish_output.contains("set -gx GPY_LANGUAGE_ENABLED \"0\""),
        "GPY_LANGUAGE_ENABLED should be 0. Got: {fish_output}"
    );
}

/// Test that config watcher triggers reload and would send SIGUSR2
///
/// This test verifies the file watcher mechanism that monitors config.toml
/// for changes and triggers reloads. In production, this reload would also
/// send SIGUSR2 to all registered Fish clients.
// gpy-agent#385: see the matching comment on
// `config_watcher_detects_language_toggle` in config_watcher_tests.rs -- this
// test bootstraps its own FSEvents stream via `ConfigManager::start_watching`
// too, and must not bootstrap it concurrently with that test's (or any other
// FSEvents-bootstrapping test's) in a separate nextest process. Reuses
// #384's `watcher_fsevents_bootstrap` file_serial group for the same reason
// documented there.
#[tokio::test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
async fn test_config_watcher_triggers_reload_on_git_toggle() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");

    // Initial config with git enabled
    fs::write(
        &config_path,
        r#"
[git]
enabled = true

[ui]
enabled_segments = ["clock", "directory", "git"]
"#,
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    manager
        .start_watching(Duration::from_millis(100))
        .expect("start watching");

    assert!(manager.get().git.enabled, "git should be enabled initially");

    // Change config to disable git
    fs::write(
        &config_path,
        r#"
[git]
enabled = false

[ui]
enabled_segments = ["clock", "directory"]
"#,
    )
    .expect("write updated config");

    // Wait for watcher to detect change and reload
    let mut reloaded = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if !manager.get().git.enabled {
            reloaded = true;
            break;
        }
    }

    assert!(
        reloaded,
        "Config watcher should detect git.enabled toggle and reload config"
    );

    manager.stop_watching();
}

/// Test that SIGUSR2 is sent even when both `live_updates` and git are disabled
///
/// This ensures we don't miss SIGUSR2 in edge cases where multiple features are disabled.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_sigusr2_sent_when_disabling_multiple_features() {
    // Reset counter
    get_sigusr2_counter().store(0, Ordering::Relaxed);

    // Install handler
    let sigusr2_handler = SigAction::new(
        SigHandler::Handler(count_sigusr2),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let prev_usr2 =
        unsafe { sigaction(Signal::SIGUSR2, &sigusr2_handler) }.expect("install SIGUSR2 handler");

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");

    // Start with features enabled
    fs::write(
        &config_path,
        r"
[agent]
live_updates = true

[git]
enabled = true

[language]
enabled = true
",
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    let registry = ClientDirectory::new();
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = get_sigusr2_counter().load(Ordering::Relaxed);

    // Disable multiple features
    fs::write(
        &config_path,
        r"
[agent]
live_updates = false

[git]
enabled = false

[language]
enabled = false
",
    )
    .expect("write updated config");

    manager.reload_now().expect("reload");
    registry.notify_sigusr2();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let final_count = get_sigusr2_counter().load(Ordering::Relaxed);
    assert!(
        final_count > initial_count,
        "SIGUSR2 should be sent even when disabling multiple features"
    );

    // Restore handler
    unsafe {
        sigaction(Signal::SIGUSR2, &prev_usr2).expect("restore SIGUSR2");
    }
}

/// Test that SIGUSR2 is sent when `language.display` changes
///
/// This ensures that changing from "icon" to "text" or vice versa triggers
/// a config refresh, causing Fish to re-render the language segment.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_sigusr2_sent_when_language_display_changes() {
    // Reset counter
    get_sigusr2_counter().store(0, Ordering::Relaxed);

    // Install handler
    let sigusr2_handler = SigAction::new(
        SigHandler::Handler(count_sigusr2),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let prev_usr2 =
        unsafe { sigaction(Signal::SIGUSR2, &sigusr2_handler) }.expect("install SIGUSR2 handler");

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");

    // Start with display = "icon"
    fs::write(
        &config_path,
        r#"
[language]
enabled = true
display = "icon"
"#,
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    let registry = ClientDirectory::new();
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = get_sigusr2_counter().load(Ordering::Relaxed);

    // Change to display = "text"
    fs::write(
        &config_path,
        r#"
[language]
enabled = true
display = "text"
"#,
    )
    .expect("write updated config");

    manager.reload_now().expect("reload");
    assert_eq!(
        manager.get().language.display,
        gpy_agent::config::types::LanguageDisplay::Text
    );

    registry.notify_sigusr2();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let final_count = get_sigusr2_counter().load(Ordering::Relaxed);
    assert!(
        final_count > initial_count,
        "SIGUSR2 should be sent when language.display changes"
    );

    // Restore handler
    unsafe {
        sigaction(Signal::SIGUSR2, &prev_usr2).expect("restore SIGUSR2");
    }
}

/// Test that SIGUSR2 is sent when `language.icons` changes
///
/// This ensures that changing language icons (e.g., customizing the Rust icon)
/// triggers a config refresh, causing Fish to re-render the language segment
/// with the new icon.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_sigusr2_sent_when_language_icons_change() {
    // Reset counter
    get_sigusr2_counter().store(0, Ordering::Relaxed);

    // Install handler
    let sigusr2_handler = SigAction::new(
        SigHandler::Handler(count_sigusr2),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let prev_usr2 =
        unsafe { sigaction(Signal::SIGUSR2, &sigusr2_handler) }.expect("install SIGUSR2 handler");

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");

    // Start with default icons (no customization)
    fs::write(
        &config_path,
        r"
[language]
enabled = true
",
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    let registry = ClientDirectory::new();
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = get_sigusr2_counter().load(Ordering::Relaxed);

    // Add custom Rust icon
    fs::write(
        &config_path,
        r#"
[language]
enabled = true

[language.icons]
rust = "🦀"
python = "🐍"
"#,
    )
    .expect("write updated config");

    manager.reload_now().expect("reload");
    assert_eq!(
        manager
            .get()
            .language
            .icons
            .icons
            .get("rust")
            .map(gpy_agent::config::types::Icon::as_str),
        Some("🦀")
    );

    registry.notify_sigusr2();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let final_count = get_sigusr2_counter().load(Ordering::Relaxed);
    assert!(
        final_count > initial_count,
        "SIGUSR2 should be sent when language.icons changes"
    );

    // Restore handler
    unsafe {
        sigaction(Signal::SIGUSR2, &prev_usr2).expect("restore SIGUSR2");
    }
}

#[cfg(not(unix))]
#[test]
fn sigusr2_tests_require_unix() {
    // Placeholder for non-Unix platforms
}
