//! End-to-end CLI tests for `gpy-agent` and `gpy`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[path = "common/cli_harness.rs"]
mod cli_harness;
use cli_harness::CliTestEnv;
#[path = "common/git.rs"]
mod git_helpers;

type DelimiterResult = Result<(String, String), Box<dyn std::error::Error>>;

fn load_theme_delimiters() -> DelimiterResult {
    let theme_content = fs::read_to_string("../config/themes/default.toml")?;
    let theme: toml::Value = toml::from_str(&theme_content)?;

    let segment_close = theme
        .get("ui")
        .and_then(|ui| ui.get("segment_close"))
        .and_then(|sc| sc.get("icon"))
        .and_then(|d| d.as_str())
        .unwrap_or("]")
        .to_owned();

    let prompt_close = theme
        .get("ui")
        .and_then(|ui| ui.get("prompt_close"))
        .and_then(|pc| pc.get("icon"))
        .and_then(|d| d.as_str())
        .unwrap_or(")")
        .to_owned();

    Ok((segment_close, prompt_close))
}

fn strip_ansi_codes(output: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(output, "").to_string()
}

fn assert_delimiter_absent(output: &str, delimiter: &str, segment_name: &str, context: &str) {
    assert!(
        !output.contains(delimiter),
        "{context}, {segment_name} segment should NOT use delimiter '{delimiter}'. Got: {output}"
    );
}

fn assert_delimiter_present(output: &str, delimiter: &str, segment_name: &str, context: &str) {
    assert!(
        output.contains(delimiter),
        "{context}, {segment_name} segment should use delimiter '{delimiter}'. Got: {output}"
    );
}

/// Template-rendered `git`/`language` segments close with `$sep_close`:
/// `prompt_close` when `is_last`, else `segment_close` (matching the Fish-side
/// non-last cap). The other glyph must be absent.
fn assert_delimiter_usage(output: &str, is_last: bool, segment_name: &str) {
    let (segment_close, prompt_close) =
        load_theme_delimiters().expect("Failed to load theme delimiters");

    let stripped = strip_ansi_codes(output);

    if is_last {
        if !segment_close.is_empty() {
            assert_delimiter_absent(&stripped, &segment_close, segment_name, "template render");
        }
        if !prompt_close.is_empty() {
            assert_delimiter_present(
                &stripped,
                &prompt_close,
                segment_name,
                "template render (is-last)",
            );
        }
    } else {
        if !segment_close.is_empty() {
            assert_delimiter_present(
                &stripped,
                &segment_close,
                segment_name,
                "template render (not-last)",
            );
        }
        if !prompt_close.is_empty() {
            assert_delimiter_absent(
                &stripped,
                &prompt_close,
                segment_name,
                "template render (not-last)",
            );
        }
    }
}

fn run_gpy_agent(args: &[&str]) -> (i32, String, String) {
    match CliTestEnv::new().and_then(|env| env.run_gpy_agent(args)) {
        Ok(output) => (output.exit_code, output.stdout, output.stderr),
        Err(e) => (-1_i32, String::new(), format!("spawn error: {e}")),
    }
}

fn run_gpy_agent_with_config(config_toml: &str, args: &[&str]) -> (i32, String, String) {
    let env = match CliTestEnv::with_config(config_toml) {
        Ok(env) => env,
        Err(e) => return (-1, String::new(), format!("test env error: {e}")),
    };
    match env.run_gpy_agent(args) {
        Ok(output) => (output.exit_code, output.stdout, output.stderr),
        Err(e) => (-1_i32, String::new(), format!("spawn error: {e}")),
    }
}

fn create_test_git_repo() -> Result<TempDir, Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    git_helpers::git_init(temp_dir.path());

    Ok(temp_dir)
}

fn create_test_rust_project() -> Result<TempDir, Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

    let cargo_toml = r#"[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#;
    fs::write(temp_path.join("Cargo.toml"), cargo_toml)?;

    fs::create_dir_all(temp_path.join("src"))?;
    fs::write(temp_path.join("src").join("main.rs"), "fn main() {}")?;

    Ok(temp_dir)
}

#[test]
fn test_cli_help_command() {
    let (exit_code, stdout, _stderr) = run_gpy_agent(&["--help"]);

    assert_eq!(exit_code, 0_i32);
    let stdout_lower = stdout.to_lowercase();
    assert!(
        stdout_lower.contains("usage:")
            && stdout_lower.contains("gpy-agent")
            && stdout_lower.contains("<command>"),
        "Stdout did not contain expected usage pattern. Got:\n{stdout}",
    );
    assert!(stdout.contains("Commands:"));
    assert!(stdout.contains("start"));
    assert!(stdout.contains("stop"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("oneshot"));
    assert!(stdout.contains("help"));
}

#[test]
fn test_cli_version_command() {
    let (exit_code, stdout, _stderr) = run_gpy_agent(&["--version"]);

    assert_eq!(exit_code, 0_i32);
    assert!(stdout.contains("gpy-agent"));
}

#[test]
fn test_cli_oneshot_git_current_repo() {
    let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
    let temp_path = temp_repo.path();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed with stderr: {stderr}");

    assert!(stdout.starts_with('{'));
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(json.get("branch").is_some());
    assert!(json.get("ahead").is_some());
    assert!(json.get("behind").is_some());
}

#[test]
fn test_cli_oneshot_git_with_custom_path() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    git_helpers::git_init(temp_path);

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed with stderr: {stderr}");

    assert!(stdout.starts_with('{'));
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(json.get("branch").is_some());
}

#[test]
fn test_cli_oneshot_git_ignores_inherited_git_dir() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");
    let a = env.root().join("repoA");
    let b = env.root().join("repoB");
    for (repo, branch) in [(&a, "featureA"), (&b, "main")] {
        fs::create_dir_all(repo).expect("mkdir");
        git_helpers::git(repo, &["init", "-q", "-b", branch]);
        fs::write(repo.join("f"), "x\n").expect("write");
        git_helpers::git_commit_all(repo, "init");
    }
    fs::write(b.join("untracked.txt"), "z\n").expect("write");

    let result = env
        .run_gpy_agent_with_env(
            &["oneshot", "git", "--cwd", &b.to_string_lossy()],
            &[("GIT_DIR", a.join(".git")), ("GIT_WORK_TREE", a.clone())],
        )
        .expect("run gpy-agent");
    result.assert_success("oneshot git");
    let json: serde_json::Value = serde_json::from_str(&result.stdout).expect("Invalid JSON");
    assert_eq!(
        json.get("branch"),
        Some(&serde_json::json!("main")),
        "reported repo A's branch: {json}"
    );
    assert_eq!(
        json.get("untracked"),
        Some(&serde_json::json!(1_i32)),
        "reported repo A's counts: {json}"
    );
}

