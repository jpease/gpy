#!/usr/bin/env bash
# End-to-end prompt-latency, script-segment and agent-memory benchmark (real fish_prompt, bash PS1 and zsh PROMPT renders + live daemon).
# Usage: ./scripts/ci-bench.sh [--ci]   # --ci (or CI=true): budget overages fail
#
# A broken measurement (non-zero exit, empty output, unparseable timing, zero
# RSS, unresponsive daemon) is a hard failure in every mode. A missing jq,
# budget file (PERF_BUDGETS_FILE overrides it) or budget key exits 2.

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


# Hard failures (broken measurement) are always fatal; budget failures only in CI.
HARD_FAILURES=0
BUDGET_FAILURES=0
REPORT_ROWS=()

record_hard() { HARD_FAILURES=$((HARD_FAILURES + 1)); bench_fail "$1"; }
record_budget() {
  BUDGET_FAILURES=$((BUDGET_FAILURES + 1))
  if [[ "$CI_MODE" == true ]]; then
    bench_fail "$1"
  else
    bench_warn "$1 (warning in local mode)"
  fi
}

cleanup() {
  stop_daemon >/dev/null 2>&1 || true
  g_stop_daemon >/dev/null 2>&1 || true
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
    bench_warn "Skipping ci-bench: missing tools (${missing[*]}). Install them to run locally."
    exit 0
  fi
  return 0
}

# Budgets, read once in main from the budget file.
load_budgets() {
  bench_load_budgets \
    TOLERANCE_PCT=.regression_thresholds.measurement_variance_percent \
    BUDGET_PROMPT_WARM=.budgets.prompt_render.max_ms \
    BUDGET_PROMPT_COLD=.budgets.prompt_render_cold.max_ms \
    BUDGET_PROMPT_STALE=.budgets.prompt_render_stale.max_ms \
    BUDGET_PROMPT_DEGRADED=.budgets.prompt_render_degraded.max_ms \
    BUDGET_PROMPT_BASH=.budgets.prompt_render_bash.max_ms \
    BUDGET_PROMPT_ZSH=.budgets.prompt_render_zsh.max_ms \
    BUDGET_GIT_COLD=.budgets.git_status_cold.max_ms \
    BUDGET_MEMORY_STARTUP=.budgets.memory_usage.max_mb \
    BUDGET_MEMORY_LONG=.budgets.memory_long_running.max_mb \
    BUDGET_G_BASH=.budgets.script_segments_bash.max_ms \
    BUDGET_G_ZSH=.budgets.script_segments_zsh.max_ms \
    BUDGET_G_FISH=.budgets.script_segments_fish.max_ms \
    BUDGET_G_SHARED=.budgets.script_segments_shared.max_ms \
    BUDGET_G_SCAN=.budgets.script_segment_scan.max_ms
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
    if XDG_CACHE_HOME="$BENCH_CACHE" "$AGENT_BIN" status --socket "$BENCH_SOCKET" >/dev/null 2>&1; then
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
      bench_warn "bash prompt: bash >= 4.4 not found; skipping"
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
      bench_warn "zsh prompt: zsh not found; skipping"
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
  bench_info "Scenario: $label"

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
    bench_pass "$label: ${mean_ms}ms/render (budget ${budget}ms +${TOLERANCE_PCT}%)"
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
    bench_pass "$label: ${measured}MB (budget ${budget}MB)"
    REPORT_ROWS+=("| $label | <${budget}MB | ${measured}MB | pass |")
  else
    record_budget "$label: ${measured}MB exceeds budget ${budget}MB"
    REPORT_ROWS+=("| $label | <${budget}MB | ${measured}MB | over budget |")
  fi
}

