# zsh/core/signals.zsh

# Live Update Handler
#
# PROMPT is a string rendered once per precmd (init.zsh), so `zle
# reset-prompt` on its own only redraws the bytes already there: a git change
# while the user sat at an idle prompt could not appear until the next Enter
# (#637). Re-render first, with the exit code of the last command so the
# status colouring does not change, then repaint. Only while zle is active:
# outside the line editor there is nothing on screen to repaint and the next
# precmd renders anyway.
function TRAPUSR1() {
    if zle; then
        PROMPT=$(__gpy_render_prompt "${__gpy_last_exit_code:-0}")
        __gpy_load_cache_relay "$__gpy_char_cache_relay_path" __gpy_char_cache_key __gpy_char_cache_val
        __gpy_load_cache_relay "$__gpy_dir_cache_relay_path" __gpy_dir_cache_key __gpy_dir_cache_val
        zle reset-prompt
    fi
}

# Agent (re)start nudge (#638): the agent sends SIGALRM to every PID recorded
# under <runtime root>/shells/ when it comes up, and again to tracked shells
# that stay unregistered. Re-register and repaint. Re-entrancy guard: a
# second nudge while one is being handled is dropped; the next precmd's own
# registration retry covers it.
__gpy_reregistering=""
function TRAPALRM() {
    [[ -z "$__gpy_reregistering" ]] || return 0
    __gpy_reregistering=1
    __gpy_reregister_with_agent &>/dev/null
    __gpy_reregistering=""
    if zle; then
        PROMPT=$(__gpy_render_prompt "${__gpy_last_exit_code:-0}")
        zle reset-prompt
    fi
}

# Config Reload Handler
function TRAPUSR2() {
    # Triggered by agent for theme/config reload
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
    # Repaint prompt
    zle && zle reset-prompt
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
if [[ ${zshexit_functions[(I)__gpy_zshexit]} -eq 0 ]]; then
    zshexit_functions+=(__gpy_zshexit)
fi
