//! Tests for `ThemeManager` functionality
//!
//! # Test Isolation
//!
//! These tests manipulate the global `XDG_CONFIG_HOME` environment variable.
//! The `serial_test` crate ensures they run sequentially to avoid interference.
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::str_to_string)]
#![allow(clippy::shadow_unrelated)]

use gpy_agent::shell::Shell;
use gpy_agent::theme::{ThemeManager, ThemeSource};
use serial_test::serial;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// Helper to create a test theme file
fn create_test_theme_toml(dir: &Path, theme_name: &str, content: &str) -> PathBuf {
    let themes_dir = dir.join("themes");
    fs::create_dir_all(&themes_dir).unwrap();

    let theme_path = themes_dir.join(format!("{theme_name}.toml"));
    fs::write(&theme_path, content).unwrap();

    theme_path
}

fn wait_until(timeout: Duration, poll_interval: Duration, predicate: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if predicate() {
            return true;
        }
        std::thread::sleep(poll_interval);
    }
    predicate()
}

// Minimal valid theme TOML
#[allow(clippy::needless_raw_string_hashes)] // Required: string contains '"#' which needs r##
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
#[allow(clippy::needless_raw_string_hashes)] // Consistency: keeping same delimiter as MINIMAL_THEME
const CUSTOM_THEME: &str = r##"
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
"##;

// Theme with an explicit hostname format/icon/trim_at for testing the
// Option-derived hostname exports (format/icon present-but-empty when unset
// elsewhere; non-empty here).
#[allow(clippy::needless_raw_string_hashes)] // Consistency: keeping same delimiter as MINIMAL_THEME
const HOSTNAME_THEME: &str = r##"
[ui]
prompt_icon = "❯"
root_prompt_icon = "#"

[segments.hostname]
format = "[$hostname]($style) "
icon = "🖥"
bg_color = "blue"
text_color = "white"
trim_at = ""
show_always = true
"##;

#[allow(clippy::needless_raw_string_hashes)] // Keep delimiter style consistent with other test themes
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
context_color = "#326ce5"
namespace_color = "#aaaaaa"
secondary_icon = "•"

[segments.my-seg]
bg_color = "cyan"
text_color = "black"
"##;

