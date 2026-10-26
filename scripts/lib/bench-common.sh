#!/usr/bin/env bash
# Sourced by scripts/bench.sh and scripts/ci-bench.sh: the performance budgets
# and the release agent they measure. The caller sets PROJECT_ROOT first.
#
# BASELINE_FILE overrides the budget file (default tests/performance-baselines.json).
# A budget that cannot be read ends the run with exit status 2 and a message
# naming it: a gate must never fall back to numbers of its own.

BASELINE_FILE="${BASELINE_FILE:-$PROJECT_ROOT/tests/performance-baselines.json}"

bench_die() {
  echo "Error: $1" >&2
  exit 2
}

# bench_load_baselines VAR=PATH[*SCALE]...
#
# Reads every PATH (a jq path such as .budgets.ipc_latency.max_ms) from
# BASELINE_FILE in one jq call and assigns it to the global VAR. With *SCALE
# the value is multiplied by SCALE and rounded to an integer, which turns a
# millisecond budget into the whole nanoseconds bash arithmetic needs
# (.budgets.ipc_latency.max_ms*1000000). Exits 2 when jq or the file is
# missing, the file is not valid JSON, or a PATH is absent or not a number.
bench_load_baselines() {
  command -v jq >/dev/null 2>&1 || bench_die "jq is required to read the budgets in $BASELINE_FILE"
  [[ -f "$BASELINE_FILE" ]] || bench_die "baseline file not found: $BASELINE_FILE"

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
  if ! values="$(jq -r "$filter" "$BASELINE_FILE" 2>"$jq_err")"; then
    jq_msg="$(<"$jq_err")"
    rm -f "$jq_err"
    bench_die "cannot read the budgets in $BASELINE_FILE: $jq_msg"
  fi
  rm -f "$jq_err"

  while IFS= read -r line; do
    vals+=("$line")
  done <<<"$values"
  [[ ${#vals[@]} -eq ${#vars[@]} ]] || bench_die "expected ${#vars[@]} budget values from $BASELINE_FILE, read ${#vals[@]}"
  for ((i = 0; i < ${#vars[@]}; i++)); do
    printf -v "${vars[i]}" '%s' "${vals[i]}"
  done
}

# bench_build_release_agent AGENT_DIR: cargo no-ops an up-to-date build, so
# a binary left behind by an older commit is never the one measured.
bench_build_release_agent() {
  (cd "$1" && cargo build --release --quiet) || bench_die "failed to build the release agent in $1"
}
