#!/usr/bin/env bash
# shellcheck disable=SC2016 # the test names are literal on purpose
# tests/bash/prompt_text_literal.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Segment text is PS1 source; untrusted dir/branch names must display literally
# (no `$(...)`, `$VAR`, backtick or `\x` expansion) in the PS1 `gpy prompt bash` draws.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"directory", "git"' false supervisor-off
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
# shellcheck source=bash/gpy.bash
source "$ROOT/bash/gpy.bash"

# Print what a child interactive bash of this version displays for prompt source $1.
shown() {
    local out
    out="$(PS1="@@B@@$1@@E@@" BASH_SILENCE_DEPRECATION_WARNING=1 \
        "$BASH" --noprofile --norc -i </dev/null 2>&1 >/dev/null)"
    out="${out#*@@B@@}"
    out="${out%%@@E@@*}"
    printf '%s' "$out" | python3 -c 'import re,sys; sys.stdout.write(re.sub(r"\x1b\[[0-9;]*m", "", sys.stdin.read()))'
}

# assert_shows LABEL SOURCE LITERAL: SOURCE must display LITERAL verbatim.
assert_shows() {
    local label="$1" source="$2" literal="$3" display
    display="$(shown "$source")"
    case "$display" in
        *"$literal"*) pass "$label shows '$literal' literally" ;;
        *) fail "$label: expected '$literal' in displayed prompt, got '$display'" ;;
    esac
}

# `\h`, not `\w`: `\w` expands to the cwd, which contains the literal name.
names=('$(echo GPY_EXPANDED)' '100%_done' 'a\u\hb' 'x!y' 'tick`echo GPY_TICK`')

for name in "${names[@]}"; do
    dir="$SHELL_E2E_ROOT/w/$name"
    mkdir -p "$dir"
    cd "$dir" || exit 1
    __gpy_precmd
    assert_shows "directory render" "$PS1" "$name"
done

branch='feat/$HOME-100%_x'
cd "$SHELL_E2E_REPO" || exit 1
git checkout -q -b "$branch"

# The agent reports the branch once it has seen the checkout.
git_shows_branch() {
    __gpy_precmd
    case "$(shown "$PS1")" in *"$branch"*) return 0 ;; esac
    return 1
}
shell_e2e_poll 5 git_shows_branch
assert_shows "git render" "$PS1" "$branch"

shell_e2e_stop_agent

test_finish
