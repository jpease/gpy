#!/usr/bin/env bash
set -euo pipefail

# Rust micro-benchmark budgets.
# Usage: ./scripts/bench.sh --ci  (cargo bench, enforcing tests/performance-baselines.json)
#
# BASELINE_FILE overrides the budget file. A missing jq, file or budget key
# exits 2 before anything is benchmarked.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGENT_DIR="$PROJECT_ROOT/gpy-agent"
# shellcheck source=scripts/lib/bench-common.sh
. "$SCRIPT_DIR/lib/bench-common.sh"

# check_budget OUTPUT BENCH_NAME BUDGET_NS METRIC_NAME
check_budget() {
  local output="$1"
  local bench_name="$2"
  local budget_ns="$3"
  local metric_name="$4"

  echo "--- Checking budget for: $metric_name ---"

  local result_line
  result_line=$(echo "$output" | grep "^$bench_name " | grep "time:" | head -n 1 || true)

  if [ -z "$result_line" ]; then
    echo "Warning: Could not find result for benchmark '$bench_name'."
    echo "Available benchmarks:"
    echo "$output" | grep -E "^test result:" || echo "No benchmark results found"
    return 1
  fi

  # "[low unit mid unit high unit]" -> mid and unit
  local time_and_unit time_val_float time_unit
  time_and_unit=$(echo "$result_line" | awk -F'time: *' '{print $2}' | awk -F'[][]' '{print $2}' | awk '{print $3, $4}')
  time_val_float=$(echo "$time_and_unit" | awk '{print $1}')
  time_unit=$(echo "$time_and_unit" | awk '{print $2}')

  local time_val_int
  time_val_int=$(printf "%.0f" "$time_val_float")
  local measured_ns

  case "$time_unit" in
    "ns") measured_ns=$time_val_int ;;
    "us"|"µs") measured_ns=$((time_val_int * 1000)) ;;
    "ms") measured_ns=$((time_val_int * 1000000)) ;;
    "s")  measured_ns=$((time_val_int * 1000000000)) ;;
    *) echo "Error: Unknown time unit '$time_unit' for metric '$metric_name'."; return 1 ;;
  esac

  local tolerance_multiplier=$((100 + TOLERANCE_PCT))
  local budget_with_tolerance=$((budget_ns * tolerance_multiplier / 100))

  echo "  - Budget: ${budget_ns} ns (~$((budget_ns / 1000000)) ms)"
  echo "  - Measured: ${measured_ns} ns (~$((measured_ns / 1000000)) ms)"
  echo "  - Budget with ${TOLERANCE_PCT}% tolerance: ${budget_with_tolerance} ns (~$((budget_with_tolerance / 1000000)) ms)"

  if [ "$measured_ns" -gt "$budget_with_tolerance" ]; then
    echo "Error: Performance budget for '$metric_name' exceeded!"
    return 1
  else
    echo "OK: '$metric_name' is within budget."
    return 0
  fi
}

