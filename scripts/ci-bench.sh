#!/usr/bin/env bash
# End-to-end prompt-latency and agent-memory benchmark for GPY.
#
# This benchmark complements scripts/bench.sh --ci (Rust micro-benchmarks). It
# measures the *user-visible* path: the real Fish `fish_prompt` implementation
# and the live `gpy-agent` daemon, across the scenarios that matter for prompt
# performance and resilience.
#
# Usage:
#   ./scripts/ci-bench.sh           # Local run: budget overages are warnings
#   ./scripts/ci-bench.sh --ci      # CI run: budget overages fail the build
#
# Scenarios:
#   warm prompt         agent up, instant cache fresh
#   cold cache          agent up, instant cache cleared before each render
#   stale cache         agent up, instant cache aged (serve-stale path)
#   agent unavailable   daemon stopped (graceful fallback path)
#   large repository    cold git scan on a large synthetic repo
#   long-running RSS    daemon memory under sustained prompt load
#
# Integrity guarantee: a broken measurement (non-zero exit, empty prompt
# output, unparseable timing, zero/empty RSS, or an unresponsive daemon) is a
# HARD failure in every mode. It can never be silently reported as a passing
# zero or empty measurement. Only a *valid* measurement that exceeds its budget
# is downgraded to a warning in local mode.

set -uo pipefail

# --- Configuration -----------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGENT_DIR="$PROJECT_ROOT/gpy-agent"
AGENT_BIN="$AGENT_DIR/target/release/gpy-agent"
BASELINE_FILE="$PROJECT_ROOT/tests/performance-baselines.json"
RESULTS_DIR="$PROJECT_ROOT/bench-results"
FISH_FUNCS="$PROJECT_ROOT/fish/functions"
FISH_CORE="$PROJECT_ROOT/fish/core/init.fish"

# Isolated benchmark sandbox so we never touch the user's real cache, socket,
# or repositories. Cleaned up on exit.
BENCH_TMP="$(mktemp -d "${TMPDIR:-/tmp}/gpy-ci-bench.XXXXXX")"
BENCH_CACHE="$BENCH_TMP/cache"
BENCH_SOCKET="$BENCH_TMP/gpy-test-cibench.sock"
INSTANT_CACHE_DIR="$BENCH_CACHE/gpy/instant-prompts"
LARGE_REPO="$BENCH_TMP/large-repo"
WARMUP_RUNS="${GPY_BENCH_WARMUP:-3}"
TIMED_RUNS="${GPY_BENCH_RUNS:-8}"
# Number of in-process prompt renders per timed run. The reported latency is the
# amortized per-render cost (total / renders), which isolates prompt-render work
# from one-time fish startup and GPY init. macOS `date` lacks %N, so portable
# high-resolution per-render timing is only achievable by amortizing a loop.
RENDER_ITERS="${GPY_BENCH_RENDER_ITERS:-100}"
LONG_RUN_ITERS="${GPY_BENCH_LONG_ITERS:-40}"

CI_MODE=false
if [[ " ${*:-} " == *" --ci "* ]] || [[ "${CI:-}" == "true" ]]; then
  CI_MODE=true
fi

