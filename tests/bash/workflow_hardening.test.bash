#!/usr/bin/env bash
# tests/bash/workflow_hardening.test.bash
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
#
# Text-level checks: no YAML parser is guaranteed where the shell gate runs.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shopt -s nullglob
workflows=(.github/workflows/*.yml)
actions=(.github/actions/*/action.yml)
if [[ ${#workflows[@]} -eq 0 ]]; then
    fail "no workflows found under .github/workflows"
fi

echo "--- (a) every uses: is a full commit SHA with a version comment ---"
for wf in "${workflows[@]}" "${actions[@]}"; do
    while IFS= read -r line; do
        ref="${line#*uses:}"
        ref="${ref# }"
        case "$ref" in ./*) continue ;; esac
        if [[ "$ref" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._/-]+@[0-9a-f]{40}\ \#\ v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            pass "$wf: pinned: ${ref%% *}"
        else
            fail "$wf: not pinned to <sha> # vX.Y.Z: $ref"
        fi
    done < <(grep -E '^\s*(- )?uses:' "$wf")
done

echo "--- (b) top-level permissions are declared and read-only ---"
for wf in "${workflows[@]}"; do
    if ! grep -qE '^permissions:' "$wf"; then
        fail "$wf declares no top-level permissions:"
        continue
    fi
    top="$(awk '/^permissions:/ { p = 1; print; next } p && /^[^ #]/ { exit } p { print }' "$wf")"
    if grep -qE ':[[:space:]]*write' <<<"$top"; then
        fail "$wf grants write at the top level"
    else
        pass "$wf: top-level permissions declared, no write"
    fi
done

echo "--- (c) write grants exist only in release.yml create-release ---"
for wf in "${workflows[@]}"; do
    writes="$(grep -cE '^\s+[a-z-]+:[[:space:]]*write' "$wf")"
    if [[ "$wf" == ".github/workflows/release.yml" ]]; then
        block="$(awk '/^  create-release:$/ { j = 1; print; next } j && /^  [a-z][a-z0-9_-]*:$/ { exit } j { print }' "$wf")"
        inblock="$(grep -cE '^\s+[a-z-]+:[[:space:]]*write' <<<"$block")"
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
    total="$(grep -cE 'uses: actions/checkout@' "$wf")"
    safe="$(grep -cE '^\s+persist-credentials: false$' "$wf")"
    if [[ "$total" -eq "$safe" ]]; then
        pass "$wf: $total checkout(s), all persist-credentials: false"
    else
        fail "$wf: $total checkout(s) but $safe persist-credentials: false"
    fi
done

echo "--- (e) no github.event.* inside run: scripts ---"
for wf in "${workflows[@]}" "${actions[@]}"; do
    hits="$(awk '
        /^[[:space:]]*(- )?run:/ { in_run = 1; match($0, /^[[:space:]]*/); ind = RLENGTH }
        in_run {
            match($0, /^[[:space:]]*/)
            if ($0 ~ /[^[:space:]]/ && RLENGTH <= ind && $0 !~ /run:/) in_run = 0
        }
        in_run && /\$\{\{[^}]*github\.event\./ { print NR ": " $0 }
    ' "$wf")"
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
    if grep -qE '^\s*- package-ecosystem: github-actions$' "$dep"; then pass "$dep: github-actions"; else fail "$dep lacks github-actions"; fi
    if grep -qE '^\s*interval: weekly$' "$dep"; then pass "$dep: weekly"; else fail "$dep lacks a weekly interval"; fi
    if grep -qE '^\s*- package-ecosystem: (cargo|"cargo")' "$dep"; then
        fail "$dep configures cargo version updates"
    else
        pass "$dep: no cargo ecosystem"
    fi
fi

echo "--- (g) no download piped into a shell ---"
for wf in "${workflows[@]}" "${actions[@]}"; do
    # Join backslash-continued lines so a split `curl ... \ | sh` is caught.
    hits="$(awk '{ if (sub(/\\$/, "")) { buf = buf $0; next } print NR ": " buf $0; buf = "" }' "$wf" |
        grep -E '(curl|wget)[^|]*\|[[:space:]]*(sudo[[:space:]]+)?(env[[:space:]]+[^|]*)?(ba|z|da|k)?sh\b')"
    if [[ -n "$hits" ]]; then
        fail "$wf pipes a download into a shell: $hits"
    else
        pass "$wf: no curl/wget piped into a shell"
    fi
done

test_finish "workflows keep their hardening"
