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
use gpy_agent::theme::ThemeManager;
use serial_test::serial;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use wait::wait_until;

/// The prompt values of `manager`'s theme under the default config.
fn values(manager: &ThemeManager) -> HashMap<String, String> {
    gpy_agent::theme::export::prompt_shell_values(
        &manager.get(),
        &gpy_agent::config::Config::default(),
        &gpy_agent::template::Palette::default(),
    )
}

// Helper to create a test theme file
fn create_test_theme_toml(dir: &Path, theme_name: &str, content: &str) -> PathBuf {
    let themes_dir = dir.join("themes");
    fs::create_dir_all(&themes_dir).unwrap();

    let theme_path = themes_dir.join(format!("{theme_name}.toml"));
    fs::write(&theme_path, content).unwrap();

    theme_path
}

const MINIMAL_THEME: &str = r##"
[ui]
root_prompt_icon = "#"

[segments.character]
success_symbol = "❯"
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

const CUSTOM_THEME: &str = r#"
[ui]
root_prompt_icon = "⚡"
root_prompt_color = "red"

[segments.character]
success_symbol = "→"
success_color = "cyan"
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
root_prompt_icon = "⚡"

[segments.character]
success_symbol = "→"
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

    assert_eq!(theme.segments.character.success_symbol, "❯");
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

    assert_eq!(theme.segments.character.success_symbol, "→");
    assert_eq!(theme.ui.root_prompt_icon, "⚡");
    assert_eq!(theme.segments.character.success_color, "cyan");
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
fn test_theme_manager_values() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "values_test", CUSTOM_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::new("values_test").unwrap();
    let values = values(&manager);

    for (name, expected) in [
        ("__icon_prompt", "→"),
        ("__icon_root_prompt", "⚡"),
        ("__prompt_color", "cyan"),
        ("__root_prompt_color", "red"),
        ("__color_clock_bg", "magenta"),
        ("__color_clock_fg", "yellow"),
        ("__time_format", "%H:%M:%S"),
        ("__clock_show_seconds", "1"),
    ] {
        assert_eq!(
            values.get(name).map(String::as_str),
            Some(expected),
            "{name}"
        );
    }
    // Duration is agent-rendered: no prompt-side duration icon.
    assert!(!values.contains_key("__icon_duration"));
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
        assert_eq!(theme.segments.character.success_symbol, "❯");
        assert_eq!(theme.segments.clock.bg_color, "blue");
    }

    // Modify theme file
    fs::write(&theme_path, CUSTOM_THEME).unwrap();

    // Reload
    manager.reload().unwrap();

    {
        let theme = manager.get();
        assert_eq!(theme.segments.character.success_symbol, "→");
        assert_eq!(theme.segments.clock.bg_color, "magenta");
    }
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
        assert_eq!(theme.segments.character.success_symbol, "❯");
    }

    // Write invalid TOML
    fs::write(&theme_path, "this is not valid TOML { } [ ]").unwrap();

    // Try to reload - should fail
    let result = manager.reload();
    assert!(result.is_err());

    {
        let theme = manager.get();
        assert_eq!(theme.segments.character.success_symbol, "❯");
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
        assert_eq!(theme.segments.character.success_symbol, "❯");
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
        assert_eq!(theme.segments.character.success_symbol, "❯");
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
    assert_eq!(
        theme1.segments.character.success_symbol,
        theme2.segments.character.success_symbol
    );
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

    assert_eq!(theme.segments.character.success_symbol, "→");
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
        ThemeManager::user_themes_dir().unwrap(),
        xdg_path.join("gpy").join("themes")
    );
    let manager = ThemeManager::new("odd_bytes").unwrap();
    assert_eq!(manager.get().segments.character.success_symbol, "→");
}

#[test]
#[serial]
fn test_theme_manager_values_include_every_builtin_segment() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "full_values", CUSTOM_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::new("full_values").unwrap();
    let values = values(&manager);

    for name in [
        "__color_clock_bg",
        "__color_status_ok_bg",
        "__color_status_fail_bg",
        "__segment_delim_first",
        "__segment_delim_start",
        "__segment_delim_end",
        "__segment_delim_last",
    ] {
        assert!(values.contains_key(name), "missing {name}");
    }
}

