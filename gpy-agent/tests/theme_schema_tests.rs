#![allow(clippy::panic)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::config::loader;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write_theme(root: &Path, name: &str, contents: &str) -> PathBuf {
    let themes_dir = root.join("gpy").join("themes");
    fs::create_dir_all(&themes_dir).expect("create themes dir");
    let path = themes_dir.join(format!("{name}.toml"));
    fs::write(&path, contents).expect("write theme");
    path
}

fn write_config(root: &Path, contents: &str) {
    let cfg_dir = root.join("gpy");
    fs::create_dir_all(&cfg_dir).expect("create config dir");
    fs::write(cfg_dir.join("config.toml"), contents).expect("write config");
}

#[test]
fn load_theme_supports_segment_hierarchy() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    let theme_path = write_theme(
        &xdg_config,
        "schema-test",
        r#"
[ui]
prompt_icon = "::"
prompt_color = "yellow"

[segments.directory]
bg_color = "magenta"
text_color = "white"

[segments.duration]
icon = "X"
bg_color = "black"
text_color = "white"
"#,
    );

    let theme =
        loader::load_theme_from_path(theme_path.to_str().expect("path str")).expect("load theme");
    assert_eq!(theme.ui.prompt_icon, "::");
    assert_eq!(theme.segments.directory.bg_color, "magenta");
    assert_eq!(theme.segments.duration.icon.as_deref(), Some("X"));
    assert!(theme.segments.plugin.is_empty());
}

#[test]
fn load_theme_supports_plugin_segment_styling() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    let theme_path = write_theme(
        &xdg_config,
        "plugin-style",
        r##"
[ui]
prompt_icon = ">"

[segments.clock]
bg_color = "black"
text_color = "white"

[segments.k8s]
bg_color = "#073642"
text_color = "white"
icon = "⎈"
context_color = "#326ce5"
namespace_color = "#aaaaaa"

[segments.k8s.open]
icon = "["
icon_color = "match_text"
bg_color = "match_bg"
"##,
    );

    let theme =
        loader::load_theme_from_path(theme_path.to_str().expect("path str")).expect("load theme");
    let k8s = theme.segments.plugin.get("k8s").expect("k8s style present");
    assert_eq!(k8s.bg_color.as_deref(), Some("#073642"));
    assert_eq!(k8s.text_color.as_deref(), Some("white"));
    assert_eq!(k8s.icon.as_deref(), Some("⎈"));
    assert_eq!(
        k8s.properties.get("context_color").map(String::as_str),
        Some("#326ce5")
    );
    assert_eq!(
        k8s.properties.get("namespace_color").map(String::as_str),
        Some("#aaaaaa")
    );
    assert_eq!(
        k8s.open.as_ref().map(|delimiter| delimiter.icon.as_str()),
        Some("[")
    );
}

#[test]
fn theme_export_emits_new_schema_variables() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    write_theme(
        &xdg_config,
        "export-test",
        r#"
[ui]
prompt_icon = ">"
prompt_color = "green"

[segments.clock]
bg_color = "black"
text_color = "white"
show_seconds = true
"#,
    );

    write_config(&xdg_config, "[ui]\ntheme = \"export-test\"\n");

    let output = Command::new("./target/debug/gpy-agent")
        .args(["theme", "export", "--format", "fish"])
        .env("XDG_CONFIG_HOME", &xdg_config)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run gpy-agent theme export");

    assert!(
        output.status.success(),
        "theme export should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("__gpy_ui_prompt_icon"),
        "Theme export should include prompt icon variable. Got: {stdout}"
    );
    assert!(
        stdout.contains("__clock_show_seconds \"1\""),
        "Theme export should emit clock seconds toggle. Got: {stdout}"
    );
}

