#!/usr/bin/env bash
set -euo pipefail

# Compare a small set of Criterion microbenchmarks against an upstream commit
# on the same machine. This is designed as an early-warning diagnostic for
# regressions, not as a publication-grade benchmark harness.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

MODE="${GPY_PERF_CANARY_MODE:-warn}"
BASE_REF="${GPY_PERF_CANARY_BASE_REF:-@{upstream}}"
SAMPLE_SIZE="${GPY_PERF_CANARY_SAMPLE_SIZE:-20}"
WARM_UP_SECONDS="${GPY_PERF_CANARY_WARMUP_SECONDS:-1}"
MEASUREMENT_SECONDS="${GPY_PERF_CANARY_MEASUREMENT_SECONDS:-1}"
WARN_THRESHOLD_PCT="${GPY_PERF_CANARY_WARN_THRESHOLD_PCT:-10}"
FAIL_THRESHOLD_PCT="${GPY_PERF_CANARY_FAIL_THRESHOLD_PCT:-20}"

# language_detect_rust/python/node were tracked here through #524, then
# dropped (#537): their 2-3-file synthetic corpus measured as unstable
# diagnostic noise, not signal -- rerunning the canary twice against the same
# two commits produced a +20.8% "regression" and a +5.9% "ok" for the same
# benchmark, and the corpus's own baseline moved 23% between runs with no code
# change on either side. The `language_detection` budget in
# tests/performance-baselines.json already gates language detection properly,
# against a realistic-sized corpus, via `./scripts/bench.sh --ci` -- this
# canary duplicating that check on a smaller, noisier corpus added false
# alarms without adding coverage. ipc_roundtrip and git_status_cold remain
# because neither showed this instability.
BENCHMARKS=(
  "ipc_roundtrip"
  "git_status_cold"
)

usage() {
  cat <<'EOF'
Usage: ./scripts/perf-canary.sh [--base <rev>] [--mode off|warn|strict]

Compares the current tree to a base revision on the same machine using a
focused Criterion canary suite. Intended for pre-push diagnostics.

This canary does not measure its own run-to-run noise floor (that would cost
a second full comparison run every invocation). Treat any reported delta as
directional only: a delta smaller than what you have seen this canary report
between two runs of the *same* commit is not a finding, it is noise. Rerun
before trusting a single "regression" or "caution" verdict (#537).

Environment:
  GPY_PERF_CANARY_MODE                 off|warn|strict (default: warn)
  GPY_PERF_CANARY_BASE_REF             git revision to compare against (default: @{upstream})
  GPY_PERF_CANARY_SAMPLE_SIZE          Criterion sample size (default: 20)
  GPY_PERF_CANARY_WARMUP_SECONDS       Criterion warmup time (default: 1)
  GPY_PERF_CANARY_MEASUREMENT_SECONDS  Criterion measurement time (default: 1)
  GPY_PERF_CANARY_WARN_THRESHOLD_PCT   warn threshold (default: 10)
  GPY_PERF_CANARY_FAIL_THRESHOLD_PCT   strict failure threshold (default: 20)
EOF
}

log() {
  printf '[perf-canary] %s\n' "$*"
}

warn() {
  printf '[perf-canary] warning: %s\n' "$*" >&2
}

die() {
  printf '[perf-canary] error: %s\n' "$*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      BASE_REF="$2"
      shift 2
      ;;
    --mode)
      MODE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

case "$MODE" in
  off|warn|strict) ;;
  *)
    die "invalid mode '$MODE' (expected off, warn, or strict)"
    ;;
esac

if [[ "$MODE" == "off" ]]; then
  log "skipping canary comparison (mode=off)"
  exit 0
fi

cd "$PROJECT_ROOT"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  die "must be run inside the repository"
fi

if ! command -v cargo >/dev/null 2>&1; then
  die "cargo is required"
fi

BASE_COMMIT="$(git rev-parse --verify "${BASE_REF}" 2>/dev/null || true)"
if [[ -z "$BASE_COMMIT" ]]; then
  warn "base ref '${BASE_REF}' not found; skipping canary comparison"
  exit 0
fi

HEAD_COMMIT="$(git rev-parse HEAD)"
if [[ "$BASE_COMMIT" == "$HEAD_COMMIT" ]]; then
  log "HEAD matches ${BASE_REF}; nothing to compare"
  exit 0
fi