#[test]
#[serial]
fn test_theme_manager_new_errors_when_missing() {
    // Regression follow-up for #572: a theme name matching no user theme,
    // plugin theme, or builtin must error, not silently succeed with a bare
    // default -- this test used to assert the opposite (the bug #572 fixed).
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().to_path_buf();

    // Set XDG_CONFIG_HOME to temp directory
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_dir.to_str().unwrap());
    }

    let Err(err) = ThemeManager::new("nonexistent") else {
        panic!("an unresolvable theme name must error, not silently succeed with a default theme");
    };
    assert!(
        err.to_string().contains("nonexistent"),
        "error should name the theme, got: {err}"
    );

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_loads_valid_theme() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create theme file
    create_test_theme_toml(&config_dir, "test", MINIMAL_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("test").unwrap();
    let theme = manager.get();

    // Verify theme loaded correctly
    assert_eq!(theme.ui.prompt_icon, "❯");
    assert_eq!(theme.ui.root_prompt_icon, "#");
    assert_eq!(theme.segments.clock.bg_color, "blue");
    assert_eq!(theme.segments.directory.bg_color, "cyan");

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_loads_custom_theme() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create custom theme file
    create_test_theme_toml(&config_dir, "custom", CUSTOM_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("custom").unwrap();
    let theme = manager.get();

    // Verify custom values
    assert_eq!(theme.ui.prompt_icon, "→");
    assert_eq!(theme.ui.root_prompt_icon, "⚡");
    assert_eq!(theme.ui.prompt_color.as_deref(), Some("cyan"));
    assert_eq!(theme.ui.root_prompt_color.as_deref(), Some("red"));
    assert_eq!(theme.segments.clock.bg_color, "magenta");
    assert_eq!(theme.segments.clock.text_color, "yellow");
    assert_eq!(
        theme.segments.clock.time_format,
        Some("%H:%M:%S".to_string())
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

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_export_fish_format() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create theme file
    create_test_theme_toml(&config_dir, "export_test", CUSTOM_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("export_test").unwrap();

    // Export to Fish format
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = vec![
        "clock".to_string(),
        "directory".to_string(),
        "git".to_string(),
    ];
    let fish_output = manager.export(Shell::Fish, &config);

    // Verify Fish output contains expected variables
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
    // __icon_duration was removed from the agent export in #204: duration is
    // agent-rendered so the shell-side icon is no longer needed from the theme.
    assert!(!fish_output.contains("__icon_duration"));
    assert!(fish_output.contains("set -g __enabled_segments clock directory git"));
    assert!(fish_output.contains("set -gx GPY_AGENT_ENABLED \"1\""));
    assert!(fish_output.contains("set -gx GPY_AGENT_SUPERVISOR_ENABLED \"1\""));
    assert!(fish_output.contains("set -gx GPY_LANGUAGE_ENABLED \"1\""));
    assert!(fish_output.contains("set -gx GPY_GIT_ENABLED \"1\""));

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_export_hostname_format_and_icon_when_set() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "hostname_export", HOSTNAME_THEME);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    let manager = ThemeManager::new("hostname_export").unwrap();
    let config = gpy_agent::config::Config::default();
    let fish_output = manager.export(Shell::Fish, &config);

    assert!(fish_output.contains("set -g __color_hostname_bg \"blue\""));
    assert!(fish_output.contains("set -g __color_hostname_fg \"white\""));
    // trim_at = "" in this theme: disables trim, but must still be emitted.
    assert!(fish_output.contains("set -g __hostname_trim_at \"\""));
    assert!(fish_output.contains("set -g __hostname_show_always \"1\""));
    // __hostname_format is a PRESENCE FLAG only ("1" when a format is
    // configured), never the raw format string — the agent re-derives the
    // real template server-side. Exporting the raw string into a
    // double-quoted, eval'd shell assignment would be a shell-injection /
    // `set -u` hazard (see manager.rs export comment).
    assert!(fish_output.contains("set -g __hostname_format \"1\""));
    assert!(!fish_output.contains("[$hostname]($style)"));
    assert!(fish_output.contains("set -g __icon_hostname \"🖥\""));

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_reload() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create initial theme file
    let theme_path = create_test_theme_toml(&config_dir, "reload_test", MINIMAL_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("reload_test").unwrap();

    // Verify initial state
    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon, "❯");
        assert_eq!(theme.segments.clock.bg_color, "blue");
    }

    // Modify theme file
    fs::write(&theme_path, CUSTOM_THEME).unwrap();

    // Reload
    manager.reload().unwrap();

    // Verify new state
    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon, "→");
        assert_eq!(theme.segments.clock.bg_color, "magenta");
    }

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_disabled_git_removed_from_segment_list() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_dir, "disabled_git", CUSTOM_THEME);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    let manager = ThemeManager::new("disabled_git").unwrap();
    let mut config = gpy_agent::config::Config::default();
    config.git.enabled = false;
    config.ui.enabled_segments = vec![
        "clock".to_string(),
        "directory".to_string(),
        "git".to_string(),
    ];

    let fish_output = manager.export(Shell::Fish, &config);

    assert!(!fish_output.contains("clock directory git"));
    assert!(fish_output.contains("set -g __enabled_segments clock directory"));
    assert!(fish_output.contains("set -gx GPY_GIT_ENABLED \"0\""));

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_reload_keeps_old_on_error() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create initial theme file
    let theme_path = create_test_theme_toml(&config_dir, "error_test", MINIMAL_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("error_test").unwrap();

    // Verify initial state
    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon, "❯");
    }

    // Write invalid TOML
    fs::write(&theme_path, "this is not valid TOML { } [ ]").unwrap();

    // Try to reload - should fail
    let result = manager.reload();
    assert!(result.is_err());

    // Verify old theme is still active
    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon, "❯");
    }

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_reload_errors_on_missing_file_and_keeps_last_good_theme() {
    // Regression follow-up for #572: a reload whose theme file vanished (and
    // doesn't match a builtin name) must error, not silently reset the
    // in-memory theme to a bare default -- this test used to assert the
    // latter (the bug #572 fixed). The in-memory theme must also be left
    // exactly as it was: `reload`'s `?` on the failed load means the
    // overwrite never happens.
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create initial theme file
    let theme_path = create_test_theme_toml(&config_dir, "missing_test", MINIMAL_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("missing_test").unwrap();

    // Verify initial state
    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon, "❯");
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

    // Verify the last-good theme is still in place, unchanged.
    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon, "❯");
    }

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_start_stop_watching() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create theme file
    create_test_theme_toml(&config_dir, "watch_test", MINIMAL_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("watch_test").unwrap();

    // Start watching
    let result = manager.start_watching(Duration::from_millis(100), None);
    assert!(result.is_ok());

    // Starting again should be a no-op
    let result = manager.start_watching(Duration::from_millis(100), None);
    assert!(result.is_ok());

    // Stop watching
    manager.stop_watching();

    // Stopping again should be safe
    manager.stop_watching();

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_get_returns_arc_clone() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create theme file
    create_test_theme_toml(&config_dir, "arc_test", MINIMAL_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

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

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_respects_xdg_config_home() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("custom_config").join("gpy");

    // Create theme file
    create_test_theme_toml(&config_dir, "xdg_test", CUSTOM_THEME);

    // Set custom XDG_CONFIG_HOME
    let xdg_path = temp_dir.path().join("custom_config");
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", xdg_path.to_str().unwrap());
    }

    // Load the theme - should find it in custom location
    let manager = ThemeManager::new("xdg_test").unwrap();
    let theme = manager.get();

    // Verify custom theme loaded
    assert_eq!(theme.ui.prompt_icon, "→");
    assert_eq!(theme.segments.clock.bg_color, "magenta");

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_manager_export_includes_all_segments() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create theme file
    create_test_theme_toml(&config_dir, "full_export", CUSTOM_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("full_export").unwrap();

    // Export with all segments
    let all_segments = vec![
        "clock".to_string(),
        "directory".to_string(),
        "duration".to_string(),
        "status".to_string(),
        "git".to_string(),
        "language".to_string(),
    ];
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = all_segments;
    let fish_output = manager.export(Shell::Fish, &config);

    // Clock and status still render shell-side and keep their color exports.
    // directory/duration colors were retired in #199 (now agent-rendered).
    assert!(fish_output.contains("__color_clock_bg"));
    assert!(fish_output.contains("__color_status_ok_bg"));
    assert!(fish_output.contains("__color_status_fail_bg"));

    // Verify delimiter exports
    assert!(fish_output.contains("__segment_delim_first"));
    assert!(fish_output.contains("__segment_delim_start"));
    assert!(fish_output.contains("__segment_delim_end"));
    assert!(fish_output.contains("__segment_delim_last"));

    // Verify enabled segments list
    assert!(
        fish_output
            .contains("set -g __enabled_segments clock directory duration status git language")
    );

    // Cleanup
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn export_omits_migrated_color_vars() {
    // Segments render from agent-provided ANSI; fg/text color exports and
    // format toggles were retired in #199 and remain retired. Only bg colors
    // for directory, duration, git, and language are re-added for powerline
    // chevron tracking (`__gpy_last_segment_bg`). Clock and status still
    // render shell-side and must keep their full `__color_*` exports.
    let temp_dir = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    let manager = ThemeManager::builtin("default").expect("builtin theme manager");
    let config = gpy_agent::config::Config::default();
    let exported = manager.export(Shell::Fish, &config);

    // fg/text color exports for agent-rendered segments remain retired.
    for still_migrated in [
        "__color_directory_fg",
        "__color_duration_fg",
        "__color_character",
        "__color_language_fg",
        "__color_language_text",
    ] {
        assert!(
            !exported.contains(still_migrated),
            "retired segment color var {still_migrated} must not be exported"
        );
    }

    // Legacy format toggles remain retired.
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

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_hot_reload_triggers_on_file_change() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    // Create initial theme file
    let theme_path = create_test_theme_toml(&config_dir, "hotreload", MINIMAL_THEME);

    // Set XDG_CONFIG_HOME
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Load the theme
    let manager = ThemeManager::new("hotreload").unwrap();

    // Verify initial state
    {
        let theme = manager.get();
        assert_eq!(theme.ui.prompt_icon, "❯");
        assert_eq!(theme.segments.clock.bg_color, "blue");
    }

    // Start watching with very short debounce for testing
    unsafe {
        std::env::set_var("GPY_THEME_WATCH_POLL_MS", "25");
    }
    manager
        .start_watching(Duration::from_millis(50), None)
        .unwrap();

    // Give watcher time to start
    std::thread::sleep(Duration::from_millis(100));

    // Modify theme file
    fs::write(&theme_path, CUSTOM_THEME).unwrap();

    // Wait for debounce + processing
    let reloaded = wait_until(Duration::from_secs(2), Duration::from_millis(25), || {
        let theme = manager.get();
        theme.ui.prompt_icon == "→" && theme.segments.clock.bg_color == "magenta"
    });
    assert!(reloaded, "theme should reload within timeout");

    // Stop watching
    manager.stop_watching();

    // Cleanup
    unsafe {
        std::env::remove_var("GPY_THEME_WATCH_POLL_MS");
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

// Test that a reload (flag file + SIGURG doorbell) reaches Fish clients when the theme file changes
//
// This ensures that when a user edits their theme file (e.g., changes colors),
// Fish shells automatically reload the theme variables without needing to restart.
#[cfg(unix)]
#[test]
#[serial]
fn test_theme_change_sends_reload() {
    use gpy_agent::ipc::ClientDirectory;
    use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DOORBELL_COUNTER: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn count_doorbell(_signal: i32) {
        DOORBELL_COUNTER.fetch_add(1, Ordering::Relaxed);
    }

    DOORBELL_COUNTER.store(0, Ordering::Relaxed);

    let doorbell_handler = SigAction::new(
        SigHandler::Handler(count_doorbell),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let previous =
        unsafe { sigaction(Signal::SIGURG, &doorbell_handler) }.expect("install SIGURG handler");

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_home = temp_dir.path();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config_home);
    }

    // Create initial theme
    let theme_path = create_test_theme_toml(&config_home.join("gpy"), "reload_test", MINIMAL_THEME);

    // Create theme manager and client registry (flags land in a tempdir)
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

    let initial_count = DOORBELL_COUNTER.load(Ordering::Relaxed);

    // Start watching with client registry
    unsafe {
        std::env::set_var("GPY_THEME_WATCH_POLL_MS", "25");
    }
    manager
        .start_watching(
            Duration::from_millis(50),
            Some(Arc::clone(&client_registry)),
        )
        .unwrap();

    // Give watcher time to start
    std::thread::sleep(Duration::from_millis(100));

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

    fs::write(&theme_path, updated_theme).unwrap();

    // Wait for both the doorbell and the actual theme reload. Under full-suite load
    // the signal can arrive slightly before the manager's read observes the new
    // theme, so asserting on the signal counter alone is racy.
    let signaled = wait_until(Duration::from_secs(2), Duration::from_millis(25), || {
        let signal_seen = DOORBELL_COUNTER.load(Ordering::Relaxed) > initial_count;
        let theme = manager.get();
        signal_seen && theme.ui.prompt_icon == ">" && theme.segments.clock.bg_color == "red"
    });
    let final_count = DOORBELL_COUNTER.load(Ordering::Relaxed);
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

    // Verify theme was actually reloaded
    let theme = manager.get();
    assert_eq!(theme.ui.prompt_icon, ">");
    assert_eq!(theme.segments.clock.bg_color, "red");

    // Stop watching
    manager.stop_watching();

    // SIGURG's default disposition is ignore, so restoring it is safe even if a
    // late doorbell is still in flight from the watcher thread.
    unsafe {
        sigaction(Signal::SIGURG, &previous).expect("restore SIGURG");
        std::env::remove_var("GPY_THEME_WATCH_POLL_MS");
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[cfg(not(unix))]
#[test]
fn test_theme_change_sends_reload() {
    // Placeholder for non-Unix platforms
}

#[test]
#[serial]
fn test_theme_export_fish_includes_plugin_segment_vars() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_dir, "plugin_export_fish", PLUGIN_SEGMENT_THEME);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    let manager = ThemeManager::new("plugin_export_fish").unwrap();
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "k8s".to_owned()];
    let output = manager.export(Shell::Fish, &config);

    assert!(output.contains("set -gx GPY_UI_DIRECTORY_DISPLAY \"basename\""));
    assert!(output.contains("set -g __gpy_segment_k8s_id \"k8s\""));
    assert!(output.contains("set -g __gpy_segment_k8s_bg_color \"#073642\""));
    assert!(output.contains("set -g __gpy_segment_k8s_text_color \"white\""));
    assert!(output.contains("set -g __gpy_segment_k8s_icon \"⎈\""));
    assert!(output.contains("set -g __gpy_segment_k8s_context_color \"#326ce5\""));
    assert!(output.contains("set -g __gpy_segment_k8s_namespace_color \"#aaaaaa\""));
    assert!(output.contains("set -g __gpy_segment_k8s_secondary_icon \"•\""));
    assert!(output.contains("set -g __color_k8s_bg \"#073642\""));
    assert!(output.contains("set -g __color_k8s_fg \"white\""));
    assert!(output.contains("set -g __color_k8s_context \"#326ce5\""));
    assert!(output.contains("set -g __icon_k8s \"⎈\""));
    assert!(output.contains("set -g __icon_k8s_secondary \"•\""));
    assert!(output.contains("set -g __gpy_segment_my_seg_id \"my-seg\""));
    assert!(output.contains("set -g __color_my_seg_bg \"cyan\""));
    assert!(output.contains("set -g __color_clock_bg \"black\""));

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_export_zsh_bash_include_plugin_segment_vars() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_dir, "plugin_export_posix", PLUGIN_SEGMENT_THEME);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    let manager = ThemeManager::new("plugin_export_posix").unwrap();
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "k8s".to_owned()];

    let zsh_output = manager.export(Shell::Zsh, &config);
    assert!(zsh_output.contains("export GPY_UI_DIRECTORY_DISPLAY=\"basename\""));
    assert!(zsh_output.contains("typeset -g __gpy_segment_k8s_id=\"k8s\""));
    assert!(zsh_output.contains("typeset -g __gpy_segment_k8s_bg_color=\"#073642\""));
    assert!(zsh_output.contains("typeset -g __color_k8s_context=\"#326ce5\""));
    assert!(zsh_output.contains("typeset -g __icon_k8s_secondary=\"•\""));

    let bash_output = manager.export(Shell::Bash, &config);
    assert!(bash_output.contains("export GPY_UI_DIRECTORY_DISPLAY=\"basename\""));
    assert!(bash_output.contains("export __gpy_segment_k8s_id=\"k8s\""));
    assert!(bash_output.contains("export __gpy_segment_k8s_bg_color=\"#073642\""));
    assert!(bash_output.contains("export __color_k8s_context=\"#326ce5\""));
    assert!(bash_output.contains("export __icon_k8s_secondary=\"•\""));

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_discovery_includes_starship_builtin() {
    let temp_dir = TempDir::new().unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        std::env::remove_var("GPY_BUNDLED_PLUGIN_DIR");
    }

    let themes = ThemeManager::discover_available_themes();
    let starship = themes
        .iter()
        .find(|theme| theme.name == "starship")
        .expect("starship preset present as builtin");
    assert_eq!(starship.source, ThemeSource::Builtin);

    // The other builtins remain discoverable (no regression).
    assert!(themes.iter().any(|t| t.name == "default"));
    assert!(themes.iter().any(|t| t.name == "text"));

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
#[serial]
fn test_theme_discovery_includes_user_themes() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("gpy");

    create_test_theme_toml(&config_dir, "community", MINIMAL_THEME);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        std::env::remove_var("GPY_BUNDLED_PLUGIN_DIR");
    }

    let themes = ThemeManager::discover_available_themes();
    let community = themes
        .iter()
        .find(|theme| theme.name == "community")
        .expect("community theme present");
    assert_eq!(community.source, ThemeSource::User);
    assert!(community.path.is_some());

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
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

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", user_temp.path());
        std::env::set_var("GPY_BUNDLED_PLUGIN_DIR", bundled_temp.path());
    }

    let themes = ThemeManager::discover_available_themes();
    let shared = themes
        .iter()
        .find(|theme| theme.name == "shared")
        .expect("shared theme present");
    assert_eq!(shared.source, ThemeSource::User);

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("GPY_BUNDLED_PLUGIN_DIR");
    }
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

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", xdg_temp.path());
        std::env::set_var("GPY_BUNDLED_PLUGIN_DIR", bundled_temp.path());
    }

    let manager = ThemeManager::new("plugin_files").unwrap();
    let mut config = gpy_agent::config::Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "k8s-tools".to_owned()];
    let output = manager.export(Shell::Fish, &config);

    let expected_path = plugin_segments_dir.join("k8s-tools.fish");
    let expected_line = format!(
        "set -g __gpy_plugin_segment_file_k8s_tools \"{}\"",
        expected_path.display()
    );
    assert!(output.contains(&expected_line));
    assert!(output.contains("set -g __enabled_segments clock k8s-tools"));

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("GPY_BUNDLED_PLUGIN_DIR");
    }
}

