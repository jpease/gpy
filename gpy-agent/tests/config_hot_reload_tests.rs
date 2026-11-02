//! Config hot-reload: a config change must request a reload (`<pid>.reload`
//! flag + SIGURG) so Fish refreshes `__enabled_segments` and delimiters.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use gpy_agent::config::manager::ConfigManager;
#[cfg(unix)]
use gpy_agent::ipc::ClientDirectory;
use serial_test::serial;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

mod common;
#[cfg(unix)]
use common::sigurg::SigurgCounter;
use common::wait::wait_until_async;

/// How long a signalled reload gets to reach this process.
#[cfg(unix)]
const DELIVERY_DEADLINE: Duration = Duration::from_secs(5);

/// Asserts the doorbell rang and `<shell_dir>/<pid>.reload` was written.
/// Polls because the SIGURG handler runs asynchronously.
#[cfg(unix)]
async fn assert_reload_delivered(
    doorbell: &SigurgCounter,
    shell_dir: &TempDir,
    initial_count: usize,
    what: &str,
) {
    let flag = shell_dir
        .path()
        .join(format!("{}.reload", std::process::id()));
    let delivered = wait_until_async(DELIVERY_DEADLINE, Duration::from_millis(10), || {
        doorbell.count() > initial_count && flag.exists()
    })
    .await;
    let final_count = doorbell.count();
    assert!(
        final_count > initial_count,
        "SIGURG doorbell should ring when {what}. Before: {initial_count}, After: {final_count}"
    );
    assert!(
        delivered,
        "reload flag {} should exist when {what}",
        flag.display()
    );
}

/// Disabling git must request a reload, not just a repaint.
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_git_disabled() {
    let doorbell = SigurgCounter::install();

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[agent]
enabled = true
live_updates = true

[git]

[ui]
enabled_segments = ["clock", "directory", "git"]
"#,
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    let config = manager.get();
    assert!(
        config.segment_enabled("git"),
        "git should be listed initially"
    );

    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    let pid = std::process::id();
    let cwd = std::env::current_dir().expect("current dir");
    registry.register(pid, Some(cwd));

    // Mirrors the agent's reload_config.
    let initial_count = doorbell.count();

    fs::write(
        &config_path,
        r#"
[agent]
enabled = true
live_updates = true

[git]

[ui]
enabled_segments = ["clock", "directory"]
"#,
    )
    .expect("write updated config");

    manager.reload_now().expect("reload config");
    let new_config = manager.get();
    assert!(
        !new_config.segment_enabled("git"),
        "git should be unlisted now"
    );

    // Disabling git must request a reload.
    registry.notify_reload();

    assert_reload_delivered(&doorbell, &shell_dir, initial_count, "git is unlisted").await;
}

#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_git_enabled() {
    let doorbell = SigurgCounter::install();

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[git]

[ui]
enabled_segments = ["clock", "directory"]
"#,
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    assert!(!manager.get().segment_enabled("git"));

    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = doorbell.count();

    fs::write(
        &config_path,
        r#"
[git]

[ui]
enabled_segments = ["clock", "directory", "git"]
"#,
    )
    .expect("write updated config");

    manager.reload_now().expect("reload");
    assert!(manager.get().segment_enabled("git"));

    registry.notify_reload();

    assert_reload_delivered(&doorbell, &shell_dir, initial_count, "git is listed again").await;
}

/// With git disabled, the theme export drops "git" from `__enabled_segments`.
#[test]
fn test_theme_export_updates_enabled_segments_when_git_disabled() {
    use gpy_agent::config::Config;
    use gpy_agent::shell::Shell;
    use gpy_agent::theme::ThemeManager;

    let mut config = Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "directory".to_owned()];

    let theme_manager = ThemeManager::builtin("default").expect("load default theme");
    let fish_output = theme_manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    assert!(
        fish_output.contains("set -g __enabled_segments clock directory"),
        "Git should be absent from enabled_segments when unlisted. Got: {fish_output}"
    );
    assert!(
        !fish_output.contains("clock directory git"),
        "Enabled segments should not include git when unlisted. Got: {fish_output}"
    );
}

#[test]
fn test_theme_export_includes_git_when_enabled() {
    use gpy_agent::config::Config;
    use gpy_agent::shell::Shell;
    use gpy_agent::theme::ThemeManager;

    let mut config = Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "directory".to_owned(), "git".to_owned()];

    let theme_manager = ThemeManager::builtin("default").expect("load default theme");
    let fish_output = theme_manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    assert!(
        fish_output.contains("set -g __enabled_segments clock directory git"),
        "Git should be included in enabled_segments when listed. Got: {fish_output}"
    );
}

#[test]
fn test_theme_export_updates_enabled_segments_when_language_disabled() {
    use gpy_agent::config::Config;
    use gpy_agent::shell::Shell;
    use gpy_agent::theme::ThemeManager;

    let mut config = Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "directory".to_owned(), "git".to_owned()];

    let theme_manager = ThemeManager::builtin("default").expect("load default theme");
    let fish_output = theme_manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    assert!(
        fish_output.contains("set -g __enabled_segments clock directory git"),
        "Language should be absent from enabled_segments when unlisted. Got: {fish_output}"
    );
    assert!(
        !fish_output.contains("__enabled_segments clock language directory git"),
        "Enabled segments should not include language when unlisted. Got: {fish_output}"
    );
}