#[test]
fn test_cli_oneshot_git_non_git_directory() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    let (exit_code, stdout, _stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
    ]);

    assert_eq!(exit_code, 0_i32);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(json.get("error").is_some());
}

fn assert_config_get(key: &str, expected: &str) {
    let (exit_code, stdout, stderr) =
        match CliTestEnv::new().and_then(|env| env.run_gpy(&["config", "get", key])) {
            Ok(output) => (output.exit_code, output.stdout, output.stderr),
            Err(e) => (-1_i32, String::new(), format!("spawn error: {e}")),
        };
    assert_eq!(exit_code, 0_i32, "config get {key} failed: {stderr}");
    assert_eq!(stdout.trim(), expected);
}

#[test]
fn test_cli_config_get_outputs_expected_values() {
    assert_config_get("agent.supervisor.enabled", "true");
    assert_config_get("agent.supervisor.check_interval_seconds", "30");
    assert_config_get("ui.enabled_segments", "clock duration directory git");
}

#[test]
fn test_cli_config_reload_succeeds_without_agent() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["config", "reload"]);
    assert_eq!(
        exit_code, 0_i32,
        "config reload should exit successfully: {stderr}"
    );
}

// The background agent exists only on Unix.
#[cfg(unix)]
#[test]
fn test_cli_start_respects_agent_enabled_false() {
    const DISABLED_CONFIG: &str = r#"
[agent]
enabled = false
timeout_seconds = 5
live_updates = true

[agent.supervisor]
enabled = true
check_interval_seconds = 30
max_restart_attempts = 5

[git]
enabled = true
show_upstream = true
timeout_seconds = 10
skip_paths = []

[language]
enabled = true
show_versions = true
cache_ttl_hours = 24
enabled_languages = []
display = "icon"

[ui]
show_icons = true
theme = "default"
directory.max_length = 80
enabled_segments = ["clock", "duration", "directory", "git"]
"#;

    let (exit_code, stdout, stderr) = run_gpy_agent_with_config(DISABLED_CONFIG, &["start"]);
    assert_eq!(
        exit_code, 6_i32,
        "start must exit 6 when disabled: {stderr}"
    );
    assert!(
        stderr.contains("Agent disabled in config"),
        "expected disabled notice in stderr, got '{stderr}'"
    );
    assert!(
        stdout.trim().is_empty(),
        "expected no stdout output when agent is disabled, got '{stdout}'"
    );
}

/// `start` refuses on native Windows before consulting `agent.enabled`.
#[cfg(windows)]
#[test]
fn test_cli_start_is_unsupported_on_native_windows() {
    let (exit_code, stdout, stderr) = run_gpy_agent_with_config("", &["start"]);
    assert_eq!(exit_code, 1_i32, "start must fail on Windows: {stderr}");
    assert!(
        stderr.contains("not yet implemented for native Windows"),
        "expected the unsupported-platform notice in stderr, got '{stderr}'"
    );
    assert!(
        stdout.trim().is_empty(),
        "expected no stdout output, got '{stdout}'"
    );
}

#[test]
fn test_cli_oneshot_git_succeeds_when_agent_disabled() {
    const DISABLED_CONFIG: &str = r#"
[agent]
enabled = false
timeout_seconds = 5
live_updates = true

[agent.supervisor]
enabled = true
check_interval_seconds = 30
max_restart_attempts = 5

[git]
enabled = true
show_upstream = true
timeout_seconds = 10
skip_paths = []

[language]
enabled = true
show_versions = true
cache_ttl_hours = 24
enabled_languages = []
display = "icon"

[ui]
show_icons = true
theme = "default"
directory.max_length = 80
enabled_segments = ["clock", "duration", "directory", "git"]
"#;

    let (exit_code, stdout, stderr) =
        run_gpy_agent_with_config(DISABLED_CONFIG, &["oneshot", "git", "--cwd", "."]);
    assert_eq!(exit_code, 0_i32, "oneshot git failed: {stderr}");

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(
        json.get("branch").is_some(),
        "expected branch information in response, got {json}"
    );
}

#[test]
fn test_theme_export_respects_supervisor_settings() {
    const SUPERVISOR_CONFIG: &str = r#"
[agent]
enabled = true
timeout_seconds = 7
live_updates = false

[agent.supervisor]
enabled = false
check_interval_seconds = 45
max_restart_attempts = 3

[git]
enabled = true
show_upstream = true
timeout_seconds = 10
skip_paths = []

[language]
enabled = true
show_versions = true
cache_ttl_hours = 24
enabled_languages = []
display = "icon"

[ui]
show_icons = true
theme = "default"
directory.max_length = 80
enabled_segments = ["clock", "duration", "directory", "git"]
"#;

    let (exit_code, stdout, stderr) =
        run_gpy_agent_with_config(SUPERVISOR_CONFIG, &["theme", "export", "--shell", "fish"]);
    assert_eq!(exit_code, 0_i32, "theme export failed: {stderr}");
    assert!(
        stdout.contains("set -gx GPY_AGENT_SUPERVISOR_ENABLED \"0\""),
        "expected supervisor enabled export to respect config: {stdout}"
    );
    assert!(
        stdout.contains("set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS \"45\""),
        "expected supervisor interval export to respect config: {stdout}"
    );
    assert!(
        stdout.contains("set -gx GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS \"3\""),
        "expected supervisor max restart export to respect config: {stdout}"
    );
    assert!(
        stdout.contains("set -gx GPY_AGENT_LIVE_UPDATES \"0\""),
        "expected agent live updates export to reflect config: {stdout}"
    );
}

