# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# CONSTANTS
# Defines raw icon strings and other immutable values.
# Themes should not need to touch this file.
# ============================================================================

# ----------------------------------------------------------------------------
# Icon Theme: Nerd
# ----------------------------------------------------------------------------
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

# ----------------------------------------------------------------------------
# Icon Theme: ASCII
# ----------------------------------------------------------------------------
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

# ----------------------------------------------------------------------------
# Language Colors
# ----------------------------------------------------------------------------
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

# ============================================================================
# IPC COMMUNICATION TIMEOUTS
# ============================================================================

# Timeout for IPC socket communication (milliseconds)
# This is the maximum time to wait for a response from the agent
set -g GPY_IPC_TIMEOUT_MS 150

# Delay after starting agent before first connection attempt (milliseconds)
# Gives the agent time to initialize and create the socket
set -g GPY_AGENT_START_DELAY_MS 200

# Maximum attempts to wait for socket to appear
# Used with exponential backoff, totals ~2 seconds
set -g GPY_SOCKET_WAIT_MAX_ATTEMPTS 20

# ============================================================================
# CIRCUIT BREAKER CONFIGURATION
# ============================================================================

# Number of consecutive failures before opening circuit breaker
# After this many failures, agent requests are blocked for a backoff period
set -g GPY_CIRCUIT_BREAKER_THRESHOLD 3

# Backoff period when circuit breaker is open (seconds)
# No agent requests are attempted during this period
set -g GPY_CIRCUIT_BREAKER_BACKOFF_SECONDS 60

# ============================================================================
# CACHE CONFIGURATION
# ============================================================================

# Git instant-cache TTL (seconds)
# This is the pull-based self-heal bound when a SIGURG repaint push is missed:
# cached git output is still served instantly, but entries older than this flag
# a throttled background refresh.
set -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS 5

# Cache TTL for language instant cache (seconds)
# Reserved for compatibility; rendered language cache is preserved until the
# agent rewrites or clears it so clock repaints do not hide stable segments.
set -g GPY_LANGUAGE_CACHE_TTL_SECONDS 30

# ============================================================================
# SUPERVISOR CONFIGURATION
# ============================================================================

# Pure: prints `value` if it is a non-negative integer, else `default`.
# Applied below to every numeric GPY_* the supervisor/autostart code used to
# re-validate at every use site with its own `string match -qr '^[0-9]+$'`
# (#612) -- validating once here means those use sites can trust the value.
#
# These six are re-applied at their ipc.fish use sites too (not just here):
# several tests (tests/fish/supervisor_cadence.test.fish,
# tests/fish/prompt_autostart_backoff.test.fish) `set -gx` these AFTER
# sourcing this file, bypassing this load-time pass entirely, and a real
# theme/config reload can do the same. The use-site call validates whatever
# value is current at read time; it's the same pure helper, not a
# reintroduced inline regex, so `rg` for the old `string match -qr
# '^[0-9]+$'` pattern in ipc.fish still finds nothing.
function __gpy_uint_or_default --argument-names value default --description 'Pure: value if a non-negative integer, else default'
    set -q value[1]; or set value ""
    if string match -qr '^[0-9]+$' -- "$value"
        printf '%s' "$value"
    else
        printf '%s' "$default"
    end
end

# Health check interval for supervisor (seconds)
# How often the supervisor checks if the agent is healthy
set -g GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS" 30)

# Maximum restart attempts before giving up
# Supervisor will stop trying after this many consecutive failures
set -g GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS" 5)

# Minimum time between restarts (seconds)
# Rate limiting to prevent restart storms
set -g GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS (__gpy_uint_or_default "$GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS" 60)

# Long backoff period after exceeding restart limit (seconds)
# Wait this long before resetting the restart counter
set -g GPY_SUPERVISOR_LONG_BACKOFF_SECONDS (__gpy_uint_or_default "$GPY_SUPERVISOR_LONG_BACKOFF_SECONDS" 300)

# Maximum autostart attempts from the per-prompt fish_prompt hook
# Bounds how many times a crash-looping agent binary gets re-forked before the
# hook gives up for this shell session (the background supervisor loop still
# retries independently with its own backoff above)
set -g GPY_AGENT_AUTOSTART_MAX_ATTEMPTS (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_MAX_ATTEMPTS" 3)

# Minimum time between per-prompt autostart attempts (seconds)
# Rate limits respawns so a crash-looping agent doesn't fork on every prompt
set -g GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS" 10)
