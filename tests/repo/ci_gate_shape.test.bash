#!/usr/bin/env bash
# tests/repo/ci_gate_shape.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# pr-gate.yml cannot silently stop gating:
#   (a) it runs on pull_request, never pull_request_target, and uses no secrets
#   (b) `gate-result` always runs and `needs` every gating job (every job
#       except itself and the continue-on-error ones), so a red leg fails the
#       one required check
#   (c) the shell job's shards (in gate-legs.yml, which pr-gate.yml calls as
#       its `gate-legs` job) run every `quality-check.sh --shell-only` part
#       exactly once, so sharding cannot drop a suite

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

PR_GATE=".github/workflows/pr-gate.yml"
GATE_LEGS=".github/workflows/gate-legs.yml"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

require_yq

echo "--- (a) fork-safe trigger, no secrets ---"
if [[ "$(yq '.on | has("pull_request")' "$PR_GATE")" == true ]]; then
    pass "pr-gate runs on pull_request"
else
    fail "pr-gate has no pull_request trigger"
fi
if [[ "$(yq '.on | has("pull_request_target")' "$PR_GATE")" == true ]]; then
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
gate_if="$(yq '.jobs.gate-result.if // ""' "$PR_GATE")"
if [[ "$(yq '.jobs | has("gate-result")' "$PR_GATE")" != true ]]; then
    fail "pr-gate.yml has no gate-result job"
    test_finish
fi
# shellcheck disable=SC2016 # a literal GitHub expression, not a shell expansion
if [[ "$gate_if" == '${{ always() }}' ]]; then
    pass "gate-result runs even when a leg failed or was cancelled"
else
    fail "gate-result needs 'if: \${{ always() }}', or a failed leg leaves it skipped (not red)"
fi
needs="$(yq '.jobs.gate-result.needs[]' "$PR_GATE")"
gating=0
while IFS= read -r job; do
    [[ "$job" == gate-result ]] && continue
    [[ "$(yq ".jobs.\"$job\".continue-on-error" "$PR_GATE")" == true ]] && continue
    gating=$((gating + 1))
    if grep -qxF -- "$job" <<<"$needs"; then
        pass "gate-result needs $job"
    else
        fail "gate-result does not need $job, so a red $job would not fail the required check: $(tr '\n' ' ' <<<"$needs")"
    fi
done < <(yq '.jobs | keys | .[]' "$PR_GATE")
[[ "$gating" -gt 0 ]] || fail "found no gating jobs in $PR_GATE"
if [[ "$(yq '.jobs."gate-legs".uses' "$PR_GATE")" == ./.github/workflows/gate-legs.yml ]]; then
    pass "pr-gate calls gate-legs.yml"
else
    fail "pr-gate's gate-legs job does not call ./.github/workflows/gate-legs.yml"
fi

echo "--- (c) the shell shards cover every --shell-only part once ---"
all_parts="$(sed -n 's/^SHELL_PARTS_ALL="\(.*\)"$/\1/p' scripts/quality-check.sh)"
shard_entries="$(yq '.jobs.shell.strategy.matrix.parts[]' "$GATE_LEGS")"
if [[ -z "$all_parts" || -z "$shard_entries" ]]; then
    fail "could not read SHELL_PARTS_ALL from quality-check.sh or the matrix parts in gate-legs.yml"
else
    want="$(tr ',' '\n' <<<"$all_parts" | sort)"
    have="$(tr ',' '\n' <<<"$shard_entries" | sort)"
    if [[ "$have" == "$want" ]]; then
        pass "the shell shards run each of $all_parts exactly once"
    else
        fail "the shell shards run [$(tr '\n' ' ' <<<"$have")] but each of [$(tr '\n' ' ' <<<"$want")] must run exactly once: $(tr '\n' ' ' <<<"$shard_entries")"
    fi
fi

test_finish "the CI gate keeps its shape"
