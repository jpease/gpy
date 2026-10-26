#!/usr/bin/env bash
# tests/bash/agent_flag_values.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# GPY_AGENT_ENABLED and GPY_AGENT_SUPERVISOR_ENABLED are off only for `0` or
# `false`; unset, empty and `1` leave them on. Same table in the Zsh and Fish tests.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

export GPY_AGENT_ENABLED=1
# shellcheck source=bash/core/ipc.bash
. "$ROOT/bash/core/ipc.bash" 2>/dev/null

# predicate VARIABLE: prints on/off for the current value of VARIABLE.
state() {
    if "$1"; then echo on; else echo off; fi
}

table=("unset on" "empty on" "0 off" "1 on" "false off")

for flag in GPY_AGENT_ENABLED GPY_AGENT_SUPERVISOR_ENABLED; do
    case "$flag" in
        GPY_AGENT_ENABLED) predicate=__gpy_agent_enabled ;;
        *) predicate=__gpy_supervisor_enabled ;;
    esac
    for row in "${table[@]}"; do
        value="${row% *}"
        expected="${row#* }"
        case "$value" in
            unset) unset "$flag" ;;
            empty) export "$flag="  ;;
            *) export "$flag=$value" ;;
        esac
        check "$flag=$value" "$expected" "$(state "$predicate")"
    done
done

test_finish "agent flag values (bash)"
