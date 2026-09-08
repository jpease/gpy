//! Comprehensive tests for error handling

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]

use gpy_agent::{Error, Result};
use std::error::Error as StdError;
use std::io;

// ===== Display Tests =====

#[test]
fn test_git_error_display() {
    let err = Error::git("Repository not found");
    assert_eq!(
        err.to_string(),
        "Git operation failed: Repository not found"
    );
}

#[test]
fn test_language_error_display() {
    let err = Error::language("Unknown language");
    assert_eq!(
        err.to_string(),
        "Language detection failed: Unknown language"
    );
}

#[test]
fn test_ipc_error_display() {
    let err = Error::ipc("Connection refused");
    assert_eq!(
        err.to_string(),
        "IPC communication failed: Connection refused"
    );
}

#[test]
fn test_watcher_error_display() {
    let err = Error::Watcher {
        message: "Failed to watch directory".to_owned(),
        source: None,
    };
    assert_eq!(
        err.to_string(),
        "File watcher failed: Failed to watch directory"
    );
}

#[test]
fn test_config_error_display() {
    let err = Error::config("Invalid TOML");
    assert_eq!(err.to_string(), "Configuration error: Invalid TOML");
}

#[test]
fn test_agent_error_display() {
    let err = Error::agent("Failed to start");
    assert_eq!(err.to_string(), "Agent process error: Failed to start");
}

#[test]
fn test_io_error_display() {
    let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
    let err = Error::from(io_err);
    assert_eq!(err.to_string(), "I/O operation failed");
}

#[test]
fn test_json_error_display() {
    let json_err = serde_json::from_str::<serde_json::Value>("invalid json")
        .expect_err("Should fail to parse");
    let err = Error::from(json_err);
    assert!(err.to_string().starts_with("JSON processing failed:"));
}

#[test]
fn test_utf8_error_display() {
    let invalid_utf8 = vec![0xFF, 0xFE, 0xFD];
    let utf8_err = std::str::from_utf8(&invalid_utf8).expect_err("Should fail");
    let err = Error::from(utf8_err);
    assert_eq!(err.to_string(), "UTF-8 processing failed");
}

#[test]
fn test_process_error_display() {
    let err = Error::process("git", "exit code 128");
    assert_eq!(
        err.to_string(),
        "Process execution failed: command 'git' exit code 128"
    );
}

#[test]
fn test_invalid_error_display() {
    let err = Error::invalid("Invalid state");
    assert_eq!(err.to_string(), "Invalid operation: Invalid state");
}

#[test]
fn test_timeout_error_display() {
    let err = Error::timeout("git status");
    assert_eq!(err.to_string(), "Operation timed out: git status");
}

// ===== Debug Tests =====

#[test]
fn test_git_error_debug() {
    let err = Error::git("test");
    let debug_str = format!("{err:?}");
    assert!(debug_str.contains("Git"));
    assert!(debug_str.contains("test"));
}

#[test]
fn test_language_error_debug() {
    let err = Error::language("test");
    let debug_str = format!("{err:?}");
    assert!(debug_str.contains("Language"));
    assert!(debug_str.contains("test"));
}

#[test]
fn test_ipc_error_debug() {
    let err = Error::ipc("test");
    let debug_str = format!("{err:?}");
    assert!(debug_str.contains("Ipc"));
    assert!(debug_str.contains("test"));
}

#[test]
fn test_json_error_debug() {
    let json_err = serde_json::from_str::<serde_json::Value>("{invalid}").expect_err("Should fail");
    let err = Error::from(json_err);
    let debug_str = format!("{err:?}");
    assert!(debug_str.contains("Json"));
}

// ===== Source Chaining Tests =====

#[test]
fn test_git_error_without_source() {
    let err = Error::git("Simple error");
    assert!(err.source().is_none());
}

#[test]
fn test_json_error_source_chain() {
    let json_err = serde_json::from_str::<serde_json::Value>("not json").expect_err("Should fail");
    let err = Error::from(json_err);

    // JSON errors should have a source
    assert!(err.source().is_some());
}

#[test]
fn test_io_error_source_chain() {
    let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
    let err = Error::from(io_err);

    // IO errors wrap the original error
    assert!(err.source().is_some());
    let source = err.source().unwrap();
    assert!(source.to_string().contains("access denied"));
}

#[test]
fn test_toml_error_conversion() {
    // Create an invalid TOML string
    let invalid_toml = "invalid = toml = syntax";
    let toml_err = toml::from_str::<toml::Value>(invalid_toml).expect_err("Should fail");
    let err = Error::from(toml_err);

    // Should convert to Config error
    assert!(err.to_string().contains("Configuration error"));
    assert!(err.to_string().contains("TOML parsing failed"));

    // Should have source
    assert!(err.source().is_some());
}

// ===== Helper Constructor Tests =====

#[test]
fn test_git_helper() {
    let err = Error::git("test message");
    assert!(matches!(err, Error::Git { .. }));
    assert_eq!(err.to_string(), "Git operation failed: test message");
}

#[test]
fn test_language_helper() {
    let err = Error::language("detection failed");
    assert!(matches!(err, Error::Language { .. }));
}

#[test]
fn test_ipc_helper() {
    let err = Error::ipc("connection lost");
    assert!(matches!(err, Error::Ipc { .. }));
}

