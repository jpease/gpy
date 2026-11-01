//! Tests for `ThemeManager` functionality
//!
//! # Test Isolation
//!
//! These tests point the process-global `XDG_CONFIG_HOME` (and related keys)
//! at temp directories through [`EnvGuard`], and run `#[serial]` so no two
//! tests observe each other's environment.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

#[path = "common/env_guard.rs"]
mod env_guard;
#[path = "common/sigurg.rs"]
mod sigurg;
#[path = "common/wait.rs"]
mod wait;

use env_guard::EnvGuard;
use gpy_agent::shell::Shell;
use gpy_agent::theme::{ThemeManager, ThemeSource};
use serial_test::serial;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use wait::wait_until;

// Helper to create a test theme file
fn create_test_theme_toml(dir: &Path, theme_name: &str, content: &str) -> PathBuf {
    let themes_dir = dir.join("themes");
    fs::create_dir_all(&themes_dir).unwrap();

    let theme_path = themes_dir.join(format!("{theme_name}.toml"));
    fs::write(&theme_path, content).unwrap();

    theme_path
}

// Minimal valid theme TOML
const MINIMAL_THEME: &str = r##"
[ui]
prompt_icon = "❯"
root_prompt_icon = "#"

[segments.clock]
bg_color = "blue"
text_color = "white"

[segments.directory]
bg_color = "cyan"
text_color = "black"

[segments.duration]
bg_color = "yellow"
text_color = "black"
show_if_exceeds_ms = 5000

[segments.status]
ok_bg_color = "green"
ok_text_color = "white"
fail_bg_color = "red"
fail_text_color = "white"

[segments.git]
clean_bg_color = "green"
clean_text_color = "black"
dirty_bg_color = "yellow"
dirty_text_color = "black"

[segments.language]
bg_color = "magenta"
text_color = "white"
"##;

// Theme with custom colors for testing
const CUSTOM_THEME: &str = r#"
[ui]
prompt_icon = "→"
root_prompt_icon = "⚡"
prompt_color = "cyan"
root_prompt_color = "red"

[segments.clock]
bg_color = "magenta"
text_color = "yellow"
time_format = "%H:%M:%S"
show_seconds = true

[segments.directory]
bg_color = "blue"
text_color = "white"

[segments.duration]
bg_color = "bright_yellow"
text_color = "black"
show_if_exceeds_ms = 1000
icon = "⏱"

[segments.status]
ok_bg_color = "green"
ok_text_color = "white"
ok_icon = "✓"
fail_bg_color = "red"
fail_text_color = "white"
fail_icon = "✗"

[segments.git]
clean_bg_color = "green"
clean_text_color = "black"
dirty_bg_color = "yellow"
dirty_text_color = "black"

[segments.language]
bg_color = "magenta"
text_color = "white"
"#;

const PLUGIN_SEGMENT_THEME: &str = r##"
[ui]
prompt_icon = "→"
root_prompt_icon = "⚡"

[segments.clock]
bg_color = "black"
text_color = "white"

[segments.directory]
bg_color = "blue"
text_color = "white"

[segments.duration]
bg_color = "yellow"
text_color = "black"
show_if_exceeds_ms = 1000

[segments.status]
ok_bg_color = "green"
ok_text_color = "white"
fail_bg_color = "red"
fail_text_color = "white"

[segments.git]
clean_bg_color = "green"
clean_text_color = "black"
dirty_bg_color = "yellow"
dirty_text_color = "black"

[segments.language]
bg_color = "magenta"
text_color = "white"

[segments.k8s]
bg_color = "#073642"
text_color = "white"
icon = "⎈"
prod_bg_color = "#326ce5"
prod_text_color = "#aaaaaa"

[segments.my-seg]
bg_color = "cyan"
text_color = "black"
"##;

#[test]
#[serial]
fn test_theme_manager_new_errors_when_missing() {
    // A theme name matching no user theme, plugin theme, or builtin must
    // error rather than silently succeed with a bare default.
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().to_path_buf();

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", &config_dir);

    let Err(err) = ThemeManager::new("nonexistent") else {
        panic!("an unresolvable theme name must error, not silently succeed with a default theme");
    };
    assert!(
        err.to_string().contains("nonexistent"),
        "error should name the theme, got: {err}"
    );
}

#[test]
#[serial]
fn test_theme_manager_loads_valid_theme() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "test", MINIMAL_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("test").unwrap();
    let theme = manager.get();

    assert_eq!(theme.ui.prompt_icon.as_deref(), Some("❯"));
    assert_eq!(theme.ui.root_prompt_icon, "#");
    assert_eq!(theme.segments.clock.bg_color, "blue");
    assert_eq!(theme.segments.directory.bg_color, "cyan");
}

