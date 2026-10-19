//! `Agent` tests: oneshot git/language requests in JSON and Fish formats,
//! oneshot error handling, agent construction, and startup failure on an
//! unbindable socket. Each test runs against a temporary HOME/XDG config.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)] // Test functions don't need panic docs
#![allow(clippy::missing_errors_doc)] // Test functions don't need error docs

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::Result;
use gpy_agent::agent::Agent;
use gpy_agent::config::{Config, loader};
use serial_test::serial;
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

    let handles = (0_i32..5_i32).map(|i| async move {
        let request = if i % 2_i32 == 0_i32 {
            r#"{"type":"git_status","path":".","format":"json"}"#
        } else {
            r#"{"type":"language_detect","path":".","format":"json"}"#
        };
        Agent::handle_oneshot(request)
    });

    for (i, result) in futures::future::join_all(handles)
        .await
        .into_iter()
        .enumerate()
    {
        let response = result?;
        assert!(
            !response.is_empty(),
            "Rapid oneshot response {i} should contain data"
        );
    }

    Ok(())
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
        config_path
            .to_str()
            .expect("config path should be valid UTF-8"),
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
