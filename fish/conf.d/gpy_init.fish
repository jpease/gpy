# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY for Fish. `gpy prompt fish` (called from functions/fish_prompt.fish)
# draws the prompt and the transient line, runs the script segments and
# keeps this shell's lifecycle (agent start, registration, supervisor). This
# file only counts commands, keeps the duration of the one that just ran,
# repaints when the agent rings and unregisters on exit.

# Non-interactive shells (fish -c, scripts) must not load GPY.
status is-interactive; or return

# The major and minor numbers alone: a beta (`4.2b1`) or a development build
# (`4.1.2-56-gabc`) carries more after them. A version without them counts as
# 3.6, the oldest supported. __gpy_test_fish_version is a non-exported test
# hook: FISH_VERSION is read-only.
set -l gpy_fish_version (string match -rg -- '^(\d+)\.(\d+)' (set -q __gpy_test_fish_version; and echo $__gpy_test_fish_version; or echo $FISH_VERSION))
set -q gpy_fish_version[2]; or set gpy_fish_version 3 6
if test "$gpy_fish_version[1]" -lt 3; or test "$gpy_fish_version[1]" -eq 3 -a "$gpy_fish_version[2]" -lt 6
    echo "gpy: GPY needs Fish 3.6 or newer (this is $FISH_VERSION); see https://fishshell.com/" >&2
    return
end

# fish_transient_prompt arrived in Fish 4.1; 4.2 fixed clearing the extra
# lines when the final rendering is shorter than the prompt (two_line).
set -g __gpy_fish_transient_capable 0
if test "$gpy_fish_version[1]" -gt 4; or test "$gpy_fish_version[1]" -eq 4 -a "$gpy_fish_version[2]" -ge 2
    set -g __gpy_fish_transient_capable 1
end

set -q __gpy_cmd_count; or set -g __gpy_cmd_count 0
# This shell, as `gpy prompt` tells it from an earlier one at the same PID
# (`exec fish`, a recycled PID): drawn once per load, never inherited.
set -g __gpy_instance (random)(random)

# fish_postexec is emitted for a command, never for an empty Enter.
function __gpy_on_postexec --on-event fish_postexec
    set -g __gpy_cmd_count (math $__gpy_cmd_count + 1)
    set -g __gpy_command_ran 1
end

# The fish_prompt event is emitted once per new prompt (never for a
# repaint), before fish_prompt runs. $CMD_DURATION changes only when a
# command runs, so a prompt with no command since the last one shows none.
function __gpy_on_prompt --on-event fish_prompt
    # Drop the dynamic-completion caches (completions/gpy-dynamic.fish).
    functions -q __gpy_complete_cache_clear; and __gpy_complete_cache_clear
    if set -q __gpy_command_ran
        set -e __gpy_command_ran
        set -g __gpy_duration $CMD_DURATION
    else
        set -e __gpy_duration
    end
end

# The agent rings SIGURG after it wrote a `.reload` or `.reregister` flag,
# or when a late git status landed. Fish allows `commandline -f repaint`
# from variable handlers, not signal handlers. SIGURG's default disposition
# is ignore, so a shell that `exec`s itself survives an early ring.
function __gpy_on_doorbell --on-signal SIGURG
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end

function __gpy_repaint_on_variable --on-variable __gpy_repaint_trigger
    commandline -f force-repaint 2>/dev/null
end

# Unregister on exit so a restarted agent never signals a recycled PID.
# Self-contained: fish_prompt.fish (which defines __gpy_bin) may never have
# autoloaded in a shell that exits before its first prompt.
function __gpy_on_exit --on-event fish_exit
    set -l bin (command -s gpy)
    if test -z "$bin"; and test -n "$HOME"; and test -x "$HOME/.local/bin/gpy"
        set bin $HOME/.local/bin/gpy
    end
    test -n "$bin"; or return 0
    command $bin prompt fish --exit --pid $fish_pid >/dev/null 2>&1
end
