#!/usr/bin/env bash
set -euo pipefail

# Rust micro-benchmark budgets.
# Usage: ./scripts/bench.sh --ci  (cargo bench, enforcing tests/performance-baselines.json)

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

  local tolerance_multiplier=$((100 + ${TOLERANCE_PCT:-15}))
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

  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
  AGENT_DIR="$PROJECT_ROOT/gpy-agent"

  if [[ ! -f "$AGENT_DIR/target/release/gpy-agent" ]]; then
    echo "Building release agent for CI benchmarks..."
    cd "$AGENT_DIR"
    cargo build --release
    cd "$PROJECT_ROOT"
  fi

  cd "$AGENT_DIR"
  BENCH_OUTPUT=$(cargo bench --workspace --quiet 2>&1 || true)
  cd "$PROJECT_ROOT"

  echo -e "\n--- Benchmark Results ---"
  echo "$BENCH_OUTPUT"
  echo "-------------------------"

  BASELINE_FILE="$PROJECT_ROOT/tests/performance-baselines.json"

  if [[ -f "$BASELINE_FILE" ]]; then
    echo "Loading performance budgets from $BASELINE_FILE"

    BUDGET_IPC_RTT=$(jq -r '.budgets.ipc_latency.max_ms * 1000000' "$BASELINE_FILE" 2>/dev/null || echo "1500000")
    BUDGET_GIT_COLD=$(jq -r '.budgets.git_status_cold.max_ms * 1000000' "$BASELINE_FILE" 2>/dev/null || echo "750000000")
    BUDGET_CACHE_OPS=$(jq -r '.budgets.cache_write.max_ms * 1000000' "$BASELINE_FILE" 2>/dev/null || echo "25000000")
    BUDGET_MEMORY_MB=$(jq -r '.budgets.memory_usage.max_mb' "$BASELINE_FILE" 2>/dev/null || echo "50")

    BUDGET_CONFIG_LOAD=$(jq -r '.budgets.config_validation.max_ms * 1000000' "$BASELINE_FILE" 2>/dev/null || echo "100000000")
    BUDGET_THEME_LOAD=$(jq -r '.budgets.theme_manager_load.max_ms * 1000000' "$BASELINE_FILE" 2>/dev/null || echo "20000000")
    BUDGET_THEME_EXPORT=$(jq -r '.budgets.theme_export_write.max_ms * 1000000' "$BASELINE_FILE" 2>/dev/null || echo "30000000")
    BUDGET_VERSION_SPAWN=$(jq -r '.budgets.version_command_spawn.max_ms * 1000000' "$BASELINE_FILE" 2>/dev/null || echo "50000000")

    BUDGET_LANG_DETECT=$(jq -r '.budgets.language_detection.max_ms * 1000000' "$BASELINE_FILE" 2>/dev/null || echo "200000000")

    MAJOR_REGRESSION_PCT=$(jq -r '.regression_thresholds.major_regression_percent' "$BASELINE_FILE" 2>/dev/null || echo "50")
    MAJOR_REGRESSION_PCT=$(printf "%.0f" "$MAJOR_REGRESSION_PCT")

    TOLERANCE_PCT=$(jq -r '.regression_thresholds.measurement_variance_percent' "$BASELINE_FILE" 2>/dev/null || echo "15")
    TOLERANCE_PCT=$(printf "%.0f" "$TOLERANCE_PCT")

    echo "  - IPC Latency: ${BUDGET_IPC_RTT} ns (~$((BUDGET_IPC_RTT / 1000000)) ms)"
    echo "  - Git Status (cold): ${BUDGET_GIT_COLD} ns (~$((BUDGET_GIT_COLD / 1000000)) ms)"
    echo "  - Cache Operations: ${BUDGET_CACHE_OPS} ns (~$((BUDGET_CACHE_OPS / 1000000)) ms)"
    echo "  - Memory Usage: ${BUDGET_MEMORY_MB} MB"
    echo "  - Config Load: ${BUDGET_CONFIG_LOAD} ns (~$((BUDGET_CONFIG_LOAD / 1000000)) ms)"
    echo "  - Theme Manager Load: ${BUDGET_THEME_LOAD} ns (~$((BUDGET_THEME_LOAD / 1000000)) ms)"
    echo "  - Theme Export Write: ${BUDGET_THEME_EXPORT} ns (~$((BUDGET_THEME_EXPORT / 1000000)) ms)"
    echo "  - Version Command Spawn: ${BUDGET_VERSION_SPAWN} ns (~$((BUDGET_VERSION_SPAWN / 1000000)) ms)"
    echo "  - Language Detection: ${BUDGET_LANG_DETECT} ns (~$((BUDGET_LANG_DETECT / 1000000)) ms)"
  else
    echo "Warning: performance-baselines.json not found at $BASELINE_FILE, using defaults"
    BUDGET_IPC_RTT=1500000      # 1.5ms
    BUDGET_GIT_COLD=750000000   # 750ms
    BUDGET_CACHE_OPS=25000000   # 25ms
    BUDGET_MEMORY_MB=50
    BUDGET_CONFIG_LOAD=100000000  # 100ms
    BUDGET_THEME_LOAD=20000000    # 20ms
    BUDGET_THEME_EXPORT=30000000  # 30ms
    BUDGET_VERSION_SPAWN=50000000 # 50ms
    BUDGET_LANG_DETECT=200000000  # 200ms
    MAJOR_REGRESSION_PCT=50
    TOLERANCE_PCT=15
  fi

  FAILURES=0

  check_budget "$BENCH_OUTPUT" "ipc_roundtrip" "$BUDGET_IPC_RTT" "IPC Roundtrip" || ((FAILURES++))
  check_budget "$BENCH_OUTPUT" "git_status_cold" "$BUDGET_GIT_COLD" "Git Status (cold)" || ((FAILURES++))
  check_budget "$BENCH_OUTPUT" "cache_write_new_entry" "$BUDGET_CACHE_OPS" "Cache Operations" || ((FAILURES++))

  # Cold-start canaries (gpy-agent/benches/agent_startup_bench.rs).
  check_budget "$BENCH_OUTPUT" "config_load_from_file" "$BUDGET_CONFIG_LOAD" "Config Load" || ((FAILURES++))
  check_budget "$BENCH_OUTPUT" "theme_manager_load" "$BUDGET_THEME_LOAD" "Theme Manager Load" || ((FAILURES++))
  check_budget "$BENCH_OUTPUT" "theme_export_write" "$BUDGET_THEME_EXPORT" "Theme Export Write" || ((FAILURES++))
  check_budget "$BENCH_OUTPUT" "version_command_spawn" "$BUDGET_VERSION_SPAWN" "Version Command Spawn" || ((FAILURES++))

  # 5k-file corpus (gpy-agent/benches/language_bench.rs).
  check_budget "$BENCH_OUTPUT" "lang_detect_5k" "$BUDGET_LANG_DETECT" "Language Detection (5k-file corpus)" || ((FAILURES++))

  if echo "$BENCH_OUTPUT" | grep -q "budget exceeded"; then
    echo "Error: Performance budget exceeded in benchmark output"
    ((FAILURES++))
  fi

  cat > "$PROJECT_ROOT/bench-results.json" << EOF
{
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "mode": "ci",
  "budgets": {
    "ipc_latency_ns": $BUDGET_IPC_RTT,
    "git_status_ns": $BUDGET_GIT_COLD,
    "cache_ops_ns": $BUDGET_CACHE_OPS,
    "memory_mb": $BUDGET_MEMORY_MB,
    "config_load_ns": $BUDGET_CONFIG_LOAD,
    "theme_manager_load_ns": $BUDGET_THEME_LOAD,
    "theme_export_write_ns": $BUDGET_THEME_EXPORT,
    "version_command_spawn_ns": $BUDGET_VERSION_SPAWN,
    "language_detection_ns": $BUDGET_LANG_DETECT
  },
  "thresholds": {
    "major_regression_percent": $MAJOR_REGRESSION_PCT,
    "measurement_tolerance_percent": $TOLERANCE_PCT
  },
  "failures": $FAILURES,
  "baseline_file_used": "$([ -f "$BASELINE_FILE" ] && echo "true" || echo "false")",
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
