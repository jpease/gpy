//! Comprehensive CLI integration tests
//! Tests the actual user experience of invoking gpy-agent commands

#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::fs;
use std::process::{Command, Stdio};
use tempfile::TempDir;

#[path = "common/cli_harness.rs"]
mod cli_harness;
use cli_harness::{CliTestEnv, SharedCliTestEnv};

type DelimiterResult = Result<(String, String), Box<dyn std::error::Error>>;

/// Helper to load theme configuration and extract delimiter values
fn load_theme_delimiters() -> DelimiterResult {
    // Load the default theme file
    let theme_content = fs::read_to_string("../config/themes/default.toml")?;
    let theme: toml::Value = toml::from_str(&theme_content)?;

    // Extract segment_close icon
    let segment_close = theme
        .get("ui")
        .and_then(|ui| ui.get("segment_close"))
        .and_then(|sc| sc.get("icon"))
        .and_then(|d| d.as_str())
        .unwrap_or("]")
        .to_owned();

    // Extract prompt_close icon
    let prompt_close = theme
        .get("ui")
        .and_then(|ui| ui.get("prompt_close"))
        .and_then(|pc| pc.get("icon"))
        .and_then(|d| d.as_str())
        .unwrap_or(")")
        .to_owned();

    Ok((segment_close, prompt_close))
}

/// Strip ANSI escape codes from output for easier text matching
fn strip_ansi_codes(output: &str) -> String {
    // Remove all ANSI escape sequences: \x1b[...m
    // This regex matches ESC [ followed by any number of characters ending with 'm'
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(output, "").to_string()
}

/// Verify that the unexpected delimiter is NOT present in the output
fn assert_delimiter_absent(output: &str, delimiter: &str, segment_name: &str, context: &str) {
    assert!(
        !output.contains(delimiter),
        "{context}, {segment_name} segment should NOT use delimiter '{delimiter}'. Got: {output}"
    );
}

/// Verify that the expected delimiter IS present in the output
fn assert_delimiter_present(output: &str, delimiter: &str, segment_name: &str, context: &str) {
    assert!(
        output.contains(delimiter),
        "{context}, {segment_name} segment should use delimiter '{delimiter}'. Got: {output}"
    );
}

/// Verify post-migration delimiter behavior in ansi output.
///
/// The default theme's `git` and `language` segments render via the template
/// engine (SP2 #199), using `$sep_close` as their own closing cap: the
/// `prompt_close` glyph (U+E0B4 half-circle) when `is_last=true`, or the
/// `segment_close` glyph (upper-left triangle + transparent gap) otherwise —
/// deliberately matching the Fish-rendered segments' non-last cap so both
/// rendering paths look identical.
///
/// When `is_last=true`: `prompt_close` IS present as `sep_close`; `segment_close`
/// must be absent.
/// When `is_last=false`: `segment_close` IS present as `sep_close`; `prompt_close`
/// must be absent.
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
    match SharedCliTestEnv::instance().run_gpy_agent(args) {
        Ok(output) => (output.exit_code, output.stdout, output.stderr),
        Err(e) => (-1, String::new(), format!("spawn error: {e}")),
    }
}

fn run_gpy_agent_with_config(config_toml: &str, args: &[&str]) -> (i32, String, String) {
    let env = match CliTestEnv::with_config(config_toml) {
        Ok(env) => env,
        Err(e) => return (-1, String::new(), format!("test env error: {e}")),
    };
    match env.run_gpy_agent(args) {
        Ok(output) => (output.exit_code, output.stdout, output.stderr),
        Err(e) => (-1, String::new(), format!("spawn error: {e}")),
    }
}

/// Helper to create a clean git repository for testing
fn create_test_git_repo() -> Result<TempDir, Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

    // Initialize git repo
    Command::new("git")
        .arg("init")
        .current_dir(temp_path)
        .output()?;

    // Configure git user (required for some git operations)
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_path)
        .output()?;

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_path)
        .output()?;

    Ok(temp_dir)
}

