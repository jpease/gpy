# bash/core/signals.bash
# Signal handlers for live updates and config reload

# Config hot-reload: re-read the theme and drop render caches. Runs from the
# doorbell handler when the agent left a `<pid>.reload` flag.
__gpy_reload_config() {
    # Reload theme from agent
    __gpy_load_theme &>/dev/null
    # Clear character/directory render caches (#343): a theme change can alter
    # the rendered templates, so a stale cache entry must not survive a reload
    # even if the input tuple happens to repeat.
    __gpy_char_cache_key=""
    __gpy_char_cache_val=""
    __gpy_dir_cache_key=""
    __gpy_dir_cache_val=""
}

# SIGURG doorbell (#674): the agent's only signal to a shell. SIGURG is
# ignored by default, so a shell that has not installed this handler yet (for
# example one that just ran `exec bash` under the same, still-registered PID)
# is not killed by it. The message travels in flag files next to this shell's
# tracking entry (__gpy_shell_flag_base, set by
# __gpy_track_shell_for_agent_recovery):
#   <pid>.reregister  the agent (re)started: forget the registration and
#                     register again (#638). Re-entrancy guard: a second nudge
#                     while one is being handled is dropped; the next
#                     prompt's own retry covers it.
#   <pid>.reload      config/theme changed: reload.
# Each flag is removed before acting on it. Then re-render, as a plain
# repaint does. Bash at an idle readline prompt defers this trap until the
# line is accepted (readline runs traps immediately only for SIGALRM), so
# both the re-registration and the new PS1 take effect at the next prompt.
__gpy_reregistering=""
__gpy_handle_doorbell() {
    local base="${__gpy_shell_flag_base:-}"
    if [[ -n "$base" && -e "$base.reregister" && -z "$__gpy_reregistering" ]]; then
        rm -f "$base.reregister" 2>/dev/null
        __gpy_reregistering=1
        __gpy_reregister_with_agent &>/dev/null
        __gpy_reregistering=""
    fi
    if [[ -n "$base" && -e "$base.reload" ]]; then
        rm -f "$base.reload" 2>/dev/null
        __gpy_reload_config
    fi
    if [[ -n "$PS1" ]]; then
        __gpy_render_prompt "${__gpy_last_exit_code:-0}"
    fi
}

# Shell exit: tell the agent this PID is gone and drop the recovery-nudge
# entry, so a restarted agent does not signal a PID that may have been
# recycled (the agent verifies liveness too; this is the polite half). Zsh
# and Fish already do this from their exit hooks. Any EXIT trap the user or
# a framework installed first is chained, never clobbered (#320's DEBUG
# treatment, applied to EXIT).
__gpy_prev_exit_trap=""
__gpy_handle_exit() {
    if [[ -n "$__gpy_registered" ]]; then
        __gpy_send_json "{\"op\":\"unregister\",\"pid\":$$}" &>/dev/null || true
    fi
    __gpy_untrack_shell_for_agent_recovery
    if [[ -n "$__gpy_prev_exit_trap" ]]; then
        eval "$__gpy_prev_exit_trap"
    fi
}

# Setup signal handlers
__gpy_setup_signals() {
    # Register signal handlers
    trap '__gpy_handle_doorbell' URG

    local existing_exit
    existing_exit="$(trap -p EXIT)"
    if [[ -n "$existing_exit" && "$existing_exit" != *"__gpy_handle_exit"* ]]; then
        existing_exit="${existing_exit#trap -- \'}"
        __gpy_prev_exit_trap="${existing_exit%\' EXIT}"
    fi
    trap '__gpy_handle_exit' EXIT
}

# Initialize signals
__gpy_setup_signals
