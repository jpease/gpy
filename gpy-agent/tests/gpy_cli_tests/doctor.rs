use super::NONEXISTENT_THEME_NAME;
use crate::cli_harness::CliTestEnv;
use std::fs;

#[test]
fn test_doctor_accepts_starship_recommended_segments() {
    // The default CliTestEnv config pins `ui.enabled_segments`, and `theme use`
    // never overrides a user-set layout field, so the starship order would not
    // be applied and this test would pass vacuously. Start from a config that
    // leaves the layout unset.
    let env = CliTestEnv::with_config("[ui]\ntheme = \"default\"\n")
        .expect("Failed to create isolated CLI test environment");

    let bundled = [("GPY_BUNDLED_PLUGIN_DIR", env.bundled_plugins_dir())];
    let theme_use = env
        .run_gpy_with_env(&["theme", "use", "starship", "--force"], &bundled)
        .expect("Failed to run gpy theme use starship --force");
    theme_use.assert_success("gpy theme use starship --force");
    assert!(
        theme_use.stdout.contains("ui.enabled_segments: ")
            && theme_use.stdout.contains("→ username hostname"),
        "starship layout should have been applied: {:?}",
        theme_use.stdout
    );

    // No agent runs in CliTestEnv, so doctor's exit code is not asserted.
    let output = env
        .run_gpy_with_env(&["doctor"], &bundled)
        .expect("Failed to run gpy doctor");
    assert!(
        output.stdout.contains("ok: All segments recognized"),
        "doctor should accept the starship layout: {:?}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("Unrecognized segments"),
        "doctor should not flag shipped segments: {:?}",
        output.stdout
    );
}

#[test]
fn doctor_points_uninstalled_bundled_segments_at_setup() {
    let env = CliTestEnv::with_config(
        "[ui]\nenabled_segments = [\"username\", \"usrname\", \"directory\"]\n",
    )
    .expect("Failed to create isolated CLI test environment");
    let empty_bundled = env.config_dir().join("empty-bundled");
    fs::create_dir_all(&empty_bundled).expect("Failed to create empty bundled dir");

    let output = env
        .run_gpy_with_env(
            &["doctor"],
            &[("GPY_BUNDLED_PLUGIN_DIR", empty_bundled.as_path())],
        )
        .expect("Failed to run gpy doctor");
    assert!(
        output.stdout.contains("Unrecognized segments found"),
        "missing bundled plugin must stay a failure: {output:?}"
    );
    assert!(
        output.stdout.contains("Remediation: Run `gpy setup`"),
        "bundled segment must point at `gpy setup`: {output:?}"
    );
    assert!(
        output.stdout.contains("Check for typos"),
        "a genuinely unknown segment keeps the typo remediation: {output:?}"
    );
}

#[test]
fn doctor_reports_unknown_config_key() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    fs::write(env.config_path(), "[git]\nenabeld = true\n").expect("Failed to write config");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    assert!(
        output.stdout.contains("Unknown key in ")
            && output.stdout.contains(": git.enabeld (ignored)"),
        "doctor must name the unknown key and its file: {output:?}"
    );
}

/// 0.6-era `starship` override: `format`/`show_always` on a segment that became a plugin.
const STALE_STARSHIP_THEME: &str =
    "[ui]\nseparator = \" \"\n\n[segments.username]\nformat = \"{value}\"\nshow_always = true\n";

#[test]
fn doctor_warns_on_broken_inactive_builtin_override_without_failing() {
    let baseline_env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let baseline = baseline_env
        .run_gpy(&["doctor"])
        .expect("Failed to run gpy doctor");

    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    fs::write(env.themes_dir().join("starship.toml"), STALE_STARSHIP_THEME)
        .expect("Failed to write stale override");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    let stdout = &output.stdout;
    assert!(
        stdout.contains("override 'starship'") && stdout.contains("starship.toml"),
        "doctor must list the override by name and path: {output:?}"
    );
    assert!(
        stdout.contains("Fails to load (inactive, not blocking)"),
        "a broken inactive override is a warning: {output:?}"
    );
    assert_eq!(
        stdout.matches("gpy theme reset starship").count(),
        1,
        "the recovery hint appears exactly once: {output:?}"
    );
    assert_eq!(
        output.exit_code, baseline.exit_code,
        "an inactive override warning must not change doctor's exit status: {output:?}"
    );
    assert_eq!(
        stdout.matches("Issues were found").count(),
        baseline.stdout.matches("Issues were found").count(),
        "the overall verdict is unchanged by the warning: {output:?}"
    );
}