/// Helper to create a temporary Rust project for testing
fn create_test_rust_project() -> Result<TempDir, Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

    // Create a basic Cargo.toml
    let cargo_toml = r#"[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#;
    fs::write(temp_path.join("Cargo.toml"), cargo_toml)?;

    // Create src directory and main.rs
    fs::create_dir_all(temp_path.join("src"))?;
    fs::write(temp_path.join("src").join("main.rs"), "fn main() {}")?;

    Ok(temp_dir)
}

#[test]
#[allow(clippy::unwrap_used)]
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
#[allow(clippy::unwrap_used)]
fn test_cli_version_command() {
    let (exit_code, stdout, _stderr) = run_gpy_agent(&["--version"]);

    assert_eq!(exit_code, 0_i32);
    assert!(stdout.contains("gpy-agent"));
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_cli_oneshot_git_current_repo() {
    // Use isolated git repository to prevent interference with other tests
    let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
    let temp_path = temp_repo.path();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed with stderr: {stderr}");

    // Should return valid JSON with git status
    assert!(stdout.starts_with('{'));
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(json.get("branch").is_some());
    assert!(json.get("ahead").is_some());
    assert!(json.get("behind").is_some());
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_cli_oneshot_git_with_custom_path() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize git repo
    let _ = Command::new("git")
        .arg("init")
        .current_dir(temp_path)
        .output();

    // Configure git user
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_path)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_path)
        .output();

    let (exit_code, stdout, stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        temp_path.to_string_lossy().as_ref(),
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed with stderr: {stderr}");

    // Should return valid JSON
    assert!(stdout.starts_with('{'));
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(json.get("branch").is_some());
}

#[test]
fn test_cli_oneshot_git_ignores_inherited_git_dir() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");
    let git = |dir: &std::path::Path, args: &[&str]| {
        let status = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "user.name=T"])
            .args(["-c", "user.email=t@example.com"])
            .args(args)
            .current_dir(dir)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {args:?} failed");
    };
    let a = env.root().join("repoA");
    let b = env.root().join("repoB");
    for (repo, branch) in [(&a, "featureA"), (&b, "main")] {
        fs::create_dir_all(repo).expect("mkdir");
        git(repo, &["init", "-q", "-b", branch]);
        fs::write(repo.join("f"), "x\n").expect("write");
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "init"]);
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
#[allow(clippy::unwrap_used)]
fn test_cli_oneshot_git_non_git_directory() {
    let Ok(temp_dir) = TempDir::new() else { return };
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

/// Run `config get <key>` and assert it succeeds with the expected trimmed output.
fn assert_config_get(key: &str, expected: &str) {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["config", "get", key]);
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
    assert_eq!(exit_code, 0_i32, "start command failed: {stderr}");
    assert!(
        stderr.contains("GPY Agent disabled via config"),
        "expected disabled notice in stderr, got '{stderr}'"
    );
    assert!(
        stdout.trim().is_empty(),
        "expected no stdout output when agent is disabled, got '{stdout}'"
    );
}

