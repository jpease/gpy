#!/usr/bin/env bash
# End-to-end prompt-latency and agent-memory benchmark (real fish_prompt, bash PS1 and zsh PROMPT renders + live daemon).
# Usage: ./scripts/ci-bench.sh [--ci]   # --ci (or CI=true): budget overages fail
#
# A broken measurement (non-zero exit, empty output, unparseable timing, zero
# RSS, unresponsive daemon) is a hard failure in every mode. A missing jq,
# baseline file (BASELINE_FILE overrides it) or budget key exits 2.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGENT_DIR="$PROJECT_ROOT/gpy-agent"
AGENT_BIN="$AGENT_DIR/target/release/gpy-agent"
# shellcheck source=scripts/lib/bench-common.sh
. "$SCRIPT_DIR/lib/bench-common.sh"
RESULTS_DIR="$PROJECT_ROOT/bench-results"
FISH_FUNCS="$PROJECT_ROOT/fish/functions"
FISH_CORE="$PROJECT_ROOT/fish/core/init.fish"

# Isolated sandbox: never touch the user's cache, socket or repos.
BENCH_TMP="$(mktemp -d "${TMPDIR:-/tmp}/gpy-ci-bench.XXXXXX")"
BENCH_CACHE="$BENCH_TMP/cache"
BENCH_SOCKET="$BENCH_TMP/gpy-test-cibench.sock"
INSTANT_CACHE_DIR="$BENCH_CACHE/gpy/instant-prompts"
LARGE_REPO="$BENCH_TMP/large-repo"
WARMUP_RUNS="${GPY_BENCH_WARMUP:-3}"
TIMED_RUNS="${GPY_BENCH_RUNS:-8}"
# Renders per timed run; latency is total / renders. macOS `date` lacks %N, so
# amortizing a loop is the portable way to get per-render timing.
RENDER_ITERS="${GPY_BENCH_RENDER_ITERS:-100}"
LONG_RUN_ITERS="${GPY_BENCH_LONG_ITERS:-40}"

CI_MODE=false
if [[ " ${*:-} " == *" --ci "* ]] || [[ "${CI:-}" == "true" ]]; then
  CI_MODE=true
fi

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BLUE='\033[0;34m'; NC='\033[0m'
info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
pass()    { echo -e "${GREEN}[PASS]${NC} $1"; }
warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
fail()    { echo -e "${RED}[FAIL]${NC} $1"; }

# Hard failures (broken measurement) are always fatal; budget failures only in CI.
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

