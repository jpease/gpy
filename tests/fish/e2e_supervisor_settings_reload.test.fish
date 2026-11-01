#!/usr/bin/env fish
# tests/fish/e2e_supervisor_settings_reload.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The Fish per-prompt check follows `gpy config set agent.supervisor.*`
# without a restart: a real agent rewrites the theme-export cache on the
# reload, the shell reloads it, and the next checks use the new check interval
# and restart attempt limit, and stop once the supervisor is disabled. Only the
# health probe, the start and the clock are stubbed so the checks are
# observable and fast.

source (dirname (status -f))/../lib/test_helpers.fish

function die
    fail "$argv"
    cleanup_test_files
    exit 1
end

set -l repo_root (path resolve (status dirname)/../..)
init_test_env; or exit 1
# The settings under test come from config.toml; an exported user override
# would win over them.
set -e GPY_AGENT_SUPERVISOR_ENABLED GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS
mkdir -p $XDG_CONFIG_HOME/gpy $XDG_CACHE_HOME/gpy
printf '%s\n' '[agent.supervisor]' 'enabled = true' 'check_interval_seconds = 7' 'max_restart_attempts = 1' >$XDG_CONFIG_HOME/gpy/config.toml
start_test_agent; or die "agent did not start"

set -g export_cache $XDG_CACHE_HOME/gpy/theme-export.fish
function __export_has --argument-names line
    test -r $export_cache; and string match -q -- $line <$export_cache
end
poll_until 5 __export_has 'set -g __gpy_cfg_supervisor_check_interval "7"'
or die "the agent never wrote the initial export"

source $repo_root/fish/core/init.fish
set -gx GPY_AGENT_ENABLED 1
set -g starts 0
set -g probes 0
function __gpy_agent_is_healthy; set -g probes (math $probes + 1); return 1; end
function __gpy_start_agent_detached; set -g starts (math $starts + 1); return 0; end
set -g fake_now 1000
function __gpy_now_seconds; echo $fake_now; end

# Read the exported settings the way a running shell does after the agent rings .reload.
function __reload_settings
    source $export_cache
end

# Run `prompts` checks, the clock advancing `step` seconds per prompt, from
# fresh counters; prints "<probes> <starts>".
function __run_checks --argument-names step prompts
    set -g starts 0
    set -g probes 0
    set -g __gpy_supervisor_check_attempts 0
    set -g __gpy_supervisor_last_check_time 0
    set -g fake_now 1000
    for i in (seq $prompts)
        __gpy_supervisor_check
        set fake_now (math $fake_now + $step)
    end
    echo $probes $starts
end

__reload_settings
check "exported interval is 7 s" 7 $__gpy_cfg_supervisor_check_interval
check "exported attempt limit is 1" 1 $__gpy_cfg_supervisor_max_restarts
check "6 s prompts on a 7 s interval, one attempt: two probes, one start" "2 1" (__run_checks 6 4)

# $1 key, $2 value, $3 variable the export must then hold, $4 its value.
function __apply --argument-names key value var want
    gpy config set $key $value >/dev/null 2>&1; or die "gpy config set $key $value failed"
    poll_until 5 __export_has "set -g $var \"$want\""; or die "the export never set $var=$want"
    __reload_settings
    pass "$key=$value reached the exported settings"
end

__apply agent.supervisor.check_interval_seconds 9 __gpy_cfg_supervisor_check_interval 9
check "8 s prompts on a 9 s interval: the second prompt is rate limited" "1 1" (__run_checks 8 2)

__apply agent.supervisor.max_restart_attempts 3 __gpy_cfg_supervisor_max_restarts 3
check "three attempts allowed: six checked prompts start the agent three times" "6 3" (__run_checks 10 6)

__apply agent.supervisor.enabled false __gpy_cfg_supervisor_enabled 0
check "a disabled supervisor neither probes nor starts" "0 0" (__run_checks 10 6)

cleanup_test_files
test_finish "Supervisor settings reach the per-prompt check without a restart"
