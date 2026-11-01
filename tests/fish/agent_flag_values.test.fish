#!/usr/bin/env fish
# tests/fish/agent_flag_values.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY_AGENT_ENABLED and GPY_AGENT_SUPERVISOR_ENABLED follow the shared boolean
# table (1/true/yes/on, 0/false/no/off, case-insensitive; anything else counts as
# unset), then the theme export's __gpy_cfg_* mirror (off only for `0`), then on.
# Same table in the Bash and Zsh tests.

set -l repo_root (path resolve (status dirname)/../..)
source $repo_root/tests/lib/test_helpers.fish
source $repo_root/fish/core/flags.fish

function state --argument-names predicate
    if $predicate
        echo on
    else
        echo off
    end
end

set -l table "unset on" "empty on" "0 off" "1 on" "false off" "true on" "YES on" "on on" "No off" "OFF off" "maybe on"

for flag in GPY_AGENT_ENABLED GPY_AGENT_SUPERVISOR_ENABLED
    switch $flag
        case GPY_AGENT_ENABLED
            set predicate __gpy_agent_enabled
            set mirror __gpy_cfg_agent_enabled
        case '*'
            set predicate __gpy_supervisor_enabled
            set mirror __gpy_cfg_supervisor_enabled
    end
    set -e $mirror
    for row in $table
        set -l parts (string split ' ' -- $row)
        switch $parts[1]
            case unset
                set -e $flag
            case empty
                set -gx $flag ''
            case '*'
                set -gx $flag $parts[1]
        end
        check "$flag=$parts[1]" $parts[2] (state $predicate)
    end

    # The mirror decides when the variable is unset or unrecognised; a
    # recognised user value wins over it.
    set -g $mirror 0
    set -e $flag
    check "$flag unset, config off" off (state $predicate)
    set -gx $flag maybe
    check "$flag=maybe, config off" off (state $predicate)
    set -gx $flag 1
    check "$flag=1 wins over config off" on (state $predicate)
    set -g $mirror 1
    set -gx $flag off
    check "$flag=off wins over config on" off (state $predicate)
    set -e $flag
    set -e $mirror
end

test_finish "agent flag values (fish)"
