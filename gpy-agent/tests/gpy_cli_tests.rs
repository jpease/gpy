//! Integration tests for the `gpy` CLI binary
//!
//! Tests user-facing commands like theme management, segment toggles,
//! config operations, and diagnostics.

#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::shadow_unrelated)]
#![allow(clippy::panic)]

use std::fs;
#[path = "common/cli_harness.rs"]
mod cli_harness;
use cli_harness::{CliTestEnv, SharedCliTestEnv};

/// Helper to run a gpy command and return stdout
fn run_gpy_command(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let result = SharedCliTestEnv::instance().run_gpy(args)?;
    result.assert_success(&format!("gpy {}", args.join(" ")));
    Ok(result.stdout)
}

#[test]
fn test_gpy_version() {
    let output = run_gpy_command(&["--version"]).expect("Failed to run gpy --version");
    assert!(
        output.contains("0.1.0"),
        "Version output should contain 0.1.0"
    );
}

#[test]
fn test_gpy_help() {
    let output = run_gpy_command(&["--help"]).expect("Failed to run gpy --help");
    assert!(output.contains("Usage:"), "Help should contain usage");
    assert!(output.contains("Commands:"), "Help should list commands");
    assert!(
        output.contains("start"),
        "Help should mention start command"
    );
    assert!(
        output.contains("theme"),
        "Help should mention theme command"
    );
    assert!(
        output.contains("config"),
        "Help should mention config command"
    );
}

#[test]
fn test_theme_subcommand_help() {
    let output = run_gpy_command(&["theme", "--help"]).expect("Failed to run gpy theme --help");
    assert!(
        output.contains("Manage themes"),
        "Theme help should describe purpose"
    );
    assert!(output.contains("list"), "Theme help should mention list");
    assert!(output.contains("use"), "Theme help should mention use");
    assert!(output.contains("show"), "Theme help should mention show");
    assert!(output.contains("new"), "Theme help should mention new");
    assert!(output.contains("save"), "Theme help should mention save");
    assert!(
        output.contains("validate"),
        "Theme help should mention validate"
    );
}

#[test]
fn test_plugin_subcommand_help() {
    let output = run_gpy_command(&["plugin", "--help"]).expect("Failed to run gpy plugin --help");
    assert!(
        output.contains("List discovered plugins"),
        "Plugin help should mention list"
    );
    assert!(
        output.contains("Create a new plugin scaffold"),
        "Plugin help should mention new"
    );
    assert!(
        output.contains("Validate a plugin by path or ID"),
        "Plugin help should mention validate"
    );
}

#[test]
fn test_config_subcommand_help() {
    let output = run_gpy_command(&["config", "--help"]).expect("Failed to run gpy config --help");
    assert!(
        output.contains("Configuration management"),
        "Config help should describe purpose"
    );
    assert!(output.contains("show"), "Config help should mention show");
    assert!(output.contains("get"), "Config help should mention get");
    assert!(output.contains("set"), "Config help should mention set");
    assert!(output.contains("open"), "Config help should mention open");
}

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
}

#[test]
fn test_completions_fish() {
    let output =
        run_gpy_command(&["completions", "fish"]).expect("Failed to generate fish completions");
    assert!(
        output.contains("function __fish_gpy"),
        "Fish completions should define functions"
    );
    assert!(
        output.contains("complete -c gpy"),
        "Fish completions should have complete commands"
    );
}

#[test]
fn test_completions_bash() {
    let output =
        run_gpy_command(&["completions", "bash"]).expect("Failed to generate bash completions");
    assert!(!output.is_empty(), "Bash completions should not be empty");
    // Bash completions typically start with shopt or function definitions
    assert!(
        output.contains("_gpy") || output.contains("complete"),
        "Bash completions should define completion functions"
    );
}

#[test]
fn test_completions_zsh() {
    let output =
        run_gpy_command(&["completions", "zsh"]).expect("Failed to generate zsh completions");
    assert!(!output.is_empty(), "Zsh completions should not be empty");
    assert!(
        output.contains("#compdef"),
        "Zsh completions should start with #compdef"
    );
}

