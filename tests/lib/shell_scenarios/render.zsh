#!/usr/bin/env zsh
# tests/lib/shell_scenarios/render.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Zsh driver for the cross-shell scenario harness: the twin of render.bash.
# Runs the real precmd (`__gpy_precmd`, which renders PROMPT) and prints what
# zsh would draw for it (`print -P`, with the prompt options the integration
# pins). SCN_RENDERS (default 1) renders that many prompts in this process and
# prints the last; between two prompts the driver simulates a command
# (`__gpy_preexec true` makes script segments due) and then runs $SCN_BETWEEN,
# an executable file, when it is set.

cd "${SCN_CWD:-$GPY_ROOT}" || exit 1
source "$GPY_ROOT/zsh/gpy.zsh" >/dev/null 2>&1

[[ "${SCN_CHECK_NOW:-0}" == 1 ]] && GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0

[[ "${SCN_ROOT:-0}" == 1 ]] && __gpy_is_root=1

# precmd reads the status of the command that ran right before it. A function
# return sets it; `(exit N)` would also run the exit hook, which removes the
# integration's per-session render-cache dir.
__scn_exit() { return $1 }
for (( __scn_i = 1; __scn_i <= ${SCN_RENDERS:-1}; __scn_i++ )); do
    if (( __scn_i > 1 )); then
        __gpy_preexec true
        [[ -n ${SCN_BETWEEN-} ]] && "$SCN_BETWEEN"
    fi
    __scn_exit ${SCN_STATUS:-0}
    __gpy_precmd
done
[[ -n "${SCN_RAW_OUT:-}" ]] && print -rn -- "$PROMPT" >"$SCN_RAW_OUT"

print -rnP -- "$PROMPT"
