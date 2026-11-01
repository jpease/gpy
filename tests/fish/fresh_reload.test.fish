#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# gpy-prompt-reload always runs `gpy-agent theme export` (the `fresh` path), so a
# stale theme-export cache never wins; the next re-registration does not source
# that stale cache either, and the .reload doorbell path still reads it. A
# failing agent leaves the running theme alone and prints one fixed line.

source (dirname (status --filename))/../lib/test_helpers.fish

set -g t_sandbox (mktemp -d)
test_track_dir $t_sandbox
mkdir -p $t_sandbox/home $t_sandbox/cache/gpy $t_sandbox/config $t_sandbox/run $t_sandbox/bin
set -g t_calls $t_sandbox/agent-calls.log

# The stand-in agent: `theme export` prints the current config's export, or
# fails when STUB_FAIL is set. It logs every call.
printf '%s\n' \
    '#!/bin/sh' \
    'echo "$*" >>"$STUB_AGENT_LOG"' \
    'if [ "$1 $2" = "theme export" ]; then' \
    '    [ -n "$STUB_FAIL" ] && exit 1' \
    '    echo "set -g __enabled_segments directory"' \
    '    echo "set -g __gpy_fresh_marker fresh"' \
    '    exit 0' \
    'fi' \
    'exit 1' >$t_sandbox/bin/gpy-agent
chmod +x $t_sandbox/bin/gpy-agent

# The cache an agent wrote from an older config.
printf '%s\n' \
    'set -g __gpy_export_version "1"' \
    'set -g __enabled_segments clock' \
    'set -g __gpy_fresh_marker stale' >$t_sandbox/cache/gpy/theme-export.fish

# Run $argv[1] after sourcing init.fish; stdout is the script's, stderr goes
# to $t_sandbox/stderr. $argv[2..] are extra environment entries.
function run_child --argument-names code
    set -l extra
    set -q argv[2]; and set extra $argv[2..-1]
    command env -i HOME=$t_sandbox/home XDG_CONFIG_HOME=$t_sandbox/config XDG_CACHE_HOME=$t_sandbox/cache \
        XDG_RUNTIME_DIR=$t_sandbox/run TMPDIR=$t_sandbox PATH=$t_sandbox/bin:/usr/bin:/bin TERM=dumb \
        GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH=$t_sandbox/run/gpy.sock \
        STUB_AGENT_LOG=$t_calls GPY_T_ROOT=$__gpy_root $extra \
        (command -s fish) --no-config -c "source \$GPY_T_ROOT/fish/core/init.fish
$code" 2>$t_sandbox/stderr
end

echo "--- a stale cache and a working agent ---"
: >$t_calls
set -l out (run_child '
    echo "start=$__gpy_fresh_marker"
    gpy-prompt-reload
    echo "reload_rc=$status"
    echo "after=$__gpy_fresh_marker segments=$__enabled_segments"
    set -g __gpy_fresh_marker probe
    __gpy_reload_if_theme_export_changed
    echo "reregister=$__gpy_fresh_marker"
    __gpy_apply_agent_reload
    echo "doorbell=$__gpy_fresh_marker"
')
check "startup reads the cache first" start=stale "$out[1]"
check "gpy-prompt-reload succeeds" reload_rc=0 "$out[2]"
check "gpy-prompt-reload applies the agent's current export" "after=fresh segments=directory" "$out[3]"
check "the next re-registration does not source the stale cache" reregister=probe "$out[4]"
check "the doorbell path still reads the cache" doorbell=stale "$out[5]"
check "gpy-prompt-reload is silent on success" "" (string collect <$t_sandbox/stderr)
check "the agent was asked for exactly one export" 1 (count (string match 'theme export*' <$t_calls))

echo "--- a failing agent ---"
: >$t_calls
set out (run_child '
    gpy-prompt-reload
    echo "reload_rc=$status"
    echo "after=$__gpy_fresh_marker segments=$__enabled_segments"
' STUB_FAIL=1)
check "a failing agent makes gpy-prompt-reload return 1" reload_rc=1 "$out[1]"
check "a failing agent leaves the running theme alone" "after=stale segments=clock" "$out[2]"
check "a failing agent prints exactly one fixed line" "gpy: theme reload failed; run gpy doctor" (string collect <$t_sandbox/stderr)

echo "--- no agent at all ---"
set out (command env -i HOME=$t_sandbox/home XDG_CONFIG_HOME=$t_sandbox/config XDG_CACHE_HOME=$t_sandbox/cache \
    XDG_RUNTIME_DIR=$t_sandbox/run TMPDIR=$t_sandbox PATH=/usr/bin:/bin TERM=dumb \
    GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH=$t_sandbox/run/gpy.sock \
    GPY_T_ROOT=$__gpy_root (command -s fish) --no-config -c '
    source $GPY_T_ROOT/fish/core/init.fish
    gpy-prompt-reload
    echo "reload_rc=$status"
    echo "after=$__gpy_fresh_marker"' 2>$t_sandbox/stderr)
check "a missing agent makes gpy-prompt-reload return 1" reload_rc=1 "$out[1]"
check "a missing agent leaves the running theme alone" after=stale "$out[2]"
check "a missing agent prints exactly one fixed line" "gpy: theme reload failed; run gpy doctor" (string collect <$t_sandbox/stderr)

test_finish "fresh reload"
