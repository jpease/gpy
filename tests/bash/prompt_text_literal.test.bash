#!/usr/bin/env bash
# shellcheck disable=SC2016 # the test names are literal on purpose
# tests/bash/prompt_text_literal.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #677: agent-rendered segment text lands in PS1 as
# prompt source, and bash expands `$(...)`, `$VAR`, backticks and `\x`
# prompt escapes in it on every draw. Directory and branch names are
# untrusted data (a cloned repo, an unpacked archive), so this ran commands.
#
# With a live sandboxed agent, render the real prompt in directories and on a
# branch whose names contain those characters, and assert the displayed
# prompt shows them literally: for a fresh IPC render, an instant-cache hit
# and a oneshot fallback. The displayed prompt is produced by a child
# interactive bash of the same version printing PS1 (identical to drawing
# it), so this runs on bash 3.2 as well as 5.x.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
# shellcheck source=bash/gpy.bash
source "$ROOT/bash/gpy.bash"

# Print what bash displays for prompt source $1: a child interactive bash of
# this same version draws it once on stderr, with ANSI SGR removed.
shown() {
    local out
    out="$(PS1="@@B@@$1@@E@@" BASH_SILENCE_DEPRECATION_WARNING=1 \
        "$BASH" --noprofile --norc -i </dev/null 2>&1 >/dev/null)"
    out="${out#*@@B@@}"
    out="${out%%@@E@@*}"
    printf '%s' "$out" | python3 -c 'import re,sys; sys.stdout.write(re.sub(r"\x1b\[[0-9;]*m", "", sys.stdin.read()))'
}

# check LABEL SOURCE LITERAL: SOURCE must display LITERAL verbatim.
check() {
    local label="$1" source="$2" literal="$3" display
    display="$(shown "$source")"
    case "$display" in
        *"$literal"*) pass "$label shows '$literal' literally" ;;
        *) fail "$label: expected '$literal' in displayed prompt, got '$display'" ;;
    esac
}

# `a\u\hb`, not `a\u\wb`: `\w` would expand to the cwd, which itself
# contains the literal name, and so hide the bug.
names=('$(echo GPY_EXPANDED)' '100%_done' 'a\u\hb' 'x!y' 'tick`echo GPY_TICK`')

# --- Fresh IPC render: directory segment ---
__enabled_segments="directory"
for name in "${names[@]}"; do
    dir="$SHELL_E2E_ROOT/w/$name"
    mkdir -p "$dir"
    cd "$dir" || exit 1
    __gpy_render_prompt 0
    check "IPC directory render" "$PS1" "$name"
done

# --- Oneshot fallback render: directory segment ---
dir="$SHELL_E2E_ROOT/w/\$(echo GPY_EXPANDED)"
cd "$dir" || exit 1
__gpy_oneshot_used=0
oneshot_out="$(__gpy_fallback_oneshot directory "$PWD" bash-prompt true "")"
check "oneshot directory render" "$oneshot_out" '$(echo GPY_EXPANDED)'

# --- Git branch: fresh IPC render, then instant-cache hit ---
branch='feat/$HOME-100%_x'
cd "$SHELL_E2E_REPO" || exit 1
git checkout -q -b "$branch"
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"

# A fresh IPC render (no cache file to serve) once the agent has seen the
# checkout. The lone git segment is both first and last.
ipc_git=""
ipc_git_shows_branch() {
    rm -f "$cache_dir"/*
    ipc_git="$(__gpy_request git "$SHELL_E2E_REPO" bash-prompt true "" true)"
    case "$(shown "$ipc_git")" in *"$branch"*) return 0 ;; esac
    return 1
}
shell_e2e_poll 5 ipc_git_shows_branch
check "IPC git render" "$ipc_git" "$branch"

suffix="$(__gpy_cache_variant_suffix git true true)"
resolved_root="$(cd "$SHELL_E2E_REPO" && pwd -P)"
cache_file="$cache_dir/$(__gpy_path_to_cache_key "$resolved_root").$suffix.none.bash"
if shell_e2e_poll 3 test -f "$cache_file"; then
    cached="$(__gpy_read_instant_cache "$suffix" "$SHELL_E2E_REPO" "")"
    check "instant-cache git hit" "$cached" "$branch"
    __enabled_segments="git"
    __gpy_render_prompt 0
    check "cached git render" "$PS1" "$branch"
else
    fail "the agent did not write the bash-dialect git cache file $cache_file"
fi

shell_e2e_stop_agent

if [ "$failures" -ne 0 ]; then
    echo "=== $failures FAILED ==="
    exit 1
fi
echo "=== All Tests Passed ==="