#[test]
#[serial]
fn test_theme_manager_loads_custom_theme() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "custom", CUSTOM_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("custom").unwrap();
    let theme = manager.get();

    assert_eq!(theme.ui.prompt_icon.as_deref(), Some("→"));
    assert_eq!(theme.ui.root_prompt_icon, "⚡");
    assert_eq!(theme.ui.prompt_color.as_deref(), Some("cyan"));
    assert_eq!(theme.ui.root_prompt_color.as_deref(), Some("red"));
    assert_eq!(theme.segments.clock.bg_color, "magenta");
    assert_eq!(theme.segments.clock.text_color, "yellow");
    assert_eq!(
        theme.segments.clock.time_format,
        Some("%H:%M:%S".to_owned())
    );
    assert_eq!(theme.segments.clock.show_seconds, Some(true));
    assert_eq!(
        theme.segments.duration.icon,
        Some(gpy_agent::config::types::Icon::new("⏱").unwrap())
    );
    assert_eq!(
        theme.segments.status.ok_icon,
        Some(gpy_agent::config::types::Icon::new("✓").unwrap())
    );
}

#[test]
#[serial]
fn test_theme_manager_export_fish_format() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "export_test", CUSTOM_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("export_test").unwrap();

    // Export to Fish format
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "directory".to_owned(), "git".to_owned()];
    let fish_output = manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    assert!(fish_output.contains("# GPY Theme: export_test"));
    assert!(fish_output.contains("set -g __gpy_theme_name \"export_test\""));
    assert!(fish_output.contains("set -g __icon_prompt \"→\""));
    assert!(fish_output.contains("set -g __icon_root_prompt \"⚡\""));
    assert!(fish_output.contains("set -g __prompt_color \"cyan\""));
    assert!(fish_output.contains("set -g __root_prompt_color \"red\""));
    assert!(fish_output.contains("set -g __color_clock_bg \"magenta\""));
    assert!(fish_output.contains("set -g __color_clock_fg \"yellow\""));
    assert!(fish_output.contains("set -g __time_format \"%H:%M:%S\""));
    assert!(fish_output.contains("set -g __clock_show_seconds \"1\""));
    // Duration is agent-rendered, so the theme export carries no shell-side
    // duration icon.
    assert!(!fish_output.contains("__icon_duration"));
    assert!(fish_output.contains("set -g __enabled_segments clock directory git"));
    assert!(fish_output.starts_with("set -g __gpy_export_version \"1\"\n"));
    assert!(fish_output.contains("set -g __gpy_cfg_agent_enabled \"1\""));
    assert!(fish_output.contains("set -g __gpy_cfg_language_enabled \"0\""));
    assert!(fish_output.contains("set -g __gpy_cfg_git_enabled \"1\""));
}

#[test]
#[serial]
fn test_theme_manager_reload() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    let theme_path = create_test_theme_toml(&config_dir, "reload_test", MINIMAL_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("reload_test").unwrap();

    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon.as_deref(), Some("❯"));
        assert_eq!(theme.segments.clock.bg_color, "blue");
    }

    // Modify theme file
    fs::write(&theme_path, CUSTOM_THEME).unwrap();

    // Reload
    manager.reload().unwrap();

    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon.as_deref(), Some("→"));
        assert_eq!(theme.segments.clock.bg_color, "magenta");
    }
}

#[test]
#[serial]
fn test_disabled_git_removed_from_segment_list() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_dir, "disabled_git", CUSTOM_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::new("disabled_git").unwrap();
    let mut config = gpy_agent::config::Config::default();
    config.git.enabled = false;
    config.ui.enabled_segments = vec!["clock".to_owned(), "directory".to_owned(), "git".to_owned()];

    let fish_output = manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    assert!(!fish_output.contains("clock directory git"));
    assert!(fish_output.contains("set -g __enabled_segments clock directory"));
    assert!(fish_output.contains("set -g __gpy_cfg_git_enabled \"0\""));
}

#[test]
#[serial]
fn test_theme_manager_reload_keeps_old_on_error() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    let theme_path = create_test_theme_toml(&config_dir, "error_test", MINIMAL_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("error_test").unwrap();

    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon.as_deref(), Some("❯"));
    }

    // Write invalid TOML
    fs::write(&theme_path, "this is not valid TOML { } [ ]").unwrap();

    // Try to reload - should fail
    let result = manager.reload();
    assert!(result.is_err());

    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon.as_deref(), Some("❯"));
    }
}

