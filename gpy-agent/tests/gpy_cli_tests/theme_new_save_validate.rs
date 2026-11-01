use super::NONEXISTENT_THEME_NAME;
use crate::cli_harness::CliTestEnv;
use std::fs;

#[test]
fn test_theme_show_with_config() {
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
enabled_segments = ["clock", "duration", "language", "directory", "git"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let output = env
        .run_gpy(&["theme", "show"])
        .expect("Failed to run gpy theme show");
    output.assert_success("gpy theme show");
    let stdout = output.stdout;
    assert!(
        stdout.contains("default"),
        "Theme show should display current theme"
    );
}

#[test]
fn test_theme_new_creates_file() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    let output = env
        .run_gpy(&["theme", "new", "test-theme"])
        .expect("Failed to run gpy theme new");
    output.assert_success("gpy theme new test-theme");
    let stdout = output.stdout;

    assert!(
        stdout.contains("Created new theme"),
        "Should confirm theme created"
    );
    assert!(stdout.contains("test-theme"), "Should mention theme name");
    assert!(
        stdout.contains("Extends: default (user)"),
        "Should name the base it used. Got: {stdout}"
    );

    let theme_file = themes_dir.join("test-theme.toml");
    assert!(theme_file.exists(), "Theme file should be created");

    let content = fs::read_to_string(&theme_file).expect("Failed to read created theme file");
    assert_eq!(
        content, "extends = \"default\"\n",
        "Without --from the new theme is an overlay of `default`"
    );
    env.run_gpy(&["theme", "validate", "test-theme"])
        .expect("Failed to run gpy theme validate")
        .assert_success("gpy theme validate test-theme");
}

#[test]
fn test_theme_new_twice_keeps_the_existing_file() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    env.run_gpy(&["theme", "new", "twice"])
        .expect("Failed to run gpy theme new")
        .assert_success("gpy theme new twice");
    let theme_file = env.themes_dir().join("twice.toml");
    fs::write(&theme_file, "# hand edited\n").expect("Failed to edit theme file");

    let second = env
        .run_gpy(&["theme", "new", "twice"])
        .expect("Failed to run gpy theme new");

    assert_ne!(second.exit_code, 0_i32, "{second:?}");
    assert!(second.stderr.contains("already exists"), "{second:?}");
    assert_eq!(
        fs::read_to_string(&theme_file).expect("Failed to read theme file"),
        "# hand edited\n",
        "an existing theme must not be overwritten"
    );
}

#[test]
fn test_theme_new_from_writes_an_overlay_of_the_base() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();

    let output = env
        .run_gpy(&["theme", "new", "cloned-theme", "--from", "starship"])
        .expect("Failed to run gpy theme new --from");
    output.assert_success("gpy theme new cloned-theme --from starship");
    assert!(
        output.stdout.contains("Extends: starship (builtin)"),
        "Should name the base. Got: {}",
        output.stdout
    );

    let cloned_content = fs::read_to_string(themes_dir.join("cloned-theme.toml"))
        .expect("Failed to read cloned theme file");
    assert_eq!(cloned_content, "extends = \"starship\"\n");

    let validate_output = env
        .run_gpy(&["theme", "validate", "cloned-theme"])
        .expect("Failed to run gpy theme validate");
    validate_output.assert_success("gpy theme validate cloned-theme");
}

#[test]
fn test_theme_new_from_unknown_base_fails() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy(&[
            "theme",
            "new",
            "cloned-theme",
            "--from",
            "no-such-theme-xyz",
        ])
        .expect("Failed to run gpy theme new --from");
    assert_ne!(
        output.exit_code, 0_i32,
        "Cloning from an unknown theme should fail. Got stdout: {} stderr: {}",
        output.stdout, output.stderr
    );
    assert!(
        output.stderr.contains("no-such-theme-xyz"),
        "Error should name the missing base theme. Got: {}",
        output.stderr
    );
}

