#!/usr/bin/env bash
# Sourced by scripts/bench.sh and scripts/ci-bench.sh: the performance budgets,
# the release agent they measure and the log helpers. The caller sets
# PROJECT_ROOT first.
#
# PERF_BUDGETS_FILE overrides the budget file (default benchmarks/perf-budgets.json).
# A budget that cannot be read ends the run with exit status 2 and a message
# naming it: a gate must never fall back to numbers of its own.

PERF_BUDGETS_FILE="${PERF_BUDGETS_FILE:-$PROJECT_ROOT/benchmarks/perf-budgets.json}"

bench_die() {
  echo "Error: $1" >&2
  exit 2
}

# Plain-text log lines in the house prefixes (stdout, no colour).
bench_info() { echo "info: $1"; }
bench_pass() { echo "PASS: $1"; }
bench_warn() { echo "warn: $1"; }
bench_fail() { echo "FAIL: $1"; }

# bench_load_budgets VAR=PATH[*SCALE]...
#
# Reads every PATH (a jq path such as .budgets.ipc_latency.max_ms) from
# PERF_BUDGETS_FILE in one jq call and assigns it to the global VAR. With *SCALE
# the value is multiplied by SCALE and rounded to an integer, which turns a
# millisecond budget into the whole nanoseconds bash arithmetic needs
# (.budgets.ipc_latency.max_ms*1000000). Exits 2 when jq or the file is
# missing, the file is not valid JSON, or a PATH is absent or not a number.
bench_load_budgets() {
  command -v jq >/dev/null 2>&1 || bench_die "jq is required to read the budgets in $PERF_BUDGETS_FILE"
  [[ -f "$PERF_BUDGETS_FILE" ]] || bench_die "budget file not found: $PERF_BUDGETS_FILE"

  local spec path scale expr filter="" values jq_err jq_msg line i
  local -a vars=() vals=()
  for spec in "$@"; do
    vars+=("${spec%%=*}")
    path="${spec#*=}"
    expr="\$v"
    if [[ "$path" == *'*'* ]]; then
      scale="${path#*\*}"
      path="${path%%\**}"
      expr="(\$v * ${scale} | round)"
    fi
    filter+="${filter:+, }( (${path}) as \$v | if (\$v | type) == \"number\" then ${expr} else error(\"${path} is missing or not a number\") end )"
  done

  jq_err="$(mktemp "${TMPDIR:-/tmp}/gpy-bench-jq.XXXXXX")"
  if ! values="$(jq -r "$filter" "$PERF_BUDGETS_FILE" 2>"$jq_err")"; then
    jq_msg="$(<"$jq_err")"
    rm -f "$jq_err"
    bench_die "cannot read the budgets in $PERF_BUDGETS_FILE: $jq_msg"
  fi
  rm -f "$jq_err"

  while IFS= read -r line; do
    vals+=("$line")
  done <<<"$values"
  [[ ${#vals[@]} -eq ${#vars[@]} ]] || bench_die "expected ${#vars[@]} budget values from $PERF_BUDGETS_FILE, read ${#vals[@]}"
  for ((i = 0; i < ${#vars[@]}; i++)); do
    printf -v "${vars[i]}" '%s' "${vals[i]}"
  done
}

# bench_build_release_agent AGENT_DIR: cargo no-ops an up-to-date build, so
# a binary left behind by an older commit is never the one measured.
bench_build_release_agent() {
  (cd "$1" && cargo build --release --quiet) || bench_die "failed to build the release agent in $1"
}