#[cfg(unix)]
#[test]
#[serial]
fn test_switch_theme_preserves_reload_notifications() {
    use gpy_agent::ipc::ClientDirectory;
    use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DOORBELL_COUNTER_AFTER_SWITCH: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn count_doorbell(_signal: i32) {
        DOORBELL_COUNTER_AFTER_SWITCH.fetch_add(1, Ordering::Relaxed);
    }

    DOORBELL_COUNTER_AFTER_SWITCH.store(0, Ordering::Relaxed);

    let doorbell_handler = SigAction::new(
        SigHandler::Handler(count_doorbell),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let previous =
        unsafe { sigaction(Signal::SIGURG, &doorbell_handler) }.expect("install SIGURG handler");

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_home = temp_dir.path().join("gpy");
    let default_path = create_test_theme_toml(&config_home, "default_switch", MINIMAL_THEME);
    let switched_path = create_test_theme_toml(&config_home, "after_switch", CUSTOM_THEME);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        std::env::set_var("GPY_THEME_WATCH_POLL_MS", "25");
    }

    let manager = ThemeManager::new("default_switch").expect("theme manager");
    let shell_dir = TempDir::new().expect("create shell dir");
    let client_registry = Arc::new(ClientDirectory::with_shell_dir(
        shell_dir.path().to_path_buf(),
    ));
    let reload_flag = shell_dir
        .path()
        .join(format!("{}.reload", std::process::id()));
    client_registry.register(std::process::id(), Some(std::env::current_dir().unwrap()));

    manager
        .start_watching(
            Duration::from_millis(50),
            Some(Arc::clone(&client_registry)),
        )
        .expect("start watching");
    manager
        .switch_theme("after_switch")
        .expect("switch theme should succeed");

    let initial_count = DOORBELL_COUNTER_AFTER_SWITCH.load(Ordering::Relaxed);
    // The switch itself may already have requested a reload; clear its flag so
    // the assertion below observes the edit's reload.
    let _ = fs::remove_file(&reload_flag);
    let updated_theme = CUSTOM_THEME.replace("prompt_color = \"cyan\"", "prompt_color = \"green\"");
    fs::write(&switched_path, updated_theme).expect("update switched theme");

    // Wait for the edit itself to land, not merely for *a* doorbell: any reload
    // raises the counter, so waiting on the counter let an unrelated reload
    // release the wait before this edit had been read, and the test then failed
    // on the theme assertion below with whatever was loaded instead (#551).
    let reloaded = wait_until(Duration::from_secs(2), Duration::from_millis(25), || {
        manager.get().ui.prompt_color.as_deref() == Some("green")
            && DOORBELL_COUNTER_AFTER_SWITCH.load(Ordering::Relaxed) > initial_count
            && reload_flag.exists()
    });
    assert!(
        reloaded,
        "the edited theme should be reloaded after a switch"
    );
    assert!(
        DOORBELL_COUNTER_AFTER_SWITCH.load(Ordering::Relaxed) > initial_count,
        "SIGURG doorbell should still ring after switch_theme"
    );
    assert!(
        reload_flag.exists(),
        "reload flag should still be written after switch_theme"
    );

    manager.stop_watching();
    client_registry.unregister(std::process::id());

    // SIGURG's default disposition is ignore, so restoring it is safe even if a
    // late doorbell is still in flight.
    unsafe {
        sigaction(Signal::SIGURG, &previous).expect("restore SIGURG");
        std::env::remove_var("GPY_THEME_WATCH_POLL_MS");
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    let _ = default_path;
}

/// #663: `switch_theme` stops the watcher and re-arms it through
/// `start_watching`, which used to build a fresh private `WatchRegistry`
/// (`Arc::new(WatchRegistry::new())`) instead of reusing the one the manager
/// was started with. From the first switch onward the theme coordinator ran
/// on an empty registry, dropping the agent-wide one the repo and config
/// coordinators share (#617).
///
/// Requires the `test-support` feature for `watch_registry_for_test`, the
/// only way to observe which registry currently backs the watcher.
///
/// # Panics
///
/// Panics if `switch_theme` re-arms the watcher on a different
/// `WatchRegistry` than the one it was started with.
#[cfg(feature = "test-support")]
#[test]
#[serial]
fn switch_theme_reuses_the_registry_it_was_started_with() {
    use gpy_agent::watcher::WatchRegistry;
    use std::sync::Arc;

    let temp_dir = TempDir::new().expect("create temp dir");
    let config_home = temp_dir.path().join("gpy");
    create_test_theme_toml(&config_home, "registry_before", MINIMAL_THEME);
    create_test_theme_toml(&config_home, "registry_after", CUSTOM_THEME);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
    }

    let manager = ThemeManager::new("registry_before").expect("theme manager");
    let registry = Arc::new(WatchRegistry::new());

    manager
        .start_watching_with(Duration::from_millis(50), None, &registry)
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
         not a private default (#663)"
    );

    manager.stop_watching();

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

/// #407 round-trip: editing a single theme field via `save_user_theme`,
/// then reloading through `ThemeManager::new`, changes only that field and
/// carries every other field through untouched.
#[test]
#[serial]
fn save_user_theme_round_trip_changes_only_intended_field() {
    let temp_dir = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Deterministic base: the embedded builtin `default`, independent of any
    // theme file the developer's machine might have installed.
    let base = (*ThemeManager::builtin("default").unwrap().get()).clone();

    // The one intended edit, applied to a clone of the base.
    let mut edited = base.clone();
    edited.segments.clock.time_format = Some("24".to_owned());
    edited.segments.clock.show_seconds = Some(true);

    let written_path = ThemeManager::save_user_theme("default", &edited).unwrap();
    assert_eq!(
        written_path,
        temp_dir
            .path()
            .join("gpy")
            .join("themes")
            .join("default.toml"),
        "must write to the user themes directory"
    );
    assert!(written_path.exists(), "the user theme file should exist");

    // Reload through the real resolution path — the freshly-written user copy
    // now shadows the builtin.
    let reloaded = ThemeManager::new("default").unwrap().get();

    // The intended field changed.
    assert_eq!(reloaded.segments.clock.time_format.as_deref(), Some("24"));
    assert_eq!(reloaded.segments.clock.show_seconds, Some(true));

    // Nothing else did. Comparing as `toml::Value` (a sorted map) makes the
    // check order-independent for the theme's `HashMap` fields
    // (`language.overrides`, `git_style`, plugin tables), which serialize in
    // nondeterministic order.
    let reloaded_val = toml::Value::try_from(&*reloaded).unwrap();
    let expected_val = toml::Value::try_from(&edited).unwrap();
    assert_eq!(
        reloaded_val, expected_val,
        "only the intended field(s) should differ from the base"
    );

    let base_val = toml::Value::try_from(&base).unwrap();
    assert_ne!(
        reloaded_val, base_val,
        "the edit must actually change the theme relative to the base"
    );

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

/// #407 AC: `save_user_theme` only ever materializes a copy under the user
/// themes directory — the written theme is discovered as a `User` source,
/// shadowing (never replacing) the builtin.
#[test]
#[serial]
fn save_user_theme_materializes_a_user_source_copy() {
    let temp_dir = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    // Before writing, `default` resolves to the builtin.
    let before = ThemeManager::discover_available_themes()
        .into_iter()
        .find(|theme| theme.name == "default")
        .expect("default is always discoverable");
    assert_eq!(before.source, ThemeSource::Builtin);

    let theme = (*ThemeManager::builtin("default").unwrap().get()).clone();
    ThemeManager::save_user_theme("default", &theme).unwrap();

    // After writing, `default` resolves to the user copy under XDG.
    let after = ThemeManager::discover_available_themes()
        .into_iter()
        .find(|theme| theme.name == "default")
        .expect("default still discoverable");
    assert_eq!(after.source, ThemeSource::User);
    assert_eq!(
        after.path,
        Some(
            temp_dir
                .path()
                .join("gpy")
                .join("themes")
                .join("default.toml")
        )
    );

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

/// `save_user_theme` rejects an unsafe theme name before touching the
/// filesystem — no path-traversal write escapes the user themes directory.
#[test]
#[serial]
fn save_user_theme_rejects_unsafe_name() {
    let temp_dir = TempDir::new().unwrap();
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path().to_str().unwrap());
    }

    let theme = (*ThemeManager::builtin("default").unwrap().get()).clone();
    let result = ThemeManager::save_user_theme("../escape", &theme);
    assert!(result.is_err(), "unsafe theme name must be rejected");

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
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

/// #588: `switch_theme` installs the new name, path, and content as one
/// atomic swap, so a concurrent `export` can never emit one theme's name
/// alongside another theme's content.
///
/// Before #588 `ThemeManager` kept `theme_name`, `theme`, and `theme_path` in
/// three independent `RwLock`s. `switch_theme` wrote them in three separate
/// critical sections and `export` read two of them in two separate
/// acquisitions, so a reader that took the theme lock just after the content
/// write — and reached the name lock before the writer got there — saw the new
/// theme under the old name. Several reader threads make that interleaving
/// routine rather than exotic: `std`'s `RwLock` is reader-preferring on this
/// platform, so every content write releases a queued burst of readers that
/// then race the writer's two remaining writes.
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
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        // Point plugin discovery at an empty directory so `export` is cheap
        // and deterministic: it runs `discover_plugins()` on every call, and
        // the developer's real plugin tree would both slow the readers down
        // and vary the export between machines.
        std::env::set_var("GPY_BUNDLED_PLUGIN_DIR", &empty_plugin_dir);
    }

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
        observed_pair(
            &ThemeManager::new(name)
                .unwrap()
                .export(Shell::Fish, &config),
        )
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
    // `switch_theme` tears the watcher down and builds a new one every call,
    // and `WatchCoordinator::stop` joins a flush thread that only checks its
    // stop flag once per debounce interval. At the 5s default that is 5s of
    // dead time per switch; a 10ms debounce keeps the loop hammering the state
    // lock, which is what the test is actually measuring.
    manager
        .start_watching(Duration::from_millis(10), None)
        .unwrap();
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
                let export = reader_manager.export(Shell::Fish, &reader_config);
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
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("GPY_BUNDLED_PLUGIN_DIR");
    }

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

/// #838: the builtin's raw source comes back verbatim. Runs with an empty
/// config root so an installed `~/.config/gpy/themes/default.toml` cannot
/// shadow the builtin.
#[test]
#[serial]
fn theme_source_content_returns_raw_builtin_content() {
    let temp_dir = TempDir::new().unwrap();

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        std::env::remove_var("GPY_BUNDLED_PLUGIN_DIR");
    }

    let content = ThemeManager::theme_source_content("default");

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    assert_eq!(
        content.expect("builtin default theme content should resolve"),
        ThemeManager::default_theme_template(),
        "should return the exact embedded builtin content, not a re-serialized copy"
    );
}
