#!/usr/bin/env bash
# tests/bash/shell_suites_build_agent.test.bash
#
# Regression test for #812. The Fish runner and `run_shell_tests` built the
# debug agent only when target/debug/gpy-agent did not exist, so after any Rust
# edit the shell suites validated the PREVIOUS binary (false green / false red).
#
# Structural checks (a behavioural one would need a stub cargo and an empty
# test glob):
#   (a) scripts/test_fish.sh builds unconditionally, not behind a `! -x` guard,
#       and builds before it runs any test
#   (b) run_shell_tests builds through run_check before its first suite
#   (c) both honour CARGO_TARGET_DIR for the binary location

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

fish_script="scripts/test_fish.sh"
gate_script="scripts/quality-check.sh"

# --- (a) test_fish.sh --------------------------------------------------------
# Everything before AGENT_DIR= is the build/ensure section.
prelude="$(awk '/^AGENT_DIR=/ {exit} {print}' "$fish_script")"
build_lines="$(printf '%s\n' "$prelude" | grep -n 'cargo build' || true)"
if [ -z "$build_lines" ]; then
    fail "$fish_script: no cargo build before AGENT_DIR="
fi
# Only text before the build can guard it (a post-build existence check is fine).
if printf '%s\n' "$prelude" | awk '/cargo build/ {exit} {print}' | grep -Eq '^[[:space:]]*if \[ ! -x'; then
    fail "$fish_script: build section still guarded by an 'if [ ! -x' existence check"
fi
if ! grep -q 'CARGO_TARGET_DIR' "$fish_script"; then
    fail "$fish_script: does not honour CARGO_TARGET_DIR"
fi

# --- (b) run_shell_tests -----------------------------------------------------
fn_body="$(awk '/^run_shell_tests\(\)/ {on=1} on {print} on && /^}/ {exit}' "$gate_script")"
build_n="$(printf '%s\n' "$fn_body" | grep -n 'cargo build' | head -1 | cut -d: -f1)"
first_check_n="$(printf '%s\n' "$fn_body" | grep -n 'run_check' | head -1 | cut -d: -f1)"
if [ -z "$build_n" ]; then
    fail "run_shell_tests: no cargo build"
elif [ -z "$first_check_n" ] || [ "$build_n" -gt "$first_check_n" ]; then
    fail "run_shell_tests: cargo build is not before its first run_check"
fi
if ! printf '%s\n' "$fn_body" | grep -q 'run_check "Build debug binaries"'; then
    fail "run_shell_tests: build does not go through run_check \"Build debug binaries\""
fi

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed"
    exit 1
fi
echo "PASS: shell suites build the agent unconditionally"
