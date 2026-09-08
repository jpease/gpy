#!/usr/bin/env bash
# tests/bash/toolchain_pin_scope.test.bash
#
# Regression test for #538. scripts/check-active-toolchain.sh asserts that the
# active rustc is the one gpy-agent/rust-toolchain.toml pins, and it used to ask
# that question from whatever directory it happened to be invoked in -- in CI,
# the repository root.
#
# A toolchain file governs its own directory and the directories below it.
# rustup searches the current directory and its parents, never its children, so
# gpy-agent/rust-toolchain.toml says nothing about the root: `rustc` there falls
# through to rustup's default toolchain. The check was therefore reporting the
# machine's default rather than the project's pin. It passed on ubuntu-latest
# and windows-latest only because those images ship the pinned version as their
# default, and failed the whole macOS quality gate in 19s because that image
# ships a different one -- while all three were in fact compiling with the pin.
#
# The properties asserted here are the two that broke:
#
#   (a) the probe is taken in the directory the pin governs, whatever the
#       caller's working directory is
#   (b) a genuine mismatch still fails, so the #518 guard is not hollowed out
#
# Both are tested against a fake `rustc` on PATH whose reported version depends
# on the working directory, so the result does not depend on which toolchain the
# machine running the suite happens to default to.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

SCRIPT="$ROOT/scripts/check-active-toolchain.sh"
TOOLCHAIN="$ROOT/gpy-agent/rust-toolchain.toml"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# Indent captured output under the failure that reported it. Parameter
# expansion rather than sed so shellcheck's SC2001 stays quiet.
indent() {
    local text="${1//$'\n'/$'\n'    }"
    printf '    %s\n' "$text"
}

[[ -x "$SCRIPT" ]] || {
    echo "FAIL: $SCRIPT is missing or not executable"
    echo "1 assertion(s) failed"
    exit 1
}

pinned="$(grep -m1 '^channel' "$TOOLCHAIN" | sed -E 's/.*"([^"]+)".*/\1/')"
[[ -n "$pinned" ]] || {
    echo "FAIL: $TOOLCHAIN declares no channel"
    echo "1 assertion(s) failed"
    exit 1
}

shim_dir="$(mktemp -d)"
trap 'rm -rf "$shim_dir"' EXIT

# Reports the pin only when run from the crate directory, the machine default
# (deliberately not the pin) anywhere else -- the exact asymmetry macos-latest
# exhibited between gpy-agent/ and the repository root.
write_dir_sensitive_rustc() {
    cat >"$shim_dir/rustc" <<SHIM
#!/usr/bin/env bash
if [[ "\$(basename "\$PWD")" == "gpy-agent" ]]; then
    echo "rustc $pinned (fake)"
else
    echo "rustc 9.9.9 (fake)"
fi
SHIM
    chmod +x "$shim_dir/rustc"
}

# A toolchain that is wrong everywhere, including in the crate directory.
write_mismatched_rustc() {
    cat >"$shim_dir/rustc" <<'SHIM'
#!/usr/bin/env bash
echo "rustc 9.9.9 (fake)"
SHIM
    chmod +x "$shim_dir/rustc"
}

echo "--- the pin is probed where it applies, not where the script was called ---"
write_dir_sensitive_rustc
for from in "$ROOT" "$ROOT/scripts" /; do
    if output="$(cd "$from" && PATH="$shim_dir:$PATH" "$SCRIPT" 2>&1)"; then
        if ! grep -q "active (rustc --version)    : $pinned" <<<"$output"; then
            fail "run from $from passed but did not report the pinned $pinned:"
            indent "$output"
        fi
    else
        fail "run from $from reported a mismatch; the probe followed the caller's directory instead of gpy-agent/ (#538)"
        indent "$output"
    fi
done

echo "--- a real mismatch still fails ---"
write_mismatched_rustc
if output="$(cd "$ROOT" && PATH="$shim_dir:$PATH" "$SCRIPT" 2>&1)"; then
    fail "a rustc that matches the pin in no directory was accepted; the #518 guard is inert"
    indent "$output"
elif ! grep -q "is active but rust-toolchain.toml pins $pinned" <<<"$output"; then
    fail "the mismatch failed without naming the pinned version, which is what makes a stale job identifiable:"
    indent "$output"
fi

echo "--- the check holds against the real toolchain ---"
# The end-to-end assertion, skipped rather than guessed when there is no rustc
# to ask. This is the invariant the CI gate exists to enforce; if it fails here
# the local toolchain genuinely is not the pinned one.
if command -v rustc >/dev/null 2>&1; then
    if ! output="$(cd / && "$SCRIPT" 2>&1)"; then
        fail "the real toolchain does not match the pin when checked from outside the repository:"
        indent "$output"
    fi
else
    echo "  SKIP: rustc is not installed"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
