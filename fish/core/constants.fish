# SPDX-License-Identifier: GPL-3.0-or-later

# Nerd icons
set -g __gpy_icon_nerd_elixir ""
set -g __gpy_icon_nerd_erlang ""
set -g __gpy_icon_nerd_go ""
set -g __gpy_icon_nerd_java "󰬷"
set -g __gpy_icon_nerd_node "󰎙"
set -g __gpy_icon_nerd_python "󰌠"
set -g __gpy_icon_nerd_ruby ""
set -g __gpy_icon_nerd_rust "󱘗"
set -g __gpy_icon_nerd_swift "󰛥"
set -g __gpy_icon_nerd_zig "⚡"
set -g __gpy_icon_nerd_git_new "󰿞"
set -g __gpy_icon_nerd_git_staged "󱓞"
set -g __gpy_icon_nerd_git_unstaged "󰇷"
set -g __gpy_icon_nerd_git_untracked "?"
set -g __gpy_icon_nerd_git_stash "󰋀"
set -g __gpy_icon_nerd_git_clean "󰄬"
set -g __gpy_icon_nerd_git_ahead "󰁞"
set -g __gpy_icon_nerd_git_behind "󰁆"
set -g __gpy_icon_nerd_git_diverged "󰩋"
set -g __gpy_icon_nerd_lock "🔒"
set -g __gpy_icon_nerd_status_ok "✔"
set -g __gpy_icon_nerd_status_fail "✖"
set -g __gpy_icon_nerd_duration "󰑧"
set -g __gpy_icon_nerd_prompt "❯"
set -g __gpy_icon_nerd_pl_segment ""
set -g __gpy_icon_nerd_pl_prompt ""

# ASCII icons
set -g __gpy_icon_ascii_elixir ex
set -g __gpy_icon_ascii_erlang erl
set -g __gpy_icon_ascii_go go
set -g __gpy_icon_ascii_java java
set -g __gpy_icon_ascii_node node
set -g __gpy_icon_ascii_python py
set -g __gpy_icon_ascii_ruby rb
set -g __gpy_icon_ascii_rust rs
set -g __gpy_icon_ascii_swift swift
set -g __gpy_icon_ascii_zig zig
set -g __gpy_icon_ascii_git_new new
set -g __gpy_icon_ascii_git_staged "+"
set -g __gpy_icon_ascii_git_unstaged "!"
set -g __gpy_icon_ascii_git_untracked "?"
set -g __gpy_icon_ascii_git_stash "\$"
set -g __gpy_icon_ascii_git_clean clean
set -g __gpy_icon_ascii_git_ahead ">"
set -g __gpy_icon_ascii_git_behind "<"
set -g __gpy_icon_ascii_git_diverged "<>"
set -g __gpy_icon_ascii_lock "[L]"
set -g __gpy_icon_ascii_status_ok ok
set -g __gpy_icon_ascii_status_fail err
set -g __gpy_icon_ascii_duration t
set -g __gpy_icon_ascii_prompt ">"
set -g __gpy_icon_ascii_pl_segment ""
set -g __gpy_icon_ascii_pl_prompt ""

# Language colors
set -g __gpy_color_node green
set -g __gpy_color_elixir magenta
set -g __gpy_color_go cyan
set -g __gpy_color_rust red
set -g __gpy_color_java yellow
set -g __gpy_color_python green
set -g __gpy_color_ruby red
set -g __gpy_color_swift red
set -g __gpy_color_zig yellow
set -g __gpy_color_erlang red

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

set -g GPY_AGENT_START_DELAY_MS 200

# Exponential backoff, ~2s total.
set -g GPY_SOCKET_WAIT_MAX_ATTEMPTS 20

set -g GPY_CIRCUIT_BREAKER_THRESHOLD 3

set -g GPY_CIRCUIT_BREAKER_BACKOFF_SECONDS 60

# Self-heal bound when a SIGURG repaint push is missed.
set -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS (__gpy_uint_or_default "$GPY_GIT_INSTANT_CACHE_TTL_SECONDS" 5)

set -g GPY_LANGUAGE_CACHE_TTL_SECONDS (__gpy_uint_or_default "$GPY_LANGUAGE_CACHE_TTL_SECONDS" 30)

set -g GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS" 30)

set -g GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS" 5)

set -g GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS (__gpy_uint_or_default "$GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS" 60)

set -g GPY_SUPERVISOR_LONG_BACKOFF_SECONDS (__gpy_uint_or_default "$GPY_SUPERVISOR_LONG_BACKOFF_SECONDS" 300)

# Caps re-forks of a crash-looping agent per shell session.
set -g GPY_AGENT_AUTOSTART_MAX_ATTEMPTS (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_MAX_ATTEMPTS" 3)

set -g GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS" 10)