#[test]
fn starship_preset_loads_with_format_templates() {
    // The builtin Starship preset is embedded; load it by name (no file on disk).
    let theme = loader::load_theme_from_path("starship.toml").expect("starship preset should load");

    // Every styled segment drives the template engine via a `format`.
    assert!(
        theme.segments.directory.format.is_some(),
        "directory format"
    );
    assert!(theme.segments.git.format.is_some(), "git format");
    assert!(theme.segments.language.format.is_some(), "language format");
    assert!(theme.segments.duration.format.is_some(), "duration format");
    assert!(
        theme.segments.character.format.is_some(),
        "character format"
    );
    assert!(theme.segments.hostname.format.is_some(), "hostname format");
    // SSH-only by default, matching Starship's ssh_only = true; the shell's
    // detect (not the agent) gates visibility off-SSH (#259).
    assert!(
        !theme.segments.hostname.show_always,
        "starship preset hostname is SSH-only by default"
    );

    // Two-line layout is on for the preset.
    assert!(theme.ui.two_line, "starship preset is two-line");

    // The exit-colored character uses Starship's green/red.
    assert_eq!(theme.segments.character.success_color, "green");
    assert_eq!(theme.segments.character.error_color, "red");

    // All format templates must render cleanly (no silent fallback on the prompt).
    gpy_agent::config::validation::templates::validate_segment_templates(
        &theme,
        &gpy_agent::template::Palette::default(),
    )
    .expect("starship templates valid");
}

#[test]
fn starship_preset_clock_blends_into_flat_look() {
    // The clock is the one segment still rendered shell-side (no `format`), so it
    // takes its color from `__color_clock_bg`. If the preset omits the clock
    // section, `ClockTheme::bg_color` defaults to "black", which renders a black
    // box that clashes with the otherwise-transparent flat Starship look. The
    // preset must explicitly give the clock a transparent background so an enabled
    // clock blends in like every other segment.
    let theme = loader::load_theme_from_path("starship.toml").expect("starship preset should load");
    assert_eq!(
        theme.segments.clock.bg_color, "transparent",
        "starship preset clock must use a transparent background to match the flat look"
    );
}

#[test]
fn starship_preset_recommends_full_ui_layout() {
    // The Starship preset declares its recommended segment order, directory
    // display, and icon visibility so `gpy theme use starship --force`
    // can yield Starship's look without manual config edits.
    let theme = loader::load_theme_from_path("starship.toml").expect("starship preset should load");
    let recommended = theme
        .ui
        .recommended
        .expect("starship preset should recommend ui settings");

    assert_eq!(
        recommended.enabled_segments,
        Some(vec![
            "username".to_owned(),
            "hostname".to_owned(),
            "directory".to_owned(),
            "git".to_owned(),
            "language".to_owned(),
            "duration".to_owned()
        ]),
        "starship preset should recommend Starship's segment order"
    );
    let directory = recommended
        .directory
        .as_ref()
        .expect("starship preset should recommend directory settings");
    assert_eq!(
        directory.display,
        Some(gpy_agent::config::types::DirectoryDisplay::Truncated),
        "starship preset should recommend a truncated path"
    );
    assert_eq!(
        directory
            .truncation_length
            .map(gpy_agent::config::types::DirectoryTruncationLength::get),
        Some(3),
        "starship preset should recommend truncation_length 3"
    );
    assert_eq!(
        directory.truncate_to_repo,
        Some(true),
        "starship preset should recommend truncate_to_repo for parity"
    );
    assert_eq!(
        recommended.show_icons,
        Some(true),
        "starship preset should recommend language icons"
    );
}

#[test]
fn default_preset_recommends_builtin_defaults() {
    // The default theme recommends GPY's stock settings so that
    // `gpy theme use default --force` cleanly round-trips back from
    // another preset; the text theme leaves config alone.
    let default = loader::load_theme_from_path("default.toml").expect("default theme should load");
    let recommended = default
        .ui
        .recommended
        .expect("default theme should recommend builtin settings");
    assert_eq!(
        recommended.enabled_segments,
        Some(vec![
            "clock".to_owned(),
            "duration".to_owned(),
            "language".to_owned(),
            "directory".to_owned(),
            "git".to_owned()
        ])
    );
    let directory = recommended
        .directory
        .as_ref()
        .expect("default theme should recommend directory settings");
    assert_eq!(
        directory.display,
        Some(gpy_agent::config::types::DirectoryDisplay::Basename)
    );
    assert_eq!(
        directory.truncate_to_repo,
        Some(false),
        "default preset should recommend truncate_to_repo off for clean round-trip"
    );
    assert_eq!(recommended.show_icons, Some(true));

    let text = loader::load_theme_from_path("text.toml").expect("text theme should load");
    assert!(
        text.ui.recommended.is_none(),
        "text theme should not recommend ui settings"
    );
}

