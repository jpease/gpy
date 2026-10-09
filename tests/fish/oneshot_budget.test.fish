#!/usr/bin/env fish
# tests/fish/oneshot_budget.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The Fish twin of tests/bash/oneshot_budget.test.bash and
# tests/zsh/oneshot_budget.test.zsh (#845, #614).
#
# A dead daemon must cost at most ONE `gpy-agent oneshot` fork per prompt
# render, and that budget must be a plain per-render variable
# (__gpy_oneshot_used) rather than the predictable
# ${TMPDIR}/.gpy_oneshot_used_$fish_pid marker file Fish used to keep: a path
# another local user could pre-create to switch the fallback off, and one a
# shell left behind whenever it exited with the budget spent.
#
# This stubs `gpy-agent` on PATH to count `oneshot` invocations into a file,
# points GPY_AGENT_SOCKET_PATH at a socket that never exists (dead daemon), and
# drives the real fish_prompt dispatch loop with stub segments that each call a
# different oneshot-capable request wrapper (git/lang request, duration,
# character), the way the real segments do inside command substitutions. Only
# one of those attempts per render may fork, and the next render gets a fresh
# budget -- fish_prompt's reset.

set -g pass_count 0
set -g fail_count 0

function check --argument-names label expected actual
    if test "$actual" = "$expected"
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label (expected [$expected], got [$actual])"
    end
end

set -l script_dir (dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l tmp (mktemp -d)
mkdir -p $tmp/bin $tmp/home/.config/gpy $tmp/tmpdir $tmp/repo/.git
set -l counter $tmp/oneshot-count
: >$counter

# Count oneshot calls into a file; the budget variable lives in the shell, the
# stub is a separate process.
printf '%s\n' '#!/bin/sh' \
    'if [ "$1" = "oneshot" ]; then' \
    '    printf x >> "$GPY_TEST_COUNTER"' \
    '    printf "STUBBED-ONESHOT-%s" "$2"' \
    '    exit 0' \
    fi \
    'exit 0' >$tmp/bin/gpy-agent
chmod +x $tmp/bin/gpy-agent

set -l saved_home $HOME
set -l saved_xdg_config $XDG_CONFIG_HOME
set -l saved_tmpdir $TMPDIR
set -gx HOME $tmp/home
set -gx XDG_CONFIG_HOME $tmp/home/.config
set -gx XDG_CACHE_HOME $tmp/home/.cache
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx TMPDIR $tmp/tmpdir
set -gx PATH $tmp/bin $PATH
set -gx GPY_TEST_COUNTER $counter
set -gx GPY_AGENT_SOCKET_PATH $tmp/dead-agent.sock
# Agent enabled, supervisor off: the full init path without a daemon.
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0

# Three segments that each try the oneshot fallback through a different
# wrapper, plus the always-rendered prompt character (fish_prompt calls it).
function segment_osa_detect
    return 0
end
function segment_osa_render --argument-names is_last is_first
    set -l out (__gpy_request git $repo true "" true)
end
function segment_osb_detect
    return 0
end
function segment_osb_render --argument-names is_last is_first
    set -l out (__gpy_request_duration 5000 "" "" "")
end
function segment_osc_detect
    return 0
end
function segment_osc_render --argument-names is_last is_first
    set -l out (__gpy_request lang $repo "" "" "")
end

set -g repo $tmp/repo
set -gx GPY_TEST_SEGMENTS "osa osb osc"
source fish/core/init.fish
source fish/functions/fish_prompt.fish

fish_prompt >/dev/null 2>&1
check "at most one oneshot fork per render" 1 (wc -c <$counter | string trim)

# The next render starts with a fresh budget: the stale one must not carry over.
fish_prompt >/dev/null 2>&1
check "the next render gets its own budget" 2 (wc -c <$counter | string trim)

# No predictable-name marker file under TMPDIR for this PID.
set -l leftover (find $tmp/tmpdir -maxdepth 1 -name "*gpy*$fish_pid*" 2>/dev/null)
check "no leftover predictable-name marker files" 0 (count $leftover)
test (count $leftover) -gt 0; and printf '  leftover: %s\n' $leftover

# A budget that is already spent is honoured: a second claim in one render fails.
set -e __gpy_oneshot_used
__gpy_oneshot_claim
set -l first $status
__gpy_oneshot_claim
set -l second $status
check "a second claim in the same render is refused" "0 1" "$first $second"

rm -rf $tmp
set -gx HOME $saved_home
test -n "$saved_xdg_config"; and set -gx XDG_CONFIG_HOME $saved_xdg_config; or set -e XDG_CONFIG_HOME
test -n "$saved_tmpdir"; and set -gx TMPDIR $saved_tmpdir; or set -e TMPDIR

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "=== oneshot budget test passed ==="
