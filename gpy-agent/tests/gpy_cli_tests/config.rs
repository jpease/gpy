use super::run_gpy_command;
use crate::cli_harness::CliTestEnv;
use std::fs;

#[test]
fn test_config_show_all_sections() {
    let output = run_gpy_command(&["config", "show"]).expect("Failed to run gpy config show");

    assert!(
        output.contains("Agent Configuration"),
        "Should show agent section"
    );
    assert!(
        output.contains("Git Configuration"),
        "Should show git section"
    );
    assert!(
        output.contains("Language Configuration"),
        "Should show language section"
    );
    assert!(
        output.contains("UI Configuration"),
        "Should show UI section"
    );
}

#[test]
fn test_config_show_specific_section() {
    let output =
        run_gpy_command(&["config", "show", "git"]).expect("Failed to run gpy config show git");

    assert!(
        output.contains("Git Configuration"),
        "Should show git section header"
    );
    assert!(output.contains("enabled"), "Should show enabled field");
    assert!(
        output.contains("show_upstream"),
        "Should show show_upstream field"
    );
    assert!(
        !output.contains("Agent Configuration"),
        "Should not show other sections"
    );
}

/// `show(None)` must equal its five `show(Some(section))` outputs joined by
/// blank lines.
#[test]
fn test_config_show_all_equals_concatenated_sections() {
    let all = run_gpy_command(&["config", "show"]).expect("Failed to run gpy config show");
    let agent =
        run_gpy_command(&["config", "show", "agent"]).expect("Failed to run gpy config show agent");
    let git =
        run_gpy_command(&["config", "show", "git"]).expect("Failed to run gpy config show git");
    let language = run_gpy_command(&["config", "show", "language"])
        .expect("Failed to run gpy config show language");
    let ui = run_gpy_command(&["config", "show", "ui"]).expect("Failed to run gpy config show ui");
    let plugin = run_gpy_command(&["config", "show", "plugin"])
        .expect("Failed to run gpy config show plugin");

    let expected = format!("{agent}\n{git}\n{language}\n{ui}\n{plugin}");
    assert_eq!(
        all, expected,
        "show(None) output must be byte-identical to its per-section outputs joined by a blank line"
    );
}

#[test]
fn test_config_get_value() {
    let output = run_gpy_command(&["config", "get", "git.show_upstream"])
        .expect("Failed to run gpy config get");

    let trimmed = output.trim();
    assert!(
        trimmed == "true" || trimmed == "false",
        "Should output a boolean value"
    );
}

#[test]
fn test_config_set_and_get() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let config_content = r#"
[agent]
enabled = true

[git]
max_branch_length = 0

[language]

[ui]
theme = "default"
directory.display = "basename"
directory.max_length = 80
enabled_segments = ["language", "directory", "git"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let set = env
        .run_gpy(&["config", "set", "ui.directory.max_length", "120"])
        .expect("Failed to run gpy config set");
    set.assert_success("gpy config set ui.directory.max_length 120");
    let set_stdout = set.stdout;
    assert!(
        set_stdout.contains("Set ui.directory.max_length = 120"),
        "Should confirm value set"
    );

    let get = env
        .run_gpy(&["config", "get", "ui.directory.max_length"])
        .expect("Failed to run gpy config get");
    get.assert_success("gpy config get ui.directory.max_length");
    let get_stdout = get.stdout;
    assert_eq!(get_stdout.trim(), "120", "Should return the updated value");
}

#[test]
fn test_config_set_and_get_directory_display() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let config_content = r#"
[ui]
theme = "default"
directory.display = "basename"
directory.max_length = 80
enabled_segments = ["directory", "git"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let set = env
        .run_gpy(&["config", "set", "ui.directory.display", "abbreviated"])
        .expect("Failed to run gpy config set");
    set.assert_success("gpy config set ui.directory.display abbreviated");
    assert!(
        set.stdout
            .contains("Set ui.directory.display = abbreviated"),
        "Should confirm directory display mode set"
    );

    let get = env
        .run_gpy(&["config", "get", "ui.directory.display"])
        .expect("Failed to run gpy config get");
    get.assert_success("gpy config get ui.directory.display");
    assert_eq!(get.stdout.trim(), "abbreviated");
}

#[test]
fn test_config_open_uses_editor_and_creates_missing_file() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    fs::remove_file(&config_path).expect("Failed to remove initial config file");

    let output = env
        .run_gpy_with_env(&["config", "open"], &[("EDITOR", "true")])
        .expect("Failed to run gpy config open");
    output.assert_success("gpy config open");

    assert!(
        config_path.exists(),
        "Config open should create the config file"
    );
    // Compared as a `Path`, not a string: on Windows the printed path joins
    // the XDG root with `/` while `config_path()` joins with `\`, and `Path`
    // equality is per component.
    let created = output
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("Created config: "));
    assert_eq!(
        created.map(std::path::Path::new),
        Some(config_path.as_path()),
        "Config open should report the created path"
    );

    let again = env
        .run_gpy_with_env(&["config", "open"], &[("EDITOR", "true")])
        .expect("Failed to run gpy config open");
    again.assert_success("gpy config open (existing file)");
    assert!(
        again.stdout.is_empty(),
        "Nothing was created, so nothing is printed. Got: {}",
        again.stdout
    );
}

#[test]
fn test_config_set_with_validation_errors() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let initial_config = r#"
[agent]
enabled = true
timeout_seconds = 5

[ui]
theme = "default"
directory.max_length = 80
"#;
    fs::write(&config_path, initial_config).expect("Failed to write config");

    let valid_output = env
        .run_gpy(&["config", "set", "ui.directory.max_length", "120"])
        .expect("Failed to run gpy config set");
    valid_output.assert_success("gpy config set ui.directory.max_length 120");

    let get_output = env
        .run_gpy(&["config", "get", "ui.directory.max_length"])
        .expect("Failed to run gpy config get");
    get_output.assert_success("gpy config get ui.directory.max_length");
    let get_stdout = get_output.stdout;
    assert_eq!(get_stdout.trim(), "120", "Should return the updated value");

    let invalid_output = env
        .run_gpy(&["config", "set", "ui.directory.max_length", "-10"])
        .expect("Failed to run gpy config set with invalid value");

    assert!(
        invalid_output.exit_code != 0_i32
            || invalid_output.stderr.contains("error")
            || invalid_output.stderr.contains("invalid"),
        "Should reject invalid value"
    );

    let nonexistent_output = env
        .run_gpy(&["config", "set", "nonexistent.key", "value"])
        .expect("Failed to run gpy config set with non-existent key");

    assert!(
        nonexistent_output.exit_code != 0_i32
            || nonexistent_output.stderr.contains("error")
            || nonexistent_output.stderr.contains("not found"),
        "Should reject non-existent key"
    );
}
