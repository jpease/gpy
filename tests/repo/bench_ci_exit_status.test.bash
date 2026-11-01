#!/usr/bin/env bash
# tests/repo/bench_ci_exit_status.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# scripts/bench.sh --ci fails when `cargo bench` fails, even when the output
# it did print has every budgeted benchmark within budget, and still writes
# its summary. It rebuilds the release agent first, lists the benchmark IDs
# before measuring and fails when two bench targets register the same one,
# runs cargo bench without `--workspace` (there is no workspace), and exits 2 before benchmarking when
# jq, the budget file or a budget key is missing. A stub cargo stands in for
# the real one, so nothing is built.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command jq

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# A copy of the tree bench.sh reads, so its bench-results.json lands in $TMP.
mkdir -p "$TMP/tree/scripts/lib" "$TMP/tree/benchmarks" "$TMP/tree/gpy-agent/target/release" "$TMP/bin"
cp "$ROOT/scripts/bench.sh" "$TMP/tree/scripts/"
cp "$ROOT/scripts/lib/bench-common.sh" "$TMP/tree/scripts/lib/"
cp "$ROOT/benchmarks/perf-budgets.json" "$TMP/tree/benchmarks/"
touch "$TMP/tree/gpy-agent/target/release/gpy-agent"

# Logs its arguments. `bench -- --list` prints only the IDs in STUB_BENCH_LIST
# (default: the nine budgeted ones, each once) and exits 0. Any other `bench`
# prints every budgeted benchmark at 1 ns and exits with the status the test
# asks for.
cat >"$TMP/bin/cargo" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$STUB_CARGO_LOG"
[[ "$1" == bench ]] || exit 0
budgeted="ipc_roundtrip git_status_cold cache_write_new_entry cache_read_hit config_load_from_file
    theme_manager_load theme_export_write version_command_spawn lang_detect_5k"
if [[ " $* " == *" --list "* ]]; then
    for name in ${STUB_BENCH_LIST:-$budgeted}; do
        printf '%s: benchmark\n' "$name"
    done
    exit 0
fi
for name in $budgeted; do
    printf '%s time:   [1.0000 ns 1.0000 ns 1.0000 ns]\n' "$name"
done
exit "$STUB_CARGO_STATUS"
EOF
chmod +x "$TMP/bin/cargo"

run_bench() {
    rm -f "$TMP/tree/bench-results.json" "$TMP/cargo.log"
    STUB_BENCH_LIST="${STUB_BENCH_LIST:-}" STUB_CARGO_LOG="$TMP/cargo.log" STUB_CARGO_STATUS="$1" PATH="$TMP/bin:$PATH" \
        bash "$TMP/tree/scripts/bench.sh" --ci >"$TMP/out" 2>&1
}

if run_bench 0; then
    pass "--ci passes when cargo bench succeeds within budget"
else
    fail "--ci failed with every benchmark within budget: $(tail -n 20 "$TMP/out")"
fi

if run_bench 101; then
    fail "--ci passed although cargo bench exited 101"
else
    pass "--ci fails when cargo bench fails"
fi
if grep -q 'cargo bench exited with status 101' "$TMP/out"; then
    pass "--ci names cargo bench's exit status"
else
    fail "--ci does not report cargo bench's exit status: $(tail -n 20 "$TMP/out")"
fi
if [[ "$(jq -r .failures "$TMP/tree/bench-results.json" 2>/dev/null)" == 1 ]]; then
    pass "the summary still records the failure"
else
    fail "bench-results.json does not record exactly one failure"
fi


run_bench 0
if [[ "$(head -n 1 "$TMP/cargo.log")" == "build --release --quiet" ]]; then
    pass "the release agent is rebuilt before benchmarking"
else
    fail "bench.sh did not run cargo build --release first: $(cat "$TMP/cargo.log")"
fi
if [[ "$(grep -cx 'bench --quiet -- --list' "$TMP/cargo.log")" == 1 ]]; then
    pass "the benchmark IDs are listed once"
else
    fail "cargo bench --list did not run exactly once: $(cat "$TMP/cargo.log")"
fi
if grep -qx 'bench --quiet' "$TMP/cargo.log"; then
    pass "cargo bench runs without --workspace"