#[test]
fn test_theme_export_respects_ui_preferences() {
    const UI_CONFIG: &str = r#"
[ui]
show_icons = false
theme = "default"
directory.display = "abbreviated"
directory.max_length = 42
enabled_segments = ["directory", "git"]
"#;

    let (exit_code, stdout, stderr) =
        run_gpy_agent_with_config(UI_CONFIG, &["theme", "export", "--shell", "fish"]);
    assert_eq!(exit_code, 0_i32, "theme export failed: {stderr}");

    assert!(
        stdout.contains("set -g __prompt_icons \"ascii\""),
        "expected prompt icons to switch to ascii when show_icons=false: {stdout}"
    );
    assert!(
        stdout.contains("set -gx GPY_UI_DIRECTORY_MAX_LENGTH \"42\""),
        "expected GPY_UI_DIRECTORY_MAX_LENGTH export to honour config: {stdout}"
    );
    assert!(
        stdout.contains("set -gx GPY_UI_DIRECTORY_DISPLAY \"abbreviated\""),
        "expected GPY_UI_DIRECTORY_DISPLAY export to honour config: {stdout}"
    );
    assert!(
        stdout.contains("set -g __enabled_segments directory git"),
        "expected enabled segments export to match config: {stdout}"
    );
}

#[test]
fn test_theme_export_honors_directory_display_for_zsh_and_bash() {
    const UI_CONFIG: &str = r#"
[ui]
show_icons = false
theme = "default"
directory.display = "full"
directory.max_length = 42
enabled_segments = ["directory", "git"]
"#;

    let (exit_code, zsh_stdout, stderr) =
        run_gpy_agent_with_config(UI_CONFIG, &["theme", "export", "--shell", "zsh"]);
    assert_eq!(exit_code, 0_i32, "zsh theme export failed: {stderr}");
    assert!(
        zsh_stdout.contains("export GPY_UI_DIRECTORY_DISPLAY=\"full\"")
            || zsh_stdout.contains("typeset -gx GPY_UI_DIRECTORY_DISPLAY=\"full\"")
            || zsh_stdout.contains("typeset -g -x GPY_UI_DIRECTORY_DISPLAY=\"full\""),
        "expected zsh export to include GPY_UI_DIRECTORY_DISPLAY: {zsh_stdout}"
    );

    let (bash_exit_code, bash_stdout, bash_stderr) =
        run_gpy_agent_with_config(UI_CONFIG, &["theme", "export", "--shell", "bash"]);
    assert_eq!(
        bash_exit_code, 0_i32,
        "bash theme export failed: {bash_stderr}"
    );
    assert!(
        bash_stdout.contains("export GPY_UI_DIRECTORY_DISPLAY=\"full\""),
        "expected bash export to include GPY_UI_DIRECTORY_DISPLAY: {bash_stdout}"
    );
}

#[test]
fn test_cli_oneshot_lang_current_repo() {
    let temp_project = create_test_rust_project().expect("Failed to create test Rust project");
    let temp_path = temp_project.path();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "lang",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed with stderr: {stderr}");

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(json.get("languages").is_some());
}

#[test]
fn test_cli_oneshot_lang_rust_project() {
    // The crate root has a Cargo.toml.
    let (exit_code, stdout, stderr) = run_gpy_agent(&["oneshot", "lang", "--cwd", "."]);

    assert_eq!(exit_code, 0_i32, "Command failed with stderr: {stderr}");

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    let languages = json.get("languages").and_then(|v| v.as_array()).unwrap();
    assert!(
        languages
            .iter()
            .any(|lang| lang.get("name").and_then(|n| n.as_str()) == Some("rust"))
    );
}

#[test]
fn test_cli_invalid_command() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["invalid-command"]);

    assert_ne!(exit_code, 0_i32);
    assert!(
        stderr.contains("error:"),
        "clap usage error expected: {stderr}"
    );
}

#[test]
fn test_cli_oneshot_missing_operation() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["oneshot"]);

    assert_ne!(exit_code, 0_i32);
    // Windows prints `gpy-agent.exe`.
    assert!(
        stderr.contains("Usage: gpy-agent") && stderr.contains("oneshot <COMMAND>"),
        "{stderr}"
    );
}

#[test]
fn test_cli_oneshot_invalid_operation() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["oneshot", "invalid-op"]);

    assert_ne!(exit_code, 0_i32);
    assert!(
        stderr.contains("error:"),
        "clap usage error expected: {stderr}"
    );
}

/// A negative exit code is a value, not a flag, and counts as failure.
#[test]
fn test_cli_oneshot_character_accepts_negative_exit_code() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "character",
        "--exit-code",
        "-1",
        "--format",
        "json",
    ]);

    assert_eq!(exit_code, 0_i32, "oneshot character failed: {stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert_eq!(json.get("success"), Some(&serde_json::json!(false)));
}

#[test]
fn test_cli_oneshot_git_invalid_path() {
    let (exit_code, stdout, _stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        "/nonexistent/path/that/should/not/exist",
    ]);

    // A non-git directory is a JSON error, not a process failure.
    assert_eq!(exit_code, 0_i32);
    assert!(stdout.contains(r#""error":"Not in a git repository""#));
}

/// `git.skip_paths` entries written as `~/...` or through a symlink must
/// suppress `oneshot git` (JSON error, empty ANSI), while an unrelated repo
/// under the same config still reports status.
#[cfg(unix)]
#[test]
fn test_cli_oneshot_git_honors_tilde_and_symlinked_skip_paths() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");
    let root = env.root().to_path_buf();
    let real = root.join("real");
    env.write_config(&format!(
        "[git]\nskip_paths = [\"~/big\", \"{}\"]\n",
        root.join("link").display()
    ))
    .expect("write config");
    std::os::unix::fs::symlink(&real, root.join("link")).expect("symlink");

    let big = root.join("big");
    let linked = real.join("repo");
    let kept = root.join("kept");
    for repo in [&big, &linked, &kept] {
        fs::create_dir_all(repo).expect("mkdir");
        git_helpers::git_init(repo);
    }

    for skipped in [&big, &linked] {
        let cwd = skipped.to_string_lossy();
        let json = env
            .run_gpy_agent(&["oneshot", "git", "--cwd", &cwd, "--format", "json"])
            .expect("run gpy-agent");
        assert!(
            json.stdout.contains("Git segment disabled via config"),
            "{cwd} must be skipped, got: {}",
            json.stdout
        );
        let ansi = env
            .run_gpy_agent(&["oneshot", "git", "--cwd", &cwd, "--format", "ansi"])
            .expect("run gpy-agent");
        assert_eq!(ansi.stdout, "\n", "{cwd} must render an empty segment");
    }

    let cwd = kept.to_string_lossy();
    let json = env
        .run_gpy_agent(&["oneshot", "git", "--cwd", &cwd, "--format", "json"])
        .expect("run gpy-agent");
    assert!(
        json.stdout.contains("\"branch\""),
        "unrelated repo must still report status, got: {}",
        json.stdout
    );
}

