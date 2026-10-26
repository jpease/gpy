//! Skip contract for integration tests; shell twin is `test_skip` in
//! `tests/lib/test_helpers.fish` and `tests/lib/shell_e2e.sh`.

/// Skip the calling test: prints `SKIP: <reason>` locally, panics under `CI`
/// so a gate never goes green on a test that didn't run.
#[track_caller]
// The lint exempts `#[test]` bodies; this prints on their behalf.
#[allow(clippy::print_stderr)]
pub fn skip_test(reason: &str) {
    eprintln!("SKIP: {reason}");
    assert!(
        !ci_is_set(),
        "a skipped test is a failure under CI: {reason}"
    );
}

/// Whether the `CI` environment variable is set to a non-empty value.
#[must_use]
pub fn ci_is_set() -> bool {
    std::env::var_os("CI").is_some_and(|value| !value.is_empty())
}