#[test]
fn theme_use_force_writes_recommended_ui_settings() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    // Config sets no explicit layout fields, so --force fills every recommended
    // field from the preset.
    write_config(&xdg_config, "[ui]\ntheme = \"default\"\n");

    let output = Command::new("./target/debug/gpy")
        .args(["theme", "use", "starship", "--force"])
        .env("XDG_CONFIG_HOME", &xdg_config)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run gpy theme use --force");
    assert!(
        output.status.success(),
        "theme use --force should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let written = fs::read_to_string(xdg_config.join("gpy").join("config.toml"))
        .expect("read updated config");
    let config: gpy_agent::config::Config = toml::from_str(&written).expect("parse config");
    assert_eq!(config.ui.theme.as_str(), "starship");
    assert_eq!(
        config.ui.enabled_segments,
        vec![
            "username",
            "hostname",
            "directory",
            "git",
            "language",
            "duration"
        ],
        "--force should write the preset's recommended segment order"
    );
    assert_eq!(
        config.ui.directory.display,
        gpy_agent::config::types::DirectoryDisplay::Truncated,
        "--force should write the recommended directory display"
    );
    assert_eq!(
        config.ui.directory.truncation_length.get(),
        3,
        "--force should write the recommended truncation length"
    );
    assert!(
        config.ui.directory.truncate_to_repo,
        "--force should write the recommended repo anchoring"
    );
    assert!(
        config.ui.show_icons,
        "--force should keep recommended language icons on"
    );
}

#[test]
fn theme_use_force_preserves_explicit_user_preferences() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    // The user has explicitly chosen a segment order and turned icons off. Note
    // `show_icons = false` differs from the outgoing `default` theme's
    // recommendation (true), so it is a genuine user edit, not a theme write.
    write_config(
        &xdg_config,
        "[ui]\ntheme = \"default\"\nenabled_segments = [\"clock\", \"git\"]\nshow_icons = false\n",
    );

    let output = Command::new("./target/debug/gpy")
        .args(["theme", "use", "starship", "--force"])
        .env("XDG_CONFIG_HOME", &xdg_config)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run gpy theme use --force");
    assert!(
        output.status.success(),
        "theme use --force should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("Preserved your explicit settings"),
        "--force should report which explicit settings it kept: {stdout}"
    );

    let written = fs::read_to_string(xdg_config.join("gpy").join("config.toml"))
        .expect("read updated config");
    let config: gpy_agent::config::Config = toml::from_str(&written).expect("parse config");
    assert_eq!(config.ui.theme.as_str(), "starship");
    // Explicitly-set fields are preserved...
    assert_eq!(
        config.ui.enabled_segments,
        vec!["clock", "git"],
        "--force must preserve the user's explicit segment order"
    );
    assert!(
        !config.ui.show_icons,
        "--force must preserve the user's explicit show_icons = false"
    );
    // ...while fields the user never set are filled from the preset.
    assert_eq!(
        config.ui.directory.display,
        gpy_agent::config::types::DirectoryDisplay::Truncated,
        "--force should still apply recommended directory display the user did not set"
    );
    assert_eq!(
        config.ui.directory.truncation_length.get(),
        3,
        "--force should still apply recommended truncation length the user did not set"
    );
}

#[test]
fn theme_use_apply_layout_alias_warns_but_still_applies() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    write_config(&xdg_config, "[ui]\ntheme = \"default\"\n");

    let output = Command::new("./target/debug/gpy")
        .args(["theme", "use", "starship", "--apply-layout"])
        .env("XDG_CONFIG_HOME", &xdg_config)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run gpy theme use --apply-layout");
    assert!(
        output.status.success(),
        "deprecated --apply-layout alias should still succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(
        stderr.contains("--apply-layout is deprecated") && stderr.contains("--force"),
        "the alias should emit a deprecation warning pointing at --force: {stderr}"
    );

    // ...and it behaves exactly like --force.
    let written = fs::read_to_string(xdg_config.join("gpy").join("config.toml"))
        .expect("read updated config");
    let config: gpy_agent::config::Config = toml::from_str(&written).expect("parse config");
    assert_eq!(
        config.ui.enabled_segments,
        vec![
            "username",
            "hostname",
            "directory",
            "git",
            "language",
            "duration"
        ],
        "the deprecated alias should apply the recommended layout like --force"
    );
}