#[test]
fn test_theme_export_updates_enabled_segments_when_both_disabled() {
    use gpy_agent::config::Config;
    use gpy_agent::shell::Shell;
    use gpy_agent::theme::ThemeManager;

    let mut config = Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "directory".to_owned()];

    let theme_manager = ThemeManager::builtin("default").expect("load default theme");
    let fish_output = theme_manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    assert!(
        fish_output.contains("set -g __enabled_segments clock directory"),
        "Both git and language should be absent when unlisted. Got: {fish_output}"
    );
}

/// The config watcher triggers a reload.
// Shares the `watcher_fsevents_bootstrap` serial group: concurrent FSEvents bootstrap across processes is unsafe.
#[tokio::test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
async fn test_config_watcher_triggers_reload_on_git_toggle() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");

    fs::write(
        &config_path,
        r#"
[git]

[ui]
enabled_segments = ["clock", "directory", "git"]
"#,
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    manager
        .start_watching(Duration::from_millis(100))
        .expect("start watching");

    assert!(
        manager.get().segment_enabled("git"),
        "git should be listed initially"
    );

    fs::write(
        &config_path,
        r#"
[git]

[ui]
enabled_segments = ["clock", "directory"]
"#,
    )
    .expect("write updated config");

    let reloaded = wait_until_async(Duration::from_secs(6), Duration::from_millis(50), || {
        !manager.get().segment_enabled("git")
    })
    .await;

    assert!(
        reloaded,
        "Config watcher should detect the git segment being unlisted and reload config"
    );

    manager.stop_watching();
}

#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_disabling_multiple_features() {
    let doorbell = SigurgCounter::install();

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");

    fs::write(
        &config_path,
        r"
[agent]
live_updates = true

[git]

[language]
",
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = doorbell.count();

    fs::write(
        &config_path,
        r#"
[agent]
live_updates = false

[ui]
enabled_segments = ["clock", "directory"]
"#,
    )
    .expect("write updated config");

    manager.reload_now().expect("reload");
    registry.notify_reload();

    assert_reload_delivered(
        &doorbell,
        &shell_dir,
        initial_count,
        "disabling multiple features",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_language_display_changes() {
    let doorbell = SigurgCounter::install();

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");

    fs::write(
        &config_path,
        r#"
[language]
display = "icon"
"#,
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = doorbell.count();

    fs::write(
        &config_path,
        r#"
[language]
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

    assert_reload_delivered(
        &doorbell,
        &shell_dir,
        initial_count,
        "language.display changes",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_reload_sent_when_language_icons_change() {
    let doorbell = SigurgCounter::install();

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_path = temp_dir.path().join("config.toml");

    fs::write(
        &config_path,
        r"
[language]
",
    )
    .expect("write initial config");

    let manager = ConfigManager::from_path(&config_path).expect("create manager");
    let shell_dir = TempDir::new().expect("create shell dir");
    let registry = ClientDirectory::with_shell_dir(shell_dir.path().to_path_buf());
    registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = doorbell.count();

    fs::write(
        &config_path,
        r#"
[language]

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

    assert_reload_delivered(
        &doorbell,
        &shell_dir,
        initial_count,
        "language.icons changes",
    )
    .await;
}

/// Re-applies `trigger` every 200 ms until `git.show_upstream == want` or 10 s pass.
/// A watch has no "armed" signal, so repeat the idempotent change; 200 ms exceeds the 100 ms debounce.
/// `want` or 10 s pass.
#[cfg(unix)]
async fn wait_for_show_upstream(manager: &ConfigManager, want: bool, trigger: impl Fn()) -> bool {
    wait_until_async(Duration::from_secs(10), Duration::from_millis(200), || {
        trigger();
        manager.get().git.show_upstream == want
    })
    .await
}

/// `config.toml` is a symlink into another directory (stow/dotfiles
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
    fs::write(&target, "[git]\n").expect("write target");
    std::os::unix::fs::symlink(&target, &link).expect("symlink config");

    let manager = ConfigManager::from_path(&link).expect("create manager");
    manager
        .start_watching(Duration::from_millis(100))
        .expect("start watching");

    let write_through_link =
        || fs::write(&link, "[git]\nshow_upstream = false\n").expect("write through link");
    assert!(
        wait_for_show_upstream(&manager, false, write_through_link).await,
        "a write through the symlink should hot-reload"
    );

    let write_target = || fs::write(&target, "[git]\n").expect("write target directly");
    assert!(
        wait_for_show_upstream(&manager, true, write_target).await,
        "a write to the symlink target should hot-reload"
    );

    manager.stop_watching();
}

/// `ln -sf other config.toml` reloads, and edits to the new target
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
    fs::write(&first, "[git]\n").expect("write first target");
    fs::write(&second, "[git]\nshow_upstream = false\n").expect("write second target");
    std::os::unix::fs::symlink(&first, &link).expect("symlink config");

    let manager = ConfigManager::from_path(&link).expect("create manager");
    manager
        .start_watching(Duration::from_millis(100))
        .expect("start watching");

    let staged = dir_b.join("staged-link");
    let retarget = || {
        std::os::unix::fs::symlink(&second, &staged).expect("stage link");
        fs::rename(&staged, &link).expect("retarget link");
    };
    assert!(
        wait_for_show_upstream(&manager, false, retarget).await,
        "retargeting the symlink should hot-reload"
    );

    // Keep writing until the watcher has re-armed the new target's directory.
    let edit_new_target = || fs::write(&second, "[git]\n").expect("edit new target");
    assert!(
        wait_for_show_upstream(&manager, true, edit_new_target).await,
        "an edit to the new target should hot-reload"
    );

    manager.stop_watching();
}
