use super::{fish_enabled_segments, run_gpy_command};
use crate::cli_harness::CliTestEnv;
use std::fs;

#[test]
fn test_segments_list() {
    let output = run_gpy_command(&["segments"]).expect("Failed to run gpy segments");
    assert!(
        output.contains("Prompt Segments:"),
        "Output should have header"
    );
    assert!(output.contains("git"), "Output should list git segment");
    assert!(
        output.contains("language"),
        "Output should list language segment"
    );
    assert!(output.contains("clock"), "Output should list clock segment");
    assert!(
        output.contains("duration"),
        "Output should list duration segment"
    );
    assert!(
        output.contains("directory"),
        "Output should list directory segment"
    );
    let line_of = |name: &str| {
        output
            .lines()
            .find(|line| {
                line.get(4..)
                    .and_then(|rest| rest.split_whitespace().next())
                    == Some(name)
            })
            .unwrap_or_else(|| panic!("no `{name}` line in:\n{output}"))
    };
    let clock = line_of("clock");
    assert!(
        clock.contains("gpy-core") && clock.contains("default"),
        "a default builtin names its owner and says default: {clock}"
    );
    let status = line_of("status");
    assert!(
        status.contains("gpy-core") && !status.contains("default"),
        "a non-default builtin names its owner only: {status}"
    );
}

#[test]
fn test_enable_preserves_existing_segment_order() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_content = r#"
[ui]
enabled_segments = ["directory", "clock"]
"#;
    fs::write(env.config_path(), config_content).expect("Failed to write config");

    let enable = env
        .run_gpy(&["enable", "status"])
        .expect("Failed to run gpy enable status");
    enable.assert_success("gpy enable status");

    let segments = env
        .run_gpy(&["config", "get", "ui.enabled_segments"])
        .expect("Failed to run gpy config get");
    segments.assert_success("gpy config get ui.enabled_segments");
    assert_eq!(segments.stdout.trim(), "directory clock status");
}

#[test]
fn test_enable_disable_segment() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let config_content = r#"
[agent]
enabled = true

[git]
enabled = true

[language]
enabled = true

[ui]
theme = "default"
enabled_segments = ["duration", "language", "directory", "git"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let enable = env
        .run_gpy(&["enable", "clock"])
        .expect("Failed to run gpy enable clock");
    enable.assert_success("gpy enable clock");
    let enable_stdout = enable.stdout;
    assert!(
        enable_stdout.contains("Enabled clock"),
        "Should confirm clock segment enabled"
    );

    let updated_config = fs::read_to_string(&config_path).expect("Failed to read updated config");
    assert!(
        updated_config.contains("clock"),
        "Config should now contain clock segment"
    );

    let disable = env
        .run_gpy(&["disable", "clock"])
        .expect("Failed to run gpy disable clock");
    disable.assert_success("gpy disable clock");
    let disable_stdout = disable.stdout;
    assert!(
        disable_stdout.contains("Disabled clock"),
        "Should confirm clock segment disabled"
    );
}

#[test]
fn test_enable_hostname_segment() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    fs::write(
        env.config_path(),
        "[ui]\nenabled_segments = [\"directory\"]\n",
    )
    .expect("Failed to write config");

    let bundled = [("GPY_BUNDLED_PLUGIN_DIR", env.bundled_plugins_dir())];
    let enable = env
        .run_gpy_with_env(&["enable", "hostname"], &bundled)
        .expect("Failed to run gpy enable hostname");
    enable.assert_success("gpy enable hostname");

    let segments = env
        .run_gpy(&["config", "get", "ui.enabled_segments"])
        .expect("Failed to run gpy config get ui.enabled_segments");
    segments.assert_success("gpy config get ui.enabled_segments");
    assert!(
        segments.stdout.contains("hostname"),
        "ui.enabled_segments should list hostname: {:?}",
        segments.stdout
    );
}