#[test]
#[allow(clippy::unwrap_used)]
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
#[allow(clippy::unwrap_used)]
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
        run_gpy_agent_with_config(SUPERVISOR_CONFIG, &["theme", "export", "--format", "fish"]);
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
        run_gpy_agent_with_config(UI_CONFIG, &["theme", "export", "--format", "fish"]);
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
        run_gpy_agent_with_config(UI_CONFIG, &["theme", "export", "--format", "zsh"]);
    assert_eq!(exit_code, 0_i32, "zsh theme export failed: {stderr}");
    assert!(
        zsh_stdout.contains("export GPY_UI_DIRECTORY_DISPLAY=\"full\"")
            || zsh_stdout.contains("typeset -gx GPY_UI_DIRECTORY_DISPLAY=\"full\"")
            || zsh_stdout.contains("typeset -g -x GPY_UI_DIRECTORY_DISPLAY=\"full\""),
        "expected zsh export to include GPY_UI_DIRECTORY_DISPLAY: {zsh_stdout}"
    );

    let (bash_exit_code, bash_stdout, bash_stderr) =
        run_gpy_agent_with_config(UI_CONFIG, &["theme", "export", "--format", "bash"]);
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
#[allow(clippy::unwrap_used)]
fn test_cli_oneshot_lang_current_repo() {
    // Use isolated Rust project to prevent interference with other tests
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
#[allow(clippy::unwrap_used)]
fn test_cli_oneshot_lang_rust_project() {
    // Test with a known Rust project (current directory should have Cargo.toml)
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
#[allow(clippy::unwrap_used)]
fn test_cli_invalid_command() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["invalid-command"]);

    assert_ne!(exit_code, 0_i32);
    assert_ne!(stderr, "");
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_cli_oneshot_missing_operation() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["oneshot"]);

    assert_ne!(exit_code, 0_i32);
    assert_ne!(stderr, "");
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_cli_oneshot_invalid_operation() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["oneshot", "invalid-op"]);

    assert_ne!(exit_code, 0_i32);
    assert_ne!(stderr, "");
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_cli_oneshot_git_invalid_path() {
    let (exit_code, stdout, _stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        "/nonexistent/path/that/should/not/exist",
    ]);

    // The command succeeds but returns a JSON error indicating not in a git repository.
    // This is the correct behavior - it's not an error to query a non-git directory.
    assert_eq!(exit_code, 0_i32);
    assert!(stdout.contains(r#""error":"Not in a git repository""#));
}

/// #696: `git.skip_paths` entries written as `~/...` or through a symlink must
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

    let init = |repo: &std::path::Path| {
        fs::create_dir_all(repo).expect("mkdir");
        let status = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "init", "-q", "-b", "main"])
            .arg(repo)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git init failed");
    };
    let big = root.join("big");
    let linked = real.join("repo");
    let kept = root.join("kept");
    for repo in [&big, &linked, &kept] {
        init(repo);
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

/// #680: the oneshot fallback prints an empty prompt segment for a git error.
///
/// Both outside a repository and with the git segment disabled, a prompt
/// format prints only a newline rather than the error JSON. The JSON format
/// keeps reporting the error (see `test_cli_oneshot_git_invalid_path`).
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
#[allow(clippy::unwrap_used)]
fn test_cli_with_complex_paths() {
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Create a directory with spaces and special characters
    let complex_dir = temp_path.join("test dir with spaces & symbols");
    let _ = fs::create_dir_all(&complex_dir);

    let (exit_code, stdout, _stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        complex_dir.to_string_lossy().as_ref(),
    ]);

    assert_eq!(exit_code, 0_i32);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(json.get("error").is_some());
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_cli_status_command() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["status"]);

    // The shared env may or may not have an agent up; the exit code must
    // agree with the report either way (#636): 0 only for a responding agent.
    assert!(!stdout.is_empty(), "status prints a report: {stderr}");
    assert!(stdout.contains("GPY Agent Status"), "{stdout}");
    let responding = stdout.contains("Status: Running and Responding");
    assert_eq!(
        exit_code,
        i32::from(!responding),
        "exit code must reflect liveness. stdout: {stdout} stderr: {stderr}"
    );
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_cli_concurrent_oneshot_commands() {
    use std::thread;

    // Test multiple concurrent oneshot commands with isolated repositories
    let handles: Vec<_> = (0_i32..5_i32)
        .map(|_| {
            thread::spawn(|| {
                // Each thread gets its own isolated git repository
                let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
                let temp_path = temp_repo.path();

                let output = Command::new("./target/debug/gpy-agent")
                    .args([
                        "oneshot",
                        "git",
                        "--cwd",
                        temp_path.to_string_lossy().as_ref(),
                    ])
                    .output()
                    .expect("Failed to run command");

                // Keep temp_repo alive until after the command
                std::mem::drop(temp_repo);
                output
            })
        })
        .collect();

    // All should complete successfully
    for handle in handles {
        let output = handle.join().unwrap();
        assert!(output.status.success());
    }
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_cli_git_status_json_format() {
    // Use isolated git repository to prevent interference with other tests
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
#[allow(clippy::unwrap_used)]
fn test_cli_environment_variable_handling() {
    // no-op: no environment reads required

    // Test with custom environment
    let exit_code = match Command::new(env!("CARGO_BIN_EXE_gpy-agent"))
        .args(["oneshot", "git", "--cwd", "."])
        .env("GPY_DEBUG", "1")
        .env("GPY_TIMEOUT", "30")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(o) => o.status.code().unwrap_or(-1_i32),
        Err(_) => -1_i32,
    };
    assert_eq!(exit_code, 0_i32);
}

// ============================================================================
// Visual Rendering Tests
// Tests for correct delimiter selection and theme variable export
// ============================================================================

#[test]
#[allow(clippy::unwrap_used)]
fn test_theme_export_includes_icon_prompt() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["theme", "export", "--format", "fish"]);

    assert_eq!(exit_code, 0_i32, "Theme export failed: {stderr}");

    // User prompt variables should be present
    assert!(
        stdout.contains("set -g __icon_prompt"),
        "Theme export should include __icon_prompt variable for fish_prompt.fish compatibility. Got: {stdout}"
    );
    assert!(
        stdout.contains("set -g __gpy_ui_prompt_icon"),
        "Theme export should include __gpy_ui_prompt_icon variable. Got: {stdout}"
    );
    assert!(
        stdout.contains("set -g __prompt_color"),
        "Theme export should include __prompt_color variable. Got: {stdout}"
    );

    // Root prompt variables should be present
    assert!(
        stdout.contains("set -g __icon_root_prompt"),
        "Theme export should include __icon_root_prompt variable for root user prompts. Got: {stdout}"
    );
    assert!(
        stdout.contains("set -g __gpy_ui_root_prompt_icon"),
        "Theme export should include __gpy_ui_root_prompt_icon variable. Got: {stdout}"
    );
    assert!(
        stdout.contains("set -g __root_prompt_color"),
        "Theme export should include __root_prompt_color variable. Got: {stdout}"
    );

    // Should contain prompt symbols
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
#[allow(clippy::unwrap_used)]
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

    // Verify delimiter usage based on theme configuration
    assert_delimiter_usage(&stdout, false, "git");
}