#[test]
fn doctor_lists_valid_builtin_override_as_ok() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    fs::write(
        env.themes_dir().join("starship.toml"),
        include_str!("../../../config/themes/starship.toml"),
    )
    .expect("Failed to write override");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    let line = output
        .stdout
        .lines()
        .find(|l| l.contains("override 'starship'"))
        .expect("doctor must list the override");
    assert!(
        line.contains("starship.toml") && line.contains("ok: Loaded successfully"),
        "a valid override loads OK: {output:?}"
    );
    assert!(
        !output.stdout.contains("Fails to load"),
        "a valid override raises no warning: {output:?}"
    );
}

#[test]
fn test_doctor_checks_config() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let valid_config = r#"
[agent]
enabled = true

[git]

[language]

[ui]
theme = "default"
enabled_segments = ["clock", "duration", "language", "directory", "git"]
"#;
    fs::write(&config_path, valid_config).expect("Failed to write config");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    // No agent runs in this env: a warning, not a failure.
    assert_eq!(
        output.exit_code, 0_i32,
        "a not-running agent only warns: {output:?}"
    );
    assert!(output.stdout.contains("warn:"), "{output:?}");
    let stdout = output.stdout;
    assert!(
        stdout.contains("Checking process... warn: Not running"),
        "doctor must not report a stopped agent as running: {stdout}"
    );
    assert!(
        stdout.contains("GPY Doctor - System Health Report"),
        "Should show diagnostics header"
    );
    assert!(
        stdout.contains("Checking config file"),
        "Should check config"
    );
    assert!(
        stdout.contains("Checking active theme"),
        "Should check theme"
    );
    assert!(
        stdout.contains("Checking enabled segments"),
        "Should check segments"
    );
    assert!(
        stdout.contains("Checking gpy-agent binary"),
        "Should check binaries"
    );
}

#[test]
fn test_doctor_reports_missing_configured_theme() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let config = format!(
        r#"
[agent]
enabled = true

[ui]
theme = "{NONEXISTENT_THEME_NAME}"
enabled_segments = ["clock"]
"#
    );
    fs::write(&config_path, config).expect("Failed to write config");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    let combined = format!("{}{}", output.stdout, output.stderr);

    assert!(
        combined.contains(NONEXISTENT_THEME_NAME) && combined.contains("not found"),
        "doctor should report the configured theme as missing. Output: {combined}"
    );
    // Scoped to the theme's own line: the palette check legitimately prints
    // the same "Loaded successfully" phrase for the (valid) default palette.
    assert!(
        !combined.contains(&format!("Loaded successfully ('{NONEXISTENT_THEME_NAME}'")),
        "doctor must not report a missing theme as loaded. Output: {combined}"
    );
    assert!(
        !output.stdout.contains("Fix the segment `format` template"),
        "a missing theme is not a template error. Output: {}",
        output.stdout
    );
    assert!(
        output.stdout.contains("Skipped (theme failed to load)"),
        "template line should be skipped when the theme fails to load. Output: {}",
        output.stdout
    );
}

#[test]
fn test_doctor_basic_checks() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let valid_config = r#"
[agent]
enabled = true

[git]

[language]

[ui]
theme = "default"
enabled_segments = ["clock", "duration", "language", "directory", "git"]
"#;
    fs::write(&config_path, valid_config).expect("Failed to write config");

    // No agent runs in this env: a warning, not a failure.
    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    assert_eq!(
        output.exit_code, 0_i32,
        "a not-running agent only warns: {output:?}"
    );
    assert!(output.stdout.contains("warn:"), "{output:?}");
    let stdout = output.stdout;
    assert!(
        stdout.contains("Checking config file... ok: Valid"),
        "a valid config passes its check: {stdout}"
    );

    assert!(
        stdout.contains("diagnostics") || stdout.contains("Checking"),
        "Should show diagnostics output"
    );
}