#[test]
#[serial]
fn test_theme_manager_reload_errors_on_missing_file_and_keeps_last_good_theme() {
    // A reload whose theme file vanished (and doesn't match a builtin name)
    // must error rather than reset the in-memory theme to a bare default. The
    // in-memory theme must also be left exactly as it was: `reload`'s `?` on
    // the failed load means the overwrite never happens.
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    let theme_path = create_test_theme_toml(&config_dir, "missing_test", MINIMAL_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("missing_test").unwrap();

    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon.as_deref(), Some("❯"));
    }

    // Delete theme file
    fs::remove_file(&theme_path).unwrap();

    let err = manager
        .reload()
        .expect_err("reload of a deleted, non-builtin theme file must error");
    assert!(
        err.to_string().contains("missing_test"),
        "error should name the theme, got: {err}"
    );

    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon.as_deref(), Some("❯"));
    }
}

#[test]
#[serial]
fn test_theme_manager_start_stop_watching() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "watch_test", MINIMAL_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("watch_test").unwrap();

    // Start watching
    manager
        .start_watching(Duration::from_millis(100))
        .expect("start watching");

    // Starting again should be a no-op
    manager
        .start_watching(Duration::from_millis(100))
        .expect("second start watching");

    // Stop watching
    manager.stop_watching();

    // Stopping again should be safe
    manager.stop_watching();
}

#[test]
#[serial]
fn test_theme_manager_get_returns_arc_clone() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "arc_test", MINIMAL_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("arc_test").unwrap();

    // Get theme multiple times
    let theme1 = manager.get();
    let theme2 = manager.get();

    // Both should have same content
    assert_eq!(theme1.ui.prompt_icon, theme2.ui.prompt_icon);
    assert_eq!(
        theme1.segments.clock.bg_color,
        theme2.segments.clock.bg_color
    );
}

#[test]
#[serial]
fn test_theme_manager_respects_xdg_config_home() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("custom_config").join("gpy");

    create_test_theme_toml(&config_dir, "xdg_test", CUSTOM_THEME);

    // Set custom XDG_CONFIG_HOME
    let xdg_path = temp_dir.path().join("custom_config");
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", &xdg_path);

    // Load the theme - should find it in custom location
    let manager = ThemeManager::new("xdg_test").unwrap();
    let theme = manager.get();

    assert_eq!(theme.ui.prompt_icon.as_deref(), Some("→"));
    assert_eq!(theme.segments.clock.bg_color, "magenta");
}

/// Linux paths are raw bytes: a user theme under a non-UTF-8 `XDG_CONFIG_HOME`
/// is found where the directory really is, not under a lossy spelling of it.
#[cfg(target_os = "linux")]
#[test]
#[serial]
fn test_theme_manager_loads_from_non_utf8_xdg_config_home() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let temp_dir = TempDir::new().unwrap();
    let xdg_path = temp_dir
        .path()
        .join(PathBuf::from(OsString::from_vec(b"cfg-\xff".to_vec())));
    create_test_theme_toml(&xdg_path.join("gpy"), "odd_bytes", CUSTOM_THEME);
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", &xdg_path);

    assert_eq!(
        ThemeManager::user_themes_dir(),
        xdg_path.join("gpy").join("themes")
    );
    let manager = ThemeManager::new("odd_bytes").unwrap();
    assert_eq!(manager.get().ui.prompt_icon.as_deref(), Some("→"));
}

#[test]
#[serial]
fn test_theme_manager_export_includes_all_segments() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "full_export", CUSTOM_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("full_export").unwrap();

    // Export with all segments
    let all_segments = vec![
        "clock".to_owned(),
        "directory".to_owned(),
        "duration".to_owned(),
        "status".to_owned(),
        "git".to_owned(),
        "language".to_owned(),
    ];
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = all_segments;
    let fish_output = manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    // Clock and status still render shell-side and keep their color exports.
    // Directory/duration colors are agent-rendered and not exported here.
    assert!(fish_output.contains("__color_clock_bg"));
    assert!(fish_output.contains("__color_status_ok_bg"));
    assert!(fish_output.contains("__color_status_fail_bg"));

    assert!(fish_output.contains("__segment_delim_first"));
    assert!(fish_output.contains("__segment_delim_start"));
    assert!(fish_output.contains("__segment_delim_end"));
    assert!(fish_output.contains("__segment_delim_last"));

    assert!(
        fish_output
            .contains("set -g __enabled_segments clock directory duration status git language")
    );
}