#[test]
fn test_config_helper() {
    let err = Error::config("missing field");
    assert!(matches!(err, Error::Config { .. }));
}

#[test]
fn test_agent_helper() {
    let err = Error::agent("startup failed");
    assert!(matches!(err, Error::Agent { .. }));
}

#[test]
fn test_process_helper() {
    let err = Error::process("npm", "not found");
    assert!(matches!(err, Error::Process { .. }));
    assert!(err.to_string().contains("npm"));
    assert!(err.to_string().contains("not found"));
}

#[test]
fn test_invalid_helper() {
    let err = Error::invalid("bad input");
    assert!(matches!(err, Error::Invalid { .. }));
}

#[test]
fn test_timeout_helper() {
    let err = Error::timeout("network request");
    assert!(matches!(err, Error::Timeout { .. }));
}

// ===== Result Type Tests =====

#[test]
fn test_result_type_ok() {
    let value = 42_i32;
    let result: Result<i32> = Ok(value);
    assert!(result.is_ok());
    if let Ok(val) = result {
        assert_eq!(val, 42_i32);
    }
}

#[test]
fn test_result_type_err() {
    let result: Result<i32> = Err(Error::invalid("test"));
    assert!(result.is_err());
}

#[test]
fn test_result_question_mark_propagation() {
    fn inner_fn() -> Result<String> {
        Err(Error::git("inner error"))
    }

    fn outer_fn() -> Result<String> {
        let _value = inner_fn()?;
        Ok("success".to_owned())
    }

    let result = outer_fn();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("inner error"));
}

// ===== Error Conversion Tests =====

#[test]
fn test_from_io_error() {
    let io_err = io::Error::new(io::ErrorKind::NotFound, "test");
    let err: Error = io_err.into();
    assert!(matches!(err, Error::Io(_)));
}

#[test]
fn test_from_utf8_error() {
    let invalid = vec![0x80, 0x81];
    let utf8_err = std::str::from_utf8(&invalid).expect_err("Should fail");
    let err: Error = utf8_err.into();
    assert!(matches!(err, Error::Utf8(_)));
}

#[test]
fn test_from_serde_json_error() {
    let json_err = serde_json::from_str::<i32>("not a number").expect_err("Should fail");
    let err: Error = json_err.into();
    assert!(matches!(err, Error::Json { .. }));
}

#[test]
fn test_from_toml_error() {
    let toml_err = toml::from_str::<toml::Value>("bad { toml").expect_err("Should fail");
    let err: Error = toml_err.into();
    assert!(matches!(err, Error::Config { .. }));
}

// ===== Complex Error Chain Tests =====

#[test]
fn test_error_with_multiple_conversions() {
    // Test that errors can be converted through multiple paths
    fn cause_io_error() -> Result<()> {
        Err(io::Error::other("io error").into())
    }

    fn cause_json_error() -> Result<()> {
        let _: serde_json::Value = serde_json::from_str("bad")?;
        Ok(())
    }

    fn cause_utf8_error() -> Result<()> {
        let bytes = vec![0xFF];
        let _ = std::str::from_utf8(&bytes)?;
        Ok(())
    }

    assert!(cause_io_error().is_err());
    assert!(cause_json_error().is_err());
    assert!(cause_utf8_error().is_err());
}

// ===== Edge Cases =====

#[test]
fn test_error_with_empty_message() {
    let err = Error::git("");
    assert_eq!(err.to_string(), "Git operation failed: ");
}

#[test]
fn test_error_with_unicode_message() {
    let err = Error::language("言語検出に失敗しました 🔍");
    assert!(err.to_string().contains("言語検出に失敗しました 🔍"));
}

#[test]
fn test_error_with_very_long_message() {
    let long_message = "error ".repeat(1000);
    let err = Error::config(&long_message);
    assert!(err.to_string().contains("error error error"));
}

#[test]
fn test_error_with_newlines() {
    let err = Error::invalid("line1\nline2\nline3");
    assert!(err.to_string().contains("line1\nline2\nline3"));
}

// ===== Send + Sync Tests =====

#[test]
fn test_error_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Error>();
}

#[test]
fn test_error_is_sync() {
    fn assert_sync<T: Sync>() {}
    assert_sync::<Error>();
}

// ===== Clone/Debug Derivation Tests =====

#[test]
fn test_error_debug_formatting() {
    let err = Error::timeout("test operation");
    let debug = format!("{err:?}");

    // Debug output should be readable
    assert!(debug.contains("Timeout"));
    assert!(debug.contains("test operation"));
}

#[test]
fn test_all_variants_have_proper_display() {
    // Ensure all error variants produce non-empty display strings
    let errors = vec![
        Error::git("git"),
        Error::language("lang"),
        Error::ipc("ipc"),
        Error::config("config"),
        Error::agent("agent"),
        Error::process("cmd", "msg"),
        Error::invalid("invalid"),
        Error::timeout("timeout"),
        Error::from(io::Error::other("io")),
        Error::from(serde_json::from_str::<i32>("bad").unwrap_err()),
    ];

    for err in errors {
        let display = err.to_string();
        assert!(
            !display.is_empty(),
            "Error should have non-empty display: {err:?}"
        );
        assert!(
            display.len() > 5,
            "Error message should be descriptive: {display}"
        );
    }
}