#[test]
fn test_doctor_with_invalid_theme() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();

    fs::write(
        themes_dir.join("broken.toml"),
        r#"
[ui]

[segments.character]
success_symbol = "❯"
success_color = "invalid-color"
"#,
    )
    .expect("Failed to write broken theme");

    let config_path = env.config_path();
    let invalid_config = r#"
[agent]
enabled = true

[ui]
theme = "broken"
enabled_segments = ["clock"]
"#;
    fs::write(&config_path, invalid_config).expect("Failed to write config");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    let combined = format!("{}{}", output.stdout, output.stderr);

    assert!(
        combined.contains("theme")
            && combined.contains("reason:")
            && combined.contains("remediation:"),
        "Should detect missing theme with actionable guidance. Output: {combined}"
    );
}

#[test]
fn test_doctor_with_broken_segment_template() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();

    // A theme whose git.format has an unbalanced bracket must fail doctor.
    fs::write(
        themes_dir.join("badtemplate.toml"),
        r#"
[ui]

[segments.character]
success_symbol = "❯"
success_color = "white"

[segments.git]
format = "on [$branch(green)"
"#,
    )
    .expect("Failed to write broken-template theme");

    let config_path = env.config_path();
    let invalid_config = r#"
[agent]
enabled = true

[ui]
theme = "badtemplate"
enabled_segments = ["git"]
"#;
    fs::write(&config_path, invalid_config).expect("Failed to write config");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    let combined = format!("{}{}", output.stdout, output.stderr);

    assert_ne!(
        output.exit_code, 0_i32,
        "Doctor should exit non-zero for a broken segment template. Output: {combined}"
    );
    assert!(
        combined.contains("segment 'git'"),
        "Output should name the offending segment. Output: {combined}"
    );
}

#[test]
fn test_doctor_with_malformed_config() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    let config_path = env.config_path();
    let malformed_config = r"
[agent
enabled = true
this is not valid toml
";
    fs::write(&config_path, malformed_config).expect("Failed to write malformed config");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    let combined = format!("{}{}", output.stdout, output.stderr);

    assert!(
        combined.contains("config") || combined.contains("error") || combined.contains("TOML"),
        "Should detect malformed config. Output: {combined}"
    );
}

#[test]
fn doctor_lists_agent_tuning_and_warns_on_ignored_env() {
    let env = CliTestEnv::with_config("[agent]\nenabled = false\n")
        .expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy_with_env(&["doctor"], &[("GPY_DEBOUNCE_MS", "abc")])
        .expect("Failed to run gpy doctor");
    assert!(
        output.stdout.contains("[Agent tuning]"),
        "doctor must print the Agent tuning section: {output:?}"
    );
    let line = output
        .stdout
        .lines()
        .find(|line| line.contains("GPY_DEBOUNCE_MS"))
        .unwrap_or_else(|| panic!("no GPY_DEBOUNCE_MS line: {output:?}"));
    assert!(
        line.contains("ignored") && line.contains("100 ms"),
        "an ignored value must say so and show the default: {line:?}"
    );
    assert_eq!(
        output.exit_code, 0_i32,
        "an ignored tunable is a warning, not a failure: {output:?}"
    );
}

#[test]
fn doctor_warns_on_an_unknown_config_key() {
    let env = CliTestEnv::with_config(
        "[agent]\nenabled = false\n\n[git]\nenabledd = false\n\n[ui]\nenabled_segments = [\"directory\", \"git\"]\n",
    )
    .expect("Failed to create isolated CLI test environment");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");

    output.assert_success("gpy doctor");
    let warnings: Vec<&str> = output
        .stdout
        .lines()
        .filter(|line| line.contains("Unknown key") && line.contains("git.enabledd"))
        .collect();
    assert_eq!(warnings.len(), 1, "one unknown-key warning: {output:?}");
    assert!(
        warnings.iter().all(|line| line.contains("(ignored)")),
        "{output:?}"
    );
}
