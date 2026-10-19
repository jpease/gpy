#!/usr/bin/env bash
# tests/bash/bench_ci_exit_status.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# scripts/bench.sh --ci fails when `cargo bench` fails, even when the output
# it did print has every budgeted benchmark within budget, and still writes
# its summary. A stub cargo stands in for the real one, so nothing is built.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command jq

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# A copy of the tree bench.sh reads, so its bench-results.json lands in $TMP.
mkdir -p "$TMP/tree/scripts" "$TMP/tree/tests" "$TMP/tree/gpy-agent/target/release" "$TMP/bin"
cp "$ROOT/scripts/bench.sh" "$TMP/tree/scripts/"
cp "$ROOT/tests/performance-baselines.json" "$TMP/tree/tests/"
touch "$TMP/tree/gpy-agent/target/release/gpy-agent"

# Every budgeted benchmark at 1 ns, then the exit status the test asks for.
cat >"$TMP/bin/cargo" <<'EOF'
#!/usr/bin/env bash
[[ "$1" == bench ]] || exit 0
for name in ipc_roundtrip git_status_cold cache_write_new_entry config_load_from_file \
    theme_manager_load theme_export_write version_command_spawn lang_detect_5k; do
    printf '%s time:   [1.0000 ns 1.0000 ns 1.0000 ns]\n' "$name"
done
exit "$STUB_CARGO_STATUS"
EOF
chmod +x "$TMP/bin/cargo"

run_bench() {
    rm -f "$TMP/tree/bench-results.json"
    STUB_CARGO_STATUS="$1" PATH="$TMP/bin:$PATH" bash "$TMP/tree/scripts/bench.sh" --ci >"$TMP/out" 2>&1
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

test_finish "bench.sh --ci propagates cargo bench failures"