#[test]
fn test_enable_git_adds_to_enabled_segments_when_absent() {
    // `git.enabled = true` alone does not render git; `gpy enable git`
    // must also list it so the export (the shells' render list) includes it.
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    fs::write(
        env.config_path(),
        "[git]\nenabled = true\n\n[ui]\nenabled_segments = [\"directory\"]\n",
    )
    .expect("Failed to write config");

    let segments = env
        .run_gpy(&["segments"])
        .expect("Failed to run gpy segments");
    segments.assert_success("gpy segments");
    assert!(
        segments.stdout.contains("[ ] git"),
        "unlisted git must not show as enabled: {segments:?}"
    );

    env.run_gpy(&["enable", "git"])
        .expect("Failed to run gpy enable git")
        .assert_success("gpy enable git");

    let line = fish_enabled_segments(&env);
    assert!(
        line.split_whitespace().any(|word| word == "git"),
        "export must render git after `gpy enable git`: {line}"
    );
}

#[test]
fn test_disable_then_enable_git_restores_fish_export() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let before = fish_enabled_segments(&env);

    env.run_gpy(&["disable", "git"])
        .expect("Failed to run gpy disable git")
        .assert_success("gpy disable git");
    assert!(
        !fish_enabled_segments(&env)
            .split_whitespace()
            .any(|word| word == "git"),
        "disabled git must leave the export"
    );

    env.run_gpy(&["enable", "git"])
        .expect("Failed to run gpy enable git")
        .assert_success("gpy enable git");
    assert_eq!(fish_enabled_segments(&env), before);
}

#[test]
fn test_lang_versions_toggle() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let config_content = r#"
[agent]
enabled = true

[git]
enabled = true

[language]
enabled = true
show_versions = false

[ui]
theme = "default"
enabled_segments = ["language", "directory", "git"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let output = env
        .run_gpy(&["lang", "versions", "on"])
        .expect("Failed to run gpy lang versions on");
    output.assert_success("gpy lang versions on");
    assert_eq!(
        output
            .stderr
            .lines()
            .filter(|line| line.starts_with("warn: 'gpy lang versions' is deprecated; use "))
            .count(),
        1_usize,
        "exactly one deprecation line on stderr: {}",
        output.stderr
    );
    let shown = env
        .run_gpy(&["config", "get", "language.show_versions"])
        .expect("Failed to run gpy config get");
    shown.assert_success("gpy config get language.show_versions");
    assert_eq!(shown.stdout.trim(), "true");

    let updated_config = fs::read_to_string(&config_path).expect("Failed to read updated config");
    assert!(
        updated_config.contains("show_versions = true"),
        "Config should have show_versions = true"
    );
}

#[test]
fn test_segment_enable_disable_round_trip() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let initial_config = r#"
[agent]
enabled = true

[git]
enabled = true

[language]
enabled = true

[ui]
theme = "default"
enabled_segments = ["duration", "language", "directory", "git"]
"#;
    fs::write(&config_path, initial_config).expect("Failed to write config");

    let enable_output = env
        .run_gpy(&["enable", "clock"])
        .expect("Failed to run gpy enable clock");
    enable_output.assert_success("gpy enable clock");

    let config_after_enable =
        fs::read_to_string(&config_path).expect("Failed to read config after enable");
    assert!(
        config_after_enable.contains("clock"),
        "Config should contain clock segment"
    );

    let disable_output = env
        .run_gpy(&["disable", "clock"])
        .expect("Failed to run gpy disable clock");
    disable_output.assert_success("gpy disable clock");

    let config_after_disable =
        fs::read_to_string(&config_path).expect("Failed to read config after disable");

    assert!(
        !config_after_disable.contains("enabled_segments = [\"clock\"")
            && !config_after_disable.contains("\"clock\",")
            && !config_after_disable.contains(", \"clock\""),
        "Clock should be removed from enabled segments"
    );
}