# --- Output helpers ----------------------------------------------------------

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
pass()    { echo -e "${GREEN}[PASS]${NC} $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
fail()    { echo -e "${RED}[FAIL]${NC} $1"; }

# Failure accounting. HARD failures are always fatal (broken measurement).
# BUDGET failures are fatal only in CI mode (a valid but slow measurement).
HARD_FAILURES=0
BUDGET_FAILURES=0
REPORT_ROWS=()

record_hard() { HARD_FAILURES=$((HARD_FAILURES + 1)); fail "$1"; }
record_budget() {
  BUDGET_FAILURES=$((BUDGET_FAILURES + 1))
  if [[ "$CI_MODE" == true ]]; then
    fail "$1"
  else
    warn "$1 (warning in local mode)"
  fi
}

# --- Cleanup -----------------------------------------------------------------

cleanup() {
  stop_daemon >/dev/null 2>&1 || true
  rm -rf "$BENCH_TMP" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# --- Dependency checks -------------------------------------------------------

require_tools() {
  local missing=()
  command -v jq >/dev/null 2>&1 || missing+=("jq")
  command -v hyperfine >/dev/null 2>&1 || missing+=("hyperfine")
  command -v fish >/dev/null 2>&1 || missing+=("fish")
  command -v git >/dev/null 2>&1 || missing+=("git")

  if [[ ${#missing[@]} -gt 0 ]]; then
    if [[ "$CI_MODE" == true ]]; then
      record_hard "Required tools missing in CI: ${missing[*]}"
      return 1
    fi
    warn "Skipping ci-bench: missing tools (${missing[*]}). Install them to run locally."
    exit 0
  fi
  return 0
}

ensure_agent_built() {
  if [[ ! -x "$AGENT_BIN" ]]; then
    info "Building release agent for benchmarks..."
    if ! (cd "$AGENT_DIR" && RUSTC_WRAPPER="" cargo build --release --quiet); then
      record_hard "Failed to build release agent"
      return 1
    fi
  fi
  return 0
}

# --- Budget loading ----------------------------------------------------------

# budget_ms KEY -> prints the max_ms budget, or empty if undefined.
budget_ms() { jq -r ".budgets.${1}.max_ms // empty" "$BASELINE_FILE" 2>/dev/null; }
# budget_mb KEY -> prints the max_mb budget, or empty if undefined.
budget_mb() { jq -r ".budgets.${1}.max_mb // empty" "$BASELINE_FILE" 2>/dev/null; }

TOLERANCE_PCT="$(jq -r '.regression_thresholds.measurement_variance_percent // 15' "$BASELINE_FILE" 2>/dev/null || echo 15)"
TOLERANCE_PCT="$(printf '%.0f' "$TOLERANCE_PCT")"

# within_budget MEASURED BUDGET -> 0 if measured <= budget*(1+tolerance)
within_budget() {
  local measured="$1" budget="$2"
  awk -v m="$measured" -v b="$budget" -v t="$TOLERANCE_PCT" \
    'BEGIN { exit !(m <= b * (1 + t / 100)) }'
}

# --- Daemon lifecycle --------------------------------------------------------

daemon_pid() { pgrep -f "gpy-agent start --socket ${BENCH_SOCKET}" 2>/dev/null | head -1; }

start_daemon() {
  XDG_CACHE_HOME="$BENCH_CACHE" "$AGENT_BIN" start --socket "$BENCH_SOCKET" >/dev/null 2>&1

  # Wait for the socket to become responsive rather than guessing with sleep.
  for _ in $(seq 1 50); do
    if XDG_CACHE_HOME="$BENCH_CACHE" "$AGENT_BIN" status --socket "$BENCH_SOCKET" 2>/dev/null \
        | grep -q "Running and Responding"; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

stop_daemon() {
  XDG_CACHE_HOME="$BENCH_CACHE" "$AGENT_BIN" stop --socket "$BENCH_SOCKET" >/dev/null 2>&1 || true
  for _ in $(seq 1 30); do
    [[ -z "$(daemon_pid)" ]] && return 0
    sleep 0.1
  done
  # Force kill if it refused to exit, so the next scenario starts clean.
  local pid; pid="$(daemon_pid)"
  [[ -n "$pid" ]] && kill -9 "$pid" 2>/dev/null
  return 0
}

# --- Prompt command builder --------------------------------------------------

# Fish setup that loads THIS checkout's prompt for directory $1.
# fish_function_path is prepended with the repo's functions dir so we always
# measure this checkout's fish_prompt, never a globally installed copy.
# --no-config isolates the run from the user's conf.d/config.fish.
prompt_setup() {
  local target="$1"
  # The single quotes are the point: this is a Fish snippet assembled here and
  # evaluated by Fish later, so $PATH and $fish_function_path must reach it
  # unexpanded.
  # shellcheck disable=SC2016
  printf 'set -gx PATH "%s/target/release" $PATH; set -gx XDG_CACHE_HOME "%s"; set -gx fish_function_path "%s" $fish_function_path; builtin cd "%s"; source "%s" >/dev/null 2>&1' \
    "$AGENT_DIR" "$BENCH_CACHE" "$FISH_FUNCS" "$target" "$FISH_CORE"
}

# Single-render command (used for preflight output validation).
prompt_command() {
  printf "fish --no-config -c '%s; fish_prompt'" "$(prompt_setup "$1")"
}

# Looped render command for timing: runs $2 renders, optionally running the
# Fish snippet $3 before each render to put the cache in the desired state.
prompt_loop_command() {
  local target="$1" iters="$2" prep="${3:-}"
  printf "fish --no-config -c '%s; for __i in (seq %s); %sfish_prompt >/dev/null; end'" \
    "$(prompt_setup "$target")" "$iters" "${prep:+$prep; }"
}

# --- Measurement helpers -----------------------------------------------------

# preflight_render CMD -> 0 if the command exits 0 and prints non-empty output.
# This is what makes a silent empty/zero measurement impossible: a render that
# produces nothing is a hard failure before hyperfine ever runs.
preflight_render() {
  local cmd="$1" out rc
  out="$(eval "$cmd" 2>/dev/null)"; rc=$?
  if [[ $rc -ne 0 ]]; then
    return 1
  fi
  [[ -n "$out" ]]
}

# run_latency LABEL BUDGET_MS TIMED_CMD ITERS PREFLIGHT_CMD
# Times TIMED_CMD with hyperfine, divides by ITERS to get per-render latency,
# validates the result, and checks it against budget. PREFLIGHT_CMD is run once
# up front; it must exit 0 and produce non-empty output, which is what makes a
# silent empty/zero measurement impossible.
run_latency() {
  local label="$1" budget="$2" cmd="$3" iters="${4:-1}" preflight="${5:-$3}"
  info "Scenario: $label"

  if [[ -z "$budget" ]]; then
    record_hard "$label: no budget defined in $(basename "$BASELINE_FILE")"
    return 1
  fi

  if ! preflight_render "$preflight"; then
    record_hard "$label: command failed or produced empty output"
    return 1
  fi

  local json
  json="$RESULTS_DIR/$(echo "$label" | tr ' /' '__').json"
  if ! hyperfine --warmup "$WARMUP_RUNS" --runs "$TIMED_RUNS" \
      --export-json "$json" --command-name "$label" --shell sh \
      "$cmd" >/dev/null 2>&1; then
    record_hard "$label: hyperfine reported a command failure"
    return 1
  fi

  local mean_s mean_ms
  mean_s="$(jq -r '.results[0].mean // empty' "$json" 2>/dev/null)"
  if [[ -z "$mean_s" ]]; then
    record_hard "$label: could not parse timing result"
    return 1
  fi
  # Per-render latency: total run time divided by the in-process render count.
  mean_ms="$(awk -v s="$mean_s" -v n="$iters" 'BEGIN { printf "%.2f", (s * 1000) / n }')"
  if awk -v m="$mean_ms" 'BEGIN { exit !(m <= 0) }'; then
    record_hard "$label: implausible non-positive measurement (${mean_ms}ms)"
    return 1
  fi

  if within_budget "$mean_ms" "$budget"; then
    pass "$label: ${mean_ms}ms/render (budget ${budget}ms +${TOLERANCE_PCT}%)"
    REPORT_ROWS+=("| $label | <${budget}ms | ${mean_ms}ms | ✅ |")
  else
    record_budget "$label: ${mean_ms}ms exceeds budget ${budget}ms (+${TOLERANCE_PCT}%)"
    REPORT_ROWS+=("| $label | <${budget}ms | ${mean_ms}ms | ❌ |")
  fi
  return 0
}

# sample_rss PID -> prints RSS in MB, or empty if the process is gone.
sample_rss_mb() {
  local pid="$1" kb
  kb="$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ')"
  [[ -z "$kb" || ! "$kb" =~ ^[0-9]+$ || "$kb" -eq 0 ]] && return 1
  awk -v kb="$kb" 'BEGIN { printf "%.1f", kb / 1024 }'
}

# check_memory LABEL BUDGET_MB MEASURED_MB
check_memory() {
  local label="$1" budget="$2" measured="$3"
  if [[ -z "$budget" ]]; then
    record_hard "$label: no memory budget defined"
    return 1
  fi
  # An empty or non-positive RSS is never a valid measurement; treat it as a
  # hard failure rather than letting it look like a passing zero.
  if [[ -z "$measured" ]] || awk -v m="$measured" 'BEGIN { exit !(m <= 0) }'; then
    record_hard "$label: invalid RSS measurement (\"$measured\")"
    return 1
  fi
  if within_budget "$measured" "$budget"; then
    pass "$label: ${measured}MB (budget ${budget}MB)"
    REPORT_ROWS+=("| $label | <${budget}MB | ${measured}MB | ✅ |")
  else
    record_budget "$label: ${measured}MB exceeds budget ${budget}MB"
    REPORT_ROWS+=("| $label | <${budget}MB | ${measured}MB | ❌ |")
  fi
}

# --- Fixtures ----------------------------------------------------------------

make_large_repo() {
  info "Building large synthetic repository (offline)..."
  rm -rf "$LARGE_REPO"
  git init -q "$LARGE_REPO"
  git -C "$LARGE_REPO" config user.email "bench@gpy.local"
  git -C "$LARGE_REPO" config user.name "GPY Bench"
  git -C "$LARGE_REPO" config core.fsmonitor false
  local i dir
  for i in $(seq 1 1500); do
    dir="$LARGE_REPO/src/$((i % 50))"
    mkdir -p "$dir"
    printf 'content line for file %s\n' "$i" >"$dir/file$i.rs"
  done
  git -C "$LARGE_REPO" add -A >/dev/null 2>&1
  git -C "$LARGE_REPO" commit -qm "initial" >/dev/null 2>&1
  # Introduce a realistic dirty state.
  printf 'change\n' >>"$LARGE_REPO/src/0/file50.rs"
  printf 'new file\n' >"$LARGE_REPO/untracked.txt"
}

# --- Scenarios ---------------------------------------------------------------

# Fish snippets (run before each in-process render) that put the instant cache
# into a given state. A for-loop glob is used deliberately: it is quote-free (so
# it never breaks the surrounding `fish -c '...'` quoting) and does not error
# when the glob matches nothing (unlike an argument glob).
COLD_PREP="for f in $INSTANT_CACHE_DIR/*.ansi; command rm -f \$f; end"
STALE_PREP="for f in $INSTANT_CACHE_DIR/*.ansi; touch -t 200001010000 \$f; end"

scenario_prompt_latency() {
  mkdir -p "$INSTANT_CACHE_DIR"
  local preflight; preflight="$(prompt_command "$PROJECT_ROOT")"
  local warm cold stale
  warm="$(prompt_loop_command "$PROJECT_ROOT" "$RENDER_ITERS")"
  cold="$(prompt_loop_command "$PROJECT_ROOT" "$RENDER_ITERS" "$COLD_PREP")"
  stale="$(prompt_loop_command "$PROJECT_ROOT" "$RENDER_ITERS" "$STALE_PREP")"

  # 1. Warm prompt: daemon up, instant cache hit on every render.
  if start_daemon; then
    eval "$preflight" >/dev/null 2>&1  # prime the instant cache
    run_latency "warm prompt" "$(budget_ms prompt_render)" "$warm" "$RENDER_ITERS" "$preflight"

    # 2. Cold cache: instant cache cleared before every render (always a miss).
    run_latency "cold cache" "$(budget_ms prompt_render_cold)" "$cold" "$RENDER_ITERS" "$preflight"

    # 3. Stale cache: cache aged before every render (serve-stale path).
    eval "$preflight" >/dev/null 2>&1  # re-prime
    run_latency "stale cache" "$(budget_ms prompt_render_stale)" "$stale" "$RENDER_ITERS" "$preflight"
  else
    record_hard "warm/cold/stale scenarios: daemon failed to start"
  fi

  # 4. Agent unavailable: daemon stopped, prompt must still render via fallback.
  stop_daemon
  if [[ -n "$(daemon_pid)" ]]; then
    record_hard "agent unavailable: daemon did not shut down cleanly"
  else
    run_latency "agent unavailable" "$(budget_ms prompt_render_degraded)" "$warm" "$RENDER_ITERS" "$preflight"
  fi
}

scenario_large_repo() {
  make_large_repo
  # Cold git scan via oneshot (a fresh process with no daemon cache), measured
  # against the cold git budget. A single invocation is the unit here.
  local cmd
  cmd="$(printf 'XDG_CACHE_HOME=%q %q oneshot git --cwd %q' "$BENCH_CACHE" "$AGENT_BIN" "$LARGE_REPO")"
  run_latency "large repository" "$(budget_ms git_status_cold)" "$cmd" 1 "$cmd"
}

scenario_memory() {
  # 5. Startup RSS: confirm PID + responsive socket, then sample memory.
  if ! start_daemon; then
    record_hard "memory: daemon failed to start or never became responsive"
    return
  fi
  local pid; pid="$(daemon_pid)"
  if [[ -z "$pid" ]]; then
    record_hard "memory: could not resolve daemon PID"
    stop_daemon
    return
  fi
  info "Scenario: startup RSS (daemon PID $pid)"
  local rss
  if ! rss="$(sample_rss_mb "$pid")"; then
    record_hard "startup RSS: measured an empty/zero RSS (process not alive)"
  else
    check_memory "startup RSS" "$(budget_mb memory_usage)" "$rss"
  fi

  # 6. Long-running RSS: drive sustained prompt load and track peak memory.
  info "Scenario: long-running RSS (${LONG_RUN_ITERS} iterations)"
  local repo_cmd; repo_cmd="$(prompt_command "$PROJECT_ROOT")"
  local peak="$rss" i sample
  for i in $(seq 1 "$LONG_RUN_ITERS"); do
    eval "$repo_cmd" >/dev/null 2>&1
    if [[ -z "$(daemon_pid)" ]]; then
      record_hard "long-running RSS: daemon died under load at iteration $i"
      return
    fi
    if sample="$(sample_rss_mb "$pid")"; then
      awk -v s="$sample" -v p="$peak" 'BEGIN { exit !(s > p) }' && peak="$sample"
    fi
  done
  check_memory "long-running RSS" "$(budget_mb memory_long_running)" "$peak"
  stop_daemon
}

# --- Report ------------------------------------------------------------------

generate_report() {
  local report="$RESULTS_DIR/prompt-memory-report.md"
  {
    echo "# GPY Prompt & Memory Benchmark"
    echo
    echo "Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "Mode: $([[ "$CI_MODE" == true ]] && echo CI || echo local)"
    echo
    echo "| Scenario | Budget | Measured | Status |"
    echo "|----------|--------|----------|--------|"
    [[ ${#REPORT_ROWS[@]} -gt 0 ]] && printf '%s\n' "${REPORT_ROWS[@]}"
  } >"$report"
  info "Report written to $report"
}

# --- Main --------------------------------------------------------------------

main() {
  info "Starting GPY prompt & memory benchmark (mode: $([[ "$CI_MODE" == true ]] && echo CI || echo local))"
  mkdir -p "$RESULTS_DIR"

  require_tools || { print_summary; return $?; }
  ensure_agent_built || { print_summary; return $?; }

  if [[ ! -f "$BASELINE_FILE" ]]; then
    record_hard "Baseline file not found: $BASELINE_FILE"
    print_summary
    return $?
  fi

  scenario_prompt_latency
  scenario_large_repo
  scenario_memory
  generate_report
  print_summary
}

print_summary() {
  echo
  info "Summary: ${HARD_FAILURES} hard failure(s), ${BUDGET_FAILURES} budget overage(s)"

  if [[ "$HARD_FAILURES" -gt 0 ]]; then
    fail "❌ Benchmark integrity failures detected — results are not trustworthy"
    return 1
  fi

  if [[ "$BUDGET_FAILURES" -gt 0 ]]; then
    if [[ "$CI_MODE" == true ]]; then
      fail "❌ ${BUDGET_FAILURES} budget(s) exceeded"
      return 1
    fi
    warn "⚠️  ${BUDGET_FAILURES} budget(s) exceeded (warnings only in local mode)"
    return 0
  fi

  pass "✅ All scenarios measured and within budget"
  return 0
}

main "$@"
