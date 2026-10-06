#!/usr/bin/env bash
# tests/bash/toolchain_pin_hooks.test.bash
#
# Regression test for #823. CI asserts the active rustc is the
# gpy-agent/rust-toolchain.toml pin (scripts/check-active-toolchain.sh), but the
# local entry points did not: with RUSTUP_TOOLCHAIN=<newer> exported, rustup's
# override beats the file and clippy-strict failed on lints the pin does not
# raise, which reads as a code problem rather than a toolchain one.
#
# Asserted, for each Rust entry point (just clippy-strict, just lint,
# quality-check.sh --rust-only / --fast / --fix):
#   (a) with a non-pin rustc active it exits non-zero
#   (b) the message names the RUSTUP_TOOLCHAIN override
#   (c) no cargo/moon command ran (fail fast, not after a lint run)
#
# `rustc` is a shim reporting a wrong version and `cargo`/`moon` are shims that
# only record being called, so no second real toolchain need be installed.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

shim_dir="$(mktemp -d)"
trap 'rm -rf "$shim_dir"' EXIT
marker="$shim_dir/ran"

printf '#!/bin/sh\necho "rustc 0.0.1 (shim)"\n' > "$shim_dir/rustc"
for tool in cargo moon; do
    printf '#!/bin/sh\necho "%s $*" >> "%s"\nexit 0\n' "$tool" "$marker" > "$shim_dir/$tool"
done
chmod +x "$shim_dir"/*

override="bogus-override-9.9.9"

check_entry() {
    local label="$1"
    shift
    local output
    : > "$marker"
    if output="$(PATH="$shim_dir:$PATH" RUSTUP_TOOLCHAIN="$override" "$@" 2>&1)"; then
        fail "$label: succeeded despite a non-pin toolchain"
        return
    fi
    if [[ "$output" != *"RUSTUP_TOOLCHAIN=$override"* ]]; then
        fail "$label: message does not name the override"
        printf '%s\n' "$output" | sed 's/^/    /'
    fi
    if [[ -s "$marker" ]]; then
        fail "$label: ran cargo/moon before failing: $(cat "$marker")"
    fi
}

echo "--- Rust entry points fail fast naming the override ---"
check_entry "quality-check --rust-only" ./scripts/quality-check.sh --rust-only
check_entry "quality-check --fast" ./scripts/quality-check.sh --fast
check_entry "quality-check --fix" ./scripts/quality-check.sh --fix
check_entry "quality-check (full)" ./scripts/quality-check.sh
if command -v just >/dev/null 2>&1; then
    check_entry "just clippy-strict" just clippy-strict
    check_entry "just lint" just lint
else
    echo "SKIP: just not installed"
fi

if [[ $failures -ne 0 ]]; then
    echo "FAILED: $failures"
    exit 1
fi
echo "PASS"
