#!/usr/bin/env fish
# tests/fish/e2e_supervisor_settings_reload.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A running Fish supervisor loop follows `gpy config set agent.supervisor.*`
# without a restart: a real agent rewrites the theme-export cache on the
# reload, and the loop's next pass uses the new check interval and restart
# attempt limit, and exits once the supervisor is disabled. The loop is the
# real one in a separate fish process; only the agent health probe, restart,
# clock and sleep are stubbed so restart cycles are observable and fast.

source (dirname (status -f))/../lib/test_helpers.fish

function fail
    echo "❌ $argv"
    cleanup_test_files
    exit 1
end

set -l repo_root (path resolve (status dirname)/../..)
init_test_env; or exit 1
mkdir -p $XDG_CONFIG_HOME/gpy $XDG_CACHE_HOME/gpy
printf '%s\n' '[agent.supervisor]' 'enabled = true' 'check_interval_seconds = 7' 'max_restart_attempts = 1' >$XDG_CONFIG_HOME/gpy/config.toml
start_test_agent; or fail "agent did not start"

set -g export_cache $XDG_CACHE_HOME/gpy/theme-export.fish
function __export_has --argument-names line
    test -r $export_cache; and string match -q -- $line <$export_cache
end
poll_until 5 __export_has 'set -gx GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS "7"'
or fail "the agent never wrote the initial export"

# Every restart attempt fails and each `date +%s` advances 10 s past the 5 s
# rate limit, so the loop cycles: <attempt limit> restarts, then the 123 s
# long backoff. The log records restarts and the sleep lengths the loop asks for.
set -g loop_log $__gpy_test_tmp_dir/loop.log
touch $loop_log
env GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=1 \
    GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS=5 GPY_SUPERVISOR_LONG_BACKOFF_SECONDS=123 \
    fish --no-config -c '
        source $argv[1]/fish/core/constants.fish
        source $argv[1]/fish/core/util.fish
        source $argv[1]/fish/core/ipc.fish
        set -g loop_log $argv[2]
        set -g fake_now 100
        function __gpy_resolve_agent_binary; echo stub; end
        function __gpy_agent_is_healthy; return 1; end
        function __gpy_agent_restart; echo restart >>$loop_log; return 1; end
        function date
            if test "$argv" = "+%s"
                set -g fake_now (math $fake_now + 10)
                echo $fake_now
                return 0
            end
            command date $argv
        end
        function sleep; echo "sleep $argv[1]" >>$loop_log; command sleep 0.05; end
        __gpy_agent_supervisor_loop' -- $repo_root $loop_log >/dev/null 2>&1 &
set -g loop_pid $last_pid
set -a __gpy_test_client_pids $loop_pid

# Restarts in the first full cycle (between two long backoffs) logged after
# line $offset, provided the loop has slept $interval since then.
function __cycle_matches --argument-names offset interval restarts
    set -l lines (tail -n +(math $offset + 1) $loop_log)
    contains -- "sleep $interval" $lines; or return 1
    set -l in_cycle 0
    set -l count 0
    for line in $lines
        if test "$line" = "sleep 123"
            test $in_cycle = 1; and test $count = $restarts; and return 0
            test $in_cycle = 1; and return 1
            set in_cycle 1
            set count 0
        else if test "$line" = restart
            set count (math $count + 1)
        end
    end
    return 1
end

function __log_offset
    count <$loop_log
end

poll_until 5 __cycle_matches 0 7 1
or fail "the loop never ran 7 s checks with 1 restart attempt (log: "(string join ', ' (tail -n 12 $loop_log))")"
echo "✅ the loop starts on the exported interval (7 s) and attempt limit (1)"

# $1 key, $2 value, $3/$4 the interval and restart attempts the loop must then use.
function __apply --argument-names key value interval restarts
    gpy config set $key $value >/dev/null 2>&1; or fail "gpy config set $key $value failed"
    set -l offset (__log_offset)
    poll_until 5 __cycle_matches $offset $interval $restarts
    or fail "after setting $key=$value the loop never used $interval s / $restarts attempts (log: "(string join ', ' (tail -n 12 $loop_log))")"
    echo "✅ $key=$value reached the running loop"
end

__apply agent.supervisor.check_interval_seconds 9 9 1
__apply agent.supervisor.max_restart_attempts 3 9 3

function __loop_exited
    not kill -0 $loop_pid 2>/dev/null
end
gpy config set agent.supervisor.enabled false >/dev/null 2>&1; or fail "gpy config set agent.supervisor.enabled false failed"
poll_until 5 __loop_exited; or fail "the loop kept running after agent.supervisor.enabled=false"
echo "✅ agent.supervisor.enabled=false stopped the running loop"

cleanup_test_files
echo "✅ Supervisor settings reach a running Fish supervisor without a restart"