/// Outside a repo or with git disabled, prompt formats print only a newline;
/// JSON keeps reporting the error.
#[test]
fn test_cli_oneshot_git_error_renders_empty_prompt_segment() {
    let temp_dir = TempDir::new().expect("temp dir");
    let cwd = temp_dir.path().to_str().expect("utf8 temp path");

    for format in ["ansi", "bash-prompt", "zsh-prompt"] {
        let args = ["oneshot", "git", "--cwd", cwd, "--format", format];
        let outside_repo = run_gpy_agent(&args);
        let git_disabled = run_gpy_agent_with_config("[git]\nenabled = false\n", &args);

        for (case, (exit_code, stdout, stderr)) in [
            ("outside a repo", outside_repo),
            ("with git disabled", git_disabled),
        ] {
            assert_eq!(exit_code, 0_i32, "{format} {case}: {stderr}");
            assert_eq!(stdout, "\n", "{format} {case} must print only a newline");
        }
    }
}

#[test]
fn test_cli_with_complex_paths() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let temp_path = temp_dir.path();

    let complex_dir = temp_path.join("test dir with spaces & symbols");
    fs::create_dir_all(&complex_dir).expect("create complex dir");
    let args = [
        "oneshot",
        "git",
        "--cwd",
        complex_dir.to_str().expect("utf-8 temp path"),
    ];

    let (exit_code, stdout, stderr) = run_gpy_agent(&args);
    assert_eq!(exit_code, 0_i32, "{stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert_eq!(
        json.get("error"),
        Some(&serde_json::json!("Not in a git repository"))
    );

    git_helpers::git_init(&complex_dir);
    let (repo_exit, repo_stdout, repo_stderr) = run_gpy_agent(&args);
    assert_eq!(repo_exit, 0_i32, "{repo_stderr}");
    let repo_json: serde_json::Value = serde_json::from_str(&repo_stdout).expect("Invalid JSON");
    assert!(repo_json.get("error").is_none(), "{repo_stdout}");
    assert!(repo_json.get("branch").is_some(), "{repo_stdout}");
}

#[test]
fn test_cli_status_command() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["status"]);

    // The shared env may or may not have an agent up; exit code must agree.
    assert!(!stdout.is_empty(), "status prints a report: {stderr}");
    assert!(stdout.contains("state: "), "{stdout}");
    let responding = stdout.contains("state: running");
    assert_eq!(
        exit_code,
        if responding { 0_i32 } else { 3_i32 },
        "exit code must reflect liveness. stdout: {stdout} stderr: {stderr}"
    );
}

#[test]
fn test_cli_concurrent_oneshot_commands() {
    use std::thread;

    let handles: Vec<_> = (0_i32..5_i32)
        .map(|_| {
            thread::spawn(|| {
                let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
                let temp_path = temp_repo.path();

                let output = Command::new(env!("CARGO_BIN_EXE_gpy-agent"))
                    .args([
                        "oneshot",
                        "git",
                        "--cwd",
                        temp_path.to_string_lossy().as_ref(),
                    ])
                    .output()
                    .expect("Failed to run command");

                std::mem::drop(temp_repo);
                output
            })
        })
        .collect();

    for handle in handles {
        let output = handle.join().unwrap();
        assert!(output.status.success());
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
        assert!(json.get("branch").is_some(), "no branch in {json}");
    }
}

#[test]
fn test_cli_git_status_json_format() {
    let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
    let temp_path = temp_repo.path();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed: {stderr}");

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("Failed to parse JSON output");

    assert!(json.get("branch").is_some());
    assert!(json.get("ahead").is_some());
    assert!(json.get("behind").is_some());
    assert!(json.get("staged").is_some());
    assert!(json.get("unstaged").is_some());
    assert!(json.get("untracked").is_some());
    assert!(json.get("conflicts").is_some());
    assert!(json.get("state").is_some());
}

#[test]
fn test_theme_export_includes_icon_prompt() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["theme", "export", "--shell", "fish"]);

    assert_eq!(exit_code, 0_i32, "Theme export failed: {stderr}");

    assert!(
        stdout.contains("set -g __icon_prompt"),
        "Theme export should include __icon_prompt variable for fish_prompt.fish compatibility. Got: {stdout}"
    );
    assert!(
        stdout.contains("set -g __prompt_color"),
        "Theme export should include __prompt_color variable. Got: {stdout}"
    );

    assert!(
        stdout.contains("set -g __icon_root_prompt"),
        "Theme export should include __icon_root_prompt variable for root user prompts. Got: {stdout}"
    );
    assert!(
        stdout.contains("set -g __root_prompt_color"),
        "Theme export should include __root_prompt_color variable. Got: {stdout}"
    );

    assert!(
        stdout.contains("\"❯\"") || stdout.contains("\">\""),
        "Theme export should contain a user prompt symbol. Got: {stdout}"
    );
    assert!(
        stdout.contains("\"!❯!\""),
        "Theme export should contain the default root prompt symbol. Got: {stdout}"
    );
}

#[test]
fn test_fish_rendered_delimiter_when_not_last_git() {
    let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
    let temp_path = temp_repo.path();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
        "--format",
        "ansi",
        "--not-last",
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed: {stderr}");
    assert!(!stdout.is_empty(), "Output should not be empty");

    assert_delimiter_usage(&stdout, false, "git");
}

#[test]
fn test_fish_rendered_delimiter_when_is_last_git() {
    let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
    let temp_path = temp_repo.path();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
        "--format",
        "ansi",
        // No --not-last: is_last=true.
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed: {stderr}");
    assert!(!stdout.is_empty(), "Output should not be empty");

    assert_delimiter_usage(&stdout, true, "git");
}

#[test]
fn test_fish_rendered_delimiter_when_not_last_lang() {
    let temp_project = create_test_rust_project().expect("Failed to create test Rust project");
    let temp_path = temp_project.path();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "lang",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
        "--format",
        "ansi",
        "--not-last",
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed: {stderr}");

    if stdout.is_empty() {
        return;
    }

    assert_delimiter_usage(&stdout, false, "language");
}

