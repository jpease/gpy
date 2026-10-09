#!/usr/bin/env zsh
# tests/lib/shell_scenarios/render.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Zsh driver for the cross-shell scenario harness: the twin of render.bash.
# Runs the real precmd (`__gpy_precmd`, which renders PROMPT) and prints what
# zsh would draw for it (`print -P`, with the prompt options the integration
# pins).

cd "${SCN_CWD:-$GPY_ROOT}" || exit 1
source "$GPY_ROOT/zsh/gpy.zsh" >/dev/null 2>&1

[[ "${SCN_ROOT:-0}" == 1 ]] && __gpy_is_root=1

# precmd reads the status of the command that ran right before it. A function
# return sets it; `(exit N)` would also run the exit hook, which removes the
# integration's per-session render-cache dir.
__scn_exit() { return $1 }
__scn_exit ${SCN_STATUS:-0}
__gpy_precmd
[[ -n "${SCN_RAW_OUT:-}" ]] && print -rn -- "$PROMPT" >"$SCN_RAW_OUT"

print -rnP -- "$PROMPT"
