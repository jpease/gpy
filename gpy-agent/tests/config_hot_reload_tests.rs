//! Config hot-reload integration tests
//!
//! Tests the complete hot-reload flow:
//! 1. Config file changes
//! 2. Agent detects change via file watcher
//! 3. Agent writes `<pid>.reload` flag files and rings the SIGURG doorbell on registered Fish processes
//! 4. Fish reloads theme variables (`GPY_GIT_ENABLED`, `__enabled_segments`, etc.)
//! 5. Prompt updates with correct delimiters
//!
//! These tests prevent regressions of the critical bug where disabling git.enabled
//! would not request a reload, causing Fish to use stale segment lists and wrong delimiters.

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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

#[cfg(unix)]
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};

// Counter for SIGURG doorbell deliveries
static DOORBELL_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn get_doorbell_counter() -> &'static AtomicUsize {
    &DOORBELL_COUNTER
}

#[cfg(unix)]
extern "C" fn count_doorbell(_signal: i32) {
    get_doorbell_counter().fetch_add(1, Ordering::Relaxed);
}

/// Reset the doorbell counter and install the counting SIGURG handler,
/// returning the previous disposition for [`restore_doorbell`].
#[cfg(unix)]
fn install_doorbell() -> SigAction {
    get_doorbell_counter().store(0, Ordering::Relaxed);
    let handler = SigAction::new(
        SigHandler::Handler(count_doorbell),
        SaFlags::empty(),
        SigSet::empty(),
    );
    unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install SIGURG handler")
}

#[cfg(unix)]
fn restore_doorbell(previous: SigAction) {
    unsafe { sigaction(Signal::SIGURG, &previous) }.expect("restore SIGURG");
}

/// Assert a reload reached this process: the doorbell rang and the
/// `<shell_dir>/<pid>.reload` flag was written.
#[cfg(unix)]
fn assert_reload_delivered(shell_dir: &TempDir, initial_count: usize, what: &str) {
    let final_count = get_doorbell_counter().load(Ordering::Relaxed);
    assert!(
        final_count > initial_count,
        "SIGURG doorbell should ring when {what}. Before: {initial_count}, After: {final_count}"
    );
    let flag = shell_dir
        .path()
        .join(format!("{}.reload", std::process::id()));
    assert!(
        flag.exists(),
        "reload flag {} should exist when {what}",
        flag.display()
    );
}

/// Test that a reload is requested when git.enabled changes from true to false
///
/// This is a regression test for the critical bug where the agent would only request
/// a repaint (git status update) but not a reload (config/theme) when disabling git.
/// Without a reload, Fish keeps the old `__enabled_segments` list and uses the wrong delimiter.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_git_disabled() {
    let prev = install_doorbell();

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
    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    let pid = std::process::id();
    let cwd = std::env::current_dir().expect("current dir");
    registry.register(pid, Some(cwd));

    // Simulate agent detecting config change and sending signals
    // (In real agent, this happens in agent.rs reload_config)
    let initial_count = get_doorbell_counter().load(Ordering::Relaxed);

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

    // Agent should request a reload when git is disabled
    registry.notify_reload();

    // Wait for signal delivery
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_reload_delivered(&shell_dir, initial_count, "git.enabled changes to false");

    restore_doorbell(prev);
}

/// Test that a reload is requested when git.enabled changes from false to true
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_git_enabled() {
    let prev = install_doorbell();

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
    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = get_doorbell_counter().load(Ordering::Relaxed);

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

    registry.notify_reload();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_reload_delivered(&shell_dir, initial_count, "git.enabled changes to true");

    restore_doorbell(prev);
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

    let theme_manager = ThemeManager::builtin("default").expect("load default theme");
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

    let theme_manager = ThemeManager::builtin("default").expect("load default theme");
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

    let theme_manager = ThemeManager::builtin("default").expect("load default theme");
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

    let theme_manager = ThemeManager::builtin("default").expect("load default theme");
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

/// Test that config watcher triggers reload
///
/// This test verifies the file watcher mechanism that monitors config.toml
/// for changes and triggers reloads. In production, this reload would also
/// write reload flags and ring the SIGURG doorbell on all registered Fish clients.
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

/// Test that a reload is requested even when both `live_updates` and git are disabled
///
/// This ensures we don't miss a reload in edge cases where multiple features are disabled.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_disabling_multiple_features() {
    let prev = install_doorbell();

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
    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = get_doorbell_counter().load(Ordering::Relaxed);

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
    registry.notify_reload();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_reload_delivered(&shell_dir, initial_count, "disabling multiple features");

    restore_doorbell(prev);
}

