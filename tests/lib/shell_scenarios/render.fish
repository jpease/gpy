#!/usr/bin/env fish
# tests/lib/shell_scenarios/render.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fish driver for the cross-shell scenario harness: the twin of render.bash.
# Sources the real Fish integration and prints what `fish_prompt` draws.
# Fish prints its prompt verbatim, so the raw source and the displayed prompt
# are the same bytes.

cd (test -n "$SCN_CWD"; and echo $SCN_CWD; or echo $GPY_ROOT)
source $GPY_ROOT/fish/core/init.fish
source $GPY_ROOT/fish/functions/fish_prompt.fish

# An interactive Fish starts the agent, runs the supervisor and registers from
# its fish_prompt event handler (the twin of Bash/Zsh's startup __gpy_init), so
# the same event fires here before the prompt is drawn.
emit fish_prompt >/dev/null 2>&1

test "$SCN_ROOT" = 1; and set -g __gpy_is_root 1

# fish_prompt reads the status of the command that ran right before it.
function __scn_exit --argument-names code
    return $code
end
__scn_exit (test -n "$SCN_STATUS"; and echo $SCN_STATUS; or echo 0)
fish_prompt
