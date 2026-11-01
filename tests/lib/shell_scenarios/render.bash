#!/usr/bin/env bash
# tests/lib/shell_scenarios/render.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Bash driver for the cross-shell scenario harness (see scenarios.tsv and
# tests/bash/shell_contract.test.bash). Sources this checkout's
# real Bash integration, renders ONE prompt through its real render path
# (`__gpy_render_prompt`), and prints what readline would draw.
#
# Environment (all required unless noted):
#   GPY_ROOT        the checkout
#   SCN_STATUS      the previous command's exit status
#   SCN_ROOT        1 forces the root-session flag (optional)
#   SCN_RAW_OUT     file receiving the raw PS1 source (optional)
#   SCN_RENDERS     how many prompts to render in this one process; only the
#                   last is printed (default 1). Between two prompts the
#                   driver simulates a command, then runs $SCN_BETWEEN
#   SCN_BETWEEN     executable file run with no arguments between prompts
#                   (optional)
# The agent, config and XDG sandbox come from the caller's environment.

cd "${SCN_CWD:-$GPY_ROOT}" || exit 1
# shellcheck source=/dev/null
source "$GPY_ROOT/bash/gpy.bash" >/dev/null 2>&1

# Read by the integration at call time.
# shellcheck disable=SC2034
[[ "${SCN_CHECK_NOW:-0}" == 1 ]] && GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0

# Root-ness is cached once at source time, so a scenario forces it afterwards.
[[ "${SCN_ROOT:-0}" == 1 ]] && __gpy_is_root=1

# precmd reads the status of the command that ran right before it.
__scn_exit() { return "$1"; }

if ((${SCN_RENDERS:-1} > 1)); then
    for ((__scn_i = 1; __scn_i <= SCN_RENDERS; __scn_i++)); do
        if ((__scn_i > 1)); then
            # A command ran. Bash 4.4+ decides "due" from the \# command
            # number, which stays constant in a non-interactive driver, so
            # clearing the stored number makes the next prompt see a new one;
            # __gpy_g_ran covers Bash < 4.4.
            __gpy_preexec
            __gpy_g_cmd=""
            __gpy_g_ran=1
            [[ -n "${SCN_BETWEEN:-}" ]] && "$SCN_BETWEEN"
        fi
        __scn_exit "${SCN_STATUS:-0}"
        __gpy_precmd
    done
else
    # Script (G) segments run in __gpy_precmd before the render, as here.
    __gpy_g_maybe_run
    __gpy_render_prompt "${SCN_STATUS:-0}"
fi
[[ -n "${SCN_RAW_OUT:-}" ]] && printf '%s' "$PS1" >"$SCN_RAW_OUT"

# What bash displays for the prompt source: a child interactive bash of the
# same version draws it once on stderr (works on bash 3.2, which has no
# `${PS1@P}`).
out="$(PS1="@@B@@${PS1}@@E@@" BASH_SILENCE_DEPRECATION_WARNING=1 \
    "$BASH" --noprofile --norc -i </dev/null 2>&1 >/dev/null)"
out="${out#*@@B@@}"
out="${out%@@E@@*}"
printf '%s' "$out"
