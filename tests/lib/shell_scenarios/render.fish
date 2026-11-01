#!/usr/bin/env fish
# tests/lib/shell_scenarios/render.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fish driver for the cross-shell scenario harness: the twin of render.bash.
# Sources the real Fish integration and prints what `fish_prompt` draws.
# Fish prints its prompt verbatim, so the raw source and the displayed prompt
# are the same bytes. SCN_RENDERS (default 1) draws that many prompts in this
# process and prints the last; between two prompts the driver emits
# fish_preexec and fish_postexec (a command ran) and then runs $SCN_BETWEEN, an
# executable file, when it is set.

cd (test -n "$SCN_CWD"; and echo $SCN_CWD; or echo $GPY_ROOT)
source $GPY_ROOT/fish/core/init.fish
source $GPY_ROOT/fish/functions/fish_prompt.fish

# An interactive Fish starts the agent, runs the supervisor and registers from
# its fish_prompt event handler (the twin of Bash/Zsh's startup __gpy_init), so
# the same event fires here before the prompt is drawn. Stdout is dropped;
# stderr stays, for the contract's gpy: line count.
emit fish_prompt >/dev/null

test "$SCN_CHECK_NOW" = 1; and set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS 0
test "$SCN_ROOT" = 1; and set -g __gpy_is_root 1

# fish_prompt reads the status of the command that ran right before it.
function __scn_exit --argument-names code
    return $code
end
set -l renders 1
test -n "$SCN_RENDERS"; and set renders $SCN_RENDERS
set -l status_code 0
test -n "$SCN_STATUS"; and set status_code $SCN_STATUS
for i in (seq $renders)
    if test $i -gt 1
        emit fish_preexec true
        emit fish_postexec true
        test -n "$SCN_BETWEEN"; and $SCN_BETWEEN
        emit fish_prompt >/dev/null
    end
    # The status must be set right before fish_prompt reads it.
    if test $i -lt $renders
        __scn_exit $status_code
        fish_prompt >/dev/null
    else
        __scn_exit $status_code
        fish_prompt
    end
end
