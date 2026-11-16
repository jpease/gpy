# gpy.zsh - Entry point for Zsh integration

# `%x` names this file whether it is sourced from a shell, a function or
# another file (unlike $0 under NO_FUNCTION_ARGZERO). `:a` makes it absolute
# lexically, so a symlinked tree keeps its siblings (core/script.zsh finds
# bash/core/g_runner.bash beside the link); when that directory has no core/
# (the entry file alone is a symlink), `:A` follows the link to the real tree.
__gpy_zsh_root=${${(%):-%x}:a:h}
[[ -f "$__gpy_zsh_root/core/init.zsh" ]] || __gpy_zsh_root=${${(%):-%x}:A:h}

# Check every core file before sourcing any, so a partial install prints one
# diagnostic instead of half-loading the prompt on every shell.
for __gpy_core_file in constants renderer ipc signals supervisor script init; do
    if [[ ! -f "$__gpy_zsh_root/core/$__gpy_core_file.zsh" ]]; then
        print -r -- "gpy[init]: core files not found at $__gpy_zsh_root - GPY disabled" >&2
        unset __gpy_core_file __gpy_zsh_root
        return 0
    fi
done
unset __gpy_core_file

source "$__gpy_zsh_root/core/constants.zsh"
source "$__gpy_zsh_root/core/renderer.zsh"
source "$__gpy_zsh_root/core/ipc.zsh"

# No usable XDG_RUNTIME_DIR, XDG_CACHE_HOME or HOME: the agent refuses to run
# and there is no shared /tmp fallback, so GPY stays off for this session
# (no hooks, no agent, no oneshot) and the prompt is left alone.
if ! __gpy_runtime_root __gpy_core_file; then
    if [[ ${GPY_DEBUG-} == 1 ]]; then
        print -r -- "gpy[init]: debug: no runtime directory (set XDG_RUNTIME_DIR, XDG_CACHE_HOME or HOME to an absolute path) - GPY disabled" >&2
    fi
    unset __gpy_core_file __gpy_zsh_root
    return 0
fi
unset __gpy_core_file

source "$__gpy_zsh_root/core/signals.zsh"
source "$__gpy_zsh_root/core/supervisor.zsh"
source "$__gpy_zsh_root/core/script.zsh"

source "$__gpy_zsh_root/core/init.zsh"

# Load theme from agent (overrides defaults in constants.zsh); script.zsh
# cleared the `__gpy_script_*` names it reads first.
__gpy_load_theme &>/dev/null
__gpy_g_reload

# Once per shell, in this shell and with stderr visible.
__gpy_warn_flag_values

# Start only after the theme export set the flags; [agent.supervisor]
# enabled = false disables restarts, not this start.
# The start just made is the first check: the next probe waits one interval.
# One registration attempt now, in this shell (as Bash does): a running agent
# knows the shell before the first render, whose segments run in subshells
# that cannot keep a registration. precmd retries after each render.
if __gpy_agent_enabled; then
    __gpy_start_agent
    __gpy_supervisor_last_check_time=$EPOCHSECONDS
    __gpy_register_with_agent &>/dev/null
fi

for __gpy_seg_file in "$__gpy_zsh_root"/segments/*.zsh; do
    source "$__gpy_seg_file"
done

# `_gpy` (generated, optional) and `_gpy-dynamic` (dynamic values) are
# best-effort. Our rc block runs after the user's compinit, so fpath alone
# would never take effect; autoload + compdef explicitly instead.
if [[ -d "$__gpy_zsh_root/completions" ]]; then
    # -U: re-sourcing must not grow fpath.
    typeset -gU fpath
    fpath=("$__gpy_zsh_root/completions" $fpath)

    if [[ -f "$__gpy_zsh_root/completions/_gpy-dynamic" ]]; then
        autoload -Uz _gpy-dynamic

        if [[ -f "$__gpy_zsh_root/completions/_gpy" ]]; then
            autoload -Uz _gpy
        fi

        if whence compdef >/dev/null 2>&1; then
            compdef _gpy-dynamic gpy
        fi
    fi
fi

# Nothing reads these after loading; keep the shell's variable namespace clean.
unset __gpy_zsh_root __gpy_seg_file