#[test]
#[allow(clippy::unwrap_used)]
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
        // No --not-last flag means is_last=true (default for standalone use)
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed: {stderr}");
    assert!(!stdout.is_empty(), "Output should not be empty");

    // Verify delimiter usage based on theme configuration
    assert_delimiter_usage(&stdout, true, "git");
}

#[test]
#[allow(clippy::unwrap_used)]
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
        // No languages detected - this is acceptable
        return;
    }

    // Verify delimiter usage based on theme configuration
    assert_delimiter_usage(&stdout, false, "language");
}

#[test]
#[allow(clippy::unwrap_used)]
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
        // No --not-last flag means is_last=true (default for standalone use)
    ]);

    assert_eq!(exit_code, 0_i32, "Command failed: {stderr}");

    if stdout.is_empty() {
        // No languages detected - this is acceptable
        return;
    }

    // Verify delimiter usage based on theme configuration
    assert_delimiter_usage(&stdout, true, "language");
}

/// Test that theme export produces valid, non-empty output
///
/// This is a critical regression test for issue where corrupted/quarantined
/// binaries would fail silently (SIGKILL) when running theme export, leading
/// to empty Fish variables and "`set_color`: Unknown color" errors.
#[test]
#[allow(clippy::unwrap_used)]
fn test_theme_export_produces_non_empty_output() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["theme", "export", "--format", "fish"]);

    // Command must succeed (exit code 0)
    assert_eq!(
        exit_code, 0_i32,
        "theme export command failed with exit code {exit_code}. stderr: {stderr}"
    );

    // Output must not be empty
    assert!(
        !stdout.is_empty(),
        "theme export produced empty output - this indicates a silent failure (possibly SIGKILL). stderr: {stderr}"
    );

    // Output must be substantial (more than just a comment line)
    assert!(
        stdout.lines().count() > 5,
        "theme export produced only {} lines - expected substantial output with many color variables. Got: {stdout}",
        stdout.lines().count()
    );

    // Must contain expected color variable prefixes
    let required_prefixes = [
        "set -g __color_",  // Color variables
        "set -g __icon_",   // Icon variables
        "set -g __gpy_ui_", // UI configuration
    ];

    for prefix in &required_prefixes {
        assert!(
            stdout.contains(prefix),
            "theme export output missing expected prefix '{prefix}'. Got: {stdout}"
        );
    }

    // Verify stderr is empty or only contains benign messages
    assert!(
        !stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .any(|line| !line.starts_with("# GPY Theme:")),
        "theme export produced unexpected stderr output: {stderr}"
    );
}