#[test]
#[serial]
fn export_omits_migrated_color_vars() {
    // Segments render from agent-provided ANSI, so their fg/text color exports
    // and format toggles are not exported. Only bg colors for directory,
    // duration, git, and language are exported, for powerline chevron
    // tracking (`__gpy_last_segment_bg`). Clock and status still render
    // shell-side and must keep their full `__color_*` exports.
    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::builtin("default").expect("builtin theme manager");
    let config = gpy_agent::config::Config::default();
    let exported = manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    // No fg/text color exports for agent-rendered segments.
    for still_migrated in [
        "__color_directory_fg",
        "__color_duration_fg",
        "__color_character",
        "__color_language_fg",
        "__color_language_text",
    ] {
        assert!(
            !exported.contains(still_migrated),
            "agent-rendered segment color var {still_migrated} must not be exported"
        );
    }

    // No legacy format toggles.
    for toggle in [
        "__gpy_directory_format",
        "__gpy_duration_format",
        "__gpy_character_format",
    ] {
        assert!(
            !exported.contains(toggle),
            "legacy format toggle {toggle} must not be exported"
        );
    }

    // Re-added bg color exports for powerline chevron tracking.
    for bg_var in [
        "__color_directory_bg",
        "__color_duration_bg",
        "__color_git_clean_bg",
        "__color_git_dirty_bg",
        "__color_language_bg",
    ] {
        assert!(
            exported.contains(bg_var),
            "powerline bg var {bg_var} must be exported for chevron tracking"
        );
    }

    // Surgical keep: clock and status remain shell-rendered.
    assert!(
        exported.contains("__color_clock_"),
        "clock color vars must still be exported (shell-rendered)"
    );
    assert!(
        exported.contains("__color_status_"),
        "status color vars must still be exported (shell-rendered)"
    );
}

#[test]
#[serial]
fn test_theme_hot_reload_triggers_on_file_change() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    let theme_path = create_test_theme_toml(&config_dir, "hotreload", MINIMAL_THEME);

    let mut env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    // Load the theme
    let manager = ThemeManager::new("hotreload").unwrap();

    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon.as_deref(), Some("❯"));
        assert_eq!(theme.segments.clock.bg_color, "blue");
    }

    // Start watching with very short debounce for testing
    env.set("GPY_THEME_WATCH_POLL_MS", "25");
    manager.start_watching(Duration::from_millis(50)).unwrap();

    // A watch has no observable "armed" signal, so each poll re-applies the
    // edit (idempotent) until the watcher sees it. The 100 ms interval is
    // longer than the 50 ms debounce, so repeats never starve the flush.
    let reloaded = wait_until(Duration::from_secs(5), Duration::from_millis(100), || {
        fs::write(&theme_path, CUSTOM_THEME).unwrap();
        let theme = manager.get();
        theme.ui.prompt_icon.as_deref() == Some("→") && theme.segments.clock.bg_color == "magenta"
    });
    assert!(reloaded, "theme should reload within timeout");

    // Stop watching
    manager.stop_watching();
}

/// A reload (flag file + SIGURG doorbell) reaches Fish clients when the theme file changes.
//
// This ensures that when a user edits their theme file (e.g., changes colors),
// Fish shells automatically reload the theme variables without needing to restart.
#[cfg(unix)]
#[test]
#[serial]
fn test_theme_change_sends_reload() {
    use gpy_agent::ipc::ClientDirectory;
    use sigurg::SigurgCounter;
    use std::sync::Arc;

    let doorbell = SigurgCounter::install();

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_home = temp_dir.path();

    let mut env = EnvGuard::set_one("XDG_CONFIG_HOME", config_home);

    let theme_path = create_test_theme_toml(&config_home.join("gpy"), "reload_test", MINIMAL_THEME);

    let manager = ThemeManager::new("reload_test").unwrap();
    let shell_dir = TempDir::new().expect("create shell dir");
    let client_registry = Arc::new(ClientDirectory::with_shell_dir(
        shell_dir.path().to_path_buf(),
    ));
    let reload_flag = shell_dir
        .path()
        .join(format!("{}.reload", std::process::id()));

    // Register this process
    client_registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let initial_count = doorbell.count();

    // Start watching; the reload hook rings the client registry's doorbell
    env.set("GPY_THEME_WATCH_POLL_MS", "25");
    let registry_in_hook = Arc::clone(&client_registry);
    manager.set_reload_callback(Arc::new(move |reload: &dyn Fn() -> bool| {
        if reload() {
            registry_in_hook.notify_reload();
        }
    }));
    manager.start_watching(Duration::from_millis(50)).unwrap();

    // Modify theme file
    let updated_theme = r#"
[ui]
prompt_icon = ">"
prompt_color = "cyan"

[segments.clock]
bg_color = "red"
text_color = "white"

[segments.directory]
bg_color = "blue"
text_color = "white"

[segments.duration]
bg_color = "yellow"
text_color = "black"
show_if_exceeds_ms = 1000

[segments.status]
ok_bg_color = "green"
ok_text_color = "black"
fail_bg_color = "red"
fail_text_color = "white"
"#;

    // Wait for both the doorbell and the actual theme reload. Under full-suite load
    // the signal can arrive slightly before the manager's read observes the new
    // theme, so asserting on the signal counter alone is racy. A watch has no
    // observable "armed" signal, so each poll re-applies the edit (idempotent);
    // the 100 ms interval exceeds the 50 ms debounce.
    let signaled = wait_until(Duration::from_secs(5), Duration::from_millis(100), || {
        fs::write(&theme_path, updated_theme).unwrap();
        let signal_seen = doorbell.count() > initial_count;
        let theme = manager.get();
        signal_seen
            && theme.ui.prompt_icon.as_deref() == Some(">")
            && theme.segments.clock.bg_color == "red"
    });
    let final_count = doorbell.count();
    assert!(signaled, "SIGURG doorbell should ring within timeout");
    assert!(
        final_count > initial_count,
        "SIGURG doorbell should ring when theme file changes. Before: {initial_count}, After: {final_count}"
    );
    assert!(
        reload_flag.exists(),
        "reload flag {} should be written when theme file changes",
        reload_flag.display()
    );

    let theme = manager.get();
    assert_eq!(theme.ui.prompt_icon.as_deref(), Some(">"));
    assert_eq!(theme.segments.clock.bg_color, "red");

    // Stop watching
    manager.stop_watching();
}

