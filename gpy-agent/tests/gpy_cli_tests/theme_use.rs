use crate::cli_harness::CliTestEnv;
use std::fs;

#[test]
#[allow(clippy::too_many_lines)]
fn test_theme_switch_end_to_end() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let themes_dir = env.themes_dir();

    let initial_config = r#"
[agent]
enabled = true

[ui]
theme = "default"
enabled_segments = ["clock", "duration"]
"#;
    fs::write(&config_path, initial_config).expect("Failed to write initial config");

    let custom_theme = r##"
[ui]

[segments.character]
success_symbol = "❯"
success_color = "white"

[segments.clock]
text_color = "white"
bg_color = "#ff0000"
time_format = "12"

[segments.duration]
icon = "⏱"
text_color = "white"
bg_color = "black"

[segments.directory]
text_color = "black"
bg_color = "blue"

[segments.status]
ok_icon = "✓"
ok_text_color = "black"
ok_bg_color = "green"
fail_icon = "✗"
fail_text_color = "black"
fail_bg_color = "red"

[segments.git]
text_color = "black"
bg_color = "white"

[segments.language]
text_color = "black"
bg_color = "white"
"##;
    fs::write(themes_dir.join("custom.toml"), custom_theme).expect("Failed to write custom theme");

    let output = env
        .run_gpy(&["theme", "use", "custom"])
        .expect("Failed to run gpy theme use");
    output.assert_success("gpy theme use custom");
    let stdout = output.stdout;
    assert!(
        stdout.contains("Switched to 'custom'") || stdout.contains("custom"),
        "Should confirm theme switch"
    );

    let updated_config = fs::read_to_string(&config_path).expect("Failed to read updated config");
    assert!(
        updated_config.contains("theme = \"custom\""),
        "Config should reference custom theme"
    );

    let show_output = env
        .run_gpy(&["theme", "show"])
        .expect("Failed to run gpy theme show");
    show_output.assert_success("gpy theme show");
    let show_stdout = show_output.stdout;
    assert!(
        show_stdout.contains("custom"),
        "Theme show should display custom theme"
    );
}

#[test]
fn test_theme_use_rejects_broken_template_preserving_config() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    fs::create_dir_all(&themes_dir).expect("Failed to create themes dir");

    let broken_theme = r#"
[ui]
[segments.git]
format = "[$branch](fg:does_not_exist)"
"#;
    fs::write(themes_dir.join("broken.toml"), broken_theme).expect("Failed to write broken theme");

    let original_config = b"[ui]\ntheme = \"default\"\n";
    fs::write(env.config_path(), original_config).expect("Failed to write config");

    let env_overrides = [
        ("GPY_CONFIG_PATH", env.config_path().display().to_string()),
        (
            "GPY_AGENT_SOCKET_PATH",
            env.xdg_runtime_dir().join("gpy.sock").display().to_string(),
        ),
        (
            "GPY_BUNDLED_PLUGIN_DIR",
            env.root().join("empty_plugins").display().to_string(),
        ),
    ];

    let res = env
        .run_gpy_with_env(&["theme", "use", "broken"], &env_overrides)
        .expect("Failed to run theme use");
    assert_ne!(res.exit_code, 0_i32, "theme use broken must fail");
    assert_eq!(
        fs::read(env.config_path()).expect("read config"),
        original_config,
        "config bytes must be unchanged after rejected theme use"
    );

    let res_force = env
        .run_gpy_with_env(&["theme", "use", "broken", "--force"], &env_overrides)
        .expect("Failed to run theme use --force");
    assert_ne!(
        res_force.exit_code, 0_i32,
        "theme use broken --force must fail"
    );
    assert_eq!(
        fs::read(env.config_path()).expect("read config"),
        original_config,
        "config bytes must be unchanged after rejected theme use --force"
    );
}

#[test]
fn test_theme_use_missing_config_environment() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    fs::create_dir_all(&themes_dir).expect("Failed to create themes dir");

    let broken_theme = r#"