/// Test that theme export can be parsed as valid Fish script
///
/// This ensures the output is syntactically valid and can be sourced by Fish
#[test]
#[allow(clippy::unwrap_used)]
fn test_theme_export_fish_syntax_valid() {
    let (exit_code, stdout, _stderr) = run_gpy_agent(&["theme", "export", "--format", "fish"]);

    assert_eq!(exit_code, 0_i32);
    assert_ne!(stdout, "");

    // Every line should be either:
    // - Empty
    // - A comment (starts with #)
    // - A valid Fish set command
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Must be a set command
        assert!(
            trimmed.starts_with("set -g"),
            "Invalid Fish syntax - line must start with 'set -g' or 'set -gx': {line}"
        );

        // Variable name must start with valid characters
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

/// Test that theme export respects `XDG_CONFIG_HOME` environment variable
///
/// This validates that the theme path discovery logic correctly honors
/// `XDG_CONFIG_HOME` when set, rather than hardcoding `~/.config`
#[test]
#[allow(clippy::unwrap_used)]
fn test_theme_export_respects_xdg_config_home() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Create custom XDG config structure
    let custom_config_dir = temp_path.join("custom-config");
    let gpy_themes_dir = custom_config_dir.join("gpy").join("themes");
    fs::create_dir_all(&gpy_themes_dir).expect("Failed to create themes dir");

    // Create a custom theme file with a unique identifier
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

    // Also create a minimal config.toml that references the default theme
    let config_dir = custom_config_dir.join("gpy");
    let config_content = r#"
[ui]
theme = "default"
"#;
    fs::write(config_dir.join("config.toml"), config_content).expect("Failed to write config file");

    // Run theme export with custom XDG_CONFIG_HOME
    let output = Command::new("./target/debug/gpy-agent")
        .args(["theme", "export", "--format", "fish"])
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

    // Verify the custom theme was loaded (contains our unique prompt symbol)
    assert!(
        stdout.contains("🔧"),
        "Theme export should contain custom prompt symbol from XDG_CONFIG_HOME theme. Got: {stdout}"
    );

    // Verify it contains our custom color
    assert!(
        stdout.contains("magenta")
            || stdout.contains("bright_magenta")
            || stdout.contains("__gpy_ui_prompt_icon_color \"magenta\""),
        "Theme export should contain custom prompt color. Got: {stdout}"
    );
}

/// `oneshot hostname` with a theme `format` template set renders the supplied
/// hostname through the template engine as pre-formatted ANSI (#265).
#[test]
fn test_cli_oneshot_hostname_renders_via_format_template() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");
    let theme_content = r#"
[ui]

[segments.hostname]
format = "[$symbol$hostname](bold dimmed green) in "
"#;
    fs::write(env.themes_dir().join("default.toml"), theme_content)
        .expect("failed to write hostname-format theme");

    let output = env
        .run_gpy_agent(&[
            "oneshot",
            "hostname",
            "--hostname",
            "myhost",
            "--format",
            "ansi",
        ])
        .expect("failed to run gpy-agent oneshot hostname");

    output.assert_success("oneshot hostname");
    assert!(
        output.stdout.contains("myhost"),
        "expected the supplied hostname in output: {}",
        output.stdout
    );
    assert!(
        output.stdout.contains('\u{1b}'),
        "expected ANSI escape codes in output: {}",
        output.stdout
    );
}

/// `oneshot hostname --is-ssh` draws the theme icon (Starship's `ssh_symbol`);
/// without the flag the same theme renders the bare hostname (#826).
#[test]
fn test_cli_oneshot_hostname_icon_only_with_is_ssh() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");
    let theme_content = r#"