/// Test that a reload is requested when `language.display` changes
///
/// This ensures that changing from "icon" to "text" or vice versa triggers
/// a config refresh, causing Fish to re-render the language segment.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_language_display_changes() {
    let prev = install_doorbell();

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
    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = get_doorbell_counter().load(Ordering::Relaxed);

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

    registry.notify_reload();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_reload_delivered(&shell_dir, initial_count, "language.display changes");

    restore_doorbell(prev);
}

/// Test that a reload is requested when `language.icons` changes
///
/// This ensures that changing language icons (e.g., customizing the Rust icon)
/// triggers a config refresh, causing Fish to re-render the language segment
/// with the new icon.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_language_icons_change() {
    let prev = install_doorbell();

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
    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = get_doorbell_counter().load(Ordering::Relaxed);

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

    registry.notify_reload();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_reload_delivered(&shell_dir, initial_count, "language.icons changes");

    restore_doorbell(prev);
}

#[cfg(not(unix))]
#[test]
fn reload_tests_require_unix() {
    // Placeholder for non-Unix platforms
}

/// Poll `manager.get().git.enabled` until it equals `want` or 10 s pass.
#[cfg(unix)]
async fn wait_for_git_enabled(manager: &ConfigManager, want: bool) -> bool {
    for _ in 0..50 {
        if manager.get().git.enabled == want {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    manager.get().git.enabled == want
}

/// #720: `config.toml` is a symlink into another directory (stow/dotfiles
/// layout); a write through the link lands on the target inode, which the
/// link's own directory never reports.
#[cfg(unix)]
#[tokio::test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
async fn symlinked_config_file_hot_reloads() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let base = fs::canonicalize(temp_dir.path()).expect("canonical base");
    let dir_a = base.join("dirA");
    let dir_b = base.join("dirB").join("gpy");
    fs::create_dir_all(&dir_a).expect("create dirA");
    fs::create_dir_all(&dir_b).expect("create dirB");
    let target = dir_a.join("gpy-config.toml");
    let link = dir_b.join("config.toml");
    fs::write(&target, "[git]\nenabled = true\n").expect("write target");
    std::os::unix::fs::symlink(&target, &link).expect("symlink config");

    let manager = ConfigManager::from_path(&link).expect("create manager");
    manager
        .start_watching(Duration::from_millis(100))
        .expect("start watching");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    fs::write(&link, "[git]\nenabled = false\n").expect("write through link");
    assert!(
        wait_for_git_enabled(&manager, false).await,
        "a write through the symlink should hot-reload"
    );

    fs::write(&target, "[git]\nenabled = true\n").expect("write target directly");
    assert!(
        wait_for_git_enabled(&manager, true).await,
        "a write to the symlink target should hot-reload"
    );

    manager.stop_watching();
}

/// #720: `ln -sf other config.toml` reloads, and edits to the new target
/// (in a third directory) reload too.
#[cfg(unix)]
#[tokio::test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
async fn symlinked_config_retarget_hot_reloads() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let base = fs::canonicalize(temp_dir.path()).expect("canonical base");
    let dir_a = base.join("dirA");
    let dir_c = base.join("dirC");
    let dir_b = base.join("dirB").join("gpy");
    for dir in [&dir_a, &dir_b, &dir_c] {
        fs::create_dir_all(dir).expect("create dir");
    }
    let first = dir_a.join("gpy-config.toml");
    let second = dir_c.join("other-config.toml");
    let link = dir_b.join("config.toml");
    fs::write(&first, "[git]\nenabled = true\n").expect("write first target");
    fs::write(&second, "[git]\nenabled = false\n").expect("write second target");
    std::os::unix::fs::symlink(&first, &link).expect("symlink config");

    let manager = ConfigManager::from_path(&link).expect("create manager");
    manager
        .start_watching(Duration::from_millis(100))
        .expect("start watching");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let staged = dir_b.join("staged-link");
    std::os::unix::fs::symlink(&second, &staged).expect("stage link");
    fs::rename(&staged, &link).expect("retarget link");
    assert!(
        wait_for_git_enabled(&manager, false).await,
        "retargeting the symlink should hot-reload"
    );

    // Let the re-arm of the new target's directory settle.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    fs::write(&second, "[git]\nenabled = true\n").expect("edit new target");
    assert!(
        wait_for_git_enabled(&manager, true).await,
        "an edit to the new target should hot-reload"
    );

    manager.stop_watching();
}