#[test]
#[serial]
fn values_omit_migrated_color_vars() {
    // Segments render from agent-provided ANSI, so their fg/text colors and
    // format toggles are not prompt values. Only bg colors for directory,
    // duration, git, and language are, for powerline chevrons. Clock and
    // status keep their full `__color_*` values.
    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::builtin("default").expect("builtin theme manager");
    let values = values(&manager);
    let has = |prefix: &str| values.keys().any(|name| name.starts_with(prefix));

    // No fg/text colors for agent-rendered segments.
    for still_migrated in [
        "__color_directory_fg",
        "__color_duration_fg",
        "__color_character",
        "__color_language_fg",
        "__color_language_text",
    ] {
        assert!(
            !has(still_migrated),
            "agent-rendered segment color var {still_migrated} must not be set"
        );
    }

    // No legacy format toggles.
    for toggle in [
        "__gpy_directory_format",
        "__gpy_duration_format",
        "__gpy_character_format",
    ] {
        assert!(
            !has(toggle),
            "legacy format toggle {toggle} must not be set"
        );
    }

    // Bg colors for powerline chevrons.
    for bg_var in [
        "__color_directory_bg",
        "__color_duration_bg",
        "__color_git_clean_bg",
        "__color_git_dirty_bg",
        "__color_language_bg",
    ] {
        assert!(
            values.contains_key(bg_var),
            "powerline bg var {bg_var} must be set"
        );
    }

    assert!(has("__color_clock_"), "clock color vars must be set");
    assert!(has("__color_status_"), "status color vars must be set");
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
        assert_eq!(theme.segments.character.success_symbol, "❯");
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
        theme.segments.character.success_symbol == "→" && theme.segments.clock.bg_color == "magenta"
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

[segments.character]
success_symbol = ">"
success_color = "cyan"

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
            && theme.segments.character.success_symbol == ">"
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
    assert_eq!(theme.segments.character.success_symbol, ">");
    assert_eq!(theme.segments.clock.bg_color, "red");

    // Stop watching
    manager.stop_watching();
}

#[test]
#[serial]
fn test_theme_values_include_plugin_segment_vars() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_dir, "plugin_values", PLUGIN_SEGMENT_THEME);

    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());

    let manager = ThemeManager::new("plugin_values").unwrap();
    let values = values(&manager);

    for (name, expected) in [
        ("__gpy_segment_k8s_id", "k8s"),
        ("__gpy_segment_k8s_bg_color", "#073642"),
        ("__gpy_segment_k8s_text_color", "white"),
        ("__gpy_segment_k8s_icon", "⎈"),
        ("__gpy_segment_k8s_variant_prod_bg_color", "#326ce5"),
        ("__gpy_segment_k8s_variant_prod_text_color", "#aaaaaa"),
        ("__color_k8s_bg", "#073642"),
        ("__color_k8s_fg", "white"),
        ("__color_k8s_prod_bg", "#326ce5"),
        ("__color_k8s_prod_fg", "#aaaaaa"),
        ("__icon_k8s", "⎈"),
        ("__gpy_segment_my_seg_id", "my-seg"),
        ("__color_my_seg_bg", "cyan"),
        ("__color_clock_bg", "black"),
    ] {
        assert_eq!(
            values.get(name).map(String::as_str),
            Some(expected),
            "{name}"
        );
    }
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
    let updated_theme =
        CUSTOM_THEME.replace("success_color = \"cyan\"", "success_color = \"green\"");
    fs::write(&switched_path, updated_theme).expect("update switched theme");

    // Wait for the edit itself to land, not merely for *a* doorbell: any reload
    // raises the counter, so an unrelated reload could otherwise release the
    // wait before this edit has been read.
    let reloaded = wait_until(Duration::from_secs(2), Duration::from_millis(25), || {
        manager.get().segments.character.success_color == "green"
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
            .segments
            .character
            .success_symbol
            .clone();
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
