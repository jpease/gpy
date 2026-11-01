#!/usr/bin/env bash
# tests/repo/file_size_gate.test.bash
#
# The 800-line cap applies to test files too: `tests/` targets, `tests.rs` and
# `*_tests.rs`. Exercises scripts/check-file-size.py against scratch git repos,
# so the gate itself is tested.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECKER="$ROOT/scripts/check-file-size.py"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command python3
test_require_command git

# Scratch repo holding the checker, an optional allowlist ($1: text, or empty
# for none), and `lines` lines in each given path. Echoes the repo path.
make_repo() {
    local allowlist="$1" lines="$2"
    shift 2
    local repo
    repo="$(mktemp -d "$WORKDIR/repo.XXXXXX")"
    mkdir -p "$repo/scripts"
    cp "$CHECKER" "$repo/scripts/check-file-size.py"
    local path
    for path in "$@"; do
        mkdir -p "$repo/$(dirname "$path")"
        seq 1 "$lines" | sed 's/.*/\/\/ line/' >"$repo/$path"
    done
    if [[ -n "$allowlist" ]]; then
        printf '%s\n' "$allowlist" >"$repo/scripts/file-size-allowlist.txt"
    fi
    git -C "$repo" init -q
    git -C "$repo" add -A
    printf '%s' "$repo"
}

# Runs the checker in $1; sets CHECK_STATUS and CHECK_OUTPUT.
run_checker() {
    CHECK_STATUS=0
    CHECK_OUTPUT="$(cd "$1" && python3 scripts/check-file-size.py 2>&1)" || CHECK_STATUS=$?
}

expect_over_cap() {
    local label="$1" path="$2" repo
    repo="$(make_repo "" 801 "$path")"
    run_checker "$repo"
    [[ "$CHECK_STATUS" -eq 1 ]] || fail "$label: exit $CHECK_STATUS, expected 1"
    [[ "$CHECK_OUTPUT" == *"$path"* ]] || fail "$label: output does not name $path: $CHECK_OUTPUT"
    [[ "$CHECK_STATUS" -eq 1 && "$CHECK_OUTPUT" == *"$path"* ]] && pass "$label"
}

expect_listed_clean() {
    local label="$1" path="$2" repo
    repo="$(make_repo "801 $path" 801 "$path")"
    run_checker "$repo"
    [[ "$CHECK_STATUS" -eq 0 ]] || fail "$label: exit $CHECK_STATUS, expected 0: $CHECK_OUTPUT"
    [[ "$CHECK_STATUS" -eq 0 ]] && pass "$label"
}

expect_growth_rejected() {
    local label="$1" path="$2" repo
    repo="$(make_repo "801 $path" 802 "$path")"
    run_checker "$repo"
    [[ "$CHECK_STATUS" -eq 1 ]] || fail "$label: exit $CHECK_STATUS, expected 1"
    [[ "$CHECK_OUTPUT" == *"grew to"* ]] || fail "$label: output lacks 'grew to': $CHECK_OUTPUT"
    [[ "$CHECK_STATUS" -eq 1 && "$CHECK_OUTPUT" == *"grew to"* ]] && pass "$label"
}

echo "--- an over-cap test file must be flagged ---"
expect_over_cap "tests/*_tests.rs" "gpy-agent/tests/foo_tests.rs"
expect_over_cap "src tests.rs" "gpy-agent/src/x/tests.rs"
expect_over_cap "src *_tests.rs" "gpy-agent/src/x/y_tests.rs"
expect_over_cap "benches" "gpy-agent/benches/b.rs"
expect_over_cap "examples" "gpy-agent/examples/e.rs"
expect_over_cap "tests/ directory target" "gpy-agent/tests/foo_tests/main.rs"

echo "--- listing the file at its size must pass ---"
expect_listed_clean "tests/*_tests.rs listed" "gpy-agent/tests/foo_tests.rs"
expect_listed_clean "src tests.rs listed" "gpy-agent/src/x/tests.rs"

echo "--- a listed file that grows must fail ---"
expect_growth_rejected "tests/*_tests.rs grew" "gpy-agent/tests/foo_tests.rs"
expect_growth_rejected "src tests.rs grew" "gpy-agent/src/x/tests.rs"

echo "--- a file at the cap must pass ---"
repo="$(make_repo "" 800 gpy-agent/tests/foo_tests.rs gpy-agent/src/x/tests.rs)"
run_checker "$repo"
if [[ "$CHECK_STATUS" -eq 0 ]]; then
    pass "800-line test files"
else
    fail "800-line test files: exit $CHECK_STATUS: $CHECK_OUTPUT"
fi

test_finish "file-size gate covers test files and the allowlist ratchet"