if [[ " $* " == *" --ci "* ]]; then
  echo "--- Running benchmarks in CI mode ---"

  bench_load_baselines \
    BUDGET_IPC_RTT=.budgets.ipc_latency.max_ms*1000000 \
    BUDGET_GIT_COLD=.budgets.git_status_cold.max_ms*1000000 \
    BUDGET_CACHE_WRITE=.budgets.cache_write.max_ms*1000000 \
    BUDGET_CACHE_READ=.budgets.cache_read.max_ms*1000000 \
    BUDGET_CONFIG_LOAD=.budgets.config_validation.max_ms*1000000 \
    BUDGET_THEME_LOAD=.budgets.theme_manager_load.max_ms*1000000 \
    BUDGET_THEME_EXPORT=.budgets.theme_export_write.max_ms*1000000 \
    BUDGET_VERSION_SPAWN=.budgets.version_command_spawn.max_ms*1000000 \
    BUDGET_LANG_DETECT=.budgets.language_detection.max_ms*1000000 \
    TOLERANCE_PCT=.regression_thresholds.measurement_variance_percent*1
  echo "Loaded performance budgets from $BASELINE_FILE"

  bench_build_release_agent "$AGENT_DIR"

  cd "$AGENT_DIR"
  # A bench that fails to build or panics must fail --ci, even when the
  # benches that did run printed results within budget.
  BENCH_STATUS=0
  BENCH_OUTPUT=$(cargo bench --quiet 2>&1) || BENCH_STATUS=$?
  cd "$PROJECT_ROOT"

  echo -e "\n--- Benchmark Results ---"
  echo "$BENCH_OUTPUT"
  echo "-------------------------"

  # FAILURES=$((…)), not ((FAILURES++)): the latter returns 1 when FAILURES is
  # 0, and set -e would end the run before the other budgets and the summary.
  FAILURES=0

  if [[ $BENCH_STATUS -ne 0 ]]; then
    echo "Error: cargo bench exited with status $BENCH_STATUS"
    FAILURES=$((FAILURES + 1))
  fi

  check_budget "$BENCH_OUTPUT" "ipc_roundtrip" "$BUDGET_IPC_RTT" "IPC Roundtrip" || FAILURES=$((FAILURES + 1))
  check_budget "$BENCH_OUTPUT" "git_status_cold" "$BUDGET_GIT_COLD" "Git Status (cold)" || FAILURES=$((FAILURES + 1))
  check_budget "$BENCH_OUTPUT" "cache_write_new_entry" "$BUDGET_CACHE_WRITE" "Cache Write" || FAILURES=$((FAILURES + 1))
  check_budget "$BENCH_OUTPUT" "cache_read_hit" "$BUDGET_CACHE_READ" "Cache Read" || FAILURES=$((FAILURES + 1))

  # Cold-start canaries (gpy-agent/benches/agent_startup_bench.rs).
  check_budget "$BENCH_OUTPUT" "config_load_from_file" "$BUDGET_CONFIG_LOAD" "Config Load" || FAILURES=$((FAILURES + 1))
  check_budget "$BENCH_OUTPUT" "theme_manager_load" "$BUDGET_THEME_LOAD" "Theme Manager Load" || FAILURES=$((FAILURES + 1))
  check_budget "$BENCH_OUTPUT" "theme_export_write" "$BUDGET_THEME_EXPORT" "Theme Export Write" || FAILURES=$((FAILURES + 1))
  check_budget "$BENCH_OUTPUT" "version_command_spawn" "$BUDGET_VERSION_SPAWN" "Version Command Spawn" || FAILURES=$((FAILURES + 1))

  # 5k-file corpus (gpy-agent/benches/language_bench.rs).
  check_budget "$BENCH_OUTPUT" "lang_detect_5k" "$BUDGET_LANG_DETECT" "Language Detection (5k-file corpus)" || FAILURES=$((FAILURES + 1))

  if echo "$BENCH_OUTPUT" | grep -q "budget exceeded"; then
    echo "Error: Performance budget exceeded in benchmark output"
    FAILURES=$((FAILURES + 1))
  fi

  cat > "$PROJECT_ROOT/bench-results.json" << EOF
{
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "mode": "ci",
  "budgets": {
    "ipc_latency_ns": $BUDGET_IPC_RTT,
    "git_status_ns": $BUDGET_GIT_COLD,
    "cache_write_ns": $BUDGET_CACHE_WRITE,
    "cache_read_ns": $BUDGET_CACHE_READ,
    "config_load_ns": $BUDGET_CONFIG_LOAD,
    "theme_manager_load_ns": $BUDGET_THEME_LOAD,
    "theme_export_write_ns": $BUDGET_THEME_EXPORT,
    "version_command_spawn_ns": $BUDGET_VERSION_SPAWN,
    "language_detection_ns": $BUDGET_LANG_DETECT
  },
  "measurement_tolerance_percent": $TOLERANCE_PCT,
  "failures": $FAILURES,
  "output": $(echo "$BENCH_OUTPUT" | jq -R -s .)
}
EOF

  echo -e "\nBenchmark summary written to bench-results.json"

  if [[ $FAILURES -gt 0 ]]; then
    echo -e "\nError: $FAILURES benchmark(s) exceeded performance budget"
    exit 1
  else
    echo -e "\nAll benchmarks are within budget"
    exit 0
  fi
fi

echo "Usage: ./scripts/bench.sh --ci" >&2
exit 2