#[test]
fn test_theme_save_creates_new_named_theme_without_prompt() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();

    let output = env
        .run_gpy(&["theme", "save", "mynew"])
        .expect("Failed to run gpy theme save");
    output.assert_success("gpy theme save mynew");
    assert!(
        !output.stdout.contains("[y/N]"),
        "Saving a brand-new named theme must not prompt. Got: {}",
        output.stdout
    );
    assert!(
        output.stdout.contains("Saved theme"),
        "Should confirm the theme was saved. Got: {}",
        output.stdout
    );

    let saved_content =
        fs::read_to_string(themes_dir.join("mynew.toml")).expect("Failed to read saved theme");
    assert_eq!(
        saved_content, "extends = \"default\"\n",
        "Saved theme should be an overlay of the active theme"
    );

    let validate_output = env
        .run_gpy(&["theme", "validate", "mynew"])
        .expect("Failed to run gpy theme validate");
    validate_output.assert_success("gpy theme validate mynew");
}

#[test]
fn test_theme_save_named_existing_theme_declined_leaves_file_unchanged() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    let target_path = themes_dir.join("custom.toml");
    let marker_content = "# pre-existing custom marker\n[ui]\nprompt_icon = \"x\"\n[segments]\n";
    fs::write(&target_path, marker_content).expect("Failed to seed pre-existing theme");

    let output = env
        .run_gpy_with_stdin(&["theme", "save", "custom"], "n\n")
        .expect("Failed to run gpy theme save");
    assert_eq!(output.exit_code, 1_i32, "{output:?}");
    assert!(
        output.stdout.contains("[y/N]"),
        "Should prompt before overwriting an existing named theme. Got: {}",
        output.stdout
    );
    assert!(
        output.stderr.contains("cancelled"),
        "Declining should report the cancellation. Got: {}",
        output.stderr
    );

    let unchanged = fs::read_to_string(&target_path).expect("Failed to read theme file");
    assert_eq!(
        unchanged, marker_content,
        "Declining the overwrite prompt must leave the existing theme file untouched"
    );
}

#[test]
fn test_theme_save_without_an_answer_fails_and_writes_nothing() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let target_path = env.themes_dir().join("custom.toml");
    let marker_content = "# pre-existing custom marker\n[segments]\n";
    fs::write(&target_path, marker_content).expect("Failed to seed pre-existing theme");

    let output = env
        .run_gpy_with_stdin(&["theme", "save", "custom"], "")
        .expect("Failed to run gpy theme save");

    assert_eq!(output.exit_code, 1_i32, "{output:?}");
    assert!(output.stderr.contains("pass --yes"), "{output:?}");
    assert_eq!(
        fs::read_to_string(&target_path).expect("read theme"),
        marker_content,
        "nothing may be written unconfirmed"
    );
}

#[test]
fn test_theme_save_yes_overwrites_without_prompting() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let target_path = env.themes_dir().join("custom.toml");
    fs::write(&target_path, "# pre-existing\n").expect("Failed to seed pre-existing theme");

    let output = env
        .run_gpy_with_stdin(&["theme", "save", "custom", "--yes"], "")
        .expect("Failed to run gpy theme save");
    output.assert_success("gpy theme save custom --yes");

    assert!(!output.stdout.contains("[y/N]"), "{output:?}");
    assert_eq!(
        fs::read_to_string(&target_path).expect("Failed to read theme file"),
        "extends = \"default\"\n"
    );
    let no_name = env
        .run_gpy_with_stdin(&["theme", "save", "-y"], "")
        .expect("Failed to run gpy theme save");
    no_name.assert_success("gpy theme save -y");
    assert!(!no_name.stdout.contains("[y/N]"), "{no_name:?}");
    assert!(env.themes_dir().join("default.toml").is_file());
}

#[test]
fn test_theme_save_named_existing_theme_accepted_overwrites_file() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    let target_path = themes_dir.join("custom.toml");
    let marker_content = "# pre-existing custom marker\n[ui]\nprompt_icon = \"x\"\n[segments]\n";
    fs::write(&target_path, marker_content).expect("Failed to seed pre-existing theme");

    let output = env
        .run_gpy_with_stdin(&["theme", "save", "custom"], "y\n")
        .expect("Failed to run gpy theme save");
    output.assert_success("gpy theme save custom (accepted)");
    assert!(
        output.stdout.contains("Saved theme"),
        "Accepting should confirm the save. Got: {}",
        output.stdout
    );

    let overwritten = fs::read_to_string(&target_path).expect("Failed to read theme file");
    assert_eq!(
        overwritten, "extends = \"default\"\n",
        "Accepting the overwrite prompt should replace the file with an overlay of the active theme"
    );
    assert_eq!(
        fs::read_to_string(themes_dir.join("custom.toml.bak")).expect("Failed to read backup"),
        marker_content,
        "The overwritten file is kept as .bak"
    );
}

