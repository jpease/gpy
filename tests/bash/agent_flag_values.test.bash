#!/usr/bin/env bash
# tests/bash/agent_flag_values.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# GPY_AGENT_ENABLED and GPY_AGENT_SUPERVISOR_ENABLED follow the shared boolean
# table (1/true/yes/on, 0/false/no/off, case-insensitive; anything else counts as
# unset), then the theme export's __gpy_cfg_* mirror (off only for `0`), then on.
# Same table in the Zsh and Fish tests.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

# shellcheck source=bash/core/ipc.bash
. "$ROOT/bash/core/ipc.bash" 2>/dev/null

# predicate: prints on/off for the current environment.
state() {
    if "$1"; then echo on; else echo off; fi
}

table=("unset on" "empty on" "0 off" "1 on" "false off" "true on" "YES on" "on on" "No off" "OFF off" "maybe on")

for flag in GPY_AGENT_ENABLED GPY_AGENT_SUPERVISOR_ENABLED; do
    case "$flag" in
        GPY_AGENT_ENABLED)
            predicate=__gpy_agent_enabled
            mirror=__gpy_cfg_agent_enabled
            ;;
        *)
            predicate=__gpy_supervisor_enabled
            mirror=__gpy_cfg_supervisor_enabled
            ;;
    esac
    unset "$mirror"
    for row in "${table[@]}"; do
        value="${row% *}"
        expected="${row#* }"
        case "$value" in
            unset) unset "$flag" ;;
            empty) export "$flag=" ;;
            *) export "$flag=$value" ;;
        esac
        check "$flag=$value" "$expected" "$(state "$predicate")"
    done

    # The mirror decides when the variable is unset or unrecognised; a
    # recognised user value wins over it.
    printf -v "$mirror" '%s' 0
    unset "$flag"
    check "$flag unset, config off" off "$(state "$predicate")"
    export "$flag=maybe"
    check "$flag=maybe, config off" off "$(state "$predicate")"
    export "$flag=1"
    check "$flag=1 wins over config off" on "$(state "$predicate")"
    printf -v "$mirror" '%s' 1
    export "$flag=off"
    check "$flag=off wins over config on" off "$(state "$predicate")"
    unset "$flag" "$mirror"
done

test_finish "agent flag values (bash)"
