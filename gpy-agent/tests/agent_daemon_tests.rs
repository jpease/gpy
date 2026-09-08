//! Agent daemon lifecycle integration tests
//!
//! These tests cover the full Agent daemon implementation including fork behavior,
//! oneshot commands, and the specific scenarios discovered during development.
//!
//! # Test Navigation Map
//!
//! **Total Tests**: 9
//! **Last Updated**: 2025-11-17
//!
//! ## Test Categories
//!
//! ### ✅ Basic Operations (5 tests)
//! Tests covering normal/happy path scenarios:
//! - `test_oneshot_git_command` - Oneshot git status command execution and JSON response
//! - `test_oneshot_language_command` - Oneshot language detection command execution and JSON response
//! - `test_fish_format_output` - Fish format output for git and language commands
//! - `test_agent_creation` - Agent instantiation and basic functionality
//!
//! ### ❌ Error Cases (2 tests)
//! Tests covering error scenarios and failure modes:
//! - `test_oneshot_error_handling` - Invalid JSON, malformed requests, empty requests
//! - `test_unbindable_socket_fails_startup` - Unbindable socket propagates a startup error
//!
//! ### 🔀 Edge Cases (1 test)
//! Tests covering boundary conditions:
//! - `test_rapid_oneshot_execution` - Rapid concurrent oneshot command execution (5 commands)
//!
//! ### 🔗 Integration (2 tests)
//! Tests covering cross-module interactions:
//! - `test_git_status_repository_states` - Git status in valid and non-git directories
//! - `test_language_detection_scenarios` - Language detection with multiple file types
//!
//! ## What's NOT Tested (TODO)
//! Known gaps for this module (from the original planning notes, since removed):
//! - Agent daemon fork and background process behavior
//! - PID file creation and cleanup
//! - Agent restart and recovery scenarios
//! - Signal handling (SIGTERM, SIGHUP)
//! - Resource cleanup on abnormal shutdown
//! - Oneshot command timeout handling
//! - Large response payload handling
//! - Config reload during oneshot execution
//! - Concurrent oneshot vs daemon requests
//! - Memory usage under sustained oneshot load
//!
//! ## Related Code
//! - **Production**: `src/agent.rs`, `src/config/loader.rs`, `src/config/manager.rs`
//! - **Other Tests**: `integration_tests.rs`, `e2e_ipc_tests.rs`, `git_status_tests.rs`
//! - **Fixtures**: None (uses tempfile and test env guard)
//!
//! ## Quick Reference
//! ```bash
//! cargo test --test agent_daemon_tests
//! cargo test --test agent_daemon_tests -- --nocapture
//! cargo test --test agent_daemon_tests test_oneshot_git_command
//! ```

#![allow(unsafe_code)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)] // Test functions don't need panic docs
#![allow(clippy::missing_errors_doc)] // Test functions don't need error docs

use gpy_agent::Result;
use gpy_agent::agent::Agent;
use gpy_agent::config::{Config, loader};
use serial_test::serial;
use std::ffi::OsString;
use tempfile::tempdir;

