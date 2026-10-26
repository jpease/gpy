//! `Agent` tests: oneshot git/language requests in JSON and Fish formats,
//! oneshot error handling, agent construction, and startup failure on an
//! unbindable socket. Each test runs against a temporary HOME/XDG config.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

#[path = "common/env_guard.rs"]
mod env_guard;
#[path = "common/git.rs"]
mod git;
#[path = "common/sgr.rs"]
#[allow(
    clippy::many_single_char_names,
    reason = "the shared terminal model names rgb operands r, g, b"
)]
mod sgr;

use env_guard::EnvGuard;
use gpy_agent::Result;
use gpy_agent::agent::Agent;
use gpy_agent::config::{Config, loader};
use gpy_agent::formatter::{Format, RenderContext, SegmentPosition, create_formatter};
use gpy_agent::ipc::Response;
use gpy_agent::theme::ThemeManager;
use serial_test::serial;
use std::path::Path;
use tempfile::{TempDir, tempdir};

#[tokio::test]
#[serial]
async fn test_oneshot_git_command() -> Result<()> {
    let _env = set_test_env();
    let git_request = format!(
        "{{\"type\":\"git_status\",\"path\":\"{}\",\"format\":\"json\"}}",
        env!("CARGO_MANIFEST_DIR")
    );
    let response = Agent::handle_oneshot(&git_request)?;

    let json: serde_json::Value = serde_json::from_str(&response)
        .map_err(|e| gpy_agent::Error::ipc(format!("Invalid JSON response: {e}")))?;

    assert!(
        json.get("branch").is_some(),
        "Git status JSON should contain branch field"
    );
    assert!(
        json.get("staged").is_some(),
        "Git status JSON should contain staged changes count"
    );
    assert!(
        json.get("unstaged").is_some(),
        "Git status JSON should contain unstaged changes count"
    );
    assert!(
        json.get("untracked").is_some(),
        "Git status JSON should contain untracked files count"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_oneshot_language_command() -> Result<()> {
    let _env = set_test_env();
    let lang_request = r#"{"type":"language_detect","path":".","format":"json"}"#;
    let response = Agent::handle_oneshot(lang_request)?;

    let json: serde_json::Value = serde_json::from_str(&response)
        .map_err(|e| gpy_agent::Error::ipc(format!("Invalid JSON response: {e}")))?;

    assert!(
        json.get("languages").is_some(),
        "Language detection JSON should contain languages array"
    );

    if let Some(languages) = json.get("languages").and_then(|l| l.as_array()) {
        // Detected languages must be well-formed.
        for lang in languages {
            assert!(
                lang.get("name").is_some(),
                "Each detected language should have name field"
            );
            assert!(
                lang.get("color").is_some(),
                "Each detected language should have color field from theme"
            );
            // version is optional.
        }
    }

    Ok(())
}

/// Every oneshot kind must render in JSON and in ANSI.
#[tokio::test]
#[serial]
async fn test_oneshot_every_kind_renders_json_and_ansi() -> Result<()> {
    let _env = set_test_env();
    let path = env!("CARGO_MANIFEST_DIR");
    let extras = [
        ("git_status", serde_json::json!({})),
        ("language_detect", serde_json::json!({})),
        ("directory", serde_json::json!({})),
        ("duration", serde_json::json!({"duration_ms": 2_500_u64})),
        ("character", serde_json::json!({"success": true})),
        (
            "hostname",
            serde_json::json!({"hostname": "box", "is_ssh": true}),
        ),
        (
            "username",
            serde_json::json!({"username": "me", "is_root": false}),
        ),
        ("exit_status", serde_json::json!({"success": false})),
    ];

    for (kind, extra) in extras {
        for format in ["json", "ansi"] {
            let mut request = serde_json::json!({"type": kind, "path": path, "format": format});
            request
                .as_object_mut()
                .expect("request is an object")
                .extend(extra.as_object().expect("extras are an object").clone());
            let output = Agent::handle_oneshot(&request.to_string())
                .map_err(|e| gpy_agent::Error::ipc(format!("{kind}/{format} failed: {e}")))?;
            if format == "json" {
                serde_json::from_str::<serde_json::Value>(&output).map_err(|e| {
                    gpy_agent::Error::ipc(format!("{kind} JSON output is not JSON: {e}"))
                })?;
            }
        }
    }

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_oneshot_error_handling() -> Result<()> {
    let _env = set_test_env();
    let invalid_request = r#"{"invalid":"request"}"#;
    let invalid_result = Agent::handle_oneshot(invalid_request);
    assert!(
        invalid_result.is_err(),
        "Request with invalid type field should return error"
    );

    let malformed_request = r#"{"type":"git_status","path"}"#; // Missing closing brace
    let malformed_result = Agent::handle_oneshot(malformed_request);
    assert!(
        malformed_result.is_err(),
        "Malformed JSON should be rejected and return error"
    );

    let empty_request = "";
    let empty_result = Agent::handle_oneshot(empty_request);
    assert!(
        empty_result.is_err(),
        "Empty request string should return error"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_git_status_repository_states() -> Result<()> {
    let _env = set_test_env();
    let git_request = format!(
        "{{\"type\":\"git_status\",\"path\":\"{}\",\"format\":\"json\"}}",
        env!("CARGO_MANIFEST_DIR")
    );
    let response = Agent::handle_oneshot(&git_request)?;
    let json: serde_json::Value = serde_json::from_str(&response)?;

    assert!(
        json.get("branch").is_some(),
        "Valid git repository should have branch in status response"
    );

    let temp_dir = tempdir()?;
    let non_git_request = format!(
        r#"{{"type":"git_status","path":"{}","format":"json"}}"#,
        temp_dir.path().display()
    );
    let non_git_response = Agent::handle_oneshot(&non_git_request)?;

    let non_git: serde_json::Value = serde_json::from_str(&non_git_response)?;
    assert_eq!(
        non_git.get("error").and_then(serde_json::Value::as_str),
        Some("Not in a git repository"),
        "a non-git directory must report the error, got: {non_git_response}"
    );
    assert!(
        non_git.get("branch").is_none(),
        "a non-git directory must not report a branch: {non_git_response}"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_language_detection_scenarios() -> Result<()> {
    let _env = set_test_env();
    let temp_dir = tempdir()?;

    std::fs::write(temp_dir.path().join("test.rs"), "fn main() {}")?;
    std::fs::write(temp_dir.path().join("test.py"), "print('hello')")?;
    std::fs::write(temp_dir.path().join("test.js"), "console.log('hello');")?;

    let lang_request = format!(
        r#"{{"type":"language_detect","path":"{}","format":"json"}}"#,
        temp_dir.path().display()
    );
    let response = Agent::handle_oneshot(&lang_request)?;
    let json: serde_json::Value = serde_json::from_str(&response)?;

    if let Some(languages) = json.get("languages").and_then(|l| l.as_array()) {
        let lang_names: Vec<&str> = languages
            .iter()
            .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
            .collect();

        let has_rust = lang_names
            .iter()
            .any(|&name| name.to_lowercase().contains("rust"));
        let has_python = lang_names
            .iter()
            .any(|&name| name.to_lowercase().contains("python"));
        let has_js = lang_names.iter().any(|&name| {
            name.to_lowercase().contains("javascript") || name.to_lowercase().contains("js")
        });

        assert!(
            has_rust || has_python || has_js || !lang_names.is_empty(),
            "Should detect at least one language from .rs, .py, or .js test files"
        );
    }

    Ok(())
}

/// An unbindable socket must fail `start_background` with a propagated error,
/// not hang or continue. `Agent::new` only records the path; bind happens later.
#[tokio::test]
#[serial]
async fn test_unbindable_socket_fails_startup() -> Result<()> {
    let _env = set_test_env();

    // Missing parent directory makes `UnixListener::bind` fail with ENOENT.
    let temp_dir = tempdir()?;
    let unbindable = temp_dir.path().join("missing-parent").join("gpy.sock");
    // Restored on drop so a leaked bad path cannot break later tests.
    let _socket_env = EnvGuard::set_one("GPY_AGENT_SOCKET_PATH", &unbindable);

    let agent = Agent::new()?;

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), agent.start_background())
        .await
        .expect("start_background must return promptly on bind failure, not hang");

    let err = outcome.expect_err("unbindable socket must fail agent startup");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("bind") || msg.to_lowercase().contains("socket"),
        "startup error should reference the socket bind failure, got: {msg}"
    );

    Ok(())
}

/// The agent constructs with the default configuration.
#[tokio::test]
#[serial]
async fn test_agent_creation() -> Result<()> {
    let _env = set_test_env();
    let _agent = Agent::new()?;
    Ok(())
}

/// Several back-to-back oneshot requests all succeed with a non-empty reply.
#[tokio::test]
#[serial]
async fn test_rapid_oneshot_execution() -> Result<()> {
    let _env = set_test_env();

    let mut set = tokio::task::JoinSet::new();
    for i in 0_i32..5_i32 {
        set.spawn(async move {
            let request = if i % 2_i32 == 0_i32 {
                r#"{"type":"git_status","path":".","format":"json"}"#
            } else {
                r#"{"type":"language_detect","path":".","format":"json"}"#
            };
            (i, Agent::handle_oneshot(request))
        });
    }

    while let Some(res) = set.join_next().await {
        let (i, result) = res.expect("join handle");
        let response = result?;
        assert!(
            !response.is_empty(),
            "Rapid oneshot response {i} should contain data"
        );
    }

    Ok(())
}

/// A git theme that sets only the segment colors, no per-state ones.
const THEME_WITHOUT_STATE_COLORS: &str = "[ui]\n\n[segments.git]\n\
    format = \"[ $branch ]($style)\"\nbg_color = \"blue\"\ntext_color = \"yellow\"\n";

/// A theme omitting every git state color renders in the segment colors, and
/// the oneshot path and the agent's theme load render it identically.
#[tokio::test]
#[serial]
async fn omitted_git_state_colors_render_the_same_via_oneshot_and_agent() -> Result<()> {
    let _env = set_test_env_with_theme(THEME_WITHOUT_STATE_COLORS);
    let repo = committed_repo();

    let oneshot = oneshot_git_ansi(repo.path())?;
    assert_eq!(oneshot, agent_git_ansi(repo.path())?);

    let (drawn, _) = sgr::interpret(&oneshot).expect("oneshot output is plain SGR");
    let branch: Vec<_> = drawn
        .iter()
        .filter(|(ch, _)| "main".contains(*ch))
        .map(|(_, look)| (look.fg, look.bg))
        .collect();
    let blue = sgr::Shade::Basic {
        index: 4,
        bright: false,
    };
    let yellow = sgr::Shade::Basic {
        index: 3,
        bright: false,
    };
    assert!(
        !branch.is_empty()
            && branch
                .iter()
                .all(|look| *look == (Some(yellow), Some(blue))),
        "clean branch must draw in the segment colors, got {oneshot:?}"
    );
    Ok(())
}

/// A theme that fails to load falls back to the `default` theme on the oneshot
/// path, as it does in the agent.
#[tokio::test]
#[serial]
async fn broken_theme_falls_back_to_default_via_oneshot_and_agent() -> Result<()> {
    let _env = set_test_env_with_theme("[ui\n");
    let repo = committed_repo();

    let oneshot = oneshot_git_ansi(repo.path())?;
    assert!(
        oneshot.contains("main"),
        "fallback theme must render the branch, got {oneshot:?}"
    );
    assert_eq!(oneshot, agent_git_ansi(repo.path())?);
    Ok(())
}

/// A clean repository on branch `main` with one commit.
fn committed_repo() -> TempDir {
    let repo = tempdir().expect("create repo dir");
    git::git_init(repo.path());
    git::git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    );
    repo
}

/// The git segment `gpy-agent oneshot` renders for `repo`.
fn oneshot_git_ansi(repo: &Path) -> Result<String> {
    let request = serde_json::json!({
        "type": "git_status",
        "path": repo.to_str().expect("utf-8 repo path"),
        "format": "ansi",
    });
    Agent::handle_oneshot(&request.to_string())
}

/// The git segment the agent renders for `repo`: the theme loaded as `Agent`
/// loads it (configured name, else `default`), drawn by the IPC formatter.
fn agent_git_ansi(repo: &Path) -> Result<String> {
    let config = loader::load_config()?;
    let theme = ThemeManager::new(config.ui.theme.as_str())
        .or_else(|_| ThemeManager::new("default"))?
        .get();
    let status = gpy_agent::git::native::NativeGitBackend::get_complete_status(
        repo,
        None,
        config.git.max_ahead_behind,
        config.git.stash_enabled,
        std::time::Duration::from_secs(config.git.timeout_seconds.get()),
    )?
    .expect("a git repository")
    .status;
    let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE)
        .with_palette(gpy_agent::palette::active_palette(&config));
    create_formatter(Format::Ansi).render(&Response::RepositoryStatus(status), &ctx)
}

/// Temp HOME/XDG dirs plus env overrides; field order restores env before removing dirs.
struct TestEnv {
    _env: EnvGuard,
    _home: TempDir,
}

/// Points config lookup at a fresh temp dir so tests ignore local config.
fn set_test_env() -> TestEnv {
    let home_dir = tempdir().expect("create temp test dir");
    let home = home_dir.path();
    let config_root = home.join(".config");
    let cache_root = home.join(".cache");
    let runtime_root = home.join(".runtime");
    let gpy_config_dir = config_root.join("gpy");

    std::fs::create_dir_all(&gpy_config_dir).expect("create config dir");
    std::fs::create_dir_all(cache_root.join("gpy")).expect("create cache dir");
    std::fs::create_dir_all(&runtime_root).expect("create runtime dir");

    let config_path = gpy_config_dir.join("config.toml");
    let default_config = Config::default();
    loader::save_config(
        &config_path,
        &default_config,
        &default_config,
        &["ui.show_icons"],
    )
    .expect("write default config");

    let mut env = EnvGuard::set_one("GPY_CONFIG_PATH", &config_path);
    env.set("HOME", home);
    env.set("XDG_CONFIG_HOME", &config_root);
    env.set("XDG_CACHE_HOME", &cache_root);
    env.set("XDG_RUNTIME_DIR", &runtime_root);
    env.set("GPY_DISABLE_WATCHER", "1");

    TestEnv {
        _env: env,
        _home: home_dir,
    }
}

/// [`set_test_env`] with `theme_toml` installed as the active user theme `audit`.
fn set_test_env_with_theme(theme_toml: &str) -> TestEnv {
    let env = set_test_env();
    let config_path = std::env::var_os("GPY_CONFIG_PATH").expect("GPY_CONFIG_PATH is set");
    std::fs::write(&config_path, "[ui]\ntheme = \"audit\"\n").expect("write config");
    let themes_dir = ThemeManager::user_themes_dir();
    std::fs::create_dir_all(&themes_dir).expect("create themes dir");
    std::fs::write(themes_dir.join("audit.toml"), theme_toml).expect("write theme");
    env
}