#[test]
fn theme_use_force_applies_recommended_detection_mode() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    // default theme recommends content; user has not set detection_mode.
    write_config(&xdg_config, "[ui]\ntheme = \"default\"\n");

    let output = Command::new("./target/debug/gpy")
        .args(["theme", "use", "starship", "--force"])
        .env("XDG_CONFIG_HOME", &xdg_config)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run gpy theme use --force");
    assert!(
        output.status.success(),
        "theme use --force should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("language detection (markers)"),
        "applying the starship preset should report the markers detection mode: {stdout}"
    );

    let written = fs::read_to_string(xdg_config.join("gpy").join("config.toml"))
        .expect("read updated config");
    let config: gpy_agent::config::Config = toml::from_str(&written).expect("parse config");
    assert_eq!(
        config.language.detection_mode,
        gpy_agent::config::types::DetectionMode::Markers,
        "--force should apply the starship preset's recommended markers detection mode"
    );
}

#[test]
fn theme_use_force_preserves_explicit_detection_mode() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    // Start on the text theme (no recommended block) with an explicit user choice
    // of markers, then force the default theme (which recommends content). Because
    // the user's value differs from the outgoing theme's recommendation, it is a
    // genuine edit and must be preserved.
    write_config(
        &xdg_config,
        "[ui]\ntheme = \"text\"\n\n[language]\ndetection_mode = \"markers\"\n",
    );

    let output = Command::new("./target/debug/gpy")
        .args(["theme", "use", "default", "--force"])
        .env("XDG_CONFIG_HOME", &xdg_config)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run gpy theme use --force");
    assert!(
        output.status.success(),
        "theme use --force should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("Preserved your explicit settings")
            && stdout.contains("language detection (markers)"),
        "--force should preserve the user's explicit detection mode: {stdout}"
    );

    let written = fs::read_to_string(xdg_config.join("gpy").join("config.toml"))
        .expect("read updated config");
    let config: gpy_agent::config::Config = toml::from_str(&written).expect("parse config");
    assert_eq!(
        config.language.detection_mode,
        gpy_agent::config::types::DetectionMode::Markers,
        "--force must preserve the user's explicit markers detection mode"
    );
}

#[test]
fn theme_use_without_force_preserves_config_and_hints() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    write_config(
        &xdg_config,
        "[ui]\ntheme = \"default\"\nenabled_segments = [\"clock\", \"git\"]\n",
    );

    let output = Command::new("./target/debug/gpy")
        .args(["theme", "use", "starship"])
        .env("XDG_CONFIG_HOME", &xdg_config)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run gpy theme use");
    assert!(output.status.success(), "theme use should succeed");

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("--force"),
        "plain theme use should hint at --force when the theme recommends settings: {stdout}"
    );

    let written = fs::read_to_string(xdg_config.join("gpy").join("config.toml"))
        .expect("read updated config");
    let config: gpy_agent::config::Config = toml::from_str(&written).expect("parse config");
    assert_eq!(config.ui.theme.as_str(), "starship");
    assert_eq!(
        config.ui.enabled_segments,
        vec!["clock", "git"],
        "plain theme use must not change enabled_segments"
    );
    assert_eq!(
        config.ui.directory.display,
        gpy_agent::config::types::DirectoryDisplay::Basename,
        "plain theme use must not change directory.display"
    );
}