[ui]
[segments.git]
format = "[$branch](fg:does_not_exist)"
"#;
    fs::write(themes_dir.join("broken.toml"), broken_theme).expect("Failed to write broken theme");

    let config_path = env.config_path();
    if config_path.exists() {
        fs::remove_file(&config_path).expect("Failed to remove config file");
    }

    let env_overrides = [
        ("GPY_CONFIG_PATH", config_path.display().to_string()),
        (
            "GPY_AGENT_SOCKET_PATH",
            env.xdg_runtime_dir().join("gpy.sock").display().to_string(),
        ),
        (
            "GPY_BUNDLED_PLUGIN_DIR",
            env.root().join("empty_plugins").display().to_string(),
        ),
    ];

    let res = env
        .run_gpy_with_env(&["theme", "use", "broken"], &env_overrides)
        .expect("Failed to run theme use");
    assert_ne!(
        res.exit_code, 0_i32,
        "theme use broken must fail in fileless environment"
    );
    assert!(
        !config_path.exists(),
        "no config file should be materialized on rejected theme use"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_theme_use_prospective_palette_reconciliation() {
    let initial_config = "[ui]\ntheme = \"default\"\n";
    let env = CliTestEnv::with_config(initial_config)
        .expect("Failed to create isolated CLI test environment");

    let themes_dir = env.themes_dir();
    fs::create_dir_all(&themes_dir).expect("Failed to create themes dir");
    let palettes_dir = env.config_dir().join("palettes");
    fs::create_dir_all(&palettes_dir).expect("Failed to create palettes dir");

    let activation_theme = r#"
[ui]
[ui.recommended]
palette = "activation-palette"
[segments.git]
format = "[$branch](fg:activation_accent)"
"#;
    fs::write(themes_dir.join("activation-case.toml"), activation_theme)
        .expect("Failed to write activation-case theme");

    let activation_palette = r##"
[colors]
activation_accent = "#123456"
"##;
    fs::write(
        palettes_dir.join("activation-palette.toml"),
        activation_palette,
    )
    .expect("Failed to write activation-palette");

    let env_overrides = [
        ("GPY_CONFIG_PATH", env.config_path().display().to_string()),
        (
            "GPY_AGENT_SOCKET_PATH",
            env.xdg_runtime_dir().join("gpy.sock").display().to_string(),
        ),
        (
            "GPY_BUNDLED_PLUGIN_DIR",
            env.root().join("empty_plugins").display().to_string(),
        ),
    ];

    // 1. Ordinary activation: rejects because activation_accent is not in default palette
    let res_ordinary = env
        .run_gpy_with_env(&["theme", "use", "activation-case"], &env_overrides)
        .expect("Failed to run ordinary theme use");
    assert_ne!(
        res_ordinary.exit_code, 0_i32,
        "ordinary theme use must reject unresolved accent"
    );
    assert_eq!(
        fs::read_to_string(env.config_path()).expect("read config"),
        initial_config,
        "config must remain unchanged on rejected ordinary activation"
    );

    // 2. Forced activation: succeeds and persists palette = "activation-palette"
    let res_forced = env
        .run_gpy_with_env(
            &["theme", "use", "activation-case", "--force"],
            &env_overrides,
        )
        .expect("Failed to run forced theme use");
    res_forced.assert_success("forced theme use activation-case");
    let persisted_toml = fs::read_to_string(env.config_path()).expect("read persisted config");
    assert!(
        persisted_toml.contains("theme = \"activation-case\""),
        "config must contain theme = activation-case: {persisted_toml}"
    );
    assert!(
        persisted_toml.contains("palette = \"activation-palette\""),
        "config must contain palette = activation-palette: {persisted_toml}"
    );

    // 3. Explicit palette = "default": --force overwrites it with the recommended palette
    let explicit_config = "[ui]\ntheme = \"default\"\npalette = \"default\"\n";
    fs::write(env.config_path(), explicit_config).expect("Failed to write explicit config");
    let res_explicit = env
        .run_gpy_with_env(
            &["theme", "use", "activation-case", "--force"],
            &env_overrides,
        )
        .expect("Failed to run forced theme use with explicit palette");
    res_explicit.assert_success("forced theme use over an explicit palette");
    assert!(
        res_explicit
            .stdout
            .contains("ui.palette: default → activation-palette"),
        "change line must show the overwritten palette: {}",
        res_explicit.stdout
    );
    let overwritten = fs::read_to_string(env.config_path()).expect("read config");
    assert!(
        overwritten.contains("palette = \"activation-palette\""),
        "forced activation must overwrite the explicit palette: {overwritten}"
    );
}

/// A palette written by the outgoing theme's recommendation is
/// replaced, and a user-chosen palette is preserved and reported.
#[test]
fn test_theme_use_force_replaces_palette_from_outgoing_recommendation() {
    let env = CliTestEnv::with_config("[ui]\ntheme = \"starship\"\npalette = \"starship\"\n")
        .expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    fs::create_dir_all(&themes_dir).expect("Failed to create themes dir");
    fs::write(
        themes_dir.join("starnord.toml"),
        "[ui.recommended]\npalette = \"nord\"\n\n[segments.git]\nformat = \"$branch\"\n",
    )
    .expect("Failed to write starnord theme");

    let env_overrides = [
        ("GPY_CONFIG_PATH", env.config_path().display().to_string()),
        (
            "GPY_AGENT_SOCKET_PATH",
            env.xdg_runtime_dir().join("gpy.sock").display().to_string(),
        ),
        (
            "GPY_BUNDLED_PLUGIN_DIR",
            env.root().join("empty_plugins").display().to_string(),
        ),
    ];

    let res = env
        .run_gpy_with_env(&["theme", "use", "starnord", "--force"], &env_overrides)
        .expect("Failed to run forced theme use");
    res.assert_success("forced theme use starnord");
    let persisted = fs::read_to_string(env.config_path()).expect("read config");
    assert!(
        persisted.contains("palette = \"nord\""),
        "palette must be replaced: {persisted}"
    );
    assert!(
        res.stdout.contains("ui.palette:") && res.stdout.contains("→ nord"),
        "change line must name palette: {}",
        res.stdout
    );

    // A hand-set palette is overwritten too, and the change line shows both values.
    fs::write(env.config_path(), "[ui]\npalette = \"nord\"\n").expect("write config");
    let res_forced = env
        .run_gpy_with_env(&["theme", "use", "starship", "--force"], &env_overrides)
        .expect("Failed to run forced theme use starship");
    res_forced.assert_success("forced theme use starship");
    let forced = fs::read_to_string(env.config_path()).expect("read config");
    assert!(
        forced.contains("palette = \"starship\""),
        "palette must be overwritten: {forced}"
    );
    assert!(
        res_forced.stdout.contains("ui.palette: nord → starship"),
        "change line must show old and new palette: {}",
        res_forced.stdout
    );
}

/// After another CLI write, `--force`
/// still applies every recommendation including the palette.
#[test]
fn test_theme_use_force_applies_palette_after_prior_cli_writes() {
    for prior in [
        &["theme", "use", "text"][..],
        &["config", "set", "git.timeout_seconds", "5"][..],
    ] {
        let env =
            CliTestEnv::with_config("").expect("Failed to create isolated CLI test environment");
        let env_overrides = [
            ("GPY_CONFIG_PATH", env.config_path().display().to_string()),
            (
                "GPY_AGENT_SOCKET_PATH",
                env.xdg_runtime_dir().join("gpy.sock").display().to_string(),
            ),
            (
                "GPY_BUNDLED_PLUGIN_DIR",
                env.root().join("empty_plugins").display().to_string(),
            ),
        ];
        let first = env
            .run_gpy_with_env(prior, &env_overrides)
            .expect("Failed to run prior CLI write");
        first.assert_success("prior CLI write");
        let res = env
            .run_gpy_with_env(&["theme", "use", "starship", "--force"], &env_overrides)
            .expect("Failed to run forced theme use starship");
        res.assert_success("forced theme use starship");
        let persisted = fs::read_to_string(env.config_path()).expect("read config");
        assert!(
            persisted.contains("palette = \"starship\""),
            "palette must be applied after {prior:?}: {persisted}"
        );
        assert!(
            res.stdout.contains("ui.palette:") && res.stdout.contains("→ starship"),
            "change line after {prior:?}: {}",
            res.stdout
        );
        assert!(
            !res.stdout.contains("Preserved"),
            "nothing may be preserved after {prior:?}: {}",
            res.stdout
        );
    }
}
