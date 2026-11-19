#!/usr/bin/env zsh
# tests/zsh/basic.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Sourcing zsh/gpy.zsh installs its hooks, and one precmd with the real
# binary draws a non-empty prompt that is more than the bare fallback.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

# No agent: rendering needs only the binary.
source "$ROOT/tests/lib/supervisor_off.zsh"
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
test_require_command gpy

source zsh/gpy.zsh
check "precmd hook installed" 1 $(( ${precmd_functions[(I)__gpy_precmd]} > 0 ))
check "preexec hook installed" 1 $(( ${preexec_functions[(I)__gpy_preexec]} > 0 ))
check "zshexit hook installed" 1 $(( ${zshexit_functions[(I)__gpy_zshexit]} > 0 ))

__gpy_precmd
if [[ -n $PROMPT && $PROMPT != '❯ ' ]]; then
    pass "the binary drew the prompt"
else
    fail "the prompt is empty or the bare fallback: '$PROMPT'"
fi

test_finish "gpy.zsh basics"
