//! Skip contract for integration tests; shell twin is `test_skip` in
//! `tests/lib/test_helpers.fish` and `tests/lib/shell_e2e.sh`.

/// Skip the calling test: prints `SKIP: <reason>` locally, panics under `CI` or
/// `GPY_GATE_STRICT` so a gate never goes green on a test that didn't run.
#[track_caller]
// The lint exempts `#[test]` bodies; this prints on their behalf.
#[allow(clippy::print_stderr)]
pub fn skip_test(reason: &str) {
    eprintln!("SKIP: {reason}");
    assert!(
        !skips_fail(),
        "a skipped test is a failure under CI or GPY_GATE_STRICT: {reason}"
    );
}

/// Whether `CI` or `GPY_GATE_STRICT` is set to a non-empty value.
#[must_use]
pub fn skips_fail() -> bool {
    ["CI", "GPY_GATE_STRICT"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}
