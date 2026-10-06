# zsh/core/signals.zsh

# Config hot-reload: re-read the theme and drop render caches. Runs from
# TRAPURG when the agent left a `<pid>.reload` flag.
function __gpy_reload_config() {
    # Reload theme from agent
    __gpy_load_theme &>/dev/null
    # Clear character/directory render caches (#343): a theme change can alter
    # the rendered templates, so a stale cache entry must not survive a reload
    # even if the input tuple happens to repeat.
    __gpy_char_cache_key=""
    __gpy_char_cache_val=""
    __gpy_dir_cache_key=""
    __gpy_dir_cache_val=""
    # Drop any pending relay files so an in-flight miss from just before the
    # reload isn't replayed. Guard on non-empty paths: mktemp may have failed
    # at init, in which case the paths are empty and there's nothing to remove
    # (an unguarded `rm -f ""` is harmless but the guard keeps intent clear).
    [[ -n "$__gpy_char_cache_relay_path" ]] && rm -f "$__gpy_char_cache_relay_path" 2>/dev/null
    [[ -n "$__gpy_dir_cache_relay_path" ]] && rm -f "$__gpy_dir_cache_relay_path" 2>/dev/null
    return 0
}

# SIGURG doorbell (#674): the agent's only signal to a shell. SIGURG is
# ignored by default, so a shell that has not defined TRAPURG yet (for
# example one that just ran `exec zsh` under the same, still-registered PID)
# is not killed by it. The message travels in flag files next to this shell's
# tracking entry (__gpy_shell_flag_base, set by
# __gpy_track_shell_for_agent_recovery):
#   <pid>.reregister  the agent (re)started: forget the registration and
#                     register again (#638). Re-entrancy guard: a second nudge
#                     while one is being handled is dropped; the next
#                     precmd's own registration retry covers it.
#   <pid>.reload      config/theme changed: reload.
# Each flag is removed before acting on it. Then repaint.
#
# PROMPT is a string rendered once per precmd (init.zsh), so `zle
# reset-prompt` on its own only redraws the bytes already there: a git change
# while the user sat at an idle prompt could not appear until the next Enter
# (#637). Re-render first, with the exit code of the last command so the
# status colouring does not change, then repaint. Only while zle is active:
# outside the line editor there is nothing on screen to repaint and the next
# precmd renders anyway.
__gpy_reregistering=""
function TRAPURG() {
    local base=${__gpy_shell_flag_base:-}
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
    if zle; then
        PROMPT=$(__gpy_render_prompt "${__gpy_last_exit_code:-0}")
        __gpy_load_cache_relay "$__gpy_char_cache_relay_path" __gpy_char_cache_key __gpy_char_cache_val
        __gpy_load_cache_relay "$__gpy_dir_cache_relay_path" __gpy_dir_cache_key __gpy_dir_cache_val
        zle reset-prompt
    fi
    return 0
}

# Cleanup on shell exit
function __gpy_zshexit() {
    # Unregister client
    local request="{\"op\":\"unregister\",\"pid\":$$}"
    __gpy_send_json "$request" &>/dev/null
    __gpy_untrack_shell_for_agent_recovery
    # Remove the private per-session render-cache dir (#343) and everything in
    # it. `rm -rf` on the whole dir (not just the two relay files) also sweeps
    # any stray content and the dir itself. Guarded/idempotent: empty when
    # mktemp failed at init, and a no-op if already gone.
    [[ -n "$__gpy_cache_dir" ]] && rm -rf "$__gpy_cache_dir" 2>/dev/null
}

# Register exit hook if not already present
if (( ! ${zshexit_functions[(I)__gpy_zshexit]:-0} )); then
    zshexit_functions+=(__gpy_zshexit)
fi