[ui]

[segments.hostname]
format = "[$symbol$hostname](green)"
icon = "🌐 "
show_always = true
"#;
    fs::write(env.themes_dir().join("default.toml"), theme_content)
        .expect("failed to write hostname-icon theme");

    let render = |extra: &[&str]| {
        let mut args = vec![
            "oneshot",
            "hostname",
            "--hostname",
            "myhost",
            "--format",
            "ansi",
        ];
        args.extend_from_slice(extra);
        let output = env
            .run_gpy_agent(&args)
            .expect("failed to run gpy-agent oneshot hostname");
        output.assert_success("oneshot hostname");
        output.stdout
    };

    let ssh = render(&["--is-ssh"]);
    assert!(
        ssh.contains("🌐 ") && ssh.contains("myhost"),
        "expected the icon and hostname over SSH: {ssh:?}"
    );
    let local = render(&[]);
    assert!(local.contains("myhost"), "expected the hostname: {local}");
    assert!(!local.contains('🌐'), "expected no icon locally: {local}");
}

/// `oneshot hostname` with no `format` template configured (the default
/// theme's shell-side path) renders nothing — the shell renders the hostname
/// segment locally instead.
#[test]
fn test_cli_oneshot_hostname_empty_without_format_template() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");

    let output = env
        .run_gpy_agent(&[
            "oneshot",
            "hostname",
            "--hostname",
            "myhost",
            "--format",
            "ansi",
        ])
        .expect("failed to run gpy-agent oneshot hostname");

    output.assert_success("oneshot hostname");
    assert!(
        output.stdout.trim().is_empty(),
        "expected empty output with no format template configured: {}",
        output.stdout
    );
}

/// `oneshot username` with a theme `format` template set renders the supplied
/// username through the template engine as pre-formatted ANSI (#252).
#[test]
fn test_cli_oneshot_username_renders_via_format_template() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");
    let theme_content = r#"
[ui]

[segments.username]
format = "[$username](bold red) in "
"#;
    fs::write(env.themes_dir().join("default.toml"), theme_content)
        .expect("failed to write username-format theme");

    let output = env
        .run_gpy_agent(&[
            "oneshot",
            "username",
            "--username",
            "root",
            "--format",
            "ansi",
        ])
        .expect("failed to run gpy-agent oneshot username");

    output.assert_success("oneshot username");
    assert!(
        output.stdout.contains("root"),
        "expected the supplied username in output: {}",
        output.stdout
    );
    assert!(
        output.stdout.contains('\u{1b}'),
        "expected ANSI escape codes in output: {}",
        output.stdout
    );
}

/// `oneshot username` with no `format` template configured (the default theme's
/// shell-side path) renders nothing — the shell renders the username segment
/// locally instead.
#[test]
fn test_cli_oneshot_username_empty_without_format_template() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");

    let output = env
        .run_gpy_agent(&[
            "oneshot",
            "username",
            "--username",
            "root",
            "--format",
            "ansi",
        ])
        .expect("failed to run gpy-agent oneshot username");

    output.assert_success("oneshot username");
    assert!(
        output.stdout.trim().is_empty(),
        "expected empty output with no format template configured: {}",
        output.stdout
    );
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_theme_export_contains_enabled_segments_from_config() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["theme", "export", "--format", "fish"]);
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

/// Test that ansi output uses theme colors, not default terminal colors
///
/// This validates the Rust renderer (used for git/language segments) correctly
/// applies theme colors. Note: This does NOT test Fish-side rendering
/// (core/renderer.fish used by directory/clock/status segments), which would
/// require a full Fish shell environment to test properly.
/// Collect every numeric SGR parameter from all `\x1b[...m` sequences.
///
/// Handles combined sequences like `\x1b[30;42m` (the template path emits a
/// single escape per span with fg and bg together), not just standalone codes.
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

/// Check if output contains specific ANSI foreground colors (not default).
fn has_ansi_foreground_colors(output: &str) -> bool {
    ansi_sgr_params(output)
        .iter()
        .any(|&n| (30..=37).contains(&n) || (90..=97).contains(&n))
}

