#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later

# This script runs the Fish integration tests.

set -eo pipefail

# Drop any inherited repo-scoping git env (GIT_DIR/GIT_WORK_TREE/...) so the
# fish tests' throwaway git fixtures resolve their own repos. Left set — e.g.
# when this runs from a git pre-push hook — they would redirect the tests'
# `git init`/`git commit`/`git worktree add` at THIS repo (#275).
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
    GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
    GIT_PREFIX GIT_NAMESPACE 2>/dev/null || true

echo "Running Fish integration tests..."
echo

# Ensure gpy-agent binary exists for tests that depend on theme export.
AGENT_BIN="gpy-agent/target/debug/gpy-agent"
if [ ! -x "$AGENT_BIN" ]; then
    echo "Building gpy-agent debug binary for Fish tests..."
    (cd gpy-agent && cargo build --quiet)
fi

if [ ! -x "$AGENT_BIN" ]; then
    echo "❌ gpy-agent binary not found at $AGENT_BIN after build"
    exit 1
fi

# Tests resolve `gpy-agent` from PATH. Force the freshly-built binary so the
# suite always exercises THIS checkout, not a stale binary the user may have
# installed in fish_user_paths (e.g. ~/.local/bin).
AGENT_DIR="$(cd "$(dirname "$AGENT_BIN")" && pwd)"

# Retry policy (#650, mirroring gpy-agent#620 for nextest): no blanket
# retries. A test may be retried only by a named entry here of the form
# `<file>.test.fish:#<open issue>`, so a retry is always traceable to an
# investigation rather than absorbing a regression silently. The list is
# empty: the entries it used to carry cited #381, which is closed. Every
# retry that does happen is named in the `Retried:` tally at the end of the
# run, and tests/bash/nextest_retry_policy.test.bash enforces the format.
KNOWN_FLAKY_E2E_TESTS=""

# Attempts for a test file: 3 when a policy entry names it, else 1.
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

# Run standalone fish test scripts. Collect ALL failures rather than stopping
# at the first, so one run surfaces the complete failure set (stop-on-first
# masked stacked failures where each fix only revealed the next).
failed_tests=()
skipped_tests=()
retried_tests=()
run_log="$(mktemp "${TMPDIR:-/tmp}/gpy-fish-test.XXXXXX")"
trap 'rm -f "$run_log"' EXIT
for test_file in tests/fish/*.test.fish; do
    if [ -f "$test_file" ]; then
        base="$(basename "$test_file")"
        echo "=== Running $base ==="

        attempts="$(attempts_for "$base")"

        attempt=1
        passed=0
        while [ "$attempt" -le "$attempts" ]; do
            # --no-config: skip the developer's own config.fish/conf.d. Every
            # test already sources the tree's own fish/core/*.fish explicitly,
            # so nothing here depends on it -- but a real config.fish (mise
            # activation, a locally-installed gpy init, GPY_* overrides in
            # config.toml) otherwise leaks into this process and silently
            # changes exported vars (e.g. GPY_LANGUAGE_ENABLED) and PATH
            # ordering out from under the tests (#630). fish still inherits
            # this process's exported PATH either way, so tools resolved via
            # PATH (cargo, jq, hyperfine, mise shims, ...) are unaffected.
            # Streamed AND captured (pipefail keeps fish's own status) so a
            # `SKIP:` line from the shared skip contract can be tallied.
            # The paths travel as $argv, never interpolated into the code
            # string (a checkout path may contain spaces or quotes, #822); the
            # single quotes are deliberate, fish expands $argv, not this shell.
            # shellcheck disable=SC2016
            if fish --no-config -c 'set -gx PATH $argv[1] $PATH; source $argv[2]' -- "$AGENT_DIR" "$test_file" 2>&1 | tee "$run_log"; then
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
                echo "⚠️  $base failed on attempt $attempt/$attempts (named in the retry policy) — retrying..."
            fi
            attempt=$((attempt + 1))
        done

        if [ "$passed" -eq 1 ]; then
            echo
        else
            failed_tests+=("$base")
            echo "❌ $base FAILED after $attempts attempt(s)"
            echo
        fi
    fi
done

# Tallies (#650): what did not execute and what needed a second try.
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
    echo "❌ ${#failed_tests[@]} Fish test file(s) failed:"
    printf '   - %s\n' "${failed_tests[@]}"
    exit 1
fi

echo "✅ All Fish tests passed!"
