#!/usr/bin/env fish
# tests/fish/hostile_env.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Hostile-shell-environment harness (fish counterpart of
# tests/bash/hostile_env.test.bash and tests/zsh/hostile_env.test.zsh).
# Every scenario starts a FRESH child fish with a sandboxed HOME/XDG tree in
# which GPY is "installed" the way the installer does it (the Fish tree in
# $XDG_DATA_HOME/gpy/fish). New hostile-environment issues add a
# `scenario_<name>` function plus one `run_scenario` line at the bottom.
#
# Children never start a gpy-agent: the supervisor is disabled, PATH holds no
# gpy-agent, and the socket path points at a file that does not exist. Paths
# reach the child only through the environment/argv, never through -c code.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (dirname (status --filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
set -g sandbox (mktemp -d)
mkdir -p $sandbox/cfg/fish $sandbox/data/gpy $sandbox/cache $sandbox/run $sandbox/home
ln -s $repo_root/fish $sandbox/data/gpy/fish

function cleanup_sandbox --on-event fish_exit
    rm -rf $sandbox
end

# Scrubbed child: sets CHILD_OUT / CHILD_STATUS. Args: <fish -c code> [fish flags before -c...]
# The code receives the conf.d path as $argv[1].
function run_child --argument-names code
    set -g CHILD_OUT (env -i PATH=/usr/bin:/bin TERM=dumb HOME=$sandbox/home \
        XDG_CONFIG_HOME=$sandbox/cfg XDG_DATA_HOME=$sandbox/data XDG_CACHE_HOME=$sandbox/cache \
        XDG_RUNTIME_DIR=$sandbox/run TMPDIR=$sandbox \
        GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH=$sandbox/missing.sock \
        (command -v fish) --no-config -c $code -- $sandbox/data/gpy/fish/conf.d/gpy_init.fish \
        </dev/null 2>&1)
    set -g CHILD_STATUS $status
end

# Regular files only: fish itself creates empty cache/data dirs on startup.
function files_snapshot
    find $sandbox -type f -not -path "$sandbox/data/gpy/*" 2>/dev/null | sort
end

function scenario_noninteractive_c_is_inert
    set -l before (files_snapshot)
    run_child 'source $argv[1]
echo "fns=[$(functions -n | string match -r "^(__gpy|gpy_|prompt-)" | string join ,)]"
echo "handlers=[$(functions --handlers | string match -r "gpy" | string join ,)]"
echo "vars=[$(set -n | string match -r "^(__gpy|__enabled_segments|__prompt).*" | string join ,)]"'
    set -l after (files_snapshot)

    test $CHILD_STATUS -eq 0; and pass "fish -c with conf.d exited 0"; or fail "fish -c exited $CHILD_STATUS: "(string sub -l 200 -- (string collect $CHILD_OUT))
    string match -qr 'fns=\[\]' -- (string collect $CHILD_OUT); and pass "no gpy functions defined"; or fail "gpy functions defined: "(string sub -l 200 -- (string collect $CHILD_OUT))
    string match -qr 'handlers=\[\]' -- (string collect $CHILD_OUT); and pass "no gpy event handlers defined"; or fail "gpy handlers defined: "(string sub -l 200 -- (string collect $CHILD_OUT))
    string match -qr 'vars=\[\]' -- (string collect $CHILD_OUT); and pass "no gpy variables set"; or fail "gpy variables set: "(string sub -l 200 -- (string collect $CHILD_OUT))
    test "$before" = "$after"; and pass "no marker files left"; or fail "files changed: "(printf '%s\n' $after | string collect)
end

# Shaped like a supervisor child: `fish -c` with stdin closed and a script run
# that reads no prompt, same conf.d load.
function scenario_supervisor_child_shape
    run_child 'source $argv[1]; functions -q fish_prompt; echo "done"; functions -n | string match -rq "^(__gpy|gpy_|prompt-)"; and echo LEAK'
    string match -q '*LEAK*' -- (string collect $CHILD_OUT); and fail "supervisor-shaped child loaded GPY: "(string sub -l 200 -- (string collect $CHILD_OUT)); or pass "supervisor-shaped child loaded no GPY functions"
    pgrep -f "$sandbox/missing.sock" >/dev/null 2>&1; and fail "agent process left running"; or pass "no agent started"
end

function run_scenario
    echo "=== Scenario: $argv[1] ==="
    scenario_$argv[1]
end

run_scenario noninteractive_c_is_inert
run_scenario supervisor_child_shape

test_finish "hostile-env fish scenarios"
