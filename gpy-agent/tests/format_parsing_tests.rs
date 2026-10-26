//! Parsing of shell names, the `ansi` format, and the `ansi` IPC format field.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use gpy_agent::formatter::Format;
use gpy_agent::ipc::Message;
use gpy_agent::ipc::protocol::deserialize_message;
use gpy_agent::shell::Shell;

#[test]
fn test_format_parsing_ansi() {
    let format: Format = "ansi".parse().unwrap();
    assert_eq!(format, Format::Ansi);
}

#[test]
fn test_shell_enum_parsing() {
    use std::str::FromStr;
    assert_eq!(Shell::from_str("fish"), Ok(Shell::Fish));
    assert_eq!(Shell::from_str("zsh"), Ok(Shell::Zsh));
    assert_eq!(Shell::from_str("bash"), Ok(Shell::Bash));
    assert_eq!(Shell::from_str("FISH"), Ok(Shell::Fish));
    assert!(Shell::from_str("unknown").is_err());
}

#[test]
fn test_shell_ipc_message_parsing_ansi() {
    let json = r#"{"op":"git","cwd":".","format":"ansi"}"#;
    let msg = deserialize_message(json.as_bytes()).expect("failed to parse ansi message");

    if let Message::RepositoryStatus { format, .. } = msg {
        assert_eq!(format, Format::Ansi);
    } else {
        panic!("Expected RepositoryStatus");
    }
}
