# gpy.zsh - Entry point for Zsh integration

0=${(%):-%N}
__gpy_zsh_root=${0%/*}

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
source "$__gpy_zsh_root/core/signals.zsh"
source "$__gpy_zsh_root/core/supervisor.zsh"
source "$__gpy_zsh_root/core/script.zsh"

source "$__gpy_zsh_root/core/init.zsh"

# Load theme from agent (overrides defaults in constants.zsh); script.zsh
# cleared the `__gpy_script_*` names it reads first.
__gpy_load_theme &>/dev/null
__gpy_g_reload

# Start only after the theme export set the flags; [agent.supervisor]
# enabled = false disables restarts, not this start.
__gpy_agent_enabled && __gpy_start_agent

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
