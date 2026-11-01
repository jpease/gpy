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

# Public tuning knobs are read at the use site, never assigned: the user's
# value if a non-negative integer, else the default.
function __gpy_ipc_timeout_ms --description 'IPC budget in ms (GPY_IPC_TIMEOUT_MS)'
    __gpy_uint_or_default "$GPY_IPC_TIMEOUT_MS" 150
end

# Self-heal bound when a SIGURG repaint push is missed.
function __gpy_git_cache_ttl --description 'Git instant-cache TTL in seconds (GPY_GIT_INSTANT_CACHE_TTL_SECONDS)'
    __gpy_uint_or_default "$GPY_GIT_INSTANT_CACHE_TTL_SECONDS" 5
end

function __gpy_lang_cache_ttl --description 'Language cache TTL in seconds (GPY_LANGUAGE_CACHE_TTL_SECONDS)'
    __gpy_uint_or_default "$GPY_LANGUAGE_CACHE_TTL_SECONDS" 30
end
