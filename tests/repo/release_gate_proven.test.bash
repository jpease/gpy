#!/usr/bin/env bash
# tests/repo/release_gate_proven.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# scripts/release-gate-proven.sh decides whether release.yml may skip the gate
# legs. Driven offline by a stub `gh` (GPY_GH) over a throwaway git repo:
#   (a) a squash-merged commit whose PR head has the same tree and a green
#       gate-result is proven
#   (b) every doubt is NOT proven (fails closed): different tree, no merged PR,
#       two merged PRs, PR merged elsewhere, gate-result failed or missing,
#       no pr-gate run, API error
#   (c) release.yml consults the script and gates every expensive step on it

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command git
test_require_command jq

SCRIPT="$ROOT/scripts/release-gate-proven.sh"
REPO="owner/repo"

work="$(mktemp -d "${TMPDIR:-/tmp}/release-gate-proven.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# A repo with one commit; its tree is what the PR head must match.
git_repo="$work/repo"
mkdir -p "$git_repo"
(
    cd "$git_repo" || exit 1
    unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
    git init -q .
    echo hello >file.txt
    git add file.txt
    git -c user.name=t -c user.email=t@example.com -c commit.gpgsign=false commit -q -m squash
) || {
    fail "could not build the fixture repository"
    test_finish
}
SHA="$(git -C "$git_repo" rev-parse HEAD)"
TREE="$(git -C "$git_repo" rev-parse 'HEAD^{tree}')"
HEAD_SHA="1111111111111111111111111111111111111111"
OTHER_TREE="2222222222222222222222222222222222222222"

# Stub gh: `gh api <endpoint>` prints $FIXTURES/<endpoint with / ? & = -> _>
# and exits 1 when the file is missing (an API error).
stub="$work/gh"
cat >"$stub" <<'STUB'
#!/usr/bin/env bash
[ "$1" = api ] || exit 2
name="$(printf '%s' "$2" | tr '/?&=' '____')"
[ -f "$FIXTURES/$name" ] || exit 1
cat "$FIXTURES/$name"
STUB
chmod +x "$stub"

fx_name() { printf '%s' "$1" | tr '/?&=' '____'; }

# `scenario NAME` -- a fresh happy-path fixture dir; echoes its path.
scenario() {
    local d="$work/fx-$1"
    mkdir -p "$d"
    jq -n --arg sha "$SHA" --arg head "$HEAD_SHA" \
        '[{number: 7, merged_at: "2026-01-01T00:00:00Z", merge_commit_sha: $sha, base: {ref: "main"}, head: {sha: $head}}]' \
        >"$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")"
    jq -n --arg t "$TREE" '{commit: {tree: {sha: $t}}}' >"$d/$(fx_name "repos/$REPO/commits/$HEAD_SHA")"
    jq -n '{workflow_runs: [{id: 42, path: ".github/workflows/pr-gate.yml"}]}' \
        >"$d/$(fx_name "repos/$REPO/actions/runs?head_sha=$HEAD_SHA&event=pull_request&per_page=100")"
    jq -n '{jobs: [{name: "gate-legs / rust", conclusion: "success"}, {name: "gate-result", conclusion: "success"}]}' \
        >"$d/$(fx_name "repos/$REPO/actions/runs/42/jobs?per_page=100")"
    printf '%s' "$d"
}

# `expect NAME FIXTURE_DIR true|false`
expect() {
    local out got reason
    out="$(cd "$git_repo" && FIXTURES="$2" GPY_GH="$stub" bash "$SCRIPT" --sha "$SHA" --repo "$REPO" 2>/dev/null)"
    got="$(sed -n 's/^proven=//p' <<<"$out")"
    reason="$(sed -n 's/^reason=//p' <<<"$out")"
    if [[ "$got" == "$3" ]]; then
        pass "$1: proven=$got ($reason)"
    else
        fail "$1: expected proven=$3, got '${got:-nothing}' (${reason:-no reason}); stdout: $out"
    fi
}

echo "--- (a) the proof holds ---"
d="$(scenario ok)"
expect "squash merge, same tree, green gate-result" "$d" true

echo "--- (b) every doubt runs the full gate ---"
d="$(scenario tree)"
jq -n --arg t "$OTHER_TREE" '{commit: {tree: {sha: $t}}}' >"$d/$(fx_name "repos/$REPO/commits/$HEAD_SHA")"
expect "PR head tree differs from the tagged tree" "$d" false

