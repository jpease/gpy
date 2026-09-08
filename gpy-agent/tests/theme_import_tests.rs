//! Integration tests for `gpy theme import`.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

use std::fs;

#[path = "common/cli_harness.rs"]
mod cli_harness;
use cli_harness::CliTestEnv;

const STARSHIP_SAMPLE: &str = r##"
format = "$directory$git_branch$git_status$rust$cmd_duration$character$kubernetes"
palette = "demo"

[palettes.demo]
accent = "#88c0d0"

[directory]
style = "bold cyan"

[git_branch]
symbol = " "
style = "bold purple"

[git_status]
style = "bold red"

[rust]
symbol = " "
style = "bold red"

[cmd_duration]
min_time = 2000
style = "bold yellow"

[character]
success_symbol = "[>](bold green)"
error_symbol = "[>](bold red)"
"##;

fn write_sample(env: &CliTestEnv) -> std::path::PathBuf {
    let path = env.root().join("starship.toml");
    fs::write(&path, STARSHIP_SAMPLE).unwrap();
    path
}

#[test]
fn import_writes_palette_and_theme_files() {
    let env = CliTestEnv::new().unwrap();
    let source = write_sample(&env);
    let result = env
        .run_gpy_agent(&[
            "theme",
            "import",
            source.to_str().unwrap(),
            "--name",
            "demo",
        ])
        .unwrap();
    result.assert_success("theme import");

    let palette = env.config_dir().join("palettes").join("demo.toml");
    let theme = env.themes_dir().join("demo.toml");
    assert!(palette.exists(), "palette written");
    assert!(theme.exists(), "theme written");

    let theme_body = fs::read_to_string(&theme).unwrap();
    assert!(theme_body.contains("imported from starship.toml"));
    // Activation hints + warnings (kubernetes unsupported) surfaced.
    assert!(result.stdout.contains("gpy theme use demo"));
    assert!(result.stderr.contains("kubernetes"));
}

#[test]
fn import_stdout_prints_both_artifacts_without_writing() {
    let env = CliTestEnv::new().unwrap();
    let source = write_sample(&env);
    let result = env
        .run_gpy_agent(&[
            "theme",
            "import",
            source.to_str().unwrap(),
            "--name",
            "demo",
            "--stdout",
        ])
        .unwrap();
    result.assert_success("theme import --stdout");
    assert!(result.stdout.contains("# ---- palette ----"));
    assert!(result.stdout.contains("# ---- theme ----"));
    assert!(
        !env.themes_dir().join("demo.toml").exists(),
        "no file written for --stdout"
    );
}

#[test]
fn import_refuses_overwrite_without_force() {
    let env = CliTestEnv::new().unwrap();
    let source = write_sample(&env);
    let args = [
        "theme",
        "import",
        source.to_str().unwrap(),
        "--name",
        "demo",
    ];
    env.run_gpy_agent(&args)
        .unwrap()
        .assert_success("first import");

    let second = env.run_gpy_agent(&args).unwrap();
    assert_ne!(second.exit_code, 0_i32, "second import should fail");
    assert!(
        second.stderr.contains("--force") || second.stdout.contains("--force"),
        "expected --force hint in output"
    );

    let forced = env
        .run_gpy_agent(&[
            "theme",
            "import",
            source.to_str().unwrap(),
            "--name",
            "demo",
            "--force",
        ])
        .unwrap();
    forced.assert_success("forced import");
}

#[test]
fn import_apply_layout_writes_enabled_segments() {
    let env = CliTestEnv::new().unwrap();
    let source = write_sample(&env);
    let result = env
        .run_gpy_agent(&[
            "theme",
            "import",
            source.to_str().unwrap(),
            "--name",
            "demo",
            "--apply-layout",
        ])
        .unwrap();
    result.assert_success("theme import --apply-layout");

    let config = fs::read_to_string(env.config_path()).unwrap();
    assert!(config.contains("enabled_segments"));
    assert!(config.contains("directory"));
    assert!(config.contains("git"));
    assert!(config.contains("language"));
}

#[test]
fn import_missing_file_is_hard_error() {
    let env = CliTestEnv::new().unwrap();
    let result = env
        .run_gpy_agent(&["theme", "import", "/no/such/starship.toml"])
        .unwrap();
    assert_ne!(result.exit_code, 0_i32, "missing file should fail");
    assert!(
        result.stderr.contains("not found") || result.stdout.contains("not found"),
        "expected 'not found' in output"
    );
}

#[test]
fn import_rejects_traversal_name() {
    let env = CliTestEnv::new().unwrap();
    let source = write_sample(&env);
    let result = env
        .run_gpy_agent(&[
            "theme",
            "import",
            source.to_str().unwrap(),
            "--name",
            "../../etc/passwd",
        ])
        .unwrap();
    assert_ne!(result.exit_code, 0_i32, "traversal name must fail");
    let combined = format!("{}{}", result.stdout, result.stderr);
    assert!(
        combined.contains("invalid artifact name") || combined.contains("path separator"),
        "expected path-traversal error in output, got: {combined}"
    );
}

#[test]
fn import_is_also_wired_into_the_gpy_binary() {
    let env = CliTestEnv::new().unwrap();
    let source = write_sample(&env);
    let result = env
        .run_gpy(&[
            "theme",
            "import",
            source.to_str().unwrap(),
            "--name",
            "demo",
        ])
        .unwrap();
    result.assert_success("gpy theme import");

    let palette = env.config_dir().join("palettes").join("demo.toml");
    let theme = env.themes_dir().join("demo.toml");
    assert!(palette.exists(), "palette written");
    assert!(theme.exists(), "theme written");
}

#[test]
fn import_warns_when_no_palette_defined() {
    let env = CliTestEnv::new().unwrap();
    let path = env.root().join("nopalette.toml");
    fs::write(
        &path,
        "format = \"$directory\"\n[directory]\nstyle = \"bold cyan\"\n",
    )
    .unwrap();
    let result = env
        .run_gpy_agent(&["theme", "import", path.to_str().unwrap(), "--name", "np"])
        .unwrap();
    result.assert_success("import without palette");
    assert!(
        result.stderr.contains("no palette"),
        "expected 'no palette' warning in stderr"
    );
}
