# SPDX-License-Identifier: GPL-3.0-or-later

# The boolean table shared with Bash, Zsh and the agent (tests/fixtures/
# shell_env_flag_vectors.tsv): `1/true/yes/on` is on, `0/false/no/off` is off,
# case-insensitive, no surrounding whitespace. Anything else prints DEFAULT.
# Silent: this file is sourced before util.fish, so it cannot log.
function __gpy_flag_value --argument-names value default --description 'Pure: on/off for a recognised boolean spelling, else default'
    if string match -qri -- '\A(1|true|yes|on)\z' "$value"
        echo on
    else if string match -qri -- '\A(0|false|no|off)\z' "$value"
        echo off
    else
        echo "$default"
    end
end

# Precedence: a recognised user value, then the theme export's
# __gpy_cfg_* mirror (off only for `0`), then on. A shell that failed to load
# GPY (`__gpy_load_failed`, never exported) is off. These stay silent;
# __gpy_warn_flag_values reports once.
function __gpy_agent_enabled --description 'Agent integration on (GPY_AGENT_ENABLED)'
    set -q __gpy_load_failed; and return 1
    switch (__gpy_flag_value "$GPY_AGENT_ENABLED" cfg)
        case on
            return 0
        case off
            return 1
    end
    test "$__gpy_cfg_agent_enabled" != 0
end

function __gpy_supervisor_enabled --description 'Agent restarts on (GPY_AGENT_SUPERVISOR_ENABLED)'
    set -q __gpy_load_failed; and return 1
    switch (__gpy_flag_value "$GPY_AGENT_SUPERVISOR_ENABLED" cfg)
        case on
            return 0
        case off
            return 1
    end
    test "$__gpy_cfg_supervisor_enabled" != 0
end

# A non-negative integer user value wins (0 allowed, not range-checked), then
# the config mirror, then the default.
function __gpy_supervisor_check_interval --description 'Seconds between supervisor checks'
    __gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS" (__gpy_uint_or_default "$__gpy_cfg_supervisor_check_interval" 30)
end

function __gpy_supervisor_max_restarts --description 'Supervisor restart attempts'
    __gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS" (__gpy_uint_or_default "$__gpy_cfg_supervisor_max_restarts" 5)
end

# Runs once per shell, after the theme loads (see __gpy_warn_flag_values_once):
# logs unrecognised flag values under GPY_DEBUG.
function __gpy_warn_flag_values --description 'One-time warnings for GPY_* flag values'
    for name in GPY_AGENT_ENABLED GPY_AGENT_SUPERVISOR_ENABLED GPY_SHOW_STATUS
        set -q $name; or continue
        set -l value "$$name"
        test -n "$value"; or continue
        if test (__gpy_flag_value "$value" unrecognised) = unrecognised
            __gpy_log_debug flags "$name=$value is not a boolean; ignoring it"
        end
    end
end