# Threshold, in percent combined CPU, above which Spotlight's metadata
# processes count as "doing something" rather than merely present (#537).
# Presence alone is not a signal: those processes are resident on essentially
# every macOS machine at all times, idle or not -- measured at 21 resident
# processes, 0.0% CPU total, on a deliberately quiet machine, and the
# presence-only check fired anyway. 5% is comfortably above that idle
# baseline and well below what a genuine indexing storm consumes.
SPOTLIGHT_CPU_THRESHOLD_PCT="${GPY_PERF_CANARY_SPOTLIGHT_CPU_THRESHOLD_PCT:-5}"

check_environment_noise() {
  local noisy=0

  if command -v tmutil >/dev/null 2>&1; then
    if tmutil status 2>/dev/null | grep -q 'Running = 1'; then
      warn "Time Machine is active; filesystem-heavy results may be noisy"
      noisy=1
    fi
  fi

  # Presence is not activity (#537): these processes are resident on almost
  # every macOS machine whether idle or indexing, so a plain `pgrep` fires
  # unconditionally and carries no information either way. Sum their current
  # %CPU instead and only warn once that crosses a real activity threshold,
  # stating what was actually measured rather than just that something exists.
  if command -v pgrep >/dev/null 2>&1 && command -v ps >/dev/null 2>&1; then
    local spotlight_pids spotlight_cpu
    spotlight_pids="$(pgrep -x 'mdworker_shared|mds_stores|corespotlightd|spotlightknowledged' 2>/dev/null | paste -sd, - || true)"
    if [[ -n "$spotlight_pids" ]]; then
      spotlight_cpu="$(ps -o %cpu= -p "$spotlight_pids" 2>/dev/null | awk '{sum += $1} END {printf "%.1f", sum + 0}')"
      if awk -v cpu="$spotlight_cpu" -v threshold="$SPOTLIGHT_CPU_THRESHOLD_PCT" 'BEGIN { exit !(cpu > threshold) }'; then
        warn "Spotlight metadata processes are consuming ${spotlight_cpu}% CPU (threshold ${SPOTLIGHT_CPU_THRESHOLD_PCT}%); filesystem-heavy results may be noisy"
        noisy=1
      fi
    fi
  fi

  return "$noisy"
}

to_ns() {
  awk -v value="$1" -v unit="$2" '
    BEGIN {
      mult = 0;
      if (unit == "ns") mult = 1;
      else if (unit == "us" || unit == "µs") mult = 1000;
      else if (unit == "ms") mult = 1000000;
      else if (unit == "s") mult = 1000000000;
      else exit 2;
      printf "%.0f", value * mult;
    }
  '
}

parse_benchmark_ns() {
  local output="$1"
  local benchmark="$2"
  local line
  local value
  local unit

  line="$(printf '%s\n' "$output" | grep "^${benchmark}[[:space:]]\+time:" | head -n 1 || true)"
  if [[ -z "$line" ]]; then
    return 1
  fi

  value="$(printf '%s\n' "$line" | awk -F'time:[[:space:]]*' '{print $2}' | awk -F'[][]' '{print $2}' | awk '{print $3}')"
  unit="$(printf '%s\n' "$line" | awk -F'time:[[:space:]]*' '{print $2}' | awk -F'[][]' '{print $2}' | awk '{print $4}')"
  to_ns "$value" "$unit"
}

run_canary_bench() {
  local repo_root="$1"
  if [[ ! -f "${repo_root}/gpy-agent/benches/perf_canary_bench.rs" ]]; then
    return 2
  fi
  (
    cd "${repo_root}/gpy-agent"
    GPY_PERF_CANARY_SAMPLE_SIZE="$SAMPLE_SIZE" \
    GPY_PERF_CANARY_WARMUP_SECONDS="$WARM_UP_SECONDS" \
    GPY_PERF_CANARY_MEASUREMENT_SECONDS="$MEASUREMENT_SECONDS" \
    cargo bench --bench perf_canary_bench
  )
}

# Tripwire for #555: this is the only pre-push hook that runs `git worktree
# add`/`remove` against the real repo (everything else in the gate uses
# throwaway fixture repos). A pre-push run corrupted the *shared*
# `.git/config` (visible to every worktree, `main` included) by flipping
# `core.bare` to true; the exact trigger wasn't isolated. Snapshot it here and
# restore + warn loudly if it changes, so a recurrence gets caught with full
# evidence instead of silently reappearing.
CORE_BARE_BEFORE="$(git config --get core.bare 2>/dev/null || printf 'false')"

