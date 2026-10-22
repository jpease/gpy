#!/usr/bin/env bash
# tests/bash/ci_gate_shape.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# pr-gate.yml cannot silently stop gating:
#   (a) it runs on pull_request, never pull_request_target, and uses no secrets
#   (b) `gate-result` always runs and `needs` every gating job (every job
#       except itself and the continue-on-error ones), so a red leg fails the
#       one required check
#   (c) the shell job's shards run every `quality-check.sh --shell-only`
#       part exactly once, so sharding cannot drop a suite
#
# Text-level checks: no YAML parser is guaranteed where the shell gate runs.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

PR_GATE=".github/workflows/pr-gate.yml"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

# The lines of one top-level job, up to the next job key.
job_block() {
    awk -v job="$2" '
        $0 ~ "^  " job ":$" { in_job = 1; print; next }
        in_job && /^  [a-z][a-z0-9_-]*:$/ { exit }
        in_job { print }
    ' "$1"
}

job_ids() {
    awk '/^jobs:$/ { in_jobs = 1; next } in_jobs && /^  [a-z][a-z0-9_-]*:$/ { sub(/^  /, ""); sub(/:$/, ""); print }' "$1"
}

echo "--- (a) fork-safe trigger, no secrets ---"
if grep -qE '^  pull_request:$' "$PR_GATE"; then
    pass "pr-gate runs on pull_request"
else
    fail "pr-gate has no pull_request trigger"
fi
if grep -qE '^\s*pull_request_target:' "$PR_GATE"; then
    fail "pr-gate uses pull_request_target, which hands an untrusted PR the repository's trust"
else
    pass "pr-gate has no pull_request_target trigger"
fi
if grep -qE '\$\{\{\s*secrets\.' "$PR_GATE"; then
    fail "pr-gate references secrets; a fork PR must run with none"
else
    pass "pr-gate references no secrets"
fi

echo "--- (b) gate-result needs every gating job ---"
agg="$(job_block "$PR_GATE" gate-result)"
if [[ -z "$agg" ]]; then
    fail "pr-gate.yml has no gate-result job"
    test_finish
fi
if grep -qE '^    if: \$\{\{ always\(\) \}\}$' <<<"$agg"; then
    pass "gate-result runs even when a leg failed or was cancelled"
else
    fail "gate-result needs 'if: \${{ always() }}', or a failed leg leaves it skipped (not red)"
fi
needs_line="$(grep -E '^    needs: \[' <<<"$agg" | head -n 1)"
gating=0
while IFS= read -r job; do
    [[ "$job" == gate-result ]] && continue
    job_block "$PR_GATE" "$job" | grep -qE '^    continue-on-error: true$' && continue
    gating=$((gating + 1))
    if [[ "$needs_line" =~ [\[,\ ]${job}[],\ ] ]]; then
        pass "gate-result needs $job"
    else
        fail "gate-result does not need $job, so a red $job would not fail the required check: $needs_line"
    fi
done < <(job_ids "$PR_GATE")
[[ "$gating" -gt 0 ]] || fail "found no gating jobs in $PR_GATE"

echo "--- (c) the shell shards cover every --shell-only part once ---"
all_parts="$(sed -n 's/^SHELL_PARTS_ALL="\(.*\)"$/\1/p' scripts/quality-check.sh)"
shard_line="$(job_block "$PR_GATE" shell | grep -E '^        parts: \[' | head -n 1)"
if [[ -z "$all_parts" || -z "$shard_line" ]]; then
    fail "could not read SHELL_PARTS_ALL from quality-check.sh or the shell job's 'parts:' matrix line"
else
    want="$(tr ',' '\n' <<<"$all_parts" | sort)"
    have="$(sed -E 's/^ *parts: \[//; s/\][[:space:]]*$//; s/[" ]//g' <<<"$shard_line" | tr ',' '\n' | sort)"
    if [[ "$have" == "$want" ]]; then
        pass "the shell shards run each of $all_parts exactly once"
    else
        fail "the shell shards run [$(tr '\n' ' ' <<<"$have")] but each of [$(tr '\n' ' ' <<<"$want")] must run exactly once: $shard_line"
    fi
fi

test_finish "the CI gate keeps its shape"
