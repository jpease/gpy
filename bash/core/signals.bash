# bash/core/signals.bash
# Signal handlers for live updates and config reload

# SIGUSR1: Live update from agent (prompt refresh)
__gpy_handle_sigusr1() {
    # Force prompt re-render without incrementing command count
    # Note: This is less reliable in Bash than Fish/Zsh
    # May not work inside command substitution or subshells
    if [[ -n "$PS1" ]]; then
        __gpy_render_prompt "${__gpy_last_exit_code:-0}"
    fi
}

# SIGUSR2: Config hot-reload signal
__gpy_handle_sigusr2() {
    # Reload theme from agent
    __gpy_load_theme &>/dev/null
    # Clear character/directory render caches (#343): a theme change can alter
    # the rendered templates, so a stale cache entry must not survive a reload
    # even if the input tuple happens to repeat.
    __gpy_char_cache_key=""
    __gpy_char_cache_val=""
    __gpy_dir_cache_key=""
    __gpy_dir_cache_val=""
    # Refresh prompt
    if [[ -n "$PS1" ]]; then
        __gpy_render_prompt "${__gpy_last_exit_code:-0}"
    fi
}

# SIGALRM: the agent's "I (re)started, register again" nudge (#638). The agent
# sends it to every PID recorded under <runtime root>/shells/ when it comes
# up, and re-sends it to tracked shells that stay unregistered. Re-entrancy
# guard: a second nudge while one is being handled is dropped; the next
# prompt's own retry covers it.
__gpy_reregistering=""
__gpy_handle_sigalrm() {
    [[ -z "$__gpy_reregistering" ]] || return 0
    __gpy_reregistering=1
    __gpy_reregister_with_agent &>/dev/null
    __gpy_reregistering=""
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
    trap '__gpy_handle_sigusr1' SIGUSR1
    trap '__gpy_handle_sigusr2' SIGUSR2
    trap '__gpy_handle_sigalrm' SIGALRM

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