#[test]
fn test_theme_save_without_name_always_prompts_even_when_target_missing() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_content = r#"
[agent]
enabled = true

[ui]
theme = "starship"
enabled_segments = ["clock"]
"#;
    fs::write(env.config_path(), config_content).expect("Failed to write config");
    let target_path = env.themes_dir().join("starship.toml");
    assert!(
        !target_path.exists(),
        "starship theme file should not exist yet (builtin, unmaterialized)"
    );

    let declined = env
        .run_gpy_with_stdin(&["theme", "save"], "n\n")
        .expect("Failed to run gpy theme save");
    assert_eq!(declined.exit_code, 1_i32, "{declined:?}");
    assert!(declined.stderr.contains("cancelled"), "{declined:?}");
    assert!(
        declined.stdout.contains("[y/N]"),
        "Omitting the name must still prompt, even though no file exists yet. Got: {}",
        declined.stdout
    );
    assert!(
        !target_path.exists(),
        "Declining must not materialize the theme file"
    );

    let accepted = env
        .run_gpy_with_stdin(&["theme", "save"], "y\n")
        .expect("Failed to run gpy theme save");
    accepted.assert_success("gpy theme save (no name, accepted)");
    assert_eq!(
        fs::read_to_string(&target_path).expect("Failed to read saved theme"),
        "extends = \"starship\"\n",
        "Accepting should write a self-named overlay of the active builtin"
    );
    assert!(
        !accepted.stdout.contains("Activate with"),
        "Saving over the currently active theme should not suggest activating it. Got: {}",
        accepted.stdout
    );
}

#[test]
fn test_theme_validate_active_theme() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let config_content = r#"
[agent]
enabled = true

[ui]
theme = "default"
enabled_segments = ["clock"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let output = env
        .run_gpy(&["theme", "validate"])
        .expect("Failed to run gpy theme validate");
    output.assert_success("gpy theme validate");
    let stdout = output.stdout;
    assert!(
        stdout.contains("Theme validation passed"),
        "Expected success output. Got: {stdout}"
    );
}

#[test]
fn test_theme_validate_file_reports_actionable_error() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let theme_path = env.root().join("broken.toml");
    let broken_theme = r#"
[ui]
prompt_icon = "❯"
prompt_color = "not-a-color"

[segments]
"#;
    fs::write(&theme_path, broken_theme).expect("Failed to write broken theme");

    let theme_path_string = theme_path.display().to_string();
    let output = env
        .run_gpy(&["theme", "validate", &theme_path_string])
        .expect("Failed to run gpy theme validate <path>");
    let combined = format!("{}{}", output.stdout, output.stderr);
    assert!(
        output.exit_code != 0_i32,
        "Broken theme should fail validation. Output: {combined}"
    );
    assert!(
        combined.contains("Diagnostic:") && combined.contains("Invalid color"),
        "Output should include diagnostic and invalid field. Output: {combined}"
    );
    assert!(
        combined.contains("Remediation:") && combined.contains("Ensure colors are valid"),
        "Output should include actionable remediation guidance. Output: {combined}"
    );
}

#[test]
fn test_theme_validate_rejects_nonexistent_name() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy(&["theme", "validate", NONEXISTENT_THEME_NAME])
        .expect("Failed to run gpy theme validate <nonexistent>");
    let combined = format!("{}{}", output.stdout, output.stderr);

    assert!(
        output.exit_code != 0_i32,
        "Validating a nonexistent theme must exit non-zero. Output: {combined}"
    );
    assert!(
        combined.contains(NONEXISTENT_THEME_NAME) && combined.contains("not found"),
        "Output should name the missing theme as the reason. Output: {combined}"
    );
    assert!(
        !combined.contains("Theme validation passed"),
        "A nonexistent theme must not report validation success. Output: {combined}"
    );
}

