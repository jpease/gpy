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
# The agent, config and XDG sandbox come from the caller's environment.

cd "${SCN_CWD:-$GPY_ROOT}" || exit 1
# shellcheck source=/dev/null
source "$GPY_ROOT/bash/gpy.bash" >/dev/null 2>&1

# Root-ness is cached once at source time, so a scenario forces it afterwards.
[[ "${SCN_ROOT:-0}" == 1 ]] && __gpy_is_root=1

__gpy_render_prompt "${SCN_STATUS:-0}"
[[ -n "${SCN_RAW_OUT:-}" ]] && printf '%s' "$PS1" >"$SCN_RAW_OUT"

# What bash displays for the prompt source: a child interactive bash of the
# same version draws it once on stderr (works on bash 3.2, which has no
# `${PS1@P}`).
out="$(PS1="@@B@@${PS1}@@E@@" BASH_SILENCE_DEPRECATION_WARNING=1 \
    "$BASH" --noprofile --norc -i </dev/null 2>&1 >/dev/null)"
out="${out#*@@B@@}"
out="${out%@@E@@*}"
printf '%s' "$out"