make_large_repo() {
  bench_info "Building large synthetic repository (offline)..."
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
  bench_info "Scenario: startup RSS (daemon PID $pid)"
  local rss
  if ! rss="$(sample_rss_mb "$pid")"; then
    record_hard "startup RSS: measured an empty/zero RSS (process not alive)"
  else
    check_memory "startup RSS" "$BUDGET_MEMORY_STARTUP" "$rss"
  fi

  bench_info "Scenario: long-running RSS (${LONG_RUN_ITERS} iterations)"
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

# --- Script ("G") segments ----------------------------------------------------
# What plugin script segments add to a prompt, through the real adapters
# (bash/core/g_segments.bash, zsh/core/script.zsh, fish/core/script.fish, the
# one shared bash/core/g_runner.bash) and the agent's real theme export of a fixture plugin.
# A live agent of its own (sandbox config, cache and socket; supervisor off)
# renders the one built-in segment, `directory`, in every config, so the
# paired differences isolate the plugin work.
#
# Method (spike #1006 harness `bench`): every round visits every config in
# rotated order and takes G_PER_ROUND samples of each; a sample is a fresh
# shell that sources the integration, draws the first prompt, then G_ITERS
# more. Per prompt = (median of G_ITERS-prompt samples - median of 0-prompt
# samples) / G_ITERS, so startup and the first prompt cancel out. Each
# variant is paired with its shell's `base` in the same round; the budget
# check uses the median of those paired differences. Load1 is recorded per
# round. Round 0 is a warmup and is discarded.
G_ROUNDS="${GPY_BENCH_G_ROUNDS:-5}"
G_PER_ROUND="${GPY_BENCH_G_PER_ROUND:-3}"
G_ITERS="${GPY_BENCH_G_ITERS:-20}"
# Canary self-check: seconds the per-command script sleeps (e.g. 0.05). Must
# stay under its 500 ms timeout, so the run is slow, not cut off.
G_DELAY="${GPY_BENCH_G_DELAY:-}"
G_DIR="$BENCH_TMP/g"
G_WORK="$G_DIR/work/project"
G_TSV="$RESULTS_DIR/script-segments.tsv"
G_PREFLIGHT_ITERS=2
G_SOCKET="$G_DIR/agent.sock"
G_AGENT_CACHE="$G_DIR/agent-cache"

g_start_daemon() {
  mkdir -p "$G_AGENT_CACHE"
  XDG_CONFIG_HOME="$G_DIR/v/base/config" XDG_CACHE_HOME="$G_AGENT_CACHE" \
    "$AGENT_BIN" start --socket "$G_SOCKET" >/dev/null 2>&1
  for _ in $(seq 1 50); do
    if XDG_CONFIG_HOME="$G_DIR/v/base/config" XDG_CACHE_HOME="$G_AGENT_CACHE" \
        "$AGENT_BIN" status --socket "$G_SOCKET" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

g_stop_daemon() {
  [[ -S "$G_SOCKET" ]] || return 0
  XDG_CONFIG_HOME="$G_DIR/v/base/config" XDG_CACHE_HOME="$G_AGENT_CACHE" \
    "$AGENT_BIN" stop --socket "$G_SOCKET" >/dev/null 2>&1
  local pid
  for _ in $(seq 1 30); do
    pid="$(pgrep -f "gpy-agent start --socket ${G_SOCKET}" 2>/dev/null | head -1)"
    [[ -z "$pid" ]] && return 0
    sleep 0.1
  done
  kill -9 "$pid" 2>/dev/null
  return 0
}

# 1-minute load average; empty when unreadable.
load1() {
  local l
  if [[ -r /proc/loadavg ]]; then
    read -r l _ </proc/loadavg
  else
    l="$(sysctl -n vm.loadavg 2>/dev/null | awk '{ print $2 }')"
  fi
  printf '%s' "$l"
}

# The fixture plugin: a username-like segment cached for the session
# (bench-user), one run on every command (bench-cwd: ttl 0, pwd) and one
# whose require_files never matches (bench-scan: the scan child, no run).
# Each script prints its PID, so the preflight can tell a run from a hit.
g_write_plugin() {
  local p="$G_DIR/plugins/gpy-bench" delay=""
  [[ -n "$G_DELAY" ]] && delay="sleep $G_DELAY"
  mkdir -p "$p/bin" "$p/segments" "$G_WORK" "$G_DIR/home" "$G_DIR/tmp" "$G_DIR/run"
  chmod 700 "$G_DIR/run"
  cat >"$p/plugin.toml" <<'EOF'
id = "gpy-bench"
name = "ci-bench fixture"
version = "1.0.0"
api_version = "v1"
entry_type = "script"
provided_segments = ["bench-user", "bench-cwd", "bench-scan"]
description = "Script segments measured by scripts/ci-bench.sh"

[compatibility_constraints]
min_gpy_version = "0.6.0"
EOF
  cat >"$p/segments/bench-user.toml" <<'EOF'
description = "Login name, cached for the session"
command = "bin/user.sh"
inputs = ["is_root"]
ttl_secs = 3600
EOF
  cat >"$p/segments/bench-cwd.toml" <<'EOF'
description = "Working directory name, run on every command"
command = "bin/cwd.sh"
inputs = ["pwd"]
ttl_secs = 0
EOF
  cat >"$p/segments/bench-scan.toml" <<'EOF'
description = "Never shown: its require_files never matches"
command = "bin/cwd.sh"
inputs = ["pwd"]
ttl_secs = 0
require_files = ["gpy-bench-absent.marker"]
EOF
  cat >"$p/bin/user.sh" <<'EOF'
#!/bin/sh
printf '%s gbu%s\n' "${USER:-nobody}" "$$"
if [ "${GPY_IS_ROOT:-0}" = 1 ]; then
    printf 'root\n'
fi
EOF
  # shellcheck disable=SC2016  # $ expands in the generated script, not here
  printf '#!/bin/sh\n%s\nprintf '\''%%s gbc%%s\\n'\'' "${GPY_PWD##*/}" "$$"\n' "$delay" >"$p/bin/cwd.sh"
  # Export drops a plugin whose files are group- or world-writable.
  find "$G_DIR/plugins" -type d -exec chmod 755 {} +
  find "$G_DIR/plugins" -type f -exec chmod 644 {} +
}

# g_export VARIANT TOKENS SEGMENTS [EXTRA_TOML]: config.toml and the agent's
# theme export for every shell under $G_DIR/v/VARIANT; TOKENS is the
# expected __gpy_script_segments (space separated).
g_export() {
  local variant="$1" tokens="$2" segments="$3" extra="${4:-}" d="$G_DIR/v/$1" sh want t
  mkdir -p "$d/config/gpy" "$d/cache/gpy"
  ln -s "$G_DIR/plugins" "$d/config/gpy/plugins"
  printf '[agent]\nenabled = true\n\n[agent.supervisor]\nenabled = false\n\n[ui]\nenabled_segments = [%s]\n%s' \
    "$segments" "$extra" >"$d/config/gpy/config.toml"
  for sh in bash zsh fish; do
    if ! XDG_CONFIG_HOME="$d/config" XDG_CACHE_HOME="$d/cache" "$AGENT_BIN" theme export --shell "$sh" \
        >"$d/cache/gpy/theme-export.$sh" 2>/dev/null; then
      record_hard "script segments: theme export ($variant, $sh) failed"
      return 1
    fi
  done
  want=""
  for t in $tokens; do want+="${want:+ }\"$t\""; done
  if ! grep -qxF "__gpy_script_segments=($want)" "$d/cache/gpy/theme-export.bash"; then
    record_hard "script segments: export for $variant does not list [$tokens]: $(grep '^__gpy_script_segments=' "$d/cache/gpy/theme-export.bash")"
    return 1
  fi
}

# The same export with another interpreter in __gpy_script_sh (every shell).
g_export_sh() {
  local from="$G_DIR/v/$1" to="$G_DIR/v/$2" sh_path="$3"
  mkdir -p "$to"
  cp -R "$from/config" "$from/cache" "$to/"
  sed -i.bak -e "s|^__gpy_script_sh=.*|__gpy_script_sh=\"$sh_path\"|" "$to/cache/gpy/theme-export.bash"
  sed -i.bak -e "s|^typeset -g __gpy_script_sh=.*|typeset -g __gpy_script_sh=\"$sh_path\"|" "$to/cache/gpy/theme-export.zsh"
  sed -i.bak -e "s|^set -g __gpy_script_sh .*|set -g __gpy_script_sh \"$sh_path\"|" "$to/cache/gpy/theme-export.fish"
  rm -f "$to"/cache/gpy/*.bak
}

# Sample bodies: ROOT WORK ITERS MODE. MODE `due` draws ITERS prompts after
# a command each, `idle` ITERS prompts without one (an empty Enter; a Fish
# repaint calls only the function part of it); a `show-` prefix prints each
# prompt followed by a @@P@@ line.
# Bash counts `\#` per top-level command the script reader reads, not per
# loop pass, so its body is unrolled: one line, one command, per prompt
# (body-ITERS.bash; ITERS is 0, the preflight's or G_ITERS).
g_write_bodies() {
  local n i
  # shellcheck disable=SC2016
  printf '%s\n' \
    'cd "$2" || exit 1' \
    'source "$1/bash/gpy.bash" || exit 1' \
    'show=""' \
    '[[ $4 == show-* ]] && show=1' \
    'if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4))); then' \
    '    eval '\''__gb_draw() { if [[ -n $show ]]; then printf "%s\n@@P@@\n" "${PS1@P}"; else : "${PS1@P}"; fi; }'\''' \
    'else' \
    '    __gb_draw() { [[ -z $show ]] || printf "%s\n@@P@@\n" "$PS1"; }' \
    'fi' \
    '__gpy_precmd' \
    '__gb_draw' \
    '# As the first interactive prompt does (init.bash): Bash < 4.4 needs the' \
    '# DEBUG trap to see a command once a script segment is listed.' \
    'if __gpy_needs_debug_trap; then' \
    '    __gpy_dbg_seen=""' \
    '    __gpy_install_debug_trap' \
    'fi' >"$G_DIR/body-head.bash"
  for n in 0 "$G_PREFLIGHT_ITERS" "$G_ITERS"; do
    {
      cat "$G_DIR/body-head.bash"
      for ((i = 0; i < n; i++)); do
        # shellcheck disable=SC2016
        printf '%s\n' '__gpy_arm_preexec; true; __gpy_precmd; __gb_draw'
      done
      printf 'exit 0\n'
    } >"$G_DIR/body-$n.bash"
  done
  # shellcheck disable=SC2016
  printf '%s\n' \
    'cd "$2" || exit 1' \
    'source "$1/zsh/gpy.zsh" || exit 1' \
    '# A zsh script runs preexec before each top-level line; here it runs' \
    '# exactly once per simulated command instead.' \
    '__gz_pre=($preexec_functions) preexec_functions=()' \
    'integer n=$3 i' \
    'mode=${4#show-} show=""' \
    '[[ $4 == show-* ]] && show=1' \
    '__gz_draw() { if [[ -n $show ]]; then print -rP -- "$PROMPT"; print -r -- @@P@@; else : ${(%)PROMPT}; fi }' \
    '__gz_precmd() { local f; for f in $precmd_functions; do $f; done }' \
    '__gz_precmd' \
    '__gz_draw' \
    'for ((i = 0; i < n; i++)); do' \
    '    if [[ $mode != idle ]]; then' \
    '        for f in $__gz_pre; do $f true true true; done' \
    '        true' \
    '    fi' \
    '    __gz_precmd' \
    '    __gz_draw' \
    'done' \
    'exit 0' >"$G_DIR/body.zsh"
  # shellcheck disable=SC2016
  printf '%s\n' \
    'builtin cd $argv[2]; or exit 1' \
    'set -gx fish_function_path $argv[1]/fish/functions $fish_function_path' \
    'source $argv[1]/fish/core/init.fish >/dev/null 2>&1' \
    'set -g __gf_n $argv[3]' \
    'set -g __gf_mode (string replace -r "^show-" "" -- $argv[4])' \
    'set -g __gf_show 0' \
    'string match -q "show-*" -- $argv[4]; and set __gf_show 1' \
    'function __gf_draw' \
    '    if test $__gf_show = 1' \
    '        fish_prompt; echo; echo @@P@@' \
    '    else' \
    '        fish_prompt >/dev/null' \
    '    end' \
    'end' \
    'emit fish_prompt' \
    '__gf_draw' \
    'set -l i 0' \
    'while test $i -lt $__gf_n' \
    '    if test $__gf_mode != idle' \
    '        emit fish_preexec true' \
    '        true' \
    '        emit fish_postexec true' \
    '    end' \
    '    emit fish_prompt' \
    '    __gf_draw' \
    '    set i (math $i + 1)' \
    'end' \
    'exit 0' >"$G_DIR/body.fish"
}

# g_command CONFIG ITERS [show]: the sample command (hyperfine -N syntax)
# for CONFIG = SHELL:VARIANT. A clean environment, so nothing from the
# caller's shell reaches the integration or the runner's base set.
g_command() {
  local shell="${1%%:*}" variant="${1#*:}" iters="$2" show="${3:+show-}" mode=due exe body
  local -a flags
  [[ "$variant" == idle ]] && mode=idle
  [[ "$variant" == idle ]] && variant=g
  case "$shell" in
    bash | bash3)
      exe="$G_BASH"
      [[ "$shell" == bash3 ]] && exe="$G_BASH3"
      flags=(--noprofile --norc) body="$G_DIR/body-$iters.bash"
      ;;
    zsh) exe="$(command -v zsh)" flags=(-f) body="$G_DIR/body.zsh" ;;
    fish) exe="$(command -v fish)" flags=(--no-config) body="$G_DIR/body.fish" ;;
  esac
  local -a argv=(env -i "HOME=$G_DIR/home" "USER=${USER:-$(id -un)}" "PATH=$AGENT_DIR/target/release:$PATH")
  [[ -n "${LANG:-}" ]] && argv+=("LANG=$LANG")
  argv+=("TMPDIR=$G_DIR/tmp" "XDG_RUNTIME_DIR=$G_DIR/run"
    "XDG_CONFIG_HOME=$G_DIR/v/$variant/config" "XDG_CACHE_HOME=$G_DIR/v/$variant/cache"
    "GPY_AGENT_SOCKET_PATH=$G_SOCKET"
    "$exe" "${flags[@]}" "$body" "$PROJECT_ROOT" "$G_WORK" "$iters" "$show$mode")
  local out="" a
  for a in "${argv[@]}"; do out+="$(printf '%q' "$a") "; done
  printf '%s' "${out% }"
}

# Expected runs over the preflight's prompts: `same` (one PID on every
# prompt: a cache hit or no run), `each` (a new PID on every prompt) or
# `none` (never shown), for bench-user then bench-cwd.
g_expect() {
  case "${1#*:}" in
    base | scan) echo "none none" ;;
    hit) echo "same none" ;;
    g | g-*) echo "same each" ;;
    expiry) echo "each each" ;;
    idle) echo "same same" ;;
  esac
}

# g_pids OUTPUT TAG: one line per prompt, the TAG PID shown or "-".
g_pids() {
  printf '%s\n' "$1" | awk -v tag="$2" '
    /^@@P@@$/ { print (pid == "" ? "-" : pid); pid = ""; next }
    pid == "" && match($0, tag "[0-9]+") { pid = substr($0, RSTART + length(tag), RLENGTH - length(tag)) }'
}

# Fails unless every prompt matches MODE (see g_expect).
g_check_runs() {
  local label="$1" pids="$2" mode="$3" n distinct
  n="$(printf '%s\n' "$pids" | grep -c .)"
  if [[ "$n" -ne $((G_PREFLIGHT_ITERS + 1)) ]]; then
    record_hard "$label: expected $((G_PREFLIGHT_ITERS + 1)) prompts, saw $n"
    return 1
  fi
  distinct="$(printf '%s\n' "$pids" | sort -u | grep -c .)"
  case "$mode" in
    none) [[ "$distinct" -eq 1 && "$(printf '%s\n' "$pids" | head -1)" == "-" ]] ;;
    same) [[ "$distinct" -eq 1 && "$(printf '%s\n' "$pids" | head -1)" != "-" ]] ;;
    each) [[ "$distinct" -eq "$n" ]] && ! printf '%s\n' "$pids" | grep -qx -- - ;;
  esac || {
    record_hard "$label: expected '$mode' across the prompts, got PIDs [$(printf '%s\n' "$pids" | tr '\n' ' ')]"
    return 1
  }
}

# Renders CONFIG's prompts once and checks each segment ran (or hit) as the
# variant claims, so a silently skipped script can never pass as cheap.
g_preflight() {
  local cfg="$1" out modes
  if ! out="$(eval "$(g_command "$cfg" "$G_PREFLIGHT_ITERS" show)" 2>/dev/null)"; then
    record_hard "script segments $cfg: preflight render failed"
    return 1
  fi
  read -r -a modes <<<"$(g_expect "$cfg")"
  g_check_runs "script segments $cfg: bench-user" "$(g_pids "$out" gbu)" "${modes[0]}" || return 1
  g_check_runs "script segments $cfg: bench-cwd" "$(g_pids "$out" gbc)" "${modes[1]}"
}

# Linear-interpolated quartiles of stdin numbers: "median q1 q3 n".
quartiles() {
  sort -g | awk '{ a[++n] = $1 }
    function q(p,  h, l) { h = (n - 1) * p + 1; l = int(h); return a[l] + (h - l) * (a[l + 1 < n ? l + 1 : n] - a[l]) }
    END { if (n) printf "%.3f %.3f %.3f %d\n", q(0.5), q(0.25), q(0.75), n }'
}

# g_round ROUND CONFIG...: one hyperfine call with every config's ITERS and
# 0-prompt samples, in this order; appends "round config ms load0 load1".
# Results are matched by position (hyperfine keeps the command order).
g_round() {
  local round="$1" cfg json l0 l1
  shift
  local -a args=()
  for cfg in "$@"; do
    args+=("$(g_command "$cfg" "$G_ITERS")" "$(g_command "$cfg" 0)")
  done
  json="$G_DIR/round-$round.json"
  l0="$(load1)"
  if ! hyperfine -N --warmup 0 --runs "$G_PER_ROUND" --export-json "$json" "${args[@]}" >/dev/null 2>&1; then
    record_hard "script segments: round $round: hyperfine reported a command failure"
    return 1
  fi
  l1="$(load1)"
  # hyperfine 1.x exports `.times`; 2.x `.measurements[].time_wall_clock.value`.
  jq -r '.results[] | (.times // [.measurements[].time_wall_clock.value]) | sort
      | if length % 2 == 1 then .[length / 2 | floor] else (.[length / 2 - 1] + .[length / 2]) / 2 end' "$json" |
    awk -v r="$round" -v it="$G_ITERS" -v l0="${l0:--}" -v l1="${l1:--}" -v cfgs="$*" '
      BEGIN { m = split(cfgs, c, " ") }
      { t[NR] = $1 }
      END { if (NR != 2 * m) exit 1
        for (i = 1; i <= m; i++) printf "%d\t%s\t%.4f\t%s\t%s\n", r, c[i], (t[2 * i - 1] - t[2 * i]) * 1000 / it, l0, l1 }' >>"$G_TSV" || {
    record_hard "script segments: round $round: unexpected hyperfine export"
    return 1
  }
}

# g_judge CONFIG BASE_VARIANT [BUDGET_MS BUDGET_KEY]: the median paired
# difference to the same shell's BASE_VARIANT over rounds >= 1, checked
# against BUDGET_MS when given, else only reported.
g_judge() {
  local cfg="$1" base="${1%%:*}:$2" budget="${3:-}" key="${4:-}" stats abs med q1 q3 n row label
  stats="$(awk -F'\t' -v c="$cfg" -v b="$base" '$1 > 0 && $2 == c { v[$1] = $3 } $1 > 0 && $2 == b { w[$1] = $3 }
    END { for (r in v) if (r in w) print v[r] - w[r] }' "$G_TSV" | quartiles)"
  abs="$(awk -F'\t' -v c="$cfg" '$1 > 0 && $2 == c { print $3 }' "$G_TSV" | quartiles | cut -d' ' -f1)"
  read -r med q1 q3 n <<<"$stats"
  if [[ -z "$med" ]]; then
    record_hard "script segments $cfg: no paired rounds measured"
    return 1
  fi
  label="script segments $cfg vs ${2}"
  row="+${med} ms (IQR ${q1} to ${q3}, n=${n}; ${abs} ms/prompt)"
  if [[ -z "$budget" ]]; then
    bench_info "$label: $row"
    REPORT_ROWS+=("| $label | - | $row | reported |")
  elif within_budget "$med" "$budget"; then
    bench_pass "$label: $row (budget +${budget}ms $key +${TOLERANCE_PCT}%)"
    REPORT_ROWS+=("| $label | +${budget}ms | $row | pass |")
  else
    record_budget "$label: $row exceeds +${budget}ms ($key +${TOLERANCE_PCT}%)"
    REPORT_ROWS+=("| $label | +${budget}ms | $row | over budget |")
  fi
}

# Bash < 4.4 (no `\#`, so the DEBUG trap path): GPY_BENCH_BASH3, else
# /bin/bash when it is one (macOS 3.2). Empty when none.
find_bench_bash3() {
  local c
  for c in "${GPY_BENCH_BASH3:-}" /bin/bash; do
    [[ -n "$c" && -x "$c" ]] || continue
    # shellcheck disable=SC2016
    if "$c" -c '(( BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 4) ))' 2>/dev/null; then
      printf '%s\n' "$c"
      return 0
    fi
  done
  return 1
}

scenario_script_segments() {
  bench_info "Scenario: script segments (rounds ${G_ROUNDS}+1 warmup, ${G_PER_ROUND} samples, ${G_ITERS} prompts)"
  if [[ -n "$G_DELAY" && ! "$G_DELAY" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
    record_hard "script segments: GPY_BENCH_G_DELAY must be seconds, got '$G_DELAY'"
    return 1
  fi
  [[ -n "$G_DELAY" ]] && bench_warn "script segments: bench-cwd sleeps ${G_DELAY}s per run (GPY_BENCH_G_DELAY canary check)"
  G_BASH="$(find_bench_bash)"
  G_BASH3="$(find_bench_bash3)"
  local -a shells=() configs=() variants=(base hit g expiry scan)
  local s v busybox="" cfg
  if [[ -n "$G_BASH" ]]; then shells+=(bash); fi
  if [[ -n "$G_BASH3" ]]; then
    shells+=(bash3)
  else
    bench_info "script segments: no Bash < 4.4 here; the DEBUG-trap path (Bash 3.2-4.3) is not measured"
  fi
  command -v zsh >/dev/null 2>&1 && shells+=(zsh)
  command -v fish >/dev/null 2>&1 && shells+=(fish)
  for s in bash zsh fish; do
    if [[ " ${shells[*]} " != *" $s "* ]]; then
      if [[ "$CI_MODE" == true ]]; then record_hard "script segments: $s not found"; else bench_warn "script segments: $s not found; skipping it"; fi
    fi
  done

  mkdir -p "$G_DIR"
  g_write_plugin
  g_write_bodies
  g_export base "" '"directory"' || return 1
  g_export hit "bench_user" '"directory", "bench-user"' || return 1
  g_export g "bench_user bench_cwd" '"directory", "bench-user", "bench-cwd"' || return 1
  g_export expiry "bench_user bench_cwd" '"directory", "bench-user", "bench-cwd"' \
    $'\n[plugin.bench-user]\nttl_secs = 0\n' || return 1
  g_export scan "bench_scan" '"directory", "bench-scan"' || return 1
  # Linux: /bin/sh is what the export names (dash on Debian/Ubuntu); busybox
  # sh (Alpine's) as well when installed. Runs as argv0 sh, as the runner
  # execs every interpreter.
  if [[ "$(uname -s)" == Linux ]] && busybox="$(command -v busybox)"; then
    g_export_sh g g-busybox "$busybox"
    variants+=(g-busybox)
  fi
  bench_info "script segments: interpreter $(sed -n 's/^__gpy_script_sh="\(.*\)"$/\1/p' "$G_DIR/v/g/cache/gpy/theme-export.bash") ($(readlink -f "$(sed -n 's/^__gpy_script_sh="\(.*\)"$/\1/p' "$G_DIR/v/g/cache/gpy/theme-export.bash")" 2>/dev/null || echo unresolved))${busybox:+, busybox $busybox}"

  for s in "${shells[@]}"; do
    for v in "${variants[@]}"; do configs+=("$s:$v"); done
    [[ "$s" == zsh || "$s" == fish ]] && configs+=("$s:idle")
  done
  if ! g_start_daemon; then
    record_hard "script segments: agent failed to start"
    g_stop_daemon
    return 1
  fi
  for cfg in "${configs[@]}"; do
    g_preflight "$cfg" || {
      g_stop_daemon
      return 1
    }
  done

  printf 'round\tconfig\tms_per_prompt\tload1_before\tload1_after\n' >"$G_TSV"
  local r k n=${#configs[@]}
  for ((r = 0; r <= G_ROUNDS; r++)); do
    k=$((r % n))
    g_round "$r" "${configs[@]:k}" "${configs[@]:0:k}" || {
      g_stop_daemon
      return 1
    }
  done
  g_stop_daemon
  local loads
  loads="$(awk -F'\t' 'NR > 1 && $1 > 0 && $4 != "-" { l[$1] = $4 } END { for (r in l) print l[r] }' "$G_TSV" | quartiles)"
  bench_info "script segments: load1 per round (median q1 q3 n) ${loads:-unknown}; samples in $G_TSV"

  local budget key
  for cfg in "${configs[@]}"; do
    s="${cfg%%:*}" v="${cfg#*:}"
    case "$v" in
      base) continue ;;
      expiry) budget="$BUDGET_G_SHARED" key=script_segments_shared ;;
      scan) budget="$BUDGET_G_SCAN" key=script_segment_scan ;;
      *)
        case "$s" in
          bash | bash3) budget="$BUDGET_G_BASH" key=script_segments_bash ;;
          zsh) budget="$BUDGET_G_ZSH" key=script_segments_zsh ;;
          fish) budget="$BUDGET_G_FISH" key=script_segments_fish ;;
        esac
        ;;
    esac
    g_judge "$cfg" base "$budget" "$key"
    # The uncached run itself: one script through the runner, over a hit.
    [[ "$v" == g ]] && g_judge "$cfg" hit
  done
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
  bench_info "Report written to $report"
}

main() {
  bench_info "Starting GPY prompt & memory benchmark (mode: $([[ "$CI_MODE" == true ]] && echo CI || echo local))"
  mkdir -p "$RESULTS_DIR"

  require_tools || { print_summary; return $?; }
  load_budgets
  bench_build_release_agent "$AGENT_DIR"

  scenario_prompt_latency
  scenario_script_segments
  scenario_large_repo
  scenario_memory
  generate_report
  print_summary
}

print_summary() {
  echo
  bench_info "Summary: ${HARD_FAILURES} hard failure(s), ${BUDGET_FAILURES} budget overage(s)"

  if [[ "$HARD_FAILURES" -gt 0 ]]; then
    bench_fail "Benchmark integrity failures detected — results are not trustworthy"
    return 1
  fi

  if [[ "$BUDGET_FAILURES" -gt 0 ]]; then
    if [[ "$CI_MODE" == true ]]; then
      bench_fail "${BUDGET_FAILURES} budget(s) exceeded"
      return 1
    fi
    bench_warn "${BUDGET_FAILURES} budget(s) exceeded (warnings only in local mode)"
    return 0
  fi

  bench_pass "All scenarios measured and within budget"
  return 0
}

main "$@"