d="$(scenario nopr)"
echo '[]' >"$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")"
expect "no PR landed the commit" "$d" false

d="$(scenario unmerged)"
jq '.[0].merged_at = null' "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")" >"$d/tmp" &&
    mv "$d/tmp" "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")"
expect "PR not merged" "$d" false

d="$(scenario othermerge)"
jq '.[0].merge_commit_sha = "3333333333333333333333333333333333333333"' "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")" >"$d/tmp" &&
    mv "$d/tmp" "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")"
expect "the PR's merge commit is another commit" "$d" false

d="$(scenario otherbase)"
jq '.[0].base.ref = "release"' "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")" >"$d/tmp" &&
    mv "$d/tmp" "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")"
expect "the PR merged into another branch" "$d" false

d="$(scenario twoprs)"
jq '. + .' "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")" >"$d/tmp" &&
    mv "$d/tmp" "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")"
expect "ambiguous: two merged PRs" "$d" false

d="$(scenario failed)"
jq -n '{jobs: [{name: "gate-result", conclusion: "failure"}]}' >"$d/$(fx_name "repos/$REPO/actions/runs/42/jobs?per_page=100")"
expect "gate-result failed" "$d" false

d="$(scenario skipped)"
jq -n '{jobs: [{name: "gate-legs / rust", conclusion: "success"}]}' >"$d/$(fx_name "repos/$REPO/actions/runs/42/jobs?per_page=100")"
expect "no gate-result job" "$d" false

d="$(scenario wrongworkflow)"
jq -n '{workflow_runs: [{id: 42, path: ".github/workflows/other.yml"}]}' \
    >"$d/$(fx_name "repos/$REPO/actions/runs?head_sha=$HEAD_SHA&event=pull_request&per_page=100")"
expect "only another workflow ran" "$d" false

d="$(scenario noruns)"
echo '{"workflow_runs": []}' >"$d/$(fx_name "repos/$REPO/actions/runs?head_sha=$HEAD_SHA&event=pull_request&per_page=100")"
expect "no runs on the head commit" "$d" false

d="$(scenario apierror)"
rm "$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")"
expect "the pulls API errors" "$d" false

d="$(scenario headerror)"
rm "$d/$(fx_name "repos/$REPO/commits/$HEAD_SHA")"
expect "the head commit lookup errors" "$d" false

d="$(scenario jobserror)"
rm "$d/$(fx_name "repos/$REPO/actions/runs/42/jobs?per_page=100")"
expect "the jobs API errors" "$d" false

d="$(scenario garbage)"
echo 'not json' >"$d/$(fx_name "repos/$REPO/commits/$SHA/pulls")"
expect "garbled API response" "$d" false

out="$(cd "$git_repo" && FIXTURES="$d" GPY_GH="$stub" bash "$SCRIPT" --sha "0000000000000000000000000000000000000000" --repo "$REPO" 2>/dev/null)"
if [[ "$out" == proven=false* ]]; then
    pass "an unknown commit is not proven"
else
    fail "an unknown commit must not be proven: $out"
fi

echo "--- (c) release.yml consults the proof and gates the expensive steps ---"
require_yq
WF="$ROOT/.github/workflows/release.yml"
if [[ "$(yq '.jobs.validate.steps[] | select(.id == "proven") | .run' "$WF")" == *release-gate-proven.sh* ]]; then
    pass "validate runs scripts/release-gate-proven.sh"
else
    fail "validate has no step with id 'proven' running scripts/release-gate-proven.sh"
fi
for step in "Rust toolchain gate" "Shell integration gate" "Performance budgets"; do
    cond="$(yq ".jobs.validate.steps[] | select(.name == \"$step\") | .if" "$WF")"
    if [[ "$cond" == *"steps.proven.outputs.proven != 'true'"* ]]; then
        pass "'$step' is skipped only when the gate is proven"
    else
        fail "'$step' is not conditioned on steps.proven.outputs.proven != 'true' (got '$cond')"
    fi
done
for step in "Resolve and validate version" "Require the release commit to be on main"; do
    cond="$(yq ".jobs.validate.steps[] | select(.name == \"$step\") | .if // \"\"" "$WF")"
    if [[ "$cond" != *proven* ]]; then
        pass "'$step' always runs"
    else
        fail "'$step' must not depend on the proof (got '$cond')"
    fi
done

test_finish "release gate proof fails closed"
