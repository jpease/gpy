#!/usr/bin/env zsh
# tests/zsh/prompt_text_literal.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Twin of tests/bash/prompt_text_literal.test.bash: segment text lands in
# PROMPT as prompt source, and zsh (with the prompt_subst gpy sets) expands
# `$(...)`, `$VAR`, backticks and `%x` prompt escapes in it on every draw.
# Directory and branch names are untrusted data, so `gpy prompt zsh` must
# escape them or they would run commands.
#
# Through zsh/gpy.zsh's precmd and the real binary (no agent), render the
# prompt in directories and on a branch whose names contain those characters,
# and assert that `print -rP -- "$PROMPT"` (zsh's own prompt expansion, under
# the options gpy pins) shows them literally.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
test_require_command gpy
test_require_command git

shell_e2e_init "$ROOT"
shell_e2e_text_config '"directory", "git"' false supervisor-off
export GPY_AGENT_ENABLED=0
source "$ROOT/zsh/gpy.zsh"

# What zsh displays for prompt source $1, with ANSI SGR removed.
shown() {
    print -rP -- "$1" | sed -e $'s/\x1b\\[[0-9;]*m//g'
}

# assert_shows LABEL SOURCE LITERAL: SOURCE must display LITERAL verbatim.
assert_shows() {
    local label=$1 source=$2 literal=$3 display
    display=$(shown "$source")
    if [[ "$display" == *"$literal"* ]]; then
        pass "$label shows '$literal' literally"
    else
        fail "$label: expected '$literal' in displayed prompt, got '$display'"
    fi
}

names=('$(echo GPY_EXPANDED)' '100%_done' 'a\u\hb' 'x!y' 'tick`echo GPY_TICK`')

for name in "${names[@]}"; do
    dir="$SHELL_E2E_ROOT/w/$name"
    mkdir -p "$dir"
    cd "$dir" || exit 1
    __gpy_precmd
    assert_shows "directory render" "$PROMPT" "$name"
done

branch='feat/$HOME-100%_x'
cd "$SHELL_E2E_REPO" || exit 1
git checkout -q -b "$branch"
__gpy_precmd
assert_shows "git render" "$PROMPT" "$branch"

test_finish
