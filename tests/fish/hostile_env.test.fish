#!/usr/bin/env fish
# tests/fish/hostile_env.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Hostile-shell-environment harness (fish counterpart of
# tests/bash/hostile_env.test.bash and tests/zsh/hostile_env.test.zsh).
# Every scenario starts a FRESH child fish with a sandboxed HOME/XDG tree in
# which GPY is "installed" the way the installer does it (conf.d file plus
# $XDG_CONFIG_HOME/fish/gpy). New hostile-environment issues add a
# `scenario_<name>` function plus one `run_scenario` line at the bottom.
#
# Children never start a gpy-agent: the supervisor is disabled, PATH holds no
# gpy-agent, and the socket path points at a file that does not exist. Paths
# reach the child only through the environment/argv, never through -c code.

set -g test_failures 0

function test_fail
    set -g test_failures (math $test_failures + 1)
    echo "❌ FAIL: $argv[1]"
end

function test_pass
    echo "✅ PASS: $argv[1]"
end

set -l script_dir (dirname (status --filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
set -g sandbox (mktemp -d)
mkdir -p $sandbox/cfg/fish/conf.d $sandbox/cache $sandbox/run $sandbox/home
ln -s $repo_root/fish $sandbox/cfg/fish/gpy
cp $repo_root/fish/conf.d/gpy_init.fish $sandbox/cfg/fish/conf.d/gpy_init.fish

function cleanup_sandbox --on-event fish_exit
    rm -rf $sandbox
end

# Scrubbed child: sets CHILD_OUT / CHILD_STATUS. Args: <fish -c code> [fish flags before -c...]
# The code receives the conf.d path as $argv[1].
function run_child --argument-names code
    set -g CHILD_OUT (env -i PATH=/usr/bin:/bin TERM=dumb HOME=$sandbox/home \
        XDG_CONFIG_HOME=$sandbox/cfg XDG_CACHE_HOME=$sandbox/cache \
        XDG_RUNTIME_DIR=$sandbox/run TMPDIR=$sandbox \
        GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH=$sandbox/missing.sock \
        (command -v fish) --no-config -c $code -- $sandbox/cfg/fish/conf.d/gpy_init.fish \
        </dev/null 2>&1)
    set -g CHILD_STATUS $status
end

# Regular files only: fish itself creates empty cache/data dirs on startup.
function files_snapshot
    find $sandbox -type f -not -path "$sandbox/cfg/fish/gpy/*" 2>/dev/null | sort
end

function scenario_noninteractive_c_is_inert
    set -l before (files_snapshot)
    run_child 'source $argv[1]
echo "fns=[$(functions -n | string match -r "^(__gpy|gpy_|prompt-|prompt_debug)" | string join ,)]"
echo "handlers=[$(functions --handlers | string match -r "gpy" | string join ,)]"
echo "vars=[$(set -n | string match -r "^(__gpy|__enabled_segments|__prompt).*" | string join ,)]"'
    set -l after (files_snapshot)

    test $CHILD_STATUS -eq 0; and test_pass "fish -c with conf.d exited 0"; or test_fail "fish -c exited $CHILD_STATUS: "(string sub -l 200 -- (string collect $CHILD_OUT))
    string match -qr 'fns=\[\]' -- (string collect $CHILD_OUT); and test_pass "no gpy functions defined"; or test_fail "gpy functions defined: "(string sub -l 200 -- (string collect $CHILD_OUT))
    string match -qr 'handlers=\[\]' -- (string collect $CHILD_OUT); and test_pass "no gpy event handlers defined"; or test_fail "gpy handlers defined: "(string sub -l 200 -- (string collect $CHILD_OUT))
    string match -qr 'vars=\[\]' -- (string collect $CHILD_OUT); and test_pass "no gpy variables set"; or test_fail "gpy variables set: "(string sub -l 200 -- (string collect $CHILD_OUT))
    test "$before" = "$after"; and test_pass "no marker files left"; or test_fail "files changed: "(printf '%s\n' $after | string collect)
end

# Shaped like a supervisor child: `fish -c` with stdin closed and a script run
# that reads no prompt, same conf.d load.
function scenario_supervisor_child_shape
    run_child 'source $argv[1]; functions -q fish_prompt; echo "done"; functions -n | string match -rq "^(__gpy|gpy_|prompt-)"; and echo LEAK'
    string match -q '*LEAK*' -- (string collect $CHILD_OUT); and test_fail "supervisor-shaped child loaded GPY: "(string sub -l 200 -- (string collect $CHILD_OUT)); or test_pass "supervisor-shaped child loaded no GPY functions"
    pgrep -f "$sandbox/missing.sock" >/dev/null 2>&1; and test_fail "agent process left running"; or test_pass "no agent started"
end

# #770: disabled-footprint mode never defines __gpy_oneshot_marker_path; the
# per-prompt marker reset must not fork `rm` (a stub rm first on PATH logs
# every external call). A real marker, when defined, must still be removed.
function scenario_disabled_prompt_forks_no_rm
    set -l stub_dir $sandbox/stubbin
    set -l rm_log $sandbox/rm.log
    mkdir -p $stub_dir
    printf '#!/bin/sh\necho "rm $*" >> "$RM_LOG"\n' >$stub_dir/rm
    chmod +x $stub_dir/rm
    : >$rm_log

    env -i PATH=$stub_dir:/usr/bin:/bin TERM=dumb HOME=$sandbox/home \
        XDG_CONFIG_HOME=$sandbox/cfg XDG_CACHE_HOME=$sandbox/cache \
        XDG_RUNTIME_DIR=$sandbox/run TMPDIR=$sandbox RM_LOG=$rm_log \
        GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
        GPY_AGENT_SOCKET_PATH=$sandbox/missing.sock \
        (command -v fish) --no-config -i -c 'source $argv[1]
source $argv[2]
echo BEGIN_RENDER >> $argv[3]
for i in 1 2 3 4 5
    fish_prompt >/dev/null
end
echo END_RENDER >> $argv[3]' -- $sandbox/cfg/fish/conf.d/gpy_init.fish $repo_root/fish/functions/fish_prompt.fish $rm_log </dev/null >/dev/null 2>&1
    set -l child_status $status

    set -l log_lines (cat $rm_log)
    contains -- END_RENDER $log_lines; and test_pass "child rendered 5 prompts to completion"; or test_fail "child did not finish rendering (status $child_status)"
    set -l begin_idx (contains -i -- BEGIN_RENDER $log_lines)
    set -l forked (printf '%s\n' $log_lines[(math $begin_idx + 1)..-1] | string match -r '^rm ')
    test (count $forked) -eq 0; and test_pass "disabled-footprint prompt forked no rm"; or test_fail "disabled-footprint prompt forked rm: "(string join '; ' -- $forked)

    # A defined, existing marker is still reset by the next render.
    set -l marker $sandbox/oneshot.marker
    touch $marker
    env -i PATH=/usr/bin:/bin TERM=dumb HOME=$sandbox/home \
        XDG_CONFIG_HOME=$sandbox/cfg XDG_CACHE_HOME=$sandbox/cache \
        XDG_RUNTIME_DIR=$sandbox/run TMPDIR=$sandbox \
        GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
        (command -v fish) --no-config -i -c 'source $argv[1]
source $argv[2]
set -g __gpy_oneshot_marker_path $argv[3]
fish_prompt >/dev/null' -- $sandbox/cfg/fish/conf.d/gpy_init.fish $repo_root/fish/functions/fish_prompt.fish $marker </dev/null >/dev/null 2>&1
    test -e $marker; and test_fail "existing oneshot marker was not removed"; or test_pass "existing oneshot marker removed by prompt render"
end

function run_scenario
    echo "=== Scenario: $argv[1] ==="
    scenario_$argv[1]
end

run_scenario noninteractive_c_is_inert
run_scenario supervisor_child_shape
run_scenario disabled_prompt_forks_no_rm

if test $test_failures -gt 0
    echo "❌ $test_failures failure(s)"
    exit 1
end
echo "✅ all hostile-env fish scenarios passed"
