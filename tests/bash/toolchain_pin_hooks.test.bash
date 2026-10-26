#!/usr/bin/env bash
# tests/bash/toolchain_pin_hooks.test.bash
#
# The local Rust entry points must assert the active rustc is the
# gpy-agent/rust-toolchain.toml pin (scripts/check-active-toolchain.sh), as CI
# does: an exported RUSTUP_TOOLCHAIN beats the file, and clippy then fails on
# lints the pin does not raise, which reads as a code problem.
#
# For `just lint` and quality-check.sh --rust-only / --fast / --fix / full:
#   (a) with a non-pin rustc active it exits non-zero
#   (b) the message names the RUSTUP_TOOLCHAIN override
#   (c) no cargo command ran (fail fast, not after a lint run)
#
# `rustc` is a shim reporting a wrong version and `cargo` a shim that only
# records being called, so no second real toolchain need be installed.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shim_dir="$(mktemp -d)"
trap 'rm -rf "$shim_dir"' EXIT
marker="$shim_dir/ran"

printf '#!/bin/sh\necho "rustc 0.0.1 (shim)"\n' > "$shim_dir/rustc"
printf '#!/bin/sh\necho "cargo $*" >> "%s"\nexit 0\n' "$marker" > "$shim_dir/cargo"
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
        fail "$label: ran cargo before failing: $(cat "$marker")"
    fi
}

echo "--- Rust entry points fail fast naming the override ---"
check_entry "quality-check --rust-only" ./scripts/quality-check.sh --rust-only
check_entry "quality-check --fast" ./scripts/quality-check.sh --fast
check_entry "quality-check --fix" ./scripts/quality-check.sh --fix
check_entry "quality-check (full)" ./scripts/quality-check.sh
if command -v just >/dev/null 2>&1; then
    check_entry "just lint" just lint
else
    test_skip_part "just not installed"
fi

test_finish