#[test]
fn test_fish_rendered_delimiter_when_is_last_lang() {
    let temp_project = create_test_rust_project().expect("Failed to create test Rust project");
    let temp_path = temp_project.path();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "lang",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
        "--format",
        "ansi",
        // No --not-last: is_last=true.
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed: {stderr}");

    if stdout.is_empty() {
        return;
    }

    assert_delimiter_usage(&stdout, true, "language");
}

/// A quarantined/SIGKILLed binary used to export nothing, leaving Fish
/// colors unset.
#[test]
fn test_theme_export_produces_non_empty_output() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["theme", "export", "--shell", "fish"]);

    assert_eq!(
        exit_code, 0_i32,
        "theme export command failed with exit code {exit_code}. stderr: {stderr}"
    );

    assert!(
        !stdout.is_empty(),
        "theme export produced empty output - this indicates a silent failure (possibly SIGKILL). stderr: {stderr}"
    );

    assert!(
        stdout.lines().count() > 5,
        "theme export produced only {} lines - expected substantial output with many color variables. Got: {stdout}",
        stdout.lines().count()
    );

    let required_prefixes = [
        "set -g __color_",  // Color variables
        "set -g __icon_",   // Icon variables
        "set -g __prompt_", // Prompt configuration
    ];

    for prefix in &required_prefixes {
        assert!(
            stdout.contains(prefix),
            "theme export output missing expected prefix '{prefix}'. Got: {stdout}"
        );
    }

    assert!(
        !stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .any(|line| !line.starts_with("# GPY Theme:")),
        "theme export produced unexpected stderr output: {stderr}"
    );
}

#[test]
fn test_theme_export_fish_syntax_valid() {
    let (exit_code, stdout, _stderr) = run_gpy_agent(&["theme", "export", "--shell", "fish"]);

    assert_eq!(exit_code, 0_i32);
    assert_ne!(stdout, "");

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        assert!(
            trimmed.starts_with("set -g"),
            "Invalid Fish syntax - line must start with 'set -g' or 'set -gx': {line}"
        );

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let var_name = parts
            .get(2)
            .expect("set command must have form 'set -g|-gx VARNAME VALUE'");
        assert!(
            var_name.starts_with("__") || var_name.starts_with("GPY_"),
            "Variable names should start with '__' or 'GPY_': {line}"
        );
    }
}

#[test]
fn test_theme_export_respects_xdg_config_home() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    let custom_config_dir = temp_path.join("custom-config");
    let gpy_themes_dir = custom_config_dir.join("gpy").join("themes");
    fs::create_dir_all(&gpy_themes_dir).expect("Failed to create themes dir");

    let theme_content = r#"
[ui]
prompt_icon = "🔧"
prompt_color = "magenta"

[segments.clock]
bg_color = "bright_magenta"
text_color = "white"
"#;
    fs::write(gpy_themes_dir.join("default.toml"), theme_content)
        .expect("Failed to write theme file");

    let config_dir = custom_config_dir.join("gpy");
    let config_content = r#"
[ui]
theme = "default"
"#;
    fs::write(config_dir.join("config.toml"), config_content).expect("Failed to write config file");

    let output = Command::new(env!("CARGO_BIN_EXE_gpy-agent"))
        .args(["theme", "export", "--shell", "fish"])
        .env("XDG_CONFIG_HOME", custom_config_dir.to_str().unwrap())
        .output()
        .expect("Failed to run command");

    let exit_code = output.status.code().unwrap_or(-1_i32);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        exit_code, 0_i32,
        "theme export failed with XDG_CONFIG_HOME set. stderr: {stderr}"
    );

    assert!(
        stdout.contains("🔧"),
        "Theme export should contain custom prompt symbol from XDG_CONFIG_HOME theme. Got: {stdout}"
    );

    assert!(
        stdout.contains("magenta")
            || stdout.contains("bright_magenta")
            || stdout.contains("__prompt_color \"magenta\""),
        "Theme export should contain custom prompt color. Got: {stdout}"
    );
}

/// Plain text of a default-theme `oneshot <segment>` render with `extra` flags.
fn default_theme_pill_text(env: &CliTestEnv, args: &[&str], extra: &[&str]) -> String {
    let mut all = vec!["oneshot"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--format", "ansi"]);
    all.extend_from_slice(extra);
    let output = env
        .run_gpy_agent(&all)
        .expect("failed to run gpy-agent oneshot");
    output.assert_success("oneshot pill");
    strip_ansi_codes(output.stdout.trim_end_matches('\n'))
}

/// Every default-theme pill carries exactly one leading space inside the pill:
/// first position has no opening cap, any other position opens with the cap.
#[test]
fn test_cli_oneshot_default_theme_pills_have_one_leading_space() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");
    let open = '\u{e0ba}';
    let close = '\u{e0bc}';
    let gap = ' ';
    let cases: [(&[&str], &str); 3] = [
        (&["exit-status", "--exit-code", "0"], "\u{f058}"),
        (&["exit-status", "--exit-code", "2"], "\u{f057}"),
        // A negative value parses as a value, not a flag, and is a failure.
        (&["exit-status", "--exit-code", "-1"], "\u{f057}"),
    ];
    for (args, content) in cases {
        let first = default_theme_pill_text(&env, args, &["--first", "--not-last"]);
        assert_eq!(
            first,
            format!(" {content}{gap}{close}"),
            "first pill: one leading space, no opening cap ({args:?})"
        );
        let middle = default_theme_pill_text(&env, args, &["--not-last"]);
        assert_eq!(
            middle,
            format!("{open} {content}{gap}{close}"),
            "non-first pill opens with the cap then one space ({args:?})"
        );
    }
}

#[test]
fn test_theme_export_contains_enabled_segments_from_config() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["theme", "export", "--shell", "fish"]);
    assert_eq!(exit_code, 0_i32, "theme export failed: {stderr}");

    let enabled_line = stdout
        .lines()
        .find(|line| line.starts_with("set -g __enabled_segments "))
        .expect("theme export should include __enabled_segments line");

    assert_eq!(
        enabled_line.trim(),
        "set -g __enabled_segments clock duration directory git",
        "theme export should honour ui.enabled_segments from config"
    );
}