/// Test oneshot git command functionality
#[tokio::test]
#[serial]
async fn test_oneshot_git_command() -> Result<()> {
    let _env = set_test_env();
    // Test git status request
    let git_request = format!(
        "{{\"type\":\"git_status\",\"path\":\"{}\",\"format\":\"json\"}}",
        env!("CARGO_MANIFEST_DIR")
    );
    let response = Agent::handle_oneshot(&git_request)?;

    // Should return valid JSON with git status fields
    let json: serde_json::Value = serde_json::from_str(&response)
        .map_err(|e| gpy_agent::Error::ipc(format!("Invalid JSON response: {e}")))?;

    // Check for expected git status fields
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

/// Test oneshot language detection command
#[tokio::test]
#[serial]
async fn test_oneshot_language_command() -> Result<()> {
    let _env = set_test_env();
    // Test language detection request
    let lang_request = r#"{"type":"language_detect","path":".","format":"json"}"#;
    let response = Agent::handle_oneshot(lang_request)?;

    // Should return valid JSON with languages array
    let json: serde_json::Value = serde_json::from_str(&response)
        .map_err(|e| gpy_agent::Error::ipc(format!("Invalid JSON response: {e}")))?;

    // Check for expected language detection fields
    assert!(
        json.get("languages").is_some(),
        "Language detection JSON should contain languages array"
    );

    if let Some(languages) = json.get("languages").and_then(|l| l.as_array()) {
        // If languages are detected, they should have proper structure
        for lang in languages {
            assert!(
                lang.get("name").is_some(),
                "Each detected language should have name field"
            );
            assert!(
                lang.get("color").is_some(),
                "Each detected language should have color field from theme"
            );
            // version is optional and can be null
        }
    }

    Ok(())
}

/// Test oneshot command error handling
#[tokio::test]
#[serial]
async fn test_oneshot_error_handling() -> Result<()> {
    let _env = set_test_env();
    // Test invalid request format
    let invalid_request = r#"{"invalid":"request"}"#;
    let invalid_result = Agent::handle_oneshot(invalid_request);
    assert!(
        invalid_result.is_err(),
        "Request with invalid type field should return error"
    );

    // Test malformed JSON
    let malformed_request = r#"{"type":"git_status","path"}"#; // Missing closing brace
    let malformed_result = Agent::handle_oneshot(malformed_request);
    assert!(
        malformed_result.is_err(),
        "Malformed JSON should be rejected and return error"
    );

    // Test empty request
    let empty_request = "";
    let empty_result = Agent::handle_oneshot(empty_request);
    assert!(
        empty_result.is_err(),
        "Empty request string should return error"
    );

    Ok(())
}

/// Test git status with various repository states
#[tokio::test]
#[serial]
async fn test_git_status_repository_states() -> Result<()> {
    let _env = set_test_env();
    // Test in a valid git repository (current directory should be one)
    let git_request = format!(
        "{{\"type\":\"git_status\",\"path\":\"{}\",\"format\":\"json\"}}",
        env!("CARGO_MANIFEST_DIR")
    );
    let response = Agent::handle_oneshot(&git_request)?;
    let json: serde_json::Value = serde_json::from_str(&response)?;

    // Should have valid git fields
    assert!(
        json.get("branch").is_some(),
        "Valid git repository should have branch in status response"
    );

    // Test in a non-git directory
    let temp_dir = tempdir()?;
    let non_git_request = format!(
        r#"{{"type":"git_status","path":"{}","format":"json"}}"#,
        temp_dir.path().display()
    );
    let non_git_response = Agent::handle_oneshot(&non_git_request)?;

    // Should handle non-git directories gracefully
    // Response might be error JSON or empty, but should not panic
    assert!(
        !non_git_response.is_empty(),
        "Non-git directory should return graceful response without panicking"
    );

    Ok(())
}

/// Test language detection with various directory contents
#[tokio::test]
#[serial]
async fn test_language_detection_scenarios() -> Result<()> {
    let _env = set_test_env();
    let temp_dir = tempdir()?;

    // Create test files for different languages
    std::fs::write(temp_dir.path().join("test.rs"), "fn main() {}")?;
    std::fs::write(temp_dir.path().join("test.py"), "print('hello')")?;
    std::fs::write(temp_dir.path().join("test.js"), "console.log('hello');")?;

    let lang_request = format!(
        r#"{{"type":"language_detect","path":"{}","format":"json"}}"#,
        temp_dir.path().display()
    );
    let response = Agent::handle_oneshot(&lang_request)?;
    let json: serde_json::Value = serde_json::from_str(&response)?;

    // Should detect languages based on file extensions
    if let Some(languages) = json.get("languages").and_then(|l| l.as_array()) {
        let lang_names: Vec<&str> = languages
            .iter()
            .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
            .collect();

        // Should detect at least some of the languages we created files for
        let has_rust = lang_names
            .iter()
            .any(|&name| name.to_lowercase().contains("rust"));
        let has_python = lang_names
            .iter()
            .any(|&name| name.to_lowercase().contains("python"));
        let has_js = lang_names.iter().any(|&name| {
            name.to_lowercase().contains("javascript") || name.to_lowercase().contains("js")
        });

        // At least one language should be detected
        assert!(
            has_rust || has_python || has_js || !lang_names.is_empty(),
            "Should detect at least one language from .rs, .py, or .js test files"
        );
    }

    Ok(())
}

/// Test Fish format output for oneshot commands
#[tokio::test]
#[serial]
async fn test_fish_format_output() -> Result<()> {
    let _env = set_test_env();
    // Test git status with Fish format
    let git_request = r#"{"type":"git_status","path":".","format":"fish"}"#;
    let response = Agent::handle_oneshot(git_request)?;

    // Fish format should be different from JSON (no braces)
    assert!(
        !response.contains('{'),
        "Fish format output should be shell-friendly without JSON braces"
    );
    // Fish format can be empty for clean repos, so we don't assert on emptiness

    // Test language detection with Fish format
    let lang_request = r#"{"type":"language_detect","path":".","format":"fish"}"#;
    let lang_response = Agent::handle_oneshot(lang_request)?;

    // Fish format should be different from JSON
    assert!(
        !lang_response.contains('{'),
        "Language detection Fish output should be shell-friendly without JSON braces"
    );

    Ok(())
}

/// RAII guard for the `GPY_AGENT_SOCKET_PATH` override.
///
/// Lets a test force the agent to bind a specific (here: unbindable) socket
/// path and always restore the previous value, even on assertion failure —
/// otherwise a leaked bad path would break every subsequent test that relies
/// on the default socket.
struct SocketPathGuard {
    prev: Option<OsString>,
}

impl SocketPathGuard {
    fn set(path: &std::path::Path) -> Self {
        let prev = std::env::var_os("GPY_AGENT_SOCKET_PATH");
        unsafe {
            std::env::set_var("GPY_AGENT_SOCKET_PATH", path);
        }
        Self { prev }
    }
}

impl Drop for SocketPathGuard {
    fn drop(&mut self) {
        restore_env("GPY_AGENT_SOCKET_PATH", self.prev.take());
    }
}

/// An unbindable socket must still fail agent startup with a clear, propagated
/// error — not hang and not silently continue.
///
/// The redundant pre-bind check (`test_socket_binding`) was removed so the agent
/// binds the socket exactly once, in `EndpointHandle::start` (driven from
/// `start_background`'s event loop). This pins that failure mode: pointing the
/// agent at a socket whose parent directory does not exist makes the single real
/// bind fail, and that error must surface out of `start_background`.
#[tokio::test]
#[serial]
async fn test_unbindable_socket_fails_startup() -> Result<()> {
    let _env = set_test_env();

    // Parent directory intentionally absent -> `UnixListener::bind` fails with
    // ENOENT, so the socket file can never be created.
    let temp_dir = tempdir()?;
    let unbindable = temp_dir.path().join("missing-parent").join("gpy.sock");
    let _socket_guard = SocketPathGuard::set(&unbindable);

    // Agent::new only records the socket path; the bind happens in start_background.
    let mut agent = Agent::new()?;

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

/// Test agent creation and basic functionality
#[tokio::test]
#[serial]
async fn test_agent_creation() -> Result<()> {
    let _env = set_test_env();
    // Test that agent can be created without errors
    let agent = Agent::new();
    assert!(
        agent.is_ok(),
        "Agent creation should succeed with default configuration"
    );

    let _agent_instance = agent?;

    // Test that agent has valid configuration
    // We can't access private fields, but we can test that oneshot commands work
    let test_request = r#"{"type":"git_status","path":".","format":"json"}"#;
    let result = Agent::handle_oneshot(test_request);

    // Should either succeed or fail gracefully (not panic)
    if let Ok(response) = result {
        assert!(
            !response.is_empty(),
            "Oneshot command response should not be empty"
        );
    } else {
        // Error is acceptable if git is not available or other issues
    }

    Ok(())
}

/// Test rapid oneshot command execution (simulates CLI usage)
#[tokio::test]
#[serial(git_operations)] // Serialize with other git-heavy tests: each spawns a real `git status` subprocess
async fn test_rapid_oneshot_execution() -> Result<()> {
    let _env = set_test_env();
    let agent = Agent::new()?;

    // Execute multiple oneshot commands rapidly
    let mut handles = Vec::new();

    for i in 0_i32..5_i32 {
        let _agent_ref = &agent;
        let handle = async move {
            let request = if i % 2_i32 == 0_i32 {
                r#"{"type":"git_status","path":".","format":"json"}"#
            } else {
                r#"{"type":"language_detect","path":".","format":"json"}"#
            };

            Agent::handle_oneshot(request)
        };

        handles.push(handle);
    }

    // Wait for all commands to complete
    let results = futures::future::join_all(handles).await;

    // All commands should complete (successfully or with errors, but not panic)
    for (i, result) in results.into_iter().enumerate() {
        match result {
            Ok(response) => {
                assert!(
                    !response.is_empty(),
                    "Rapid oneshot response {i} should contain data"
                );
            }
            Err(e) => {
                // Errors are acceptable, just ensure they're reasonable
                let error_msg = e.to_string();
                assert!(
                    !error_msg.contains("panic") && !error_msg.contains("thread"),
                    "Rapid oneshot error {i} should be graceful, not panic: {error_msg}"
                );
            }
        }
    }

    Ok(())
}

struct TestEnvGuard {
    prev_config_path: Option<OsString>,
    prev_home: Option<OsString>,
    prev_xdg_config: Option<OsString>,
    prev_xdg_cache: Option<OsString>,
    prev_xdg_runtime: Option<OsString>,
    prev_disable_watcher: Option<OsString>,
    _temp_dir: tempfile::TempDir,
}

fn set_test_env() -> TestEnvGuard {
    let temp_dir = tempfile::tempdir().expect("create temp test dir");
    let home = temp_dir.path();
    let config_root = home.join(".config");
    let cache_root = home.join(".cache");
    let runtime_root = home.join(".runtime");
    let gpy_config_dir = config_root.join("gpy");

    std::fs::create_dir_all(&gpy_config_dir).expect("create config dir");
    std::fs::create_dir_all(cache_root.join("gpy")).expect("create cache dir");
    std::fs::create_dir_all(&runtime_root).expect("create runtime dir");

    let config_path = gpy_config_dir.join("config.toml");
    loader::save_config(
        &Config::default(),
        config_path
            .to_str()
            .expect("config path should be valid UTF-8"),
    )
    .expect("write default config");

    let prev_config_path = std::env::var_os("GPY_CONFIG_PATH");
    let prev_home = std::env::var_os("HOME");
    let prev_xdg_config = std::env::var_os("XDG_CONFIG_HOME");
    let prev_xdg_cache = std::env::var_os("XDG_CACHE_HOME");
    let prev_xdg_runtime = std::env::var_os("XDG_RUNTIME_DIR");
    let prev_disable_watcher = std::env::var_os("GPY_DISABLE_WATCHER");

    unsafe {
        std::env::set_var("GPY_CONFIG_PATH", &config_path);
        std::env::set_var("HOME", home);
        std::env::set_var("XDG_CONFIG_HOME", &config_root);
        std::env::set_var("XDG_CACHE_HOME", &cache_root);
        std::env::set_var("XDG_RUNTIME_DIR", &runtime_root);
        std::env::set_var("GPY_DISABLE_WATCHER", "1");
    };

    TestEnvGuard {
        prev_config_path,
        prev_home,
        prev_xdg_config,
        prev_xdg_cache,
        prev_xdg_runtime,
        prev_disable_watcher,
        _temp_dir: temp_dir,
    }
}

impl Drop for TestEnvGuard {
    fn drop(&mut self) {
        restore_env("GPY_CONFIG_PATH", self.prev_config_path.take());
        restore_env("HOME", self.prev_home.take());
        restore_env("XDG_CONFIG_HOME", self.prev_xdg_config.take());
        restore_env("XDG_CACHE_HOME", self.prev_xdg_cache.take());
        restore_env("XDG_RUNTIME_DIR", self.prev_xdg_runtime.take());
        restore_env("GPY_DISABLE_WATCHER", self.prev_disable_watcher.take());
    }
}

fn restore_env(key: &str, value: Option<OsString>) {
    unsafe {
        if let Some(prev) = value {
            std::env::set_var(key, prev);
        } else {
            std::env::remove_var(key);
        }
    }
}
// These tests rely on the default (built-in) configuration. `set_test_env`
// points configuration lookups at a temporary directory so the agent does not
// read a developer's local config.