cleanup() {
  stop_daemon >/dev/null 2>&1 || true
  rm -rf "$BENCH_TMP" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

require_tools() {
  local missing=()
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

# Budgets, read once in main from the baseline file.
load_budgets() {
  bench_load_baselines \
    TOLERANCE_PCT=.regression_thresholds.measurement_variance_percent \
    BUDGET_PROMPT_WARM=.budgets.prompt_render.max_ms \
    BUDGET_PROMPT_COLD=.budgets.prompt_render_cold.max_ms \
    BUDGET_PROMPT_STALE=.budgets.prompt_render_stale.max_ms \
    BUDGET_PROMPT_DEGRADED=.budgets.prompt_render_degraded.max_ms \
    BUDGET_PROMPT_BASH=.budgets.prompt_render_bash.max_ms \
    BUDGET_PROMPT_ZSH=.budgets.prompt_render_zsh.max_ms \
    BUDGET_GIT_COLD=.budgets.git_status_cold.max_ms \
    BUDGET_MEMORY_STARTUP=.budgets.memory_usage.max_mb \
    BUDGET_MEMORY_LONG=.budgets.memory_long_running.max_mb
}

within_budget() {
  local measured="$1" budget="$2"
  awk -v m="$measured" -v b="$budget" -v t="$TOLERANCE_PCT" \
    'BEGIN { exit !(m <= b * (1 + t / 100)) }'
}

daemon_pid() { pgrep -f "gpy-agent start --socket ${BENCH_SOCKET}" 2>/dev/null | head -1; }

start_daemon() {
  XDG_CACHE_HOME="$BENCH_CACHE" "$AGENT_BIN" start --socket "$BENCH_SOCKET" >/dev/null 2>&1

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
  local pid; pid="$(daemon_pid)"
  [[ -n "$pid" ]] && kill -9 "$pid" 2>/dev/null
  return 0
}

# Fish setup loading THIS checkout's prompt for dir $1 (not a global install).
prompt_setup() {
  local target="$1"
  # $PATH and $fish_function_path must reach Fish unexpanded.
  # shellcheck disable=SC2016
  printf 'set -gx PATH "%s/target/release" $PATH; set -gx XDG_CACHE_HOME "%s"; set -gx fish_function_path "%s" $fish_function_path; builtin cd "%s"; source "%s" >/dev/null 2>&1' \
    "$AGENT_DIR" "$BENCH_CACHE" "$FISH_FUNCS" "$target" "$FISH_CORE"
}

prompt_command() {
  printf "fish --no-config -c '%s; fish_prompt'" "$(prompt_setup "$1")"
}

# prompt_loop_command DIR ITERS [PREP]: PREP is a Fish snippet run before each render.
prompt_loop_command() {
  local target="$1" iters="$2" prep="${3:-}"
  printf "fish --no-config -c '%s; for __i in (seq %s); %sfish_prompt >/dev/null; end'" \
    "$(prompt_setup "$target")" "$iters" "${prep:+$prep; }"
}

# The first bash >= 4.4 (for ${PS1@P}) among $GPY_BENCH_BASH, Homebrew's and
# the PATH's; empty when none qualifies.
find_bench_bash() {
  local c
  for c in "${GPY_BENCH_BASH:-}" "$(brew --prefix 2>/dev/null)/bin/bash" "$(command -v bash)"; do
    [[ -n "$c" && -x "$c" ]] || continue
    # shellcheck disable=SC2016
    if "$c" -c '(( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) ))' 2>/dev/null; then
      printf '%s\n' "$c"
      return 0
    fi
  done
  return 1
}

# bench_shell_command BODY SHELL [ARG...]: SHELL ARG... -c BODY against the
# bench daemon, with this checkout's agent first on PATH. The env prefix is
# per command so the Fish scenarios keep their own environment. hyperfine
# runs it under `sh`; %q output is POSIX-safe for these paths.
bench_shell_command() {
  local body="$1"; shift
  local argv="" a
  for a in "$@"; do argv+="$(printf '%q ' "$a")"; done
  printf 'env PATH=%q XDG_CACHE_HOME=%q GPY_AGENT_SOCKET_PATH=%q %s-c %q' \
    "$AGENT_DIR/target/release:$PATH" "$BENCH_CACHE" "$BENCH_SOCKET" "$argv" "$body"
}

# Bash and Zsh: cd into $PROJECT_ROOT (the tree the Fish scenarios render),
# source the integration once, then render RENDER_ITERS times.
scenario_shell_prompts() {
  local cd_src bash_bin body
  bash_bin="$(find_bench_bash)"
  if [[ -z "$bash_bin" ]]; then
    if [[ "$CI_MODE" == true ]]; then
      record_hard "bash prompt: bash >= 4.4 not found"
    else
      warn "bash prompt: bash >= 4.4 not found; skipping"
    fi
  else
    cd_src="cd $(printf %q "$PROJECT_ROOT") && source $(printf %q "$PROJECT_ROOT/bash/gpy.bash") && "
    # shellcheck disable=SC2016
    body="$cd_src"'for i in $(seq '"$RENDER_ITERS"'); do __gpy_precmd; : "${PS1@P}"; done'
    # shellcheck disable=SC2016
    run_latency "bash prompt" "$BUDGET_PROMPT_BASH" \
      "$(bench_shell_command "$body" "$bash_bin" --noprofile --norc)" "$RENDER_ITERS" \
      "$(bench_shell_command "$cd_src"'__gpy_precmd; printf %s "${PS1@P}"' "$bash_bin" --noprofile --norc)"
  fi

  if ! command -v zsh >/dev/null 2>&1; then
    if [[ "$CI_MODE" == true ]]; then
      record_hard "zsh prompt: zsh not found"
    else
      warn "zsh prompt: zsh not found; skipping"
    fi
  else
    cd_src="cd $(printf %q "$PROJECT_ROOT") && source $(printf %q "$PROJECT_ROOT/zsh/gpy.zsh") && "
    # shellcheck disable=SC2016
    body="$cd_src"'for i in {1..'"$RENDER_ITERS"'}; do for f in $precmd_functions; do $f; done; : ${(%)PROMPT}; done'
    # shellcheck disable=SC2016
    run_latency "zsh prompt" "$BUDGET_PROMPT_ZSH" \
      "$(bench_shell_command "$body" zsh -f)" "$RENDER_ITERS" \
      "$(bench_shell_command "$cd_src"'for f in $precmd_functions; do $f; done; print -P "$PROMPT"' zsh -f)"
  fi
}

# Exit 0 only if CMD succeeds with non-empty output.
preflight_render() {
  local cmd="$1" out rc
  out="$(eval "$cmd" 2>/dev/null)"; rc=$?
  if [[ $rc -ne 0 ]]; then
    return 1
  fi
  [[ -n "$out" ]]
}

# run_latency LABEL BUDGET_MS TIMED_CMD ITERS PREFLIGHT_CMD
run_latency() {
  local label="$1" budget="$2" cmd="$3" iters="${4:-1}" preflight="${5:-$3}"
  info "Scenario: $label"

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
  # hyperfine 1.x exports `.results[0].mean`; 2.x moved it under `.summary`.
  mean_s="$(jq -r '.results[0].mean // .results[0].summary.time_wall_clock.mean // empty' "$json" 2>/dev/null)"
  if [[ -z "$mean_s" ]]; then
    record_hard "$label: could not parse timing result"
    return 1
  fi
  mean_ms="$(awk -v s="$mean_s" -v n="$iters" 'BEGIN { printf "%.2f", (s * 1000) / n }')"
  if awk -v m="$mean_ms" 'BEGIN { exit !(m <= 0) }'; then
    record_hard "$label: implausible non-positive measurement (${mean_ms}ms)"
    return 1
  fi

  if within_budget "$mean_ms" "$budget"; then
    pass "$label: ${mean_ms}ms/render (budget ${budget}ms +${TOLERANCE_PCT}%)"
    REPORT_ROWS+=("| $label | <${budget}ms | ${mean_ms}ms | pass |")
  else
    record_budget "$label: ${mean_ms}ms exceeds budget ${budget}ms (+${TOLERANCE_PCT}%)"
    REPORT_ROWS+=("| $label | <${budget}ms | ${mean_ms}ms | over budget |")
  fi
  return 0
}

sample_rss_mb() {
  local pid="$1" kb
  kb="$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ')"
  [[ -z "$kb" || ! "$kb" =~ ^[0-9]+$ || "$kb" -eq 0 ]] && return 1
  awk -v kb="$kb" 'BEGIN { printf "%.1f", kb / 1024 }'
}

check_memory() {
  local label="$1" budget="$2" measured="$3"
  if [[ -z "$measured" ]] || awk -v m="$measured" 'BEGIN { exit !(m <= 0) }'; then
    record_hard "$label: invalid RSS measurement (\"$measured\")"
    return 1
  fi
  if within_budget "$measured" "$budget"; then
    pass "$label: ${measured}MB (budget ${budget}MB)"
    REPORT_ROWS+=("| $label | <${budget}MB | ${measured}MB | pass |")
  else
    record_budget "$label: ${measured}MB exceeds budget ${budget}MB"
    REPORT_ROWS+=("| $label | <${budget}MB | ${measured}MB | over budget |")
  fi
}

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
  printf 'change\n' >>"$LARGE_REPO/src/0/file50.rs"
  printf 'new file\n' >"$LARGE_REPO/untracked.txt"
}

# Fish snippets run before each render. A for-loop glob stays quote-free inside
# `fish -c '...'` and, unlike an argument glob, doesn't error on no match.
COLD_PREP="for f in $INSTANT_CACHE_DIR/*.ansi; command rm -f \$f; end"
STALE_PREP="for f in $INSTANT_CACHE_DIR/*.ansi; touch -t 200001010000 \$f; end"

scenario_prompt_latency() {
  mkdir -p "$INSTANT_CACHE_DIR"
  local preflight; preflight="$(prompt_command "$PROJECT_ROOT")"
  local warm cold stale
  warm="$(prompt_loop_command "$PROJECT_ROOT" "$RENDER_ITERS")"
  cold="$(prompt_loop_command "$PROJECT_ROOT" "$RENDER_ITERS" "$COLD_PREP")"
  stale="$(prompt_loop_command "$PROJECT_ROOT" "$RENDER_ITERS" "$STALE_PREP")"

  if start_daemon; then
    eval "$preflight" >/dev/null 2>&1
    run_latency "warm prompt" "$BUDGET_PROMPT_WARM" "$warm" "$RENDER_ITERS" "$preflight"

    run_latency "cold cache" "$BUDGET_PROMPT_COLD" "$cold" "$RENDER_ITERS" "$preflight"

    # Stale: cache aged before every render (serve-stale path).
    eval "$preflight" >/dev/null 2>&1
    run_latency "stale cache" "$BUDGET_PROMPT_STALE" "$stale" "$RENDER_ITERS" "$preflight"

    scenario_shell_prompts
  else
    record_hard "warm/cold/stale scenarios: daemon failed to start"
  fi

  # Daemon stopped: prompt must still render via fallback.
  stop_daemon
  if [[ -n "$(daemon_pid)" ]]; then
    record_hard "agent unavailable: daemon did not shut down cleanly"
  else
    run_latency "agent unavailable" "$BUDGET_PROMPT_DEGRADED" "$warm" "$RENDER_ITERS" "$preflight"
  fi
}

scenario_large_repo() {
  make_large_repo
  # Oneshot = fresh process with no daemon cache.
  local cmd
  cmd="$(printf 'XDG_CACHE_HOME=%q %q oneshot git --cwd %q' "$BENCH_CACHE" "$AGENT_BIN" "$LARGE_REPO")"
  run_latency "large repository" "$BUDGET_GIT_COLD" "$cmd" 1 "$cmd"
}

scenario_memory() {
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
    check_memory "startup RSS" "$BUDGET_MEMORY_STARTUP" "$rss"
  fi

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
  check_memory "long-running RSS" "$BUDGET_MEMORY_LONG" "$peak"
  stop_daemon
}

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

main() {
  info "Starting GPY prompt & memory benchmark (mode: $([[ "$CI_MODE" == true ]] && echo CI || echo local))"
  mkdir -p "$RESULTS_DIR"

  require_tools || { print_summary; return $?; }
  load_budgets
  bench_build_release_agent "$AGENT_DIR"

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
    fail "Benchmark integrity failures detected — results are not trustworthy"
    return 1
  fi

  if [[ "$BUDGET_FAILURES" -gt 0 ]]; then
    if [[ "$CI_MODE" == true ]]; then
      fail "${BUDGET_FAILURES} budget(s) exceeded"
      return 1
    fi
    warn "${BUDGET_FAILURES} budget(s) exceeded (warnings only in local mode)"
    return 0
  fi

  pass "All scenarios measured and within budget"
  return 0
}

main "$@"
