#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# tests/fish/script_segment_repaint_once.test.fish
#
# A script ("G") segment runs once per command in a real interactive Fish,
# never on a repaint. Fish calls the fish_prompt function again for every
# SIGURG doorbell (__gpy_doorbell_handler -> __gpy_repaint_trigger ->
# `commandline -f force-repaint`); the script runs from the fish_prompt
# event instead, which a repaint does not emit. Drives `fish -i` on a
# pseudo-terminal (tests/lib/pty_session.py), sends SIGURG from outside and
# counts the script's runs:
#
#   first prompt                       1 run, segment text on screen
#   5 x SIGURG, each repainting        still 1 run, text on every repaint
#   empty Enter                        still 1 run
#   `true` + Enter                     2 runs (ttl 0)
#
# Skips (test_skip) without python3.

source (dirname (status --filename))/../lib/test_helpers.fish

command -q python3; or test_skip "python3 not installed"

set -g scratch (realpath (mktemp -d))
test_track_dir $scratch
set -g pty $__gpy_root/tests/lib/pty_session.py

set -l home $scratch/home
set -l plugin $scratch/plugin
set -l cnt $scratch/count
mkdir -p $home/.cache/gpy $home/.config $plugin
printf '#!/bin/sh\necho run >>%s\necho SEGMENT-OK\n' (string escape -- $cnt) >$plugin/demo.sh
chmod 755 $plugin/demo.sh
begin
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

# No agent: the export comes from the cache file.
set -l test_path
for dir in $PATH
    test -x "$dir/gpy-agent"; or set -a test_path $dir
end

set -l dir $scratch/session
set -l pidfile $scratch/pid
env -i HOME=$home XDG_CACHE_HOME=$home/.cache XDG_CONFIG_HOME=$home/.config XDG_RUNTIME_DIR=$home/run \
    PATH=(string join : -- $test_path) GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_TEST_SEGMENTS=demo \
    GPY_T_ROOT=$__gpy_root GPY_T_PIDFILE=$pidfile \
    python3 $pty start $dir -- (command -s fish) --no-config -i -C '
        source $GPY_T_ROOT/fish/core/init.fish
        source $GPY_T_ROOT/fish/functions/fish_prompt.fish
        echo $fish_pid >$GPY_T_PIDFILE
    ' >/dev/null
function stop_session --inherit-variable pty --inherit-variable dir
    python3 $pty stop $dir >/dev/null 2>&1
end
test_track_cleanup stop_session

function runs --argument-names file
    test -f $file; or begin
        echo 0
        return
    end
    count <$file
end

# Occurrences of the segment text in the transcript (one per render).
function renders --inherit-variable dir
    count (string match -ar -- SEGMENT-OK <$dir/transcript)
end

function renders_at_least --argument-names n
    test (renders) -ge $n
end

if not python3 $pty wait-for $dir SEGMENT-OK 10 >/dev/null
    fail "first prompt shows the script segment"
    test_finish
end
pass "first prompt shows the script segment"
check "first prompt ran the script once" 1 (runs $cnt)

poll_until 5 test -s $pidfile
set -l shell_pid (string collect <$pidfile)
set -l before (renders)
for i in 1 2 3 4 5
    kill -s URG $shell_pid
    if not poll_until 5 renders_at_least (math $before + $i)
        fail "SIGURG $i repainted the prompt" "renders: "(renders)
        break
    end
end
if test (renders) -ge (math $before + 5)
    pass "five SIGURG doorbells repainted the prompt with the stored text"
end
check "repaints did not run the script" 1 (runs $cnt)

python3 $pty send $dir '\r'
poll_until 5 renders_at_least (math $before + 6)
check "an empty Enter did not run the script" 1 (runs $cnt)

python3 $pty send $dir 'true\r'
function two_runs --inherit-variable cnt
    test (runs $cnt) -ge 2
end
poll_until 5 two_runs
check "a command made the next prompt run it again" 2 (runs $cnt)

test_finish "script segment runs once per command across repaints"
