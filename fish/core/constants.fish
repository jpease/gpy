# SPDX-License-Identifier: GPL-3.0-or-later

# Nerd icons
set -g __gpy_icon_nerd_git_staged "󱓞"
set -g __gpy_icon_nerd_git_unstaged "󰇷"
set -g __gpy_icon_nerd_git_untracked "?"
set -g __gpy_icon_nerd_status_ok "✔"
set -g __gpy_icon_nerd_status_fail "✖"
set -g __gpy_icon_nerd_duration "󰑧"
set -g __gpy_icon_nerd_prompt "❯"

# ASCII icons
set -g __gpy_icon_ascii_git_staged "+"
set -g __gpy_icon_ascii_git_unstaged "!"
set -g __gpy_icon_ascii_git_untracked "?"
set -g __gpy_icon_ascii_status_ok ok
set -g __gpy_icon_ascii_status_fail err
set -g __gpy_icon_ascii_duration t
set -g __gpy_icon_ascii_prompt ">"

# Prints `value` if a non-negative integer, else `default`.
function __gpy_uint_or_default --argument-names value default --description 'Pure: value if a non-negative integer, else default'
    set -q value[1]; or set value ""
    if string match -qr '^[0-9]+$' -- "$value"
        printf '%s' "$value"
    else
        printf '%s' "$default"
    end
end

set -g GPY_IPC_TIMEOUT_MS (__gpy_uint_or_default "$GPY_IPC_TIMEOUT_MS" 150)

# Budget when the prompt has nothing to show for a segment (a cold git cache,
# a directory not yet rendered): waiting longer keeps the segment under load.
# Never shorter than GPY_IPC_TIMEOUT_MS; see __gpy_ipc_cold_timeout_ms.
set -g GPY_IPC_COLD_TIMEOUT_MS (__gpy_uint_or_default "$GPY_IPC_COLD_TIMEOUT_MS" 500)

# Self-heal bound when a SIGURG repaint push is missed.
set -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS (__gpy_uint_or_default "$GPY_GIT_INSTANT_CACHE_TTL_SECONDS" 5)

set -g GPY_LANGUAGE_CACHE_TTL_SECONDS (__gpy_uint_or_default "$GPY_LANGUAGE_CACHE_TTL_SECONDS" 30)

set -g GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS" 30)

set -g GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS" 5)