#[test]
fn theme_use_default_force_restores_builtin_layout() {
    // Round-trip: applying the starship preset then switching back to default
    // with --force restores GPY's stock layout/display. This works because the
    // starship-written fields equal the (outgoing) starship recommendation, so
    // they are treated as theme-owned and overwritten by default's recommendation
    // rather than mistaken for explicit user edits.
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    write_config(&xdg_config, "[ui]\ntheme = \"default\"\n");

    let run = |args: &[&str]| {
        let output = Command::new("./target/debug/gpy")
            .args(args)
            .env("XDG_CONFIG_HOME", &xdg_config)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run gpy theme use");
        assert!(
            output.status.success(),
            "{args:?} should succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run(&["theme", "use", "starship", "--force"]);
    run(&["theme", "use", "default", "--force"]);

    let written = fs::read_to_string(xdg_config.join("gpy").join("config.toml"))
        .expect("read updated config");
    let config: gpy_agent::config::Config = toml::from_str(&written).expect("parse config");
    assert_eq!(config.ui.theme.as_str(), "default");
    assert_eq!(
        config.ui.enabled_segments,
        vec!["clock", "duration", "language", "directory", "git"],
        "default --force should restore GPY's stock segment order"
    );
    assert_eq!(
        config.ui.directory.display,
        gpy_agent::config::types::DirectoryDisplay::Basename,
        "default --force should restore basename directory display"
    );
}

#[test]
fn two_line_export_follows_each_theme() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");

    let export = |theme: &str| -> String {
        write_config(&xdg_config, &format!("[ui]\ntheme = \"{theme}\"\n"));
        let output = Command::new("./target/debug/gpy-agent")
            .args(["theme", "export", "--format", "fish"])
            .env("XDG_CONFIG_HOME", &xdg_config)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run gpy-agent theme export");
        assert!(
            output.status.success(),
            "theme export should succeed for {theme}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8 stdout")
    };

    // The Starship preset enables two-line on bash/zsh.
    assert!(
        export("starship").contains("__gpy_two_line \"1\""),
        "starship preset should export two-line on"
    );
    // The default theme now does too. Fish has always rendered the prompt
    // character on its own line and ignores this flag, so leaving bash/zsh
    // single-line here made the same theme look different per shell.
    assert!(
        export("default").contains("__gpy_two_line \"1\""),
        "default theme should export two-line on"
    );
    // `text` is the deliberately minimal preset and stays single-line, which
    // keeps this test meaningful: it still proves the flag is read from the
    // theme rather than hardcoded on.
    assert!(
        export("text").contains("__gpy_two_line \"0\""),
        "text theme should export two-line off"
    );
}

#[test]
fn load_theme_missing_segments_errors() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    let broken = write_theme(
        &xdg_config,
        "missing-segments",
        "[ui]\nprompt_icon = \">\"\n",
    );

    let err = loader::load_theme_from_path(broken.to_str().expect("path"))
        .expect_err("theme load should fail");
    let message = err.to_string();
    assert!(
        message
            .to_lowercase()
            .contains("missing required [segments]"),
        "Expected missing segments error, got: {message}"
    );
}

#[test]
fn load_theme_invalid_color_errors() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    let broken = write_theme(
        &xdg_config,
        "invalid-color",
        "[ui]\nprompt_icon = \">\"\n\n[segments.clock]\nbg_color = \"not-a-color\"\ntext_color = \"white\"\n",
    );

    let err = loader::load_theme_from_path(broken.to_str().expect("path"))
        .expect_err("theme load should fail");
    let message = err.to_string();
    assert!(
        message.contains("Invalid color"),
        "Expected invalid color error, got: {message}"
    );
}

#[test]
fn load_theme_invalid_icon_errors() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    let broken = write_theme(
        &xdg_config,
        "invalid-icon",
        "[ui]\nprompt_icon = \"\\u0007\"\n\n[segments.clock]\nbg_color = \"black\"\ntext_color = \"white\"\n",
    );

    let err = loader::load_theme_from_path(broken.to_str().expect("path"))
        .expect_err("theme load should fail");
    let message = err.to_string();
    assert!(
        message.contains("contains control characters"),
        "Expected icon validation error, got: {message}"
    );
}

#[test]
fn load_theme_invalid_plugin_color_errors() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    let broken = write_theme(
        &xdg_config,
        "invalid-plugin-color",
        r#"[ui]
prompt_icon = ">"

[segments.clock]
bg_color = "black"
text_color = "white"

[segments.k8s]
context_color = "definitely-not-a-color"
"#,
    );

    let err = loader::load_theme_from_path(broken.to_str().expect("path"))
        .expect_err("theme load should fail");
    let message = err.to_string();
    assert!(
        message.contains("segments.k8s.context_color"),
        "Expected plugin color validation error, got: {message}"
    );
}

#[test]
fn load_theme_invalid_plugin_icon_errors() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let xdg_config = temp_dir.path().join("config");
    let broken = write_theme(
        &xdg_config,
        "invalid-plugin-icon",
        r#"[ui]
prompt_icon = ">"

[segments.clock]
bg_color = "black"
text_color = "white"

[segments.k8s]
secondary_icon = "\u0007"
"#,
    );

    let err = loader::load_theme_from_path(broken.to_str().expect("path"))
        .expect_err("theme load should fail");
    let message = err.to_string();
    assert!(
        message.contains("segments.k8s.secondary_icon"),
        "Expected plugin icon validation error, got: {message}"
    );
    assert!(
        message.contains("contains control characters"),
        "Expected control character error, got: {message}"
    );
}