/// Regression test for #333.
///
/// `hide`-marked subcommands/args (the internal `__complete` command and the
/// deprecated `theme use --apply-layout` alias) must not leak into generated
/// shell completions, even though `clap_complete`'s aot generators don't
/// honor `is_hide_set()` on their own. Visible items must still complete —
/// including the *unrelated*, never-hidden `theme import --apply-layout`
/// flag added later, which shares the same flag name as the deprecated alias
/// but must keep completing normally.
#[test]
fn test_completions_omit_hidden_items() {
    for shell in ["fish", "bash", "zsh"] {
        let output = run_gpy_command(&["completions", shell])
            .unwrap_or_else(|e| panic!("Failed to generate {shell} completions: {e}"));
        assert!(
            !output.contains("__complete"),
            "{shell} completions must not leak the hidden __complete command:\n{output}"
        );
        assert_apply_layout_confined_to_theme_import(shell, &output);
        // Sanity: visible commands/flags still complete after stripping.
        assert!(
            output.contains("theme"),
            "{shell} completions should still offer the visible `theme` command:\n{output}"
        );
        assert!(
            output.contains("force"),
            "{shell} completions should still offer the visible `--force` flag:\n{output}"
        );
        assert!(
            output.contains("apply-layout"),
            "{shell} completions should still offer the visible `theme import --apply-layout` flag:\n{output}"
        );
    }
}

/// `--apply-layout` names two different flags on `theme use` vs `theme import`.
///
/// A hidden, deprecated alias on `theme use` (must never complete) and a
/// real, visible flag on `theme import` (must complete). Each generator
/// groups a subcommand's options under a differently-shaped marker, so track
/// which subcommand block each line belongs to and assert every
/// `apply-layout` occurrence falls under `import`, never `use`.
fn assert_apply_layout_confined_to_theme_import(shell: &str, output: &str) {
    match shell {
        "fish" => {
            for line in output.lines().filter(|l| l.contains("apply-layout")) {
                assert!(
                    line.contains("from import") && !line.contains("from use"),
                    "fish completion for apply-layout must be scoped to `theme import`, not `theme use`:\n{line}"
                );
            }
        }
        "bash" | "zsh" => {
            let mut current_subcommand: Option<&str> = None;
            for line in output.lines() {
                let trimmed = line.trim();
                if trimmed.ends_with("theme__subcmd__use)") || trimmed == "(use)" {
                    current_subcommand = Some("use");
                } else if trimmed.ends_with("theme__subcmd__import)") || trimmed == "(import)" {
                    current_subcommand = Some("import");
                } else if trimmed == ";;" {
                    current_subcommand = None;
                }
                if line.contains("apply-layout") {
                    assert_eq!(
                        current_subcommand,
                        Some("import"),
                        "{shell} completion for apply-layout must be scoped to `theme import`, not {current_subcommand:?}:\n{line}"
                    );
                }
            }
        }
        other => panic!("unexpected shell {other}"),
    }
}

#[test]
fn test_complete_is_hidden_from_help() {
    let output = run_gpy_command(&["--help"]).expect("Failed to run gpy --help");
    assert!(
        !output.contains("__complete"),
        "Help output should not list the hidden __complete command:\n{output}"
    );
}

#[test]
fn test_complete_theme_emits_plain_names() {
    let output =
        run_gpy_command(&["__complete", "theme"]).expect("Failed to run gpy __complete theme");
    assert!(!output.trim().is_empty(), "Expected at least one theme");
    for line in output.lines() {
        assert!(
            !line.contains('*') && !line.contains('['),
            "theme completion line should be undecorated: {line:?}"
        );
    }
}

#[test]
fn test_complete_palette_emits_plain_names() {
    let output =
        run_gpy_command(&["__complete", "palette"]).expect("Failed to run gpy __complete palette");
    assert!(!output.trim().is_empty(), "Expected at least one palette");
    for line in output.lines() {
        assert!(
            !line.contains('*') && !line.contains('['),
            "palette completion line should be undecorated: {line:?}"
        );
    }
}

#[test]
fn test_complete_segment_emits_expected_builtins() {
    let output =
        run_gpy_command(&["__complete", "segment"]).expect("Failed to run gpy __complete segment");
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines,
        vec![
            "clock",
            "duration",
            "language",
            "directory",
            "git",
            "status",
            "username",
            "hostname"
        ]
    );
}

