# Runs when the agent left a `<pid>.reload` flag.
__gpy_reload_config() {
    __gpy_load_theme &>/dev/null
    # A theme change can alter rendered templates even for a repeated input tuple.
    __gpy_char_cache_key=""
    __gpy_char_cache_val=""
    __gpy_dir_cache_key=""
    __gpy_dir_cache_val=""
}

# The agent rings SIGURG and leaves `<pid>.reregister` or `<pid>.reload` next
# to __gpy_shell_flag_base. Bash installs no SIGURG trap: readline cannot
# repaint an idle prompt, bash 5 nested trap renders without bound, and the
# default disposition (ignore) never kills the shell or interrupts `wait`.
# __gpy_precmd polls the flags before each render instead.
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

# Unregister on exit so a restarted agent never signals a recycled PID.
# A pre-existing EXIT trap is chained, never clobbered.
: "${__gpy_prev_exit_trap=}"

# Prints the body of `trap -p` output; eval undoes its quoting.
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

__gpy_setup_signals() {
    # Drop a URG trap left by an older gpy in this shell.
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

__gpy_setup_signals