#[test]
#[serial]
fn test_theme_export_fish_includes_plugin_segment_vars() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_dir, "plugin_export_fish", PLUGIN_SEGMENT_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::new("plugin_export_fish").unwrap();
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "k8s".to_owned()];
    let output = manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    assert!(output.contains("set -g __gpy_cfg_transient_prompt \"1\""));
    assert!(output.contains("set -g __gpy_segment_k8s_id \"k8s\""));
    assert!(output.contains("set -g __gpy_segment_k8s_bg_color \"#073642\""));
    assert!(output.contains("set -g __gpy_segment_k8s_text_color \"white\""));
    assert!(output.contains("set -g __gpy_segment_k8s_icon \"⎈\""));
    assert!(output.contains("set -g __gpy_segment_k8s_variant_prod_bg_color \"#326ce5\""));
    assert!(output.contains("set -g __gpy_segment_k8s_variant_prod_text_color \"#aaaaaa\""));
    assert!(output.contains("set -g __color_k8s_bg \"#073642\""));
    assert!(output.contains("set -g __color_k8s_fg \"white\""));
    assert!(output.contains("set -g __color_k8s_prod_bg \"#326ce5\""));
    assert!(output.contains("set -g __color_k8s_prod_fg \"#aaaaaa\""));
    assert!(output.contains("set -g __icon_k8s \"⎈\""));
    assert!(output.contains("set -g __gpy_segment_my_seg_id \"my-seg\""));
    assert!(output.contains("set -g __color_my_seg_bg \"cyan\""));
    assert!(output.contains("set -g __color_clock_bg \"black\""));
}

#[test]
#[serial]
fn test_theme_export_zsh_bash_include_plugin_segment_vars() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_dir, "plugin_export_posix", PLUGIN_SEGMENT_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::new("plugin_export_posix").unwrap();
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "k8s".to_owned()];

    let zsh_output = manager.export(
        Shell::Zsh,
        &config,
        &gpy_agent::template::Palette::default(),
    );
    assert!(zsh_output.contains("typeset -g __gpy_cfg_git_enabled=\"0\""));
    assert!(zsh_output.contains("typeset -g __gpy_segment_k8s_id=\"k8s\""));
    assert!(zsh_output.contains("typeset -g __gpy_segment_k8s_bg_color=\"#073642\""));
    assert!(zsh_output.contains("typeset -g __color_k8s_prod_bg=\"#326ce5\""));
    assert!(
        zsh_output.contains("typeset -g __gpy_segment_k8s_variant_prod_text_color=\"#aaaaaa\"")
    );

    let bash_output = manager.export(
        Shell::Bash,
        &config,
        &gpy_agent::template::Palette::default(),
    );
    assert!(bash_output.contains("__gpy_cfg_git_enabled=\"0\""));
    assert!(bash_output.contains("__gpy_segment_k8s_id=\"k8s\""));
    assert!(bash_output.contains("__gpy_segment_k8s_bg_color=\"#073642\""));
    assert!(bash_output.contains("__color_k8s_prod_bg=\"#326ce5\""));
    assert!(bash_output.contains("__gpy_segment_k8s_variant_prod_text_color=\"#aaaaaa\""));
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
    assert_eq!(starship.source, ThemeSource::Builtin);

    // The other builtins remain discoverable.
    assert!(themes.iter().any(|t| t.name == "default"));
    assert!(themes.iter().any(|t| t.name == "text"));
}

#[test]
#[serial]
fn test_theme_discovery_includes_user_themes() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "community", MINIMAL_THEME);

    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", temp_dir.path());
    env.remove("GPY_BUNDLED_PLUGIN_DIR");

    let themes = ThemeManager::discover_available_themes();
    let community = themes
        .iter()
        .find(|theme| theme.name == "community")
        .expect("community theme present");
    assert_eq!(community.source, ThemeSource::User);
    assert!(community.path.is_some());
}

