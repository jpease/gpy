#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Decide whether the release gate (Rust + shell legs, lint, perf budgets)
# already passed on exactly the code being released, so release.yml's
# `validate` job need not run it again.
#
# A release is tagged on a commit on main made by a squash merge. The PR gate
# ran on the PR head commit: a different SHA, but usually the identical git
# TREE. The proof holds only when ALL of these are true:
#   1. exactly one merged PR into main has the tagged commit as its
#      merge_commit_sha (a squash or merge-commit landing of that PR)
#   2. that PR's head commit has the same tree SHA as the tagged commit
#   3. a `pr-gate.yml` pull_request run on that head commit has a `gate-result`
#      job whose conclusion is `success`
# Anything else, including any API or git error, is "not proven": the caller
# runs the full gate. This script fails closed and never exits non-zero for a
# negative answer.
#
# cross-platform-test.yml (push to main) is deliberately not consulted: it has
# no aggregate job and does not run the performance budgets, so a green run
# there does not equal the PR gate's `gate-result`.
#
# Usage:
#   scripts/release-gate-proven.sh --sha SHA --repo OWNER/REPO
#
# stdout (and nothing else): `proven=true|false` and `reason=<one line>`, ready
# to append to $GITHUB_OUTPUT. The explanation is also printed to stderr.
# Needs `gh` (GH_TOKEN with contents, actions and pull-requests read), `jq`,
# and `git` in a checkout containing SHA. GPY_GH overrides the gh command
# (tests/repo/release_gate_proven.test.bash).

set -uo pipefail

GH="${GPY_GH:-gh}"
SHA=""
REPO=""

emit() {
    # $1 = true|false, $2 = reason (single line)
    printf 'proven=%s\nreason=%s\n' "$1" "$2"
    printf 'release gate proof: %s (%s)\n' "$1" "$2" >&2
    exit 0
}

unproven() {
    emit false "$*; running the full gate"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --sha) SHA="${2:-}"; shift 2 ;;
        --repo) REPO="${2:-}"; shift 2 ;;
        *) unproven "unknown argument $1" ;;
    esac
done

[ -n "$SHA" ] && [ -n "$REPO" ] || unproven "usage: --sha SHA --repo OWNER/REPO"
command -v jq >/dev/null 2>&1 || unproven "jq is not installed"

api() {
    "$GH" api "$1"
}

tree="$(git rev-parse --verify "${SHA}^{tree}" 2>/dev/null)" || unproven "cannot resolve the tree of $SHA"

pulls="$(api "repos/$REPO/commits/$SHA/pulls")" || unproven "could not list pull requests for $SHA"
matches="$(jq -c --arg sha "$SHA" \
    '[.[] | select(.merged_at != null and .merge_commit_sha == $sha and .base.ref == "main")]' <<<"$pulls")" ||
    unproven "unparseable pull request response"
count="$(jq 'length' <<<"$matches")" || unproven "unparseable pull request response"
[ "$count" = 1 ] || unproven "$count merged pull requests into main landed $SHA (need exactly 1)"

pr_number="$(jq -r '.[0].number' <<<"$matches")"
head_sha="$(jq -r '.[0].head.sha // empty' <<<"$matches")"
[ -n "$head_sha" ] || unproven "PR #$pr_number has no head SHA"

head_tree="$(api "repos/$REPO/commits/$head_sha" | jq -r '.commit.tree.sha // empty')" ||
    unproven "could not read the tree of PR #$pr_number head $head_sha"
[ -n "$head_tree" ] || unproven "could not read the tree of PR #$pr_number head $head_sha"
[ "$head_tree" = "$tree" ] ||
    unproven "tree differs: PR #$pr_number head $head_sha has $head_tree, $SHA has $tree"

runs="$(api "repos/$REPO/actions/runs?head_sha=$head_sha&event=pull_request&per_page=100")" ||
    unproven "could not list workflow runs for $head_sha"
run_ids="$(jq -r '.workflow_runs[]? | select((.path // "") | startswith(".github/workflows/pr-gate.yml")) | .id' <<<"$runs")" ||
    unproven "unparseable workflow run response"
[ -n "$run_ids" ] || unproven "no pr-gate.yml run found for PR #$pr_number head $head_sha"

while IFS= read -r run_id; do
    jobs="$(api "repos/$REPO/actions/runs/$run_id/jobs?per_page=100")" || continue
    ok="$(jq -r '[.jobs[]? | select(.name == "gate-result" and .conclusion == "success")] | length' <<<"$jobs")" || continue
    if [ "$ok" -ge 1 ] 2>/dev/null; then
        emit true "PR #$pr_number head $head_sha passed gate-result in run $run_id and has the same tree $tree as $SHA"
    fi
done <<<"$run_ids"

unproven "no successful gate-result for PR #$pr_number head $head_sha"
