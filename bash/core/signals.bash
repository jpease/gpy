# bash/core/signals.bash
# Signal handlers for live updates and config reload

# Config hot-reload: re-read the theme and drop render caches. Runs from
# __gpy_consume_shell_flags when the agent left a `<pid>.reload` flag.
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

# SIGURG doorbell (#674): the agent rings SIGURG and leaves the message in
# flag files next to this shell's tracking entry (__gpy_shell_flag_base, set
# by __gpy_track_shell_for_agent_recovery):
#   <pid>.reregister  the agent (re)started: forget the registration and
#                     register again (#638). Re-entrancy guard: a second nudge
#                     while one is being handled is dropped; the next
#                     prompt's own retry covers it.
#   <pid>.reload      config/theme changed: reload.
# Bash installs no SIGURG trap (#678); the signal keeps its default
# disposition, ignore, so it never kills a shell (an `exec bash` included)
# and never interrupts `wait`. Readline cannot repaint an idle prompt, so a
# trap could only re-render a PS1 nobody sees: bash 5 nested those renders
# without bound while doorbells kept coming, and bash 3.2 ran them at the
# idle prompt. Instead __gpy_precmd calls this before every render, so a
# reload or re-registration takes effect in the prompt about to be drawn.
# Each flag is removed before acting on it. No flag costs two `[[ -e ]]`.
__gpy_reregistering=""
__gpy_consume_shell_flags() {
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
    return 0
}

# Shell exit: tell the agent this PID is gone and drop the recovery-nudge
# entry, so a restarted agent does not signal a PID that may have been
# recycled (the agent verifies liveness too; this is the polite half). Zsh
# and Fish already do this from their exit hooks. Any EXIT trap the user or
# a framework installed first is chained, never clobbered (#320's DEBUG
# treatment, applied to EXIT).
: "${__gpy_prev_exit_trap=}"

# __gpy_trap_body <trap -p output>: print the handler body ("" if none).
# Lets the shell undo trap -p's quoting (including '\'' for embedded quotes).
__gpy_trap_body() {
    [[ -n "$1" ]] || return 0
    eval "set -- $1"
    printf '%s' "${3-}"
}
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
    # Drop the URG trap an earlier gpy installed in this shell (re-sourced
    # after an upgrade); its handler no longer exists (#678).
    if [[ "$(trap -p URG)" == *"__gpy_handle_doorbell"* ]]; then
        trap - URG
    fi

    local existing_exit
    existing_exit="$(trap -p EXIT)"
    if [[ -n "$existing_exit" && "$existing_exit" != *"__gpy_handle_exit"* ]]; then
        __gpy_prev_exit_trap="$(__gpy_trap_body "$existing_exit")"
    fi
    trap '__gpy_handle_exit' EXIT
}

# Initialize signals
__gpy_setup_signals
