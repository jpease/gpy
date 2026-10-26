//! Tests for socket override behaviour and conflict rejection.

#![allow(clippy::unwrap_used, reason = "test assertions panic on failure")]
#![allow(clippy::missing_panics_doc, reason = "test helper functions")]

use gpy_agent::paths::set_socket_override;
use std::path::PathBuf;

#[test]
fn conflicting_socket_override_is_rejected() {
    let first = PathBuf::from("/tmp/gpy-test-override-first.sock");
    let second = PathBuf::from("/tmp/gpy-test-override-second.sock");

    // First override succeeds
    assert!(set_socket_override(&first).is_ok());

    // Setting the same override again is idempotent and succeeds
    assert!(set_socket_override(&first).is_ok());

    // Setting a conflicting override fails with an error
    let err = set_socket_override(&second).expect_err("conflicting override must fail");
    assert!(
        err.to_string().contains("conflicting socket override"),
        "error message should describe conflict: {err}"
    );
}