/// Every numeric SGR parameter from all `\x1b[...m` sequences, including
/// combined ones like `\x1b[30;42m`.
fn ansi_sgr_params(output: &str) -> Vec<u16> {
    let re = regex::Regex::new(r"\x1b\[([0-9;]*)m").unwrap();
    let mut params = Vec::new();
    for caps in re.captures_iter(output) {
        for part in caps[1].split(';') {
            if let Ok(value) = part.parse::<u16>() {
                params.push(value);
            }
        }
    }
    params
}

fn has_ansi_foreground_colors(output: &str) -> bool {
    ansi_sgr_params(output)
        .iter()
        .any(|&n| (30..=37).contains(&n) || (90..=97).contains(&n))
}

fn has_ansi_background_colors(output: &str) -> bool {
    ansi_sgr_params(output)
        .iter()
        .any(|&n| (40..=47).contains(&n) || (100..=107).contains(&n))
}

#[test]
fn test_fish_rendered_uses_theme_colors_not_default() {
    let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
    let temp_path = temp_repo.path();

    // Dirty repo so output is colored.
    fs::write(temp_path.join("test.txt"), "test").ok();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
        "--format",
        "ansi",
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed: {stderr}");
    assert!(!stdout.is_empty(), "Output should not be empty");

    assert!(
        stdout.contains("\x1b["),
        "ansi output should contain ANSI escape codes. Got: {stdout}"
    );

    // Theme colors, not 39m/49m defaults.
    let uses_foreground_colors = has_ansi_foreground_colors(&stdout);
    assert!(
        uses_foreground_colors,
        "Output should use specific foreground colors (30-37m or 90-97m), not default (39m). Got: {stdout}"
    );

    let uses_background_colors = has_ansi_background_colors(&stdout);
    assert!(
        uses_background_colors,
        "Output should use specific background colors (40-47m or 100-107m), not default (49m). Got: {stdout}"
    );
}

/// With git disabled, directory becomes last and must use `prompt_close`.
#[test]
fn test_theme_export_delimiter_selection_after_git_disabled() {
    const CONFIG_GIT_DISABLED: &str = r#"
[agent]
enabled = true
timeout_seconds = 5
live_updates = true

[git]
enabled = false

[language]
enabled = true

[ui]
show_icons = true
theme = "default"
directory.max_length = 80
enabled_segments = ["clock", "directory", "git"]
"#;

    let (exit_code, stdout, stderr) =
        run_gpy_agent_with_config(CONFIG_GIT_DISABLED, &["theme", "export", "--shell", "fish"]);

    assert_eq!(exit_code, 0_i32, "theme export failed: {stderr}");

    assert!(
        stdout.contains("set -g __enabled_segments clock directory"),
        "Git should be removed from enabled_segments when git.enabled=false, making directory last. Got: {stdout}"
    );

    assert!(
        stdout.contains("set -gx GPY_GIT_ENABLED \"0\""),
        "GPY_GIT_ENABLED should be 0 when git.enabled=false. Got: {stdout}"
    );
}

#[test]
fn test_config_hot_reload_git_toggle_full_scenario() {
    const CONFIG_GIT_ENABLED: &str = r#"
[git]
enabled = true

[ui]
enabled_segments = ["clock", "directory", "git"]
"#;

    const CONFIG_GIT_DISABLED: &str = r#"
[git]
enabled = false

[ui]
enabled_segments = ["clock", "directory", "git"]
"#;

    let (exit_code, stdout_enabled, stderr) =
        run_gpy_agent_with_config(CONFIG_GIT_ENABLED, &["theme", "export", "--shell", "fish"]);
    assert_eq!(
        exit_code, 0_i32,
        "theme export failed with git enabled: {stderr}"
    );
    assert!(
        stdout_enabled.contains("set -g __enabled_segments clock directory git"),
        "Git should be included when enabled. Got: {stdout_enabled}"
    );
    assert!(
        stdout_enabled.contains("set -gx GPY_GIT_ENABLED \"1\""),
        "GPY_GIT_ENABLED should be 1 when enabled. Got: {stdout_enabled}"
    );

    let (disabled_exit_code, stdout_disabled, disabled_stderr) =
        run_gpy_agent_with_config(CONFIG_GIT_DISABLED, &["theme", "export", "--shell", "fish"]);
    assert_eq!(
        disabled_exit_code, 0_i32,
        "theme export failed with git disabled: {disabled_stderr}"
    );
    assert!(
        stdout_disabled.contains("set -g __enabled_segments clock directory"),
        "Git should be removed when disabled. Got: {stdout_disabled}"
    );
    assert!(
        stdout_disabled.contains("set -gx GPY_GIT_ENABLED \"0\""),
        "GPY_GIT_ENABLED should be 0 when disabled. Got: {stdout_disabled}"
    );

    assert!(
        stdout_enabled.contains("git") && !stdout_disabled.contains("clock directory git"),
        "Enabled segments should differ between git enabled and disabled states"
    );
}

#[test]
fn test_invalid_subcommand() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["invalid-command"]);

    assert_ne!(exit_code, 0_i32, "Invalid subcommand should fail");

    let output = format!("{stdout}{stderr}");
    assert!(
        output.to_lowercase().contains("error")
            || output.to_lowercase().contains("usage")
            || output.to_lowercase().contains("invalid")
            || output.to_lowercase().contains("unrecognized"),
        "Should provide error message for invalid subcommand. Got: {output}"
    );
}

#[test]
fn test_oneshot_missing_operation() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["oneshot"]);

    assert_ne!(exit_code, 0_i32, "oneshot without operation should fail");

    let output = format!("{stdout}{stderr}");
    assert!(
        !output.is_empty(),
        "Should provide error message. Got: {output}"
    );
}

#[test]
fn test_oneshot_invalid_operation() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["oneshot", "invalid-op"]);

    assert_ne!(
        exit_code, 0_i32,
        "oneshot with invalid operation should fail"
    );

    let output = format!("{stdout}{stderr}");
    assert!(
        !output.is_empty(),
        "Should provide error message for invalid operation. Got: {output}"
    );
}

#[test]
fn test_oneshot_git_invalid_format() {
    let (exit_code, _stdout, stderr) =
        run_gpy_agent(&["oneshot", "git", "--cwd", ".", "--format", "invalid-format"]);

    assert_ne!(exit_code, 0_i32, "Invalid format should fail");
    assert!(
        !stderr.is_empty(),
        "Should provide error message for invalid format"
    );
}

