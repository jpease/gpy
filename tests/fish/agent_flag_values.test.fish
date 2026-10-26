#!/usr/bin/env fish
# tests/fish/agent_flag_values.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY_AGENT_ENABLED and GPY_AGENT_SUPERVISOR_ENABLED are off only for `0` or
# `false`; unset, empty and `1` leave them on. Same table in the Bash and Zsh tests.

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

set -l table "unset on" "empty on" "0 off" "1 on" "false off"

for flag in GPY_AGENT_ENABLED GPY_AGENT_SUPERVISOR_ENABLED
    switch $flag
        case GPY_AGENT_ENABLED
            set predicate __gpy_agent_enabled
        case '*'
            set predicate __gpy_supervisor_enabled
    end
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
end

test_finish "agent flag values (fish)"