#[test]
#[serial]
fn test_theme_discovery_prefers_user_over_plugin() {
    let user_temp = TempDir::new().unwrap();
    let bundled_temp = TempDir::new().unwrap();

    // User theme
    create_test_theme_toml(&user_temp.path().join("gpy"), "shared", MINIMAL_THEME);

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
entry_type = "file"
provided_segments = ["demo"]
"#,
    )
    .unwrap();
    fs::write(themes_dir.join("shared.toml"), MINIMAL_THEME).unwrap();

    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", user_temp.path());
    env.set("GPY_BUNDLED_PLUGIN_DIR", bundled_temp.path());

    let themes = ThemeManager::discover_available_themes();
    let shared = themes
        .iter()
        .find(|theme| theme.name == "shared")
        .expect("shared theme present");
    assert_eq!(shared.source, ThemeSource::User);
}

#[test]
#[serial]
fn test_theme_export_includes_plugin_segment_file_vars() {
    let xdg_temp = TempDir::new().unwrap();
    let bundled_temp = TempDir::new().unwrap();

    create_test_theme_toml(&xdg_temp.path().join("gpy"), "plugin_files", MINIMAL_THEME);

    let plugin_dir = bundled_temp.path().join("demo");
    let plugin_segments_dir = plugin_dir.join("segments");
    fs::create_dir_all(&plugin_segments_dir).unwrap();
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "demo"
name = "Demo"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["k8s-tools"]
"#,
    )
    .unwrap();
    fs::write(
        plugin_segments_dir.join("k8s-tools.fish"),
        "function segment_k8s-tools_render; end",
    )
    .unwrap();

    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", xdg_temp.path());
    env.set("GPY_BUNDLED_PLUGIN_DIR", bundled_temp.path());

    let manager = ThemeManager::new("plugin_files").unwrap();
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "k8s-tools".to_owned()];
    let output = manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    let expected_path = plugin_segments_dir.join("k8s-tools.fish");
    let expected_line = format!(
        "set -g __gpy_plugin_segment_file_k8s_tools \"{}\"",
        expected_path.display()
    );
    assert!(output.contains(&expected_line));
    assert!(output.contains("set -g __enabled_segments clock k8s-tools"));
}

#[cfg(unix)]
#[test]
#[serial]
fn test_switch_theme_preserves_reload_notifications() {
    use gpy_agent::ipc::ClientDirectory;
    use sigurg::SigurgCounter;
    use std::sync::Arc;

    let doorbell = SigurgCounter::install();

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_home = temp_dir.path().join("gpy");
    let default_path = create_test_theme_toml(&config_home, "default_switch", MINIMAL_THEME);
    let switched_path = create_test_theme_toml(&config_home, "after_switch", CUSTOM_THEME);

    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", temp_dir.path());
    env.set("GPY_THEME_WATCH_POLL_MS", "25");

    let manager = ThemeManager::new("default_switch").expect("theme manager");
    let shell_dir = TempDir::new().expect("create shell dir");
    let client_registry = Arc::new(ClientDirectory::with_shell_dir(
        shell_dir.path().to_path_buf(),
    ));
    let reload_flag = shell_dir
        .path()
        .join(format!("{}.reload", std::process::id()));
    client_registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    let registry_in_hook = Arc::clone(&client_registry);
    manager.set_reload_callback(Arc::new(move |reload: &dyn Fn() -> bool| {
        if reload() {
            registry_in_hook.notify_reload();
        }
    }));
    manager
        .start_watching(Duration::from_millis(50))
        .expect("start watching");
    manager
        .switch_theme("after_switch")
        .expect("switch theme should succeed");

    let initial_count = doorbell.count();
    // The switch itself may already have requested a reload; clear its flag so
    // the assertion below observes the edit's reload.
    let _ = fs::remove_file(&reload_flag);
    let updated_theme = CUSTOM_THEME.replace("prompt_color = \"cyan\"", "prompt_color = \"green\"");
    fs::write(&switched_path, updated_theme).expect("update switched theme");

    // Wait for the edit itself to land, not merely for *a* doorbell: any reload
    // raises the counter, so an unrelated reload could otherwise release the
    // wait before this edit has been read.
    let reloaded = wait_until(Duration::from_secs(2), Duration::from_millis(25), || {
        manager.get().ui.prompt_color.as_deref() == Some("green")
            && doorbell.count() > initial_count
            && reload_flag.exists()
    });
    assert!(
        reloaded,
        "the edited theme should be reloaded after a switch"
    );
    assert!(
        doorbell.count() > initial_count,
        "SIGURG doorbell should still ring after switch_theme"
    );
    assert!(
        reload_flag.exists(),
        "reload flag should still be written after switch_theme"
    );

    manager.stop_watching();
    client_registry.unregister(std::process::id());
    let _ = default_path;
}

