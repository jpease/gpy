#!/usr/bin/env bash
# tests/repo/workflow_hardening.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The hardening of .github/ cannot regress silently.
#   (a) every non-local `uses:` is pinned to a full commit SHA plus `# vX.Y.Z`
#   (b) every workflow declares top-level `permissions:` with no write grant
#   (c) only release.yml's create-release job holds write grants
#   (d) every actions/checkout sets `persist-credentials: false`
#   (e) no `run:` script interpolates `${{ github.event.* }}` (pass it via env)
#   (f) dependabot.yml covers github-actions weekly and no cargo ecosystem
#   (g) nothing downloads a script and pipes it into a shell

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

require_yq

shopt -s nullglob
workflows=(.github/workflows/*.yml)
actions=(.github/actions/*/action.yml)
if [[ ${#workflows[@]} -eq 0 ]]; then
    fail "no workflows found under .github/workflows"
fi

# Every step of a workflow or composite action, as one yq expression.
STEPS='(.jobs[].steps[]?, .runs.steps[]?)'
# Every string that contains "write" under a `permissions:` key.
WRITES='[.. | select(tag == "!!map" and has("permissions")) | .permissions | .. | select(tag == "!!str" and test("write"))] | length'

echo "--- (a) every uses: is a full commit SHA with a version comment ---"
for wf in "${workflows[@]}" "${actions[@]}"; do
    while IFS= read -r ref; do
        [[ -n "$ref" ]] || continue
        case "$ref" in ./*) continue ;; esac
        if [[ "$ref" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._/-]+@[0-9a-f]{40}\ \#\ v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            pass "$wf: pinned: ${ref%% *}"
        else
            fail "$wf: not pinned to <sha> # vX.Y.Z: $ref"
        fi
    done < <(yq '.. | select(tag == "!!map" and has("uses")) | .uses + " # " + (.uses | line_comment)' "$wf" | sed -E 's/ +$//')
done

echo "--- (b) top-level permissions are declared and read-only ---"
for wf in "${workflows[@]}"; do
    if [[ "$(yq 'has("permissions")' "$wf")" != true ]]; then
        fail "$wf declares no top-level permissions:"
        continue
    fi
    top_writes="$(yq '[.permissions | .. | select(tag == "!!str" and test("write"))] | length' "$wf")"
    if [[ "$top_writes" -ne 0 ]]; then
        fail "$wf grants write at the top level"
    else
        pass "$wf: top-level permissions declared, no write"
    fi
done

echo "--- (c) write grants exist only in release.yml create-release ---"
for wf in "${workflows[@]}"; do
    writes="$(yq "$WRITES" "$wf")"
    if [[ "$wf" == ".github/workflows/release.yml" ]]; then
        inblock="$(yq ".jobs.create-release | $WRITES" "$wf")"
        if [[ "$writes" -eq "$inblock" && "$inblock" -gt 0 ]]; then
            pass "$wf: all $writes write grants sit in create-release"
        else
            fail "$wf: write grants outside create-release ($writes total, $inblock inside)"
        fi
    elif [[ "$writes" -ne 0 ]]; then
        fail "$wf: unexpected write permission"
    else
        pass "$wf: no write permissions"
    fi
done

echo "--- (d) checkout never persists credentials ---"
for wf in "${workflows[@]}"; do
    total="$(yq "[$STEPS | select(has(\"uses\")) | select(.uses | test(\"^actions/checkout@\"))] | length" "$wf")"
    safe="$(yq "[$STEPS | select(has(\"uses\")) | select(.uses | test(\"^actions/checkout@\")) | select(.with.\"persist-credentials\" == false)] | length" "$wf")"
    if [[ "$total" -eq "$safe" ]]; then
        pass "$wf: $total checkout(s), all persist-credentials: false"
    else
        fail "$wf: $total checkout(s) but $safe persist-credentials: false"
    fi
done

echo "--- (e) no github.event.* inside run: scripts ---"
for wf in "${workflows[@]}" "${actions[@]}"; do
    hits="$(yq "$STEPS | select(has(\"run\")) | .run" "$wf" | grep -nE '\$\{\{[^}]*github\.event\.')"
    if [[ -n "$hits" ]]; then
        fail "$wf interpolates github.event.* into a run: script: $hits"
    else
        pass "$wf: no github.event.* in run: scripts"
    fi
done

echo "--- (f) dependabot covers github-actions weekly, not cargo ---"
dep=".github/dependabot.yml"
if [[ ! -f "$dep" ]]; then
    fail "$dep is missing"
else
    ecosystems="$(yq '.updates[]."package-ecosystem"' "$dep")"
    if grep -qxF github-actions <<<"$ecosystems"; then pass "$dep: github-actions"; else fail "$dep lacks github-actions"; fi
    if yq '.updates[].schedule.interval' "$dep" | grep -qxF weekly; then pass "$dep: weekly"; else fail "$dep lacks a weekly interval"; fi
    if grep -qxF cargo <<<"$ecosystems"; then
        fail "$dep configures cargo version updates"
    else
        pass "$dep: no cargo ecosystem"
    fi
fi

echo "--- (g) no download piped into a shell ---"
for wf in "${workflows[@]}" "${actions[@]}"; do
    # Join backslash-continued lines so a split `curl ... \ | sh` is caught.
    hits="$(yq "$STEPS | select(has(\"run\")) | .run" "$wf" | perl -0pe 's/\\\n//g' |
        grep -E '(curl|wget)[^|]*\|[[:space:]]*(sudo[[:space:]]+)?(env[[:space:]]+[^|]*)?(ba|z|da|k)?sh\b')"
    if [[ -n "$hits" ]]; then
        fail "$wf pipes a download into a shell: $hits"
    else
        pass "$wf: no curl/wget piped into a shell"
    fi
done

test_finish "workflows keep their hardening"