else
    fail "unexpected cargo bench arguments: $(cat "$TMP/cargo.log")"
fi
build_line="$(grep -nx 'build --release --quiet' "$TMP/cargo.log" | head -n 1 | cut -d: -f1)"
list_line="$(grep -nx 'bench --quiet -- --list' "$TMP/cargo.log" | head -n 1 | cut -d: -f1)"
bench_line="$(grep -nx 'bench --quiet' "$TMP/cargo.log" | head -n 1 | cut -d: -f1)"
if [[ -n "$build_line" && -n "$list_line" && -n "$bench_line" ]] \
    && ((build_line < list_line && list_line < bench_line)); then
    pass "the IDs are listed after the build and before measuring"
else
    fail "unexpected cargo order: $(cat "$TMP/cargo.log")"
fi

dup_status=0
STUB_BENCH_LIST="git_status_cold ipc_roundtrip git_status_cold" run_bench 0 || dup_status=$?
if [[ "$dup_status" -eq 1 ]]; then
    pass "a duplicate benchmark id fails --ci"
else
    fail "--ci exit status $dup_status with a duplicate id, expected 1: $(tail -n 20 "$TMP/out")"
fi
if grep -qF 'registered by more than one bench target: git_status_cold' "$TMP/out"; then
    pass "--ci names the duplicate id"
else
    fail "--ci does not name the duplicate id: $(tail -n 20 "$TMP/out")"
fi
if [[ "$(jq -r .failures "$TMP/tree/bench-results.json" 2>/dev/null)" == 1 ]]; then
    pass "the summary still records the duplicate-id failure"
else
    fail "bench-results.json does not record exactly one failure for the duplicate id"
fi

# expect_config_error LABEL EXPECTED_TEXT [VAR=VALUE...]: bench.sh exits 2,
# names the problem and never reaches cargo.
expect_config_error() {
    local label="$1" text="$2" status=0
    shift 2
    rm -f "$TMP/cargo.log"
    env STUB_CARGO_LOG="$TMP/cargo.log" STUB_CARGO_STATUS=0 "$@" \
        bash "$TMP/tree/scripts/bench.sh" --ci >"$TMP/out" 2>&1 || status=$?
    if [[ "$status" -eq 2 ]]; then
        pass "$label: exits 2"
    else
        fail "$label: exit status $status, expected 2: $(tail -n 5 "$TMP/out")"
    fi
    if grep -qF -- "$text" "$TMP/out"; then
        pass "$label: names the problem"
    else
        fail "$label: output does not contain '$text': $(tail -n 5 "$TMP/out")"
    fi
    if [[ -e "$TMP/cargo.log" ]]; then
        fail "$label: ran cargo: $(cat "$TMP/cargo.log")"
    fi
}

expect_config_error "a missing budget file" "budget file not found: /nonexistent" \
    PATH="$TMP/bin:$PATH" PERF_BUDGETS_FILE=/nonexistent

jq 'del(.budgets.theme_export_write)' "$ROOT/benchmarks/perf-budgets.json" >"$TMP/no-key.json"
expect_config_error "a missing budget key" ".budgets.theme_export_write.max_ms is missing or not a number" \
    PATH="$TMP/bin:$PATH" PERF_BUDGETS_FILE="$TMP/no-key.json"

jq '.budgets.ipc_latency.max_ms = "fast"' "$ROOT/benchmarks/perf-budgets.json" >"$TMP/bad-type.json"
expect_config_error "a non-numeric budget" ".budgets.ipc_latency.max_ms is missing or not a number" \
    PATH="$TMP/bin:$PATH" PERF_BUDGETS_FILE="$TMP/bad-type.json"

printf '{ not json' >"$TMP/bad.json"
expect_config_error "an unparseable budget file" "cannot read the budgets" \
    PATH="$TMP/bin:$PATH" PERF_BUDGETS_FILE="$TMP/bad.json"

# Only the tools bench.sh needs before it reads the budgets, so jq is absent.
mkdir -p "$TMP/nojq"
for tool in bash env dirname; do
    ln -s "$(command -v "$tool")" "$TMP/nojq/$tool"
done
expect_config_error "a missing jq" "jq is required" PATH="$TMP/nojq"

test_finish "bench.sh --ci propagates cargo bench failures and unreadable budgets"