/// `switch_theme` stops the watcher and re-arms it through `start_watching`,
/// which must reuse the `WatchRegistry` the manager was started with rather
/// than build a private one: the agent-wide registry is shared by the repo
/// and config coordinators, and an empty private one would silently drop it.
/// `watch_registry_for_test` is the only way to observe which registry
/// currently backs the watcher.
///
/// # Panics
///
/// Panics if `switch_theme` re-arms the watcher on a different
/// `WatchRegistry` than the one it was started with.
#[test]
#[serial]
fn switch_theme_reuses_the_registry_it_was_started_with() {
    use gpy_agent::watcher::WatchRegistry;
    use std::sync::Arc;

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_home = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_home, "registry_before", MINIMAL_THEME);
    create_test_theme_toml(&config_home, "registry_after", CUSTOM_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::new("registry_before").expect("theme manager");
    let registry = Arc::new(WatchRegistry::new());

    manager
        .start_watching_with(Duration::from_millis(50), &registry)
        .expect("start watching with shared registry");

    let armed = manager
        .watch_registry_for_test()
        .expect("registry recorded after start_watching_with");
    assert!(
        Arc::ptr_eq(&armed, &registry),
        "start_watching_with must record the registry it was armed with"
    );

    manager
        .switch_theme("registry_after")
        .expect("switch theme should succeed");

    let after_switch = manager
        .watch_registry_for_test()
        .expect("registry recorded after switch_theme re-arms the watcher");
    assert!(
        Arc::ptr_eq(&after_switch, &registry),
        "switch_theme must re-arm on the same WatchRegistry it was started with, \
         not a private default"
    );

    manager.stop_watching();
}

/// Extract the value assigned to `var` in a Fish theme export, e.g.
/// `set -g __color_clock_bg "blue"` -> `Some("blue")`.
fn exported_value(output: &str, var: &str) -> Option<String> {
    let line = output.lines().find(|line| line.contains(var))?;
    let after_var = line.split_once(var)?.1.trim_start();
    let inside = after_var.strip_prefix('"')?;
    let end = inside.find('"')?;
    inside.get(..end).map(ToOwned::to_owned)
}

