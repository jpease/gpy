#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# tests/fish/register_after_first_paint.test.fish
#
# An interactive Fish registers with the agent only once its first prompt is
# on screen (fish/core/supervisor.fish __gpy_defer_registration_past_paint):
# Fish has no post-paint hook, so the fish_prompt event queues a repaint and
# the repaint's fish_prompt call registers. Drives `fish -i` on a
# pseudo-terminal (tests/lib/pty_session.py). A stand-in __gpy_request, on the
# register request, waits (bounded) for the first prompt's text in the
# transcript: a registration made before the paint holds Fish, so the prompt
# never shows while it waits and the stand-in notes `undrawn`. No keystroke
# is sent. Non-interactive registration is covered by
# first_prompt_agent_start.test.fish.
#
# Skips (test_skip) without python3.

source (dirname (status --filename))/../lib/test_helpers.fish

command -q python3; or test_skip "python3 not installed"

set -g scratch (realpath (mktemp -d))
test_track_dir $scratch
set -g pty $__gpy_root/tests/lib/pty_session.py

set -l home $scratch/home
set -l plugin $scratch/plugin
set -g log $scratch/register.log
mkdir -p $home/.cache/gpy $home/.config $plugin
# A script segment draws a fixed mark in the first prompt, with no agent.
printf '#!/bin/sh\necho PAINT-MARK\n' >$plugin/demo.sh
chmod 755 $plugin/demo.sh
begin
    echo 'set -g __gpy_export_version "1"'
    echo 'set -g __gpy_script_sh /bin/sh'
    for line in "name demo" "root $plugin" "command $plugin/demo.sh" "ttl 0" "timeout 500" "scope session" "key 4:test4:demo"
        set -l kv (string split -m1 ' ' -- $line)
        echo "set -g __gpy_script_demo_$kv[1]" (string escape -- $kv[2])
    end
    echo 'set -g __gpy_script_demo_reqcommand ""'
    for list in inputs env opts reqfiles reqfolders reqexts reqenv
        echo "set -g __gpy_script_demo_$list"
    end
    echo 'set -g __gpy_script_segments demo'
end >$home/.cache/gpy/theme-export.fish

# No agent and no binary: the registration request itself fails at once.
set -l test_path
for path_dir in $PATH
    test -x "$path_dir/gpy-agent"; or set -a test_path $path_dir
end

set -g session $scratch/session
# Stopped on every exit, registered before the start. The driver's output
# goes to a file: it outlives `start` and must hold no pipe of the caller.
function stop_session
    python3 $pty stop $session >/dev/null 2>&1
end
test_track_cleanup stop_session
env -i HOME=$home XDG_CACHE_HOME=$home/.cache XDG_CONFIG_HOME=$home/.config XDG_RUNTIME_DIR=$home/run \
    PATH=(string join : -- $test_path) GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_T_SEGMENTS=demo \
    GPY_AGENT_SOCKET_PATH=$scratch/no-agent.sock GPY_T_ROOT=$__gpy_root GPY_T_LOG=$log \
    GPY_T_TRANSCRIPT=$session/transcript \
    python3 $pty start $session -- (command -s fish) --no-config -i -C '
        set -g __gpy_test_segments (string split " " -- "$GPY_T_SEGMENTS")
        source $GPY_T_ROOT/fish/core/init.fish
        source $GPY_T_ROOT/fish/functions/fish_prompt.fish
        functions -c __gpy_request __t_request_real
        function __gpy_request
            if test "$argv[1]" = register
                set -l seen undrawn
                for i in (seq 100)
                    if string match -q "*PAINT-MARK*" -- (string collect <$GPY_T_TRANSCRIPT 2>/dev/null)
                        set seen drawn
                        break
                    end
                    command sleep 0.05
                end
                echo $seen >>$GPY_T_LOG
            end
            __t_request_real $argv
        end
    ' >$scratch/driver.out 2>&1 </dev/null

if not python3 $pty wait-for $session PAINT-MARK 10 >/dev/null 2>&1
    fail "first prompt drawn"
    test_finish
end
pass "first prompt drawn"

# The queued repaint registers with no keystroke; a second prompt is the
# latest point a registration may come.
if not poll_until 5 test -s $log
    python3 $pty send $session '\r'
end
if poll_until 20 test -s $log
    check "the first registration request comes after the first prompt is on screen" drawn (head -n1 $log)
else
    fail "a registration attempt by the second prompt"
end

test_finish "registration after the first paint"