#[test]
fn test_complete_rejects_invalid_kind() {
    let result = SharedCliTestEnv::instance()
        .run_gpy(&["__complete", "bogus"])
        .expect("Failed to invoke gpy __complete bogus");
    assert_ne!(
        result.exit_code, 0_i32,
        "gpy __complete bogus should exit non-zero"
    );
}

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
fn test_config_show_all_sections() {
    let output = run_gpy_command(&["config", "show"]).expect("Failed to run gpy config show");

    // Should show all sections
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

/// `show(None)`'s output must equal its four `show(Some(section))` outputs
/// joined by blank lines.
///
/// This is the observable contract that lets `show(None)` call the section
/// printers directly on one parsed config instead of recursing back through
/// `show(Some(...))` (and re-reading the config file once per section) (#625).
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

    let expected = format!("{agent}\n{git}\n{language}\n{ui}");
    assert_eq!(
        all, expected,
        "show(None) output must be byte-identical to its per-section outputs joined by a blank line"
    );
}

#[test]
fn test_config_get_value() {
    let output =
        run_gpy_command(&["config", "get", "git.enabled"]).expect("Failed to run gpy config get");

    // Should output a boolean value
    let trimmed = output.trim();
    assert!(
        trimmed == "true" || trimmed == "false",
        "Should output a boolean value"
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

    let output = env
        .run_gpy(&["enable", "status"])
        .expect("Failed to run gpy enable status");
    output.assert_success("gpy enable status");

    let output = env
        .run_gpy(&["config", "get", "ui.enabled_segments"])
        .expect("Failed to run gpy config get");
    output.assert_success("gpy config get ui.enabled_segments");
    assert_eq!(output.stdout.trim(), "directory clock status");
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

    let output = env
        .run_gpy(&["enable", "clock"])
        .expect("Failed to run gpy enable clock");
    output.assert_success("gpy enable clock");
    let stdout = output.stdout;
    assert!(
        stdout.contains("Enabled clock"),
        "Should confirm clock segment enabled"
    );

    // Verify config was updated
    let updated_config = fs::read_to_string(&config_path).expect("Failed to read updated config");
    assert!(
        updated_config.contains("clock"),
        "Config should now contain clock segment"
    );

    let output = env
        .run_gpy(&["disable", "clock"])
        .expect("Failed to run gpy disable clock");
    output.assert_success("gpy disable clock");
    let stdout = output.stdout;
    assert!(
        stdout.contains("Disabled clock"),
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

    let output = env
        .run_gpy(&["enable", "hostname"])
        .expect("Failed to run gpy enable hostname");
    output.assert_success("gpy enable hostname");

    let output = env
        .run_gpy(&["config", "get", "ui.enabled_segments"])
        .expect("Failed to run gpy config get ui.enabled_segments");
    output.assert_success("gpy config get ui.enabled_segments");
    assert!(
        output.stdout.contains("hostname"),
        "ui.enabled_segments should list hostname: {:?}",
        output.stdout
    );
}

#[test]
fn test_doctor_accepts_starship_recommended_segments() {
    // The default CliTestEnv config pins `ui.enabled_segments`, and `theme use`
    // never overrides a user-set layout field, so the starship order would not
    // be applied and this test would pass vacuously. Start from a config that
    // leaves the layout unset.
    let env = CliTestEnv::with_config("[ui]\ntheme = \"default\"\n")
        .expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy(&["theme", "use", "starship", "--force"])
        .expect("Failed to run gpy theme use starship --force");
    output.assert_success("gpy theme use starship --force");
    assert!(
        output.stdout.contains("segment order (username, hostname"),
        "starship layout should have been applied: {:?}",
        output.stdout
    );

    // No agent runs in CliTestEnv, so doctor's exit code is not asserted.
    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    assert!(
        output.stdout.contains("✅ All segments recognized"),
        "doctor should accept the starship layout: {:?}",
        output.stdout
    );
    assert!(
        !output.stdout.contains("Unrecognized segments"),
        "doctor should not flag shipped segments: {:?}",
        output.stdout
    );
}

/// The `set -g __enabled_segments …` line of the fish theme export.
fn fish_enabled_segments(env: &CliTestEnv) -> String {
    let export = env
        .run_gpy_agent(&["theme", "export", "--format", "fish"])
        .expect("Failed to run gpy-agent theme export");
    export.assert_success("gpy-agent theme export --format fish");
    let Some(line) = export
        .stdout
        .lines()
        .find(|candidate| candidate.starts_with("set -g __enabled_segments"))
    else {
        panic!("no __enabled_segments line: {export:?}");
    };
    line.to_owned()
}

#[test]
fn test_enable_git_adds_to_enabled_segments_when_absent() {
    // #692: `git.enabled = true` alone does not render git; `gpy enable git`
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
fn test_enable_discovered_plugin_segment() {
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
enabled_segments = ["clock", "duration", "directory", "git"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let plugin_dir = env.plugins_dir().join("demo");
    fs::create_dir_all(&plugin_dir).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "demo"
name = "Demo"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["k8s_tools"]
"#,
    )
    .expect("Failed to write plugin manifest");

    let output = env
        .run_gpy(&["enable", "k8s_tools"])
        .expect("Failed to run gpy enable k8s_tools");

    assert!(
        output.exit_code == 0_i32,
        "Enable discovered plugin segment should succeed.\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    let updated_config = fs::read_to_string(&config_path).expect("Failed to read updated config");
    assert!(
        updated_config.contains("k8s_tools"),
        "Config should include plugin segment after enable"
    );
}

#[test]
fn test_plugin_new_creates_scaffold() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    let output = env
        .run_gpy(&["plugin", "new", "demo-tools", "--segment", "demo_status"])
        .expect("Failed to run gpy plugin new");
    output.assert_success("gpy plugin new demo-tools --segment demo_status");

    let plugin_root = env.plugins_dir().join("demo-tools");
    let manifest_path = plugin_root.join("plugin.toml");
    let segment_path = plugin_root.join("segments").join("demo_status.fish");

    assert!(manifest_path.is_file(), "Plugin manifest should be created");
    assert!(
        segment_path.is_file(),
        "Plugin segment file should be created"
    );

    let manifest = fs::read_to_string(&manifest_path).expect("Failed to read plugin manifest");
    assert!(
        manifest.contains("id = \"demo-tools\""),
        "Manifest should include plugin id"
    );
    assert!(
        manifest.contains("provided_segments = [\"demo_status\"]"),
        "Manifest should include requested segment"
    );

    let segment = fs::read_to_string(&segment_path).expect("Failed to read plugin segment file");
    assert!(
        segment.contains("function segment_demo_status_detect"),
        "Scaffold should create detect function"
    );
    assert!(
        segment.contains("gpy_section_standalone"),
        "Scaffold should use supported public helper surface"
    );
}

#[test]
fn test_plugin_validate_reports_missing_segment_file() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let plugin_dir = env.plugins_dir().join("broken");
    fs::create_dir_all(&plugin_dir).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "broken"
name = "Broken"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["broken_seg"]
"#,
    )
    .expect("Failed to write plugin manifest");

    let output = env
        .run_gpy(&["plugin", "validate", &plugin_dir.display().to_string()])
        .expect("Failed to run gpy plugin validate");

    assert_ne!(
        output.exit_code, 0_i32,
        "Validation should fail when segment file is missing"
    );
    assert!(
        output.stderr.contains("Missing segment file")
            || output.stdout.contains("Missing segment file"),
        "Output should mention the missing segment file.\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
}

#[test]
fn test_plugin_validate_rejects_misspelled_provided_segments() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let plugin_dir = env.plugins_dir().join("weather");
    fs::create_dir_all(plugin_dir.join("segments")).expect("Failed to create plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
id = "weather"
name = "Weather"
version = "1.0.0"
api_version = "v1"
entry_type = "file"
provided_segment = ["weather"]
"#,
    )
    .expect("Failed to write plugin manifest");

    let output = env
        .run_gpy(&["plugin", "validate", &plugin_dir.display().to_string()])
        .expect("Failed to run gpy plugin validate");

    assert_ne!(
        output.exit_code, 0_i32,
        "Validation should fail when provided_segments is missing"
    );
    assert!(
        output.stderr.contains("provided_segments") || output.stdout.contains("provided_segments"),
        "Output should name provided_segments.\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );

    let list = env
        .run_gpy(&["plugin", "list"])
        .expect("Failed to run gpy plugin list");
    assert!(
        !list.stdout.contains("weather 1.0.0 [user] (ready)"),
        "Plugin without provided_segments must not be listed as ready.\nstdout:\n{}",
        list.stdout
    );
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
    let stdout = output.stdout;
    assert!(
        stdout.contains("Language versions enabled"),
        "Should confirm versions enabled"
    );

    // Verify config was updated
    let updated_config = fs::read_to_string(&config_path).expect("Failed to read updated config");
    assert!(
        updated_config.contains("show_versions = true"),
        "Config should have show_versions = true"
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
enabled = true

[language]
enabled = true

[ui]
theme = "default"
enabled_segments = ["clock", "duration", "language", "directory", "git"]
"#;
    fs::write(&config_path, valid_config).expect("Failed to write config");

    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    // No agent runs in this env, so doctor reports it and exits 1 (#636);
    // the config/theme checks below are what this test is about.
    assert_eq!(
        output.exit_code, 1_i32,
        "doctor with no agent exits 1: {output:?}"
    );
    let stdout = output.stdout;
    assert!(
        stdout.contains("Checking process... ⚠️  Not running"),
        "doctor must not report a stopped agent as running (#636): {stdout}"
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
fn test_config_set_and_get() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let config_content = r#"
[agent]
enabled = true

[git]
enabled = true
max_branch_length = 0

[language]
enabled = true

[ui]
theme = "default"
directory.display = "basename"
directory.max_length = 80
enabled_segments = ["language", "directory", "git"]
"#;
    fs::write(&config_path, config_content).expect("Failed to write config");

    let output = env
        .run_gpy(&["config", "set", "ui.directory.max_length", "120"])
        .expect("Failed to run gpy config set");
    output.assert_success("gpy config set ui.directory.max_length 120");
    let stdout = output.stdout;
    assert!(
        stdout.contains("Set ui.directory.max_length = 120"),
        "Should confirm value set"
    );

    // Test getting the value
    let output = env
        .run_gpy(&["config", "get", "ui.directory.max_length"])
        .expect("Failed to run gpy config get");
    output.assert_success("gpy config get ui.directory.max_length");
    let stdout = output.stdout;
    assert_eq!(stdout.trim(), "120", "Should return the updated value");
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

    let output = env
        .run_gpy(&["config", "set", "ui.directory.display", "abbreviated"])
        .expect("Failed to run gpy config set");
    output.assert_success("gpy config set ui.directory.display abbreviated");
    assert!(
        output
            .stdout
            .contains("Set ui.directory.display = abbreviated"),
        "Should confirm directory display mode set"
    );

    let output = env
        .run_gpy(&["config", "get", "ui.directory.display"])
        .expect("Failed to run gpy config get");
    output.assert_success("gpy config get ui.directory.display");
    assert_eq!(output.stdout.trim(), "abbreviated");
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
    assert!(
        output
            .stdout
            .contains(&format!("Opened config: {}", config_path.display())),
        "Config open should report the opened path"
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

    // Verify file was created
    let theme_file = themes_dir.join("test-theme.toml");
    assert!(theme_file.exists(), "Theme file should be created");

    // Verify content is valid TOML
    let content = fs::read_to_string(&theme_file).expect("Failed to read created theme file");
    assert!(content.contains("[ui]"), "Theme should have [ui] section");
    assert!(
        content.contains("[segments"),
        "Theme should have segments sections"
    );
}

#[test]
fn test_theme_new_from_clones_base_theme_contents() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();

    let output = env
        .run_gpy(&["theme", "new", "cloned-theme", "--from", "starship"])
        .expect("Failed to run gpy theme new --from");
    output.assert_success("gpy theme new cloned-theme --from starship");
    assert!(
        output.stdout.contains("cloned from 'starship'"),
        "Should mention the clone source. Got: {}",
        output.stdout
    );

    let cloned_content = fs::read_to_string(themes_dir.join("cloned-theme.toml"))
        .expect("Failed to read cloned theme file");

    let starship_source = env
        .run_gpy(&["theme", "new", "starship-reference"])
        .expect("Failed to run gpy theme new");
    starship_source.assert_success("gpy theme new starship-reference");
    // `starship-reference` is a blank default-template theme, so instead
    // compare against the embedded starship content indirectly: the clone
    // must differ from the default template and must be valid on its own.
    let default_content = fs::read_to_string(themes_dir.join("starship-reference.toml"))
        .expect("Failed to read default-template theme file");
    assert_ne!(
        cloned_content, default_content,
        "Cloning from 'starship' should not just copy the blank default template"
    );

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
    let active_content = fs::read_to_string(themes_dir.join("default.toml"))
        .expect("Failed to read active default theme");
    assert_eq!(
        saved_content, active_content,
        "Saved theme should match the currently active theme's contents"
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
    output.assert_success("gpy theme save custom (declined)");
    assert!(
        output.stdout.contains("[y/N]"),
        "Should prompt before overwriting an existing named theme. Got: {}",
        output.stdout
    );
    assert!(
        output.stdout.contains("Aborted"),
        "Declining should report an abort. Got: {}",
        output.stdout
    );

    let unchanged = fs::read_to_string(&target_path).expect("Failed to read theme file");
    assert_eq!(
        unchanged, marker_content,
        "Declining the overwrite prompt must leave the existing theme file untouched"
    );
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
    let active_content = fs::read_to_string(themes_dir.join("default.toml"))
        .expect("Failed to read active default theme");
    assert_eq!(
        overwritten, active_content,
        "Accepting the overwrite prompt should replace the file with the active theme's contents"
    );
    assert!(
        !overwritten.contains("pre-existing custom marker"),
        "Old marker content must be gone after overwrite"
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
    declined.assert_success("gpy theme save (no name, declined)");
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
    assert!(
        target_path.exists(),
        "Accepting should materialize the currently active theme"
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

// --- #459: validation must reject a theme name that does not exist ---------
//
// `ThemeManager::new` deliberately falls back to the default theme for an
// undiscovered name so a user whose theme file vanished still gets a working
// prompt. `commands::theme::validate_by_name` is built on it, so before #459
// it silently validated the *default* theme's templates and reported success
// for a name that exists nowhere. The runtime fallback stays; only the
// validation path becomes strict.

/// A theme name that matches no builtin, user, or plugin theme.
const NONEXISTENT_THEME_NAME: &str = "this-theme-does-not-exist-459";

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
}

/// Test end-to-end theme switching workflow
#[test]
#[allow(clippy::too_many_lines)]
fn test_theme_switch_end_to_end() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let config_path = env.config_path();
    let themes_dir = env.themes_dir();

    // Create config with default theme
    let initial_config = r#"
[agent]
enabled = true

[ui]
theme = "default"
enabled_segments = ["clock", "duration"]
"#;
    fs::write(&config_path, initial_config).expect("Failed to write initial config");

    // Create a custom theme file with required segments section
    let custom_theme = r##"
[ui]
prompt_icon = "❯"
prompt_color = "white"

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

    // Switch to custom theme
    let output = env
        .run_gpy(&["theme", "use", "custom"])
        .expect("Failed to run gpy theme use");
    output.assert_success("gpy theme use custom");
    let stdout = output.stdout;
    assert!(
        stdout.contains("Switched to 'custom'") || stdout.contains("custom"),
        "Should confirm theme switch"
    );

    // Verify config was updated
    let updated_config = fs::read_to_string(&config_path).expect("Failed to read updated config");
    assert!(
        updated_config.contains("theme = \"custom\""),
        "Config should reference custom theme"
    );

    // Verify theme show reflects the change
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

/// Test segment enable/disable with round-trip verification
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

    // Enable clock segment
    let enable_output = env
        .run_gpy(&["enable", "clock"])
        .expect("Failed to run gpy enable clock");
    enable_output.assert_success("gpy enable clock");

    // Verify clock is in enabled_segments
    let config_after_enable =
        fs::read_to_string(&config_path).expect("Failed to read config after enable");
    assert!(
        config_after_enable.contains("clock"),
        "Config should contain clock segment"
    );

    // Disable clock segment
    let disable_output = env
        .run_gpy(&["disable", "clock"])
        .expect("Failed to run gpy disable clock");
    disable_output.assert_success("gpy disable clock");

    // Verify clock is no longer in enabled_segments
    let config_after_disable =
        fs::read_to_string(&config_path).expect("Failed to read config after disable");

    // Check that clock is not in the enabled_segments array
    assert!(
        !config_after_disable.contains("enabled_segments = [\"clock\"")
            && !config_after_disable.contains("\"clock\",")
            && !config_after_disable.contains(", \"clock\""),
        "Clock should be removed from enabled segments"
    );
}

/// Test config get/set with validation errors
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

    // Test setting a valid value
    let valid_output = env
        .run_gpy(&["config", "set", "ui.directory.max_length", "120"])
        .expect("Failed to run gpy config set");
    valid_output.assert_success("gpy config set ui.directory.max_length 120");

    // Verify the value was set
    let get_output = env
        .run_gpy(&["config", "get", "ui.directory.max_length"])
        .expect("Failed to run gpy config get");
    get_output.assert_success("gpy config get ui.directory.max_length");
    let get_stdout = get_output.stdout;
    assert_eq!(get_stdout.trim(), "120", "Should return the updated value");

    // Test setting an invalid value (negative number for ui.directory.max_length)
    let invalid_output = env
        .run_gpy(&["config", "set", "ui.directory.max_length", "-10"])
        .expect("Failed to run gpy config set with invalid value");

    // Should fail with invalid value
    assert!(
        invalid_output.exit_code != 0_i32
            || invalid_output.stderr.contains("error")
            || invalid_output.stderr.contains("invalid"),
        "Should reject invalid value"
    );

    // Test setting a non-existent key
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

/// Test gpy doctor with various scenarios
#[test]
fn test_doctor_basic_checks() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    // Create a valid config
    let config_path = env.config_path();
    let valid_config = r#"
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
    fs::write(&config_path, valid_config).expect("Failed to write config");

    // Run doctor with valid config. No agent runs in this env, so the
    // agent check fails and doctor exits 1 (#636); the config checks still run.
    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    assert_eq!(
        output.exit_code, 1_i32,
        "doctor with no agent exits 1: {output:?}"
    );
    let stdout = output.stdout;
    assert!(
        stdout.contains("Checking config file... ✅ Valid"),
        "a valid config passes its check: {stdout}"
    );

    // Should run diagnostics
    assert!(
        stdout.contains("diagnostics") || stdout.contains("Checking"),
        "Should show diagnostics output"
    );
}

/// Test gpy doctor with invalid theme
#[test]
fn test_doctor_with_invalid_theme() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();

    // Create config with a broken theme file
    fs::write(
        themes_dir.join("broken.toml"),
        r#"
[ui]
prompt_icon = "❯"
prompt_color = "invalid-color"
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

    // Run doctor - should detect the invalid theme
    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    let combined = format!("{}{}", output.stdout, output.stderr);

    // Should report issue with theme and remediation guidance
    assert!(
        combined.contains("theme")
            && combined.contains("reason:")
            && combined.contains("remediation:"),
        "Should detect missing theme with actionable guidance. Output: {combined}"
    );
}

/// Test gpy doctor with a broken segment format template
#[test]
fn test_doctor_with_broken_segment_template() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();

    // A theme whose git.format has an unbalanced bracket must fail doctor.
    fs::write(
        themes_dir.join("badtemplate.toml"),
        r#"
[ui]
prompt_icon = "❯"
prompt_color = "white"

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

/// Test gpy doctor with malformed config
#[test]
fn test_doctor_with_malformed_config() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");

    // Create malformed TOML config
    let config_path = env.config_path();
    let malformed_config = r"
[agent
enabled = true
this is not valid toml
";
    fs::write(&config_path, malformed_config).expect("Failed to write malformed config");

    // Run doctor - should detect the malformed config
    let output = env.run_gpy(&["doctor"]).expect("Failed to run gpy doctor");
    let combined = format!("{}{}", output.stdout, output.stderr);

    // Should report config error
    assert!(
        combined.contains("config") || combined.contains("error") || combined.contains("TOML"),
        "Should detect malformed config. Output: {combined}"
    );
}

// ===========================================================================
// Issue #669: theme validation covers clock, hostname, and username formats
// ===========================================================================

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

        // 1. theme validate <name> rejects
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

        // 2. config set ui.theme rejects and config is unchanged
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
fn test_valid_hostname_username_theme_renders_via_oneshot() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    fs::create_dir_all(&themes_dir).expect("Failed to create themes dir");

    let valid_theme = r#"
[ui]
[segments.hostname]
format = "[$hostname](fg:white)"
[segments.username]
format = "[$username](fg:white)"
"#;
    fs::write(themes_dir.join("valid-custom.toml"), valid_theme)
        .expect("Failed to write valid theme");

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

    let val_out = env
        .run_gpy_with_env(&["theme", "validate", "valid-custom"], &env_overrides)
        .expect("Failed to run validate");
    val_out.assert_success("valid-custom theme validation");

    // Set active theme to valid-custom
    let set_out = env
        .run_gpy_with_env(
            &["config", "set", "ui.theme", "valid-custom"],
            &env_overrides,
        )
        .expect("Failed to set theme");
    set_out.assert_success("set theme valid-custom");

    // oneshot hostname renders expected text
    let host_out = env
        .run_gpy_agent_with_env(
            &[
                "oneshot",
                "hostname",
                "--hostname",
                "example-box",
                "--format",
                "ansi",
            ],
            &env_overrides,
        )
        .expect("Failed to run oneshot hostname");
    assert!(
        host_out.stdout.contains("example-box"),
        "hostname rendering must contain example-box: stdout={:?} stderr={:?}",
        host_out.stdout,
        host_out.stderr
    );

    // oneshot username renders expected text
    let user_out = env
        .run_gpy_agent_with_env(
            &[
                "oneshot",
                "username",
                "--username",
                "test-user",
                "--format",
                "ansi",
            ],
            &env_overrides,
        )
        .expect("Failed to run oneshot username");
    assert!(
        user_out.stdout.contains("test-user"),
        "username rendering must contain test-user: stdout={:?} stderr={:?}",
        user_out.stdout,
        user_out.stderr
    );
}

// ===========================================================================
// Issue #668: theme use prospective activation validation
// ===========================================================================

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

    // Normal activation
    let res = env
        .run_gpy_with_env(&["theme", "use", "broken"], &env_overrides)
        .expect("Failed to run theme use");
    assert_ne!(res.exit_code, 0_i32, "theme use broken must fail");
    assert_eq!(
        fs::read(env.config_path()).expect("read config"),
        original_config,
        "config bytes must be unchanged after rejected theme use"
    );

    // Forced activation
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

    // Remove config file to test fresh file-less environment
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
    // Verified prospective-palette fixture from issue #668
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

    // 3. Explicit palette = "default": forced activation preserves explicit value and rejects
    let explicit_config = "[ui]\ntheme = \"default\"\npalette = \"default\"\n";
    fs::write(env.config_path(), explicit_config).expect("Failed to write explicit config");
    let res_explicit = env
        .run_gpy_with_env(
            &["theme", "use", "activation-case", "--force"],
            &env_overrides,
        )
        .expect("Failed to run forced theme use with explicit palette");
    assert_ne!(
        res_explicit.exit_code, 0_i32,
        "forced activation must reject when explicit default palette lacks accent"
    );
    assert_eq!(
        fs::read_to_string(env.config_path()).expect("read config"),
        explicit_config,
        "config must remain unchanged when forced activation fails due to explicit palette"
    );
}

/// Issue #731: a palette written by the outgoing theme's recommendation is
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
        res.stdout.contains("palette (nord)"),
        "applied summary must name palette: {}",
        res.stdout
    );

    // Hand-set palette differing from the outgoing recommendation is preserved and reported.
    fs::write(env.config_path(), "[ui]\npalette = \"nord\"\n").expect("write config");
    let res_kept = env
        .run_gpy_with_env(&["theme", "use", "starship", "--force"], &env_overrides)
        .expect("Failed to run forced theme use starship");
    res_kept.assert_success("forced theme use starship");
    let kept = fs::read_to_string(env.config_path()).expect("read config");
    assert!(
        kept.contains("palette = \"nord\""),
        "user palette must be kept: {kept}"
    );
    let preserved_line = res_kept
        .stdout
        .lines()
        .find(|line| line.contains("Preserved your explicit settings"))
        .unwrap_or_default();
    assert!(
        preserved_line.contains("palette (nord)"),
        "preserved summary must name palette: {}",
        res_kept.stdout
    );
}

/// Issue #731 (a)/(b), enabled by #730: after another CLI write, `--force`
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
            res.stdout.contains("palette (starship)")
                && res.stdout.contains("language detection (markers)"),
            "applied summary after {prior:?}: {}",
            res.stdout
        );
        assert!(
            !res.stdout.contains("Preserved"),
            "nothing may be preserved after {prior:?}: {}",
            res.stdout
        );
    }
}

// ===========================================================================
// Issue #672: theme new validates destination name
// ===========================================================================

#[test]
#[allow(clippy::too_many_lines)]
fn test_theme_new_rejects_empty_and_whitespace_names() {
    let env = CliTestEnv::new().expect("Failed to create isolated CLI test environment");
    let themes_dir = env.themes_dir();
    // Remove pre-created themes dir to test fresh directory-less behavior
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
