//! The skip contract for integration tests (#650).
//!
//! Included per test crate with `#[path = "common/skip.rs"] mod skip;` so a
//! crate that needs nothing else from `common` does not pull the harnesses
//! in. The shell suites' twin is `test_skip` in `tests/lib/test_helpers.fish`
//! and `tests/lib/shell_e2e.sh`.
#![allow(dead_code)]

/// Record that the calling test cannot run because `reason`.
///
/// Locally this prints `SKIP: <reason>` and returns, so the caller can
/// `return` and a developer without, say, Fish 4 is not blocked. When `CI`
/// is set it panics instead: a gate must never go green on a test that did
/// not actually execute. Never skip by returning silently.
#[track_caller]
// The lint exempts `#[test]` bodies, where every other test diagnostic
// prints from; this helper prints on their behalf.
#[allow(clippy::print_stderr)]
pub fn skip_test(reason: &str) {
    eprintln!("SKIP: {reason}");
    assert!(
        !ci_is_set(),
        "a skipped test is a failure under CI (#650): {reason}"
    );
}

/// Whether the `CI` environment variable is set to a non-empty value.
#[must_use]
pub fn ci_is_set() -> bool {
    std::env::var_os("CI").is_some_and(|value| !value.is_empty())
}
