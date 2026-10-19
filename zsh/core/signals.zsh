# zsh/core/signals.zsh

# Drop the agent-rendered character and directory memos. Their keys hold no
# config, so a theme or config change (a reload, or an agent restarted with
# another config) must not leave them serving the old render. Pending relays
# go too, so a miss rendered before the change is not replayed.
function __gpy_clear_render_memos() {
    __gpy_char_cache_key=""
    __gpy_char_cache_val=""
    __gpy_dir_cache_key=""
    __gpy_dir_cache_val=""
    [[ -n "$__gpy_char_cache_relay_path" ]] && rm -f "$__gpy_char_cache_relay_path" 2>/dev/null
    [[ -n "$__gpy_dir_cache_relay_path" ]] && rm -f "$__gpy_dir_cache_relay_path" 2>/dev/null
    return 0
}

# Runs from TRAPURG when the agent left a `<pid>.reload` flag.
function __gpy_reload_config() {
    __gpy_load_theme &>/dev/null
    __gpy_clear_render_memos
}

# SIGURG is ignored by default, so a shell without TRAPURG yet (e.g. after
# `exec zsh` under the same registered PID) is not killed. The message is in
# flag files beside the tracking entry: <pid>.reregister, <pid>.reload.
# A nudge during reregistration is left for the next pass or precmd.
__gpy_reregistering=""

# Act on the agent's flags: re-register, then reload. Returns 0 when one was
# present. Also called from precmd, so a lost ring is picked up at the next
# prompt, as in Bash.
function __gpy_consume_shell_flags() {
    local base=${__gpy_shell_flag_base:-} found=1
    [[ -n "$base" ]] || return 1
    if [[ -e "$base.reregister" && -z "$__gpy_reregistering" ]]; then
        rm -f "$base.reregister" 2>/dev/null
        __gpy_reregistering=1
        __gpy_reregister_with_agent &>/dev/null
        __gpy_reregistering=""
        found=0
    fi
    if [[ -e "$base.reload" ]]; then
        rm -f "$base.reload" 2>/dev/null
        __gpy_reload_config
        found=0
    fi
    return $found
}

# Zsh drops a SIGURG that arrives while this trap is still running (an agent
# start rings `.reload`, then `.reregister`, back to back). The agent writes a
# flag before it rings, so after acting and repainting the trap looks again,
# and goes round while a flag arrived meanwhile (at most three passes).
# `zle reset-prompt` alone only redraws the old PROMPT string, so re-render
# first with the last exit code to keep status colouring.
function TRAPURG() {
    local base=${__gpy_shell_flag_base:-} pass
    for pass in 1 2 3; do
        __gpy_consume_shell_flags
        if zle; then
            __gpy_update_prompts "${__gpy_last_exit_code:-0}"
            __gpy_load_cache_relay "$__gpy_char_cache_relay_path" __gpy_char_cache_key __gpy_char_cache_val
            __gpy_load_cache_relay "$__gpy_dir_cache_relay_path" __gpy_dir_cache_key __gpy_dir_cache_val
            zle reset-prompt
        fi
        [[ -n "$base" && ( -e "$base.reregister" || -e "$base.reload" ) ]] || break
    done
    return 0
}

function __gpy_zshexit() {
    local request="{\"op\":\"unregister\",\"pid\":$$}"
    __gpy_send_json "$request" &>/dev/null
    __gpy_untrack_shell_for_agent_recovery
    # Empty when mktemp failed at init.
    [[ -n "$__gpy_cache_dir" ]] && rm -rf "$__gpy_cache_dir" 2>/dev/null
}

if (( ! ${zshexit_functions[(I)__gpy_zshexit]:-0} )); then
    zshexit_functions+=(__gpy_zshexit)
fi