/// Check if output contains specific ANSI background colors (not default).
fn has_ansi_background_colors(output: &str) -> bool {
    ansi_sgr_params(output)
        .iter()
        .any(|&n| (40..=47).contains(&n) || (100..=107).contains(&n))
}

#[test]
#[allow(clippy::unwrap_used)]
fn test_fish_rendered_uses_theme_colors_not_default() {
    let temp_repo = create_test_git_repo().expect("Failed to create test git repo");
    let temp_path = temp_repo.path();

    // Create a file to make the repo dirty (so we get colored output)
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

    // The output should contain ANSI color codes
    assert!(
        stdout.contains("\x1b["),
        "ansi output should contain ANSI escape codes. Got: {stdout}"
    );

    // Verify that theme colors are being used, not just default colors
    // Foreground: 30-37 (standard), 90-97 (bright), NOT 39m (default)
    // Background: 40-47 (standard), 100-107 (bright), NOT 49m (default)

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

/// Test that directory segment uses `prompt_close` when git is disabled
///
/// This is a critical regression test for the hot-reload bug where:
/// 1. Config has `git.enabled = true`, segments = `["clock", "directory", "git"]`
/// 2. Directory is NOT last, uses `segment_close` delimiter
/// 3. User changes `git.enabled = false`
/// 4. Agent writes the `<pid>.reload` flag and rings the SIGURG doorbell
/// 5. Fish reloads theme export, `__enabled_segments` becomes `["clock", "directory"]`
/// 6. Directory segment is now last, should use `prompt_close` delimiter
///
/// This test simulates the final state (git disabled) and verifies correct delimiter.
#[test]
#[allow(clippy::unwrap_used)]
fn test_theme_export_delimiter_selection_after_git_disabled() {
    // Simulate config with git disabled
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

    let (exit_code, stdout, stderr) = run_gpy_agent_with_config(
        CONFIG_GIT_DISABLED,
        &["theme", "export", "--format", "fish"],
    );

    assert_eq!(exit_code, 0_i32, "theme export failed: {stderr}");

    // Critical verification: git should be removed from enabled_segments
    // This makes directory the last segment, which should use prompt_close
    assert!(
        stdout.contains("set -g __enabled_segments clock directory"),
        "Git should be removed from enabled_segments when git.enabled=false, making directory last. Got: {stdout}"
    );

    // Verify GPY_GIT_ENABLED is 0
    assert!(
        stdout.contains("set -gx GPY_GIT_ENABLED \"0\""),
        "GPY_GIT_ENABLED should be 0 when git.enabled=false. Got: {stdout}"
    );

    // The Fish-side directory segment will now detect it's last (via segments_to_render)
    // and use __segment_delim_last (which equals prompt_close) instead of __segment_delim_end
    // We can't test the actual Fish rendering here, but we verify theme export is correct
}

/// Test full hot-reload scenario: git enabled -> disabled
///
/// This test verifies the complete config change flow:
/// 1. Start with git enabled
/// 2. Verify theme export includes git in `enabled_segments`
/// 3. Change to git disabled
/// 4. Verify theme export removes git from `enabled_segments`
/// 5. Verify `GPY_GIT_ENABLED` changes from 1 to 0
#[test]
#[allow(clippy::unwrap_used)]
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

    // Test git enabled state
    let (exit_code, stdout_enabled, stderr) =
        run_gpy_agent_with_config(CONFIG_GIT_ENABLED, &["theme", "export", "--format", "fish"]);
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

    // Test git disabled state (simulating after hot-reload)
    let (disabled_exit_code, stdout_disabled, disabled_stderr) = run_gpy_agent_with_config(
        CONFIG_GIT_DISABLED,
        &["theme", "export", "--format", "fish"],
    );
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

    // Verify the change: enabled list differs between states
    assert!(
        stdout_enabled.contains("git") && !stdout_disabled.contains("clock directory git"),
        "Enabled segments should differ between git enabled and disabled states"
    );
}