#[test]
fn test_oneshot_lang_invalid_format() {
    let (exit_code, _stdout, stderr) =
        run_gpy_agent(&["oneshot", "lang", "--cwd", ".", "--format", "bad-format"]);

    assert_ne!(exit_code, 0_i32, "Invalid format should fail");
    assert!(
        !stderr.is_empty(),
        "Should provide error message for invalid format"
    );
}

#[test]
fn test_theme_export_invalid_format() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["theme", "export", "--shell", "xml"]);

    assert_ne!(exit_code, 0_i32, "Invalid theme format should fail");
    assert!(
        !stderr.is_empty(),
        "Should provide error message for invalid format"
    );
}

#[test]
fn test_help_flag() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["--help"]);

    assert_eq!(exit_code, 0_i32, "Help flag should succeed");

    let output = format!("{stdout}{stderr}");
    assert!(
        output.to_lowercase().contains("usage") || output.to_lowercase().contains("help"),
        "Help output should contain usage information. Got: {output}"
    );
}

#[test]
fn test_version_flag() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["--version"]);

    assert_eq!(exit_code, 0_i32, "Version flag should succeed");

    let output = format!("{stdout}{stderr}");
    assert!(
        !output.is_empty(),
        "Version output should not be empty. Got: {output}"
    );
}

#[test]
fn test_oneshot_git_not_last_changes_ansi_but_not_json() {
    let git_repo = create_test_git_repo().expect("create git repo for test");
    let repo_path = git_repo.path().to_string_lossy().into_owned();
    let run = |extra: &[&str], format: &str| {
        let mut args = vec![
            "oneshot",
            "git",
            "--cwd",
            repo_path.as_str(),
            "--format",
            format,
        ];
        args.extend_from_slice(extra);
        let (exit_code, stdout, stderr) = run_gpy_agent(&args);
        assert_eq!(exit_code, 0_i32, "oneshot git {args:?} failed: {stderr}");
        stdout
    };

    // The raw JSON payload carries no prompt-position styling.
    assert_eq!(run(&[], "json"), run(&["--not-last"], "json"));

    // The rendered segment closes differently when more segments follow it.
    let last = run(&[], "ansi");
    let not_last = run(&["--not-last"], "ansi");
    assert!(!last.is_empty(), "ansi segment for a git repo is empty");
    assert_ne!(last, not_last, "--not-last must change the closing cap");
}

#[test]
fn test_empty_arguments() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&[]);

    assert_ne!(exit_code, 0_i32, "No arguments should fail or show help");

    let output = format!("{stdout}{stderr}");
    assert!(!output.is_empty(), "Should provide some output");
}

/// Run `gpy-agent <args>` with `dir` as its working directory, so a relative
/// `--cwd .` resolves against it.
fn run_gpy_agent_in(dir: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let env = CliTestEnv::new().expect("failed to create CLI test env");
    let output = env
        .gpy_agent_command()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn gpy-agent");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn test_oneshot_git_relative_cwd_matches_absolute_cwd() {
    let git_repo = create_test_git_repo().expect("create git repo for test");
    let repo_path = git_repo.path().to_string_lossy().into_owned();

    let (rel_code, rel_stdout, rel_stderr) = run_gpy_agent_in(
        git_repo.path(),
        &["oneshot", "git", "--cwd", ".", "--format", "json"],
    );
    assert_eq!(rel_code, 0_i32, "relative --cwd failed: {rel_stderr}");
    let (abs_code, abs_stdout, abs_stderr) =
        run_gpy_agent(&["oneshot", "git", "--cwd", &repo_path, "--format", "json"]);
    assert_eq!(abs_code, 0_i32, "absolute --cwd failed: {abs_stderr}");

    let rel: serde_json::Value = serde_json::from_str(&rel_stdout).expect("relative JSON");
    let abs: serde_json::Value = serde_json::from_str(&abs_stdout).expect("absolute JSON");
    assert!(rel.get("branch").is_some(), "no repo status for `.`: {rel}");
    assert_eq!(rel, abs, "`--cwd .` and the absolute path must agree");
}

#[test]
fn test_oneshot_lang_relative_cwd_matches_absolute_cwd() {
    let rust_project = create_test_rust_project().expect("create rust project for language test");
    let rust_path = rust_project.path().to_string_lossy().into_owned();

    let (rel_code, rel_stdout, rel_stderr) = run_gpy_agent_in(
        rust_project.path(),
        &["oneshot", "lang", "--cwd", ".", "--format", "json"],
    );
    assert_eq!(rel_code, 0_i32, "relative --cwd failed: {rel_stderr}");
    let (abs_code, abs_stdout, abs_stderr) =
        run_gpy_agent(&["oneshot", "lang", "--cwd", &rust_path, "--format", "json"]);
    assert_eq!(abs_code, 0_i32, "absolute --cwd failed: {abs_stderr}");

    let rel: serde_json::Value = serde_json::from_str(&rel_stdout).expect("relative JSON");
    let abs: serde_json::Value = serde_json::from_str(&abs_stdout).expect("absolute JSON");
    let detected: Vec<&str> = rel
        .get("languages")
        .and_then(serde_json::Value::as_array)
        .expect("languages array")
        .iter()
        .filter_map(|lang| lang["name"].as_str())
        .collect();
    assert_eq!(detected, ["rust"], "Cargo project must be detected: {rel}");
    assert_eq!(rel, abs, "`--cwd .` and the absolute path must agree");
}

