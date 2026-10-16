#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later

# Runs the Fish integration tests (tests/fish/*.test.fish).

set -eo pipefail

# Inherited GIT_DIR & co. (e.g. from a pre-push hook) would point the tests'
# throwaway git fixtures at this repo.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
    GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
    GIT_PREFIX GIT_NAMESPACE 2>/dev/null || true

echo "Running Fish integration tests..."
echo

# Always rebuild: an existence check would test a stale binary. Assert the pin
# first, since RUSTUP_TOOLCHAIN outranks rust-toolchain.toml.
"$(dirname "$0")/check-active-toolchain.sh" || exit 1
echo "Building gpy debug binaries for Fish tests..."
(cd gpy-agent && cargo build --quiet --bin gpy --bin gpy-agent) || {
    echo "Error: cargo build failed; refusing to test a stale gpy-agent"
    exit 1
}

# Honour CARGO_TARGET_DIR, as install-dev.fish and run_shell_tests do.
AGENT_BIN="${CARGO_TARGET_DIR:-$PWD/gpy-agent/target}/debug/gpy-agent"
if [ ! -x "$AGENT_BIN" ]; then
    echo "Error: gpy-agent binary not found at $AGENT_BIN after build"
    exit 1
fi

# Put the fresh build first on PATH so tests never pick up an installed gpy-agent.
AGENT_DIR="$(cd "$(dirname "$AGENT_BIN")" && pwd)"

# No blanket retries: a test is retried only when named here as
# `<file>.test.fish:#<open issue>`. Retries are listed in the final tally.
KNOWN_FLAKY_E2E_TESTS=""

attempts_for() {
    local base="$1"
    local entry
    for entry in $KNOWN_FLAKY_E2E_TESTS; do
        if [ "${entry%%:*}" = "$base" ]; then
            echo 3
            return
        fi
    done
    echo 1
}

# Collect all failures rather than stopping at the first.
failed_tests=()
skipped_tests=()
retried_tests=()
run_log="$(mktemp "${TMPDIR:-/tmp}/gpy-fish-test.XXXXXX")"
trap 'rm -f "$run_log"' EXIT

# A SIGTERM to this script alone (CI timeout) does not reach fish; forward it
# so the test's teardown guard stops its agents and removes its sandboxes.
test_pid=""
forward_signal() {
    trap '' INT TERM
    if [ -n "$test_pid" ]; then
        pkill -TERM -P "$test_pid" -x fish 2>/dev/null || true
        wait "$test_pid" 2>/dev/null || true
    fi
    exit "$1"
}
trap 'forward_signal 130' INT
trap 'forward_signal 143' TERM
for test_file in tests/fish/*.test.fish; do
    if [ -f "$test_file" ]; then
        base="$(basename "$test_file")"
        echo "=== Running $base ==="

        attempts="$(attempts_for "$base")"

        attempt=1
        passed=0
        while [ "$attempt" -le "$attempts" ]; do
            # --no-config keeps the developer's config.fish from changing
            # exported vars and PATH under the tests. Paths travel as $argv
            # (single quotes deliberate) so spaces/quotes in the checkout path
            # are safe. Backgrounded + `wait` so a signal is handled while the
            # test still runs; output is captured to tally `SKIP:` lines.
            # shellcheck disable=SC2016
            { fish --no-config -c 'set -gx PATH $argv[1] $PATH; source $argv[2]' -- "$AGENT_DIR" "$test_file" 2>&1 | tee "$run_log"; } &
            test_pid=$!
            if wait "$test_pid"; then
                passed=1
                if grep -q '^SKIP:' "$run_log"; then
                    skipped_tests+=("$base")
                fi
                if [ "$attempt" -gt 1 ]; then
                    retried_tests+=("$base (attempt $attempt/$attempts)")
                fi
                break
            fi
            if [ "$attempt" -lt "$attempts" ]; then
                echo "Warning: $base failed on attempt $attempt/$attempts (named in the retry policy) — retrying..."
            fi
            attempt=$((attempt + 1))
        done

        if [ "$passed" -eq 1 ]; then
            echo
        else
            failed_tests+=("$base")
            echo "Error: $base FAILED after $attempts attempt(s)"
            echo
        fi
    fi
done

join_names() {
    local joined
    joined="$(printf '%s, ' "$@")"
    echo "${joined%, }"
}
if [ ${#skipped_tests[@]} -ne 0 ]; then
    echo "Skipped: ${#skipped_tests[@]} ($(join_names "${skipped_tests[@]}"))"
else
    echo "Skipped: 0"
fi
if [ ${#retried_tests[@]} -ne 0 ]; then
    echo "Retried: $(join_names "${retried_tests[@]}")"
else
    echo "Retried: none"
fi

if [ ${#failed_tests[@]} -ne 0 ]; then
    echo "Error: ${#failed_tests[@]} Fish test file(s) failed:"
    printf '   - %s\n' "${failed_tests[@]}"
    exit 1
fi

echo "All Fish tests passed!"