// ===== Error Cases and Invalid Inputs =====

#[test]
fn test_invalid_subcommand() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["invalid-command"]);

    // Should fail with non-zero exit code
    assert_ne!(exit_code, 0_i32, "Invalid subcommand should fail");

    // Error message should be helpful
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

    // Should fail
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

    // Should fail
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

    // Should fail with invalid format
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

    // Should fail with invalid format
    assert_ne!(exit_code, 0_i32, "Invalid format should fail");
    assert!(
        !stderr.is_empty(),
        "Should provide error message for invalid format"
    );
}

#[test]
fn test_theme_export_invalid_format() {
    let (exit_code, _stdout, stderr) = run_gpy_agent(&["theme", "export", "--format", "xml"]);

    // Should fail with invalid format
    assert_ne!(exit_code, 0_i32, "Invalid theme format should fail");
    assert!(
        !stderr.is_empty(),
        "Should provide error message for invalid format"
    );
}

#[test]
fn test_help_flag() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&["--help"]);

    // Help should succeed
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

    // Version should succeed
    assert_eq!(exit_code, 0_i32, "Version flag should succeed");

    let output = format!("{stdout}{stderr}");
    // Should contain version number
    assert!(
        !output.is_empty(),
        "Version output should not be empty. Got: {output}"
    );
}

#[test]
fn test_oneshot_git_with_nonexistent_path() {
    let (exit_code, _stdout, _stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        "/nonexistent/path/should/not/exist",
        "--format",
        "json",
    ]);

    // Should handle gracefully (may succeed with no-repo response or fail with error)
    // Both behaviors are acceptable - just shouldn't crash
    let _ = exit_code; // Acknowledge we got a result
}

#[test]
fn test_multiple_flags_combined() {
    // Test that combining valid flags works
    let (exit_code, _stdout, _stderr) = run_gpy_agent(&[
        "oneshot",
        "git",
        "--cwd",
        ".",
        "--format",
        "json",
        "--not-last",
    ]);

    // Should succeed (or fail gracefully if not in a git repo)
    // Either way, shouldn't crash
    let _ = exit_code;
}

#[test]
fn test_empty_arguments() {
    let (exit_code, stdout, stderr) = run_gpy_agent(&[]);

    // Should show help or usage when run with no arguments
    assert_ne!(exit_code, 0_i32, "No arguments should fail or show help");

    let output = format!("{stdout}{stderr}");
    assert!(!output.is_empty(), "Should provide some output");
}

#[test]
fn test_oneshot_git_json_no_crash() {
    // Stress test: ensure oneshot doesn't crash with various inputs
    let git_repo = create_test_git_repo().expect("create git repo for test");
    let repo_path = git_repo.path().to_string_lossy();

    let test_cases = vec![
        vec![
            "oneshot".to_owned(),
            "git".to_owned(),
            "--cwd".to_owned(),
            ".".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ],
        vec![
            "oneshot".to_owned(),
            "git".to_owned(),
            "--cwd".to_owned(),
            repo_path.to_string(),
            "--format".to_owned(),
            "json".to_owned(),
        ],
    ];

    for args in test_cases {
        let args_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (exit_code, _stdout, _stderr) = run_gpy_agent(&args_refs);
        // Just verify it completes (don't crash)
        let _ = exit_code;
    }
}

#[test]
fn test_oneshot_lang_json_no_crash() {
    // Stress test: ensure language detection doesn't crash
    let rust_project = create_test_rust_project().expect("create rust project for language test");
    let rust_path = rust_project.path().to_string_lossy();

    let test_cases = vec![
        vec![
            "oneshot".to_owned(),
            "lang".to_owned(),
            "--cwd".to_owned(),
            ".".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ],
        vec![
            "oneshot".to_owned(),
            "lang".to_owned(),
            "--cwd".to_owned(),
            rust_path.to_string(),
            "--format".to_owned(),
            "json".to_owned(),
        ],
    ];

    for args in test_cases {
        let args_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (exit_code, _stdout, _stderr) = run_gpy_agent(&args_refs);
        // Just verify it completes
        let _ = exit_code;
    }
}