/// Words of the `gpy <words>` command at the start of `text`: the leading run
/// of `[a-z][a-z0-9-]*` tokens after `gpy `. Empty for `gpy <TAB>`,
/// `gpy <command> --help` and `gpy-agent ...` (no space after `gpy`).
fn leading_command_words(text: &str) -> Vec<&str> {
    let Some(rest) = text.strip_prefix("gpy ") else {
        return Vec::new();
    };
    rest.split_whitespace()
        .take_while(|word| {
            word.starts_with(|c: char| c.is_ascii_lowercase())
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
        .collect()
}

/// Unique, sorted `gpy <words>` command paths documented in `cli-reference.md`.
///
/// Extraction rule: every inline code span, and every line of a fenced block
/// (leading whitespace trimmed), that starts with `gpy ` (with the space, so
/// `gpy-agent` is excluded) yields its leading run of `[a-z][a-z0-9-]*` words.
/// An empty run (`gpy <TAB>`, `gpy <command> --help`) is skipped.
fn documented_command_paths(doc: &str) -> Vec<String> {
    let mut paths = std::collections::BTreeSet::new();
    let mut in_fence = false;
    for line in doc.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        let candidates: Vec<&str> = if in_fence {
            vec![line.trim_start()]
        } else {
            line.split('`').skip(1).step_by(2).collect()
        };
        for candidate in candidates {
            let words = leading_command_words(candidate);
            if !words.is_empty() {
                paths.insert(words.join(" "));
            }
        }
    }
    paths.into_iter().collect()
}

/// Every `gpy <words>` command that `docs/user/cli-reference.md` mentions must
/// exist: `gpy <words> --help` must not print clap's `unrecognized subcommand`
/// marker. Argument tails (`config show nosuch`) never produce that marker,
/// so documented invalid examples and placeholders do not trip the guard. See
/// [`documented_command_paths`] for the extraction rule. The reverse direction
/// (CLI commands missing from the docs) and `gpy-agent` are out of scope.
#[test]
fn documented_cli_commands_exist() {
    const MARKER: &str = "unrecognized subcommand";
    let env = CliTestEnv::new().expect("failed to create CLI test env");

    let canary = env
        .run_gpy(&["definitely-not-a-command", "--help"])
        .expect("spawn gpy");
    assert!(
        canary.stderr.contains(MARKER),
        "clap no longer prints `{MARKER}` for an unknown subcommand; update the guard: {canary:?}"
    );

    let paths = documented_command_paths(include_str!("../../docs/user/cli-reference.md"));
    assert!(
        !paths.is_empty(),
        "no commands extracted from cli-reference.md"
    );
    for anchor in ["config show", "plugin list", "theme use"] {
        assert!(
            paths.iter().any(|path| path == anchor),
            "extraction lost the anchor `{anchor}`; got {paths:?}"
        );
    }

    let missing: Vec<&str> = paths
        .iter()
        .filter(|path| {
            let mut args: Vec<&str> = path.split(' ').collect();
            args.push("--help");
            env.run_gpy(&args)
                .expect("spawn gpy")
                .stderr
                .contains(MARKER)
        })
        .map(String::as_str)
        .collect();
    assert!(
        missing.is_empty(),
        "cli-reference.md documents commands the CLI does not have: {missing:?}"
    );
}

/// Install a script plugin `demo-tools` with segment `demo` under the env's
/// user plugin root.
fn install_script_plugin(env: &CliTestEnv) -> std::path::PathBuf {
    let root = env.plugins_dir().join("demo-tools");
    fs::create_dir_all(root.join("segments")).expect("segments dir");
    fs::write(
        root.join("plugin.toml"),
        "id = \"demo-tools\"\nname = \"Demo\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
         entry_type = \"script\"\nprovided_segments = [\"demo\"]\n",
    )
    .expect("plugin manifest");
    fs::write(root.join("demo.sh"), "printf 'demo\\n'\n").expect("script");
    fs::write(
        root.join("segments/demo.toml"),
        "command = \"demo.sh\"\ninputs = [\"pwd\"]\ntimeout_ms = 300\n\
         [settings.style]\ntype = \"string\"\ndefault = \"bold\"\nchoices = [\"bold\", \"dim\"]\n",
    )
    .expect("segment manifest");
    root
}

/// `gpy plugin validate` reports a script segment; `gpy config get/set/list`
/// reach its overrides as `plugin.<segment>.<key>` and validate them.
#[test]
fn script_plugin_validate_and_config_keys() {
    let env = CliTestEnv::new().expect("CLI test env");
    let root = install_script_plugin(&env);

    let validate = env
        .run_gpy(&["plugin", "validate", root.to_str().expect("utf8")])
        .expect("spawn gpy");
    validate.assert_success("gpy plugin validate");
    for needle in [
        "entry: script",
        "segment demo:",
        "command: demo.sh",
        "cache: folder scope, ttl_secs 5, timeout_ms 300",
    ] {
        assert!(
            validate.stdout.contains(needle),
            "{needle:?} missing:\n{}",
            validate.stdout
        );
    }

    let get = env
        .run_gpy(&["config", "get", "plugin.demo.ttl_secs"])
        .expect("spawn gpy");
    get.assert_success("gpy config get default");
    assert_eq!(get.stdout.trim(), "5");

    env.run_gpy(&["config", "set", "plugin.demo.ttl_secs", "30"])
        .expect("spawn gpy")
        .assert_success("gpy config set ttl_secs");
    env.run_gpy(&["config", "set", "plugin.demo.style", "dim"])
        .expect("spawn gpy")
        .assert_success("gpy config set style");
    let saved = fs::read_to_string(env.config_path()).expect("config written");
    assert!(saved.contains("[plugin.demo]"), "{saved}");
    let style = env
        .run_gpy(&["config", "get", "plugin.demo.style"])
        .expect("spawn gpy");
    assert_eq!(style.stdout.trim(), "dim");

    for (key, value, needle) in [
        ("plugin.demo.style", "loud", "plugin.demo.style"),
        ("plugin.demo.timeout_ms", "400", "may only lower it"),
        ("plugin.nope.ttl_secs", "1", "no installed script plugin"),
    ] {
        let rejected = env
            .run_gpy(&["config", "set", key, value])
            .expect("spawn gpy");
        assert_ne!(rejected.exit_code, 0_i32, "{key}={value} must fail");
        let output = format!("{}{}", rejected.stdout, rejected.stderr);
        assert!(output.contains(needle), "{needle:?} missing:\n{output}");
    }
    assert_eq!(
        fs::read_to_string(env.config_path()).expect("config"),
        saved
    );

    let list = env
        .run_gpy(&["config", "list", "plugin."])
        .expect("spawn gpy");
    list.assert_success("gpy config list plugin.");
    for key in [
        "plugin.demo.style",
        "plugin.demo.timeout_ms",
        "plugin.demo.ttl_secs",
    ] {
        assert!(
            list.stdout.contains(&format!("  {key} [read-write]")),
            "{key}:\n{}",
            list.stdout
        );
    }
}