#[test]
fn test_theme_validate_and_config_set_reject_broken_clock_hostname_username() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    fs::create_dir_all(&themes_dir).expect("Failed to create themes dir");

    let original_config_bytes =
        fs::read(env.config_path()).expect("Failed to read original config");
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

    for (segment, var) in [
        ("clock", "time"),
        ("hostname", "hostname"),
        ("username", "username"),
    ] {
        let theme_name = format!("broken-{segment}");
        let theme_content =
            format!("[ui]\n[segments.{segment}]\nformat = \"[${var}](fg:does_not_exist)\"\n");
        fs::write(themes_dir.join(format!("{theme_name}.toml")), theme_content)
            .expect("Failed to write theme");

        let val_out = env
            .run_gpy_with_env(&["theme", "validate", &theme_name], &env_overrides)
            .expect("Failed to run gpy theme validate");
        assert_ne!(
            val_out.exit_code, 0_i32,
            "theme validate must reject broken {segment}"
        );
        let combined_val = format!("{}{}", val_out.stdout, val_out.stderr);
        assert!(
            combined_val.contains(segment),
            "validation error should mention segment '{segment}': {combined_val}"
        );

        let set_out = env
            .run_gpy_with_env(&["config", "set", "ui.theme", &theme_name], &env_overrides)
            .expect("Failed to run gpy config set");
        assert_ne!(
            set_out.exit_code, 0_i32,
            "config set must reject broken {segment}"
        );
        let current_config_bytes = fs::read(env.config_path()).expect("Failed to re-read config");
        assert_eq!(
            current_config_bytes, original_config_bytes,
            "config bytes must remain unchanged on rejected config set for {segment}"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_theme_new_rejects_empty_and_whitespace_names() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    if themes_dir.exists() {
        fs::remove_dir_all(&themes_dir).expect("Failed to remove themes dir");
    }

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

    // 1. Empty destination name, plain and --from default
    for flags in [
        &["theme", "new", ""][..],
        &["theme", "new", "", "--from", "default"][..],
    ] {
        let res = env
            .run_gpy_with_env(flags, &env_overrides)
            .expect("Failed to run gpy theme new");
        assert_ne!(res.exit_code, 0_i32, "theme new with empty name must fail");
        assert!(
            !themes_dir.join(".toml").exists(),
            ".toml must not be created for empty name"
        );
        assert!(
            !themes_dir.exists(),
            "themes dir must not be materialized for invalid name"
        );
    }

    // 2. Whitespace-only destination name, plain and cloned
    for flags in [
        &["theme", "new", "   "][..],
        &["theme", "new", "   ", "--from", "default"][..],
    ] {
        let res = env
            .run_gpy_with_env(flags, &env_overrides)
            .expect("Failed to run gpy theme new");
        assert_ne!(
            res.exit_code, 0_i32,
            "theme new with whitespace name must fail"
        );
        assert!(
            !themes_dir.exists(),
            "themes dir must not be materialized for whitespace name"
        );
    }

    // 3. Valid plain name creates valid theme that validate and use accept
    let res_valid = env
        .run_gpy_with_env(&["theme", "new", "my-valid-theme"], &env_overrides)
        .expect("Failed to create valid theme");
    res_valid.assert_success("create valid theme");
    assert!(themes_dir.join("my-valid-theme.toml").exists());

    let val_res = env
        .run_gpy_with_env(&["theme", "validate", "my-valid-theme"], &env_overrides)
        .expect("Failed to validate new theme");
    val_res.assert_success("validate created theme");

    let use_res = env
        .run_gpy_with_env(&["theme", "use", "my-valid-theme"], &env_overrides)
        .expect("Failed to use new theme");
    use_res.assert_success("use created theme");
    let active_cfg = fs::read_to_string(env.config_path()).expect("read active config");
    assert!(
        active_cfg.contains("theme = \"my-valid-theme\""),
        "active config must have new theme: {active_cfg}"
    );

    // 4. Existing destination rejects and preserves content
    let original_bytes = fs::read(themes_dir.join("my-valid-theme.toml")).expect("read original");
    let res_collision = env
        .run_gpy_with_env(&["theme", "new", "my-valid-theme"], &env_overrides)
        .expect("Failed to run collision test");
    assert_ne!(res_collision.exit_code, 0_i32, "collision must fail");
    assert_eq!(
        fs::read(themes_dir.join("my-valid-theme.toml")).expect("read after collision"),
        original_bytes,
        "existing theme file must not be modified on collision"
    );
}
