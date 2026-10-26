//! Tests for `gpy_agent::Error` conversions, source chaining, and message composition.

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]

use gpy_agent::{Error, Result};
use std::error::Error as StdError;
use std::io;

#[test]
fn test_json_error_display() {
    let json_err = serde_json::from_str::<serde_json::Value>("invalid json")
        .expect_err("Should fail to parse");
    let err = Error::from(json_err);
    assert!(err.to_string().starts_with("JSON processing failed:"));
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
    let invalid_toml = "invalid = toml = syntax";
    let toml_err = toml::from_str::<toml::Value>(invalid_toml).expect_err("Should fail");
    let err = Error::from(toml_err);

    // Should convert to Config error
    assert!(err.to_string().contains("Configuration error"));
    assert!(err.to_string().contains("TOML parsing failed"));

    // Should have source
    assert!(err.source().is_some());
}

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

#[test]
fn test_from_io_error() {
    let io_err = io::Error::new(io::ErrorKind::NotFound, "test");
    let err: Error = io_err.into();
    assert!(matches!(err, Error::Io(_)));
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

#[test]
fn test_error_with_multiple_conversions() {
    fn cause_io_error() -> Result<()> {
        Err(io::Error::other("io error").into())
    }

    fn cause_json_error() -> Result<()> {
        let _: serde_json::Value = serde_json::from_str("bad")?;
        Ok(())
    }

    assert!(cause_io_error().is_err());
    assert!(cause_json_error().is_err());
}

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

#[test]
fn test_error_debug_formatting() {
    let err = Error::invalid("test operation");
    let debug = format!("{err:?}");

    // Debug output should be readable
    assert!(debug.contains("Invalid"));
    assert!(debug.contains("test operation"));
}

#[test]
fn test_all_variants_have_proper_display() {
    let errors = vec![
        Error::git("git"),
        Error::language("lang"),
        Error::ipc("ipc"),
        Error::config("config"),
        Error::agent("agent"),
        Error::process("cmd", "msg"),
        Error::invalid("invalid"),
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

#[test]
fn test_io_context_keeps_kind_and_names_action() {
    let source = io::Error::new(io::ErrorKind::NotFound, "no such file");
    let err = Error::io_context("failed to read x", &source);
    assert!(matches!(&err, Error::Io(e) if e.kind() == io::ErrorKind::NotFound));
    assert_eq!(
        gpy_agent::error::user_message(&err),
        "I/O operation failed: failed to read x: no such file"
    );
}

#[test]
fn test_import_errors_keep_their_type() {
    let base16: Error = gpy_agent::import::base16::ImportError::BadHex("zz".to_owned()).into();
    assert!(matches!(base16, Error::Base16Import(_)));
    let starship: Error = gpy_agent::import::starship::ImportError::Parse {
        message: "bad".to_owned(),
    }
    .into();
    assert!(matches!(starship, Error::StarshipImport(_)));
}
