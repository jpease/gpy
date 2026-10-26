# SPDX-License-Identifier: GPL-3.0-or-later

# A flag is off only when set to `0` or `false`; unset, empty and any other
# value leave it on. Bash and Zsh apply the same rule.
function __gpy_agent_enabled --description 'Agent integration on (GPY_AGENT_ENABLED)'
    not contains -- "$GPY_AGENT_ENABLED" 0 false
end

function __gpy_supervisor_enabled --description 'Agent restarts on (GPY_AGENT_SUPERVISOR_ENABLED)'
    not contains -- "$GPY_AGENT_SUPERVISOR_ENABLED" 0 false
end
