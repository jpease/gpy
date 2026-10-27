# gpy.zsh - Entry point for Zsh integration

0=${(%):-%N}
GPY_ROOT=${0%/*}

# Check every core file before sourcing any, so a partial install prints one
# diagnostic instead of half-loading the prompt on every shell.
for __gpy_core_file in constants renderer ipc signals supervisor script init; do
    if [[ ! -f "$GPY_ROOT/core/$__gpy_core_file.zsh" ]]; then
        print -r -- "gpy[init]: core files not found at $GPY_ROOT - GPY disabled" >&2
        unset __gpy_core_file
        return 0
    fi
done
unset __gpy_core_file

source "$GPY_ROOT/core/constants.zsh"
source "$GPY_ROOT/core/renderer.zsh"
source "$GPY_ROOT/core/ipc.zsh"
source "$GPY_ROOT/core/signals.zsh"
source "$GPY_ROOT/core/supervisor.zsh"
source "$GPY_ROOT/core/script.zsh"

source "$GPY_ROOT/core/init.zsh"

# Load theme from agent (overrides defaults in constants.zsh); script.zsh
# cleared the `__gpy_script_*` names it reads first.
__gpy_load_theme &>/dev/null
__gpy_g_reload

# Start only after the theme export set the flags; [agent.supervisor]
# enabled = false disables restarts, not this start.
__gpy_agent_enabled && __gpy_start_agent

for segment in "$GPY_ROOT"/segments/*.zsh; do
    source "$segment"
done

# `_gpy` (generated, optional) and `_gpy-dynamic` (dynamic values) are
# best-effort. Our rc block runs after the user's compinit, so fpath alone
# would never take effect; autoload + compdef explicitly instead.
if [[ -d "$GPY_ROOT/completions" ]]; then
    # -U: re-sourcing must not grow fpath.
    typeset -gU fpath
    fpath=("$GPY_ROOT/completions" $fpath)

    if [[ -f "$GPY_ROOT/completions/_gpy-dynamic" ]]; then
        autoload -Uz _gpy-dynamic

        if [[ -f "$GPY_ROOT/completions/_gpy" ]]; then
            autoload -Uz _gpy
        fi

        if whence compdef >/dev/null 2>&1; then
            compdef _gpy-dynamic gpy
        fi
    fi
fi
