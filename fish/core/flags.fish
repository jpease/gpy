# SPDX-License-Identifier: GPL-3.0-or-later

# A flag is off when set to `0` or `false`, or when this shell failed to load
# GPY (`__gpy_load_failed`, never exported); unset, empty and any other value
# leave it on. Bash and Zsh apply the same value rule.
function __gpy_agent_enabled --description 'Agent integration on (GPY_AGENT_ENABLED)'
    set -q __gpy_load_failed; and return 1
    not contains -- "$GPY_AGENT_ENABLED" 0 false
end

function __gpy_supervisor_enabled --description 'Agent restarts on (GPY_AGENT_SUPERVISOR_ENABLED)'
    set -q __gpy_load_failed; and return 1
    not contains -- "$GPY_AGENT_SUPERVISOR_ENABLED" 0 false
end