WORKTREE_DIR="$(mktemp -d /tmp/gpy-perf-canary.XXXXXX)"
cleanup() {
  git worktree remove --force "$WORKTREE_DIR" >/dev/null 2>&1 || rm -rf "$WORKTREE_DIR"

  local core_bare_after
  core_bare_after="$(git config --get core.bare 2>/dev/null || printf 'false')"
  if [[ "$core_bare_after" != "$CORE_BARE_BEFORE" ]]; then
    warn "core.bare changed from '${CORE_BARE_BEFORE}' to '${core_bare_after}' in the shared .git/config during this canary run (#555); restoring it"
    git config core.bare "$CORE_BARE_BEFORE"
  fi
}
trap cleanup EXIT

log "comparing ${HEAD_COMMIT:0:7} against ${BASE_COMMIT:0:7} (mode=${MODE})"
check_environment_noise || true

git worktree add --detach "$WORKTREE_DIR" "$BASE_COMMIT" >/dev/null

log "running baseline canary benchmarks"
set +e
BASE_OUTPUT="$(run_canary_bench "$WORKTREE_DIR" 2>&1)"
BASE_STATUS=$?
set -e
if [[ "$BASE_STATUS" -ne 0 ]]; then
  if [[ "$BASE_STATUS" -eq 2 ]]; then
    warn "base commit ${BASE_COMMIT:0:7} does not contain perf_canary_bench; skipping comparison"
    exit 0
  fi
  warn "baseline canary bench failed; skipping comparison"
  printf '%s\n' "$BASE_OUTPUT" >&2
  exit 0
fi
log "running current canary benchmarks"
set +e
CURRENT_OUTPUT="$(run_canary_bench "$PROJECT_ROOT" 2>&1)"
CURRENT_STATUS=$?
set -e
if [[ "$CURRENT_STATUS" -ne 0 ]]; then
  if [[ "$MODE" == "strict" ]]; then
    printf '%s\n' "$CURRENT_OUTPUT" >&2
    die "current canary bench failed"
  fi
  warn "current canary bench failed; skipping comparison"
  printf '%s\n' "$CURRENT_OUTPUT" >&2
  exit 0
fi

printf '%-24s %12s %12s %10s %s\n' "benchmark" "base" "current" "delta" "status"

strict_failures=0
warn_count=0

for benchmark in "${BENCHMARKS[@]}"; do
  base_ns="$(parse_benchmark_ns "$BASE_OUTPUT" "$benchmark" || true)"
  current_ns="$(parse_benchmark_ns "$CURRENT_OUTPUT" "$benchmark" || true)"

  if [[ -z "${base_ns:-}" || -z "${current_ns:-}" ]]; then
    printf '%-24s %12s %12s %10s %s\n' "$benchmark" "missing" "missing" "n/a" "unparsed"
    warn_count=$((warn_count + 1))
    continue
  fi

  delta_pct="$(awk -v base="$base_ns" -v current="$current_ns" 'BEGIN { printf "%.1f%%", ((current - base) / base) * 100 }')"
  delta_abs="$(awk -v base="$base_ns" -v current="$current_ns" 'BEGIN { printf "%.1f", ((current - base) / base) * 100 }')"

  status="ok"
  if awk -v delta="$delta_abs" -v fail="$FAIL_THRESHOLD_PCT" 'BEGIN { exit !(delta > fail) }'; then
    status="regression"
    warn_count=$((warn_count + 1))
    if [[ "$MODE" == "strict" ]]; then
      strict_failures=$((strict_failures + 1))
    fi
  elif awk -v delta="$delta_abs" -v warn_threshold="$WARN_THRESHOLD_PCT" 'BEGIN { exit !(delta > warn_threshold) }'; then
    status="caution"
    warn_count=$((warn_count + 1))
  fi

  printf '%-24s %12s %12s %10s %s\n' \
    "$benchmark" \
    "${base_ns}ns" \
    "${current_ns}ns" \
    "$delta_pct" \
    "$status"
done

if [[ "$warn_count" -gt 0 ]]; then
  warn "canary detected ${warn_count} notable delta(s); use this as a diagnostic signal, not a definitive claim"
  warn "this canary does not measure its own run-to-run spread -- a delta smaller than what a rerun of the SAME commit reports is noise, not a finding (#537); rerun before trusting a single result"
fi

if [[ "$strict_failures" -gt 0 ]]; then
  die "strict mode failed with ${strict_failures} regression(s)"
fi

log "canary comparison complete"