/// `switch_theme` installs the new name, path, and content as one
/// atomic swap, so a concurrent `export` can never emit one theme's name
/// alongside another theme's content.
///
/// If name, theme, and path were guarded separately, a reader that took the
/// theme lock just after the content write — and reached the name lock before
/// the writer got there — would see the new theme under the old name. Several
/// reader threads make that interleaving routine rather than exotic: `std`'s
/// `RwLock` is reader-preferring on this platform, so every content write
/// releases a queued burst of readers that then race the writer's remaining
/// writes.
///
/// The pairing is checked on two markers rather than the whole export string:
/// the exported theme name, and a color the two themes genuinely disagree on.
/// Both are asserted to differ between the themes up front, so a torn pairing
/// is detectable in either direction.
#[test]
#[serial]
fn concurrent_export_never_pairs_one_themes_name_with_anothers_colors() {
    use gpy_agent::config::Config;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Barrier};

    const READERS: usize = 3;
    const SWITCHES: usize = 60;
    /// Backstop so a starved writer fails the test instead of hanging it.
    ///
    /// Generous on purpose: the switch loop takes ~20s alone and ~45s sharing
    /// a machine with the rest of the suite, and overshooting this deadline is
    /// a test failure, so it has to sit well clear of ordinary CI load.
    const READER_DEADLINE: Duration = Duration::from_secs(300);

    let temp_dir = TempDir::new().unwrap();
    let config_home = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_home, "race_a", MINIMAL_THEME);
    create_test_theme_toml(&config_home, "race_b", CUSTOM_THEME);
    let empty_plugin_dir = temp_dir.path().join("no-plugins");
    fs::create_dir_all(&empty_plugin_dir).unwrap();
    // Point plugin discovery at an empty directory so `export` is cheap
    // and deterministic: it runs `discover_plugins()` on every call, and
    // the developer's real plugin tree would both slow the readers down
    // and vary the export between machines.
    let mut env = EnvGuard::default();
    env.set("XDG_CONFIG_HOME", temp_dir.path());
    env.set("GPY_BUNDLED_PLUGIN_DIR", &empty_plugin_dir);

    let config = Arc::new(Config::default());

    // The only two (name, clock background) pairs a consistent state can
    // produce. Built through the same public path `switch_theme` uses.
    let observed_pair = |export: &str| -> (String, String) {
        (
            exported_value(export, "__gpy_theme_name").unwrap_or_default(),
            exported_value(export, "__color_clock_bg").unwrap_or_default(),
        )
    };
    let pair_of = |name: &str| -> (String, String) {
        observed_pair(&ThemeManager::new(name).unwrap().export(
            Shell::Fish,
            &config,
            &gpy_agent::template::Palette::default(),
        ))
    };
    let pair_a = pair_of("race_a");
    let pair_b = pair_of("race_b");
    assert_ne!(
        pair_a.0, pair_b.0,
        "the two themes must export different names or this test proves nothing"
    );
    assert_ne!(
        pair_a.1, pair_b.1,
        "the two themes must export different clock colors or a torn pairing would be invisible"
    );
    let valid_pairs = [pair_a, pair_b];

    let manager = Arc::new(ThemeManager::new("race_a").unwrap());
    // `switch_theme` tears the watcher down and builds a new one every call; a
    // 10ms debounce keeps the loop hammering the state lock, which is what the
    // test is actually measuring.
    manager.start_watching(Duration::from_millis(10)).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(READERS + 1));

    let mut readers = Vec::with_capacity(READERS);
    for _ in 0_usize..READERS {
        let reader_manager = Arc::clone(&manager);
        let reader_stop = Arc::clone(&stop);
        let reader_barrier = Arc::clone(&barrier);
        let reader_config = Arc::clone(&config);
        let reader_pairs = valid_pairs.clone();
        readers.push(std::thread::spawn(move || {
            reader_barrier.wait();
            let deadline = Instant::now() + READER_DEADLINE;
            let mut torn: Option<(String, String)> = None;
            while !reader_stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                let export = reader_manager.export(
                    Shell::Fish,
                    &reader_config,
                    &gpy_agent::template::Palette::default(),
                );
                let seen = (
                    exported_value(&export, "__gpy_theme_name").unwrap_or_default(),
                    exported_value(&export, "__color_clock_bg").unwrap_or_default(),
                );
                if !reader_pairs.contains(&seen) && torn.is_none() {
                    torn = Some(seen);
                }
                // Yield rather than sleep: readers should be queued on the
                // state lock as often as possible, since the tear window opens
                // the instant a writer releases it. The switch loop's own
                // elapsed-time assertion below catches the opposite failure
                // mode, a writer starved by these readers.
                std::thread::yield_now();
            }
            torn
        }));
    }

    barrier.wait();
    let switching_started = Instant::now();
    for index in 0_usize..SWITCHES {
        let target = if index % 2 == 0 { "race_b" } else { "race_a" };
        manager.switch_theme(target).unwrap();
    }
    let switching_took = switching_started.elapsed();
    stop.store(true, Ordering::Relaxed);

    let torn: Vec<(String, String)> = readers
        .into_iter()
        .filter_map(|handle| handle.join().unwrap())
        .collect();

    manager.stop_watching();

    // If the readers had starved the writer, the switch loop would still be
    // running when they gave up at their deadline, and `torn` would be empty
    // for want of concurrency rather than because the swap is atomic. Fail on
    // that instead of passing vacuously.
    assert!(
        switching_took < READER_DEADLINE,
        "{SWITCHES} switches took {switching_took:?}, past the readers' deadline: \
         the writer was starved, so this run never tested anything"
    );

    assert!(
        torn.is_empty(),
        "export observed a torn (theme name, clock bg) pairing during switch_theme; \
         valid pairs are {valid_pairs:?}, observed {torn:?}"
    );
}

/// The reload hook's step swaps outside the state lock, so the hook may read
/// the manager after it (a hook under the lock would deadlock here).
#[test]
#[serial]
fn reload_hook_runs_after_swap_and_may_read_the_manager() {
    use std::sync::{Arc, Mutex, PoisonError};

    let temp_dir = TempDir::new().unwrap();
    let theme_path = create_test_theme_toml(&temp_dir.path().join("gpy"), "hooked", MINIMAL_THEME);
    let mut env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = Arc::new(ThemeManager::new("hooked").unwrap());
    let observed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let observed_in_hook = Arc::clone(&observed);
    let manager_in_hook = Arc::clone(&manager);
    manager.set_reload_callback(Arc::new(move |reload: &dyn Fn() -> bool| {
        if !reload() {
            return;
        }
        let icon = manager_in_hook
            .get()
            .ui
            .prompt_icon
            .as_deref()
            .unwrap_or_default()
            .to_owned();
        observed_in_hook
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(icon);
    }));

    env.set("GPY_THEME_WATCH_POLL_MS", "25");
    manager.start_watching(Duration::from_millis(50)).unwrap();
    let seen = wait_until(Duration::from_secs(5), Duration::from_millis(100), || {
        fs::write(&theme_path, CUSTOM_THEME).unwrap();
        observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|icon| icon == "→")
    });
    manager.stop_watching();
    assert!(
        seen,
        "the hook must run after the swap and see the new theme"
    );
}
