#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Watch a pull request's PR Quality Gate run on jpease/gpy until it finishes,
# then print every job that did not succeed with its check-run annotations.
# The annotations carry the reason: a test failure, a timeout, or GitHub
# never assigning a runner ("not acquired by Runner of type hosted").
#
# Usage: scripts/ci-watch.sh <pr-number> [run-id]
# Exit status: 0 when the run succeeded, 1 otherwise, 2 on usage error.
set -euo pipefail

repo=jpease/gpy
pr="${1:-}"
if [ -z "$pr" ]; then
    echo "usage: $0 <pr-number> [run-id]" >&2
    exit 2
fi

run="${2:-}"
if [ -z "$run" ]; then
    branch="$(gh pr view "$pr" -R "$repo" --json headRefName -q .headRefName)"
    run="$(gh run list -R "$repo" --branch "$branch" --workflow pr-gate.yml \
        --limit 1 --json databaseId -q '.[0].databaseId')"
fi
if [ -z "$run" ]; then
    echo "no PR Quality Gate run found for PR $pr" >&2
    exit 1
fi

echo "watching run $run (PR $pr)"
# Poll the run itself: `gh run watch` can exit non-zero while jobs still run.
while [ "$(gh run view "$run" -R "$repo" --json status -q .status)" != completed ]; do
    sleep 60
done
conclusion="$(gh run view "$run" -R "$repo" --json conclusion -q .conclusion)"
if [ "$conclusion" = success ]; then
    echo "run $run succeeded"
    exit 0
fi

echo "run $run concluded $conclusion:"
gh api "repos/$repo/actions/runs/$run/jobs" --paginate \
    -q '.jobs[] | select(.conclusion != "success" and .conclusion != "skipped")
        | "\(.id)\t\(.conclusion)\t\(.name)\trunner=\(.runner_name // "")"' |
    while IFS=$'\t' read -r job conclusion name runner; do
        printf '\n%s [%s] %s (job %s)\n' "$name" "$conclusion" "$runner" "$job"
        gh api "repos/$repo/check-runs/$job/annotations" \
            -q '.[] | "  \(.annotation_level): \(.message | gsub("\n"; " "))"' |
            sort -u
    done
exit 1
