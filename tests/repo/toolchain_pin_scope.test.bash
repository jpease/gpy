#!/usr/bin/env bash
# tests/repo/toolchain_pin_scope.test.bash
# scripts/check-active-toolchain.sh must probe rustc from gpy-agent/: rustup reads
# rust-toolchain.toml from the cwd and its parents, so a root probe reports the
# machine default. Asserts (a) the probe ignores the caller's cwd, (b) a real
# mismatch still fails, using a fake cwd-dependent `rustc` on PATH.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

SCRIPT="$ROOT/scripts/check-active-toolchain.sh"
TOOLCHAIN="$ROOT/gpy-agent/rust-toolchain.toml"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

# Indent captured output; parameter expansion avoids SC2001.
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

# Reports the pin only from the crate directory, the machine default elsewhere.
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
        fail "run from $from reported a mismatch; the probe followed the caller's directory instead of gpy-agent/"
        indent "$output"
    fi
done

echo "--- a real mismatch still fails ---"
write_mismatched_rustc
if output="$(cd "$ROOT" && PATH="$shim_dir:$PATH" "$SCRIPT" 2>&1)"; then
    fail "a rustc that matches the pin in no directory was accepted; the guard is inert"
    indent "$output"
elif ! grep -q "is active but rust-toolchain.toml pins $pinned" <<<"$output"; then
    fail "the mismatch failed without naming the pinned version, which is what makes a stale job identifiable:"
    indent "$output"
fi

echo "--- the check holds against the real toolchain ---"
# End to end; skipped when there is no rustc.
if command -v rustc >/dev/null 2>&1; then
    if ! output="$(cd / && "$SCRIPT" 2>&1)"; then
        fail "the real toolchain does not match the pin when checked from outside the repository:"
        indent "$output"
    fi
else
    test_skip_part "rustc is not installed"
fi

test_finish
