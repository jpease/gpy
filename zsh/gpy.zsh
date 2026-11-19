# SPDX-License-Identifier: GPL-3.0-or-later
# gpy.zsh - GPY for Zsh.
#
# `gpy prompt zsh` draws PROMPT and the transient line, runs the script
# segments and keeps this shell's lifecycle (agent start, registration,
# supervisor). This file only times commands, counts them, hands the binary
# what it cannot see (exit status, duration, command count, PID), repaints
# when the agent rings, and swaps in the transient line on Enter.

autoload -Uz add-zsh-hook add-zle-hook-widget
zmodload zsh/datetime
# The binary escapes the prompt for prompt_subst and prompt_percent.
setopt prompt_subst prompt_percent

typeset -g __gpy_zsh_root=${${(%):-%x}:A:h}
typeset -gi __gpy_cmd_start=0 __gpy_cmd_count=0
# The last prompt's status and duration, for a repaint outside precmd.
typeset -g __gpy_status=0 __gpy_duration=""
typeset -g __gpy_transient="" __gpy_in_precmd="" __gpy_repaint_pending=""

# The `gpy` command into variable $1: PATH first, then ~/.local/bin.
function __gpy_bin_into() {
    local b=${commands[gpy]-}
    [[ -z $b && -n ${HOME-} && -x $HOME/.local/bin/gpy ]] && b=$HOME/.local/bin/gpy
    [[ -n $b ]] || return 1
    typeset -g "$1=$b"
}

function __gpy_preexec() {
    emulate -L zsh
    __gpy_cmd_start=$((EPOCHREALTIME * 1000))
    (( __gpy_cmd_count++ ))
}

# Set PROMPT and __gpy_transient from one `gpy prompt zsh`. Under
# prompt_bang a data `!` must be written `!!`; the caller restored the
# user's prompt_bang before emulating zsh.
function __gpy_render() {
    local bin out bang=0
    [[ -o prompt_bang ]] && bang=1
    # Ctrl-C ends `gpy prompt` (and the scripts it runs), not the caller.
    setopt local_options local_traps
    TRAPINT() { return 0 }
    local -a args=(prompt zsh --status $__gpy_status --pid $$ --cmd $__gpy_cmd_count
        --shell-version "$ZSH_VERSION")
    [[ -n $__gpy_duration ]] && args+=(--duration-ms $__gpy_duration)
    # stderr stays the terminal: a tripped script segment says so there.
    if __gpy_bin_into bin && out=$("$bin" "${args[@]}") && [[ $out == *$'\x1f'* ]]; then
        if (( bang )); then
            PROMPT=${${out%%$'\x1f'*}//\!/!!}
            __gpy_transient=${${out#*$'\x1f'}//\!/!!}
        else
            PROMPT=${out%%$'\x1f'*}
            __gpy_transient=${out#*$'\x1f'}
        fi
    else
        PROMPT='❯ '
        __gpy_transient=""
    fi
}

function __gpy_precmd() {
    local __gpy_pc_status=$? __gpy_pc_bang=0
    # Keep the user's prompt_bang; every other option is zsh's own here.
    [[ -o prompt_bang ]] && __gpy_pc_bang=1
    emulate -L zsh
    (( __gpy_pc_bang )) && setopt prompt_bang
    # A ring during this function is repainted by zle-line-init.
    __gpy_in_precmd=1
    # Drop the dynamic-completion caches (completions/_gpy-dynamic).
    unset __gpy_complete_cache_theme __gpy_complete_cache_palette __gpy_complete_cache_segment
    __gpy_status=$__gpy_pc_status
    __gpy_duration=""
    if (( __gpy_cmd_start > 0 )); then
        __gpy_duration=$(( EPOCHREALTIME * 1000 - __gpy_cmd_start ))
        __gpy_duration=${__gpy_duration%.*}
        __gpy_cmd_start=0
    fi
    __gpy_render
    __gpy_in_precmd=""
}

# The agent rings SIGURG after it wrote a `.reload` or `.reregister` flag,
# or when a late git status landed: draw the prompt again. A handler
# defined before GPY loaded (a TRAPURG function or a `trap … URG` string)
# is chained; zsh has one slot per signal. A string trap is read from a
# pipeline because `$(trap)` runs in a subshell that has no traps. SIGURG is
# ignored by default, so a shell without this trap is never killed.
if (( ${+functions[TRAPURG]} )); then
    if [[ ${functions[TRAPURG]} != *__gpy_on_doorbell* ]]; then
        functions[__gpy_prev_TRAPURG]=${functions[TRAPURG]}
    fi
elif (( ! ${+functions[__gpy_prev_TRAPURG]} && ! ${+__gpy_prev_urg_trap} )); then
    trap | while IFS= read -r __gpy_l; do
        if [[ $__gpy_l == "trap -- "*" URG" ]]; then
            typeset -g __gpy_prev_urg_trap=${(Q)${${__gpy_l#trap -- }% URG}}
        fi
    done
    unset __gpy_l
fi

# The user's own handler runs outside GPY's option emulation, so this body
# must hold under any options the interrupted code runs with: braced
# expansions only.
function TRAPURG() {
    __gpy_on_doorbell
    if (( ${+functions[__gpy_prev_TRAPURG]} )); then
        __gpy_prev_TRAPURG "$@"
        return $?
    elif [[ -n ${__gpy_prev_urg_trap:-} ]]; then
        eval "$__gpy_prev_urg_trap"
        return $?
    fi
    return 0
}

# A ring during precmd (zle inactive, the prompt half drawn) only marks a
# repaint pending, for zle-line-init.
function __gpy_on_doorbell() {
    local __gpy_db_bang=0
    [[ -o prompt_bang ]] && __gpy_db_bang=1
    emulate -L zsh
    (( __gpy_db_bang )) && setopt prompt_bang
    if [[ -n $__gpy_in_precmd ]] && ! zle; then
        __gpy_repaint_pending=1
        return 0
    fi
    if zle; then
        __gpy_render
        zle reset-prompt
    fi
    return 0
}

# zle-line-init: the first moment after precmd that a repaint can reach the
# screen.
function __gpy_line_init() {
    local __gpy_li_bang=0
    [[ -o prompt_bang ]] && __gpy_li_bang=1
    emulate -L zsh
    (( __gpy_li_bang )) && setopt prompt_bang
    # precmd has ended, even one Ctrl-C cut short.
    __gpy_in_precmd=""
    [[ -n $__gpy_repaint_pending ]] || return 0
    __gpy_repaint_pending=""
    __gpy_render
    zle reset-prompt
}

# On Enter, leave the transient line in scrollback. CONTEXT=start skips
# vared and continuation lines.
function __gpy_line_finish() {
    [[ $CONTEXT == start && -n $__gpy_transient ]] || return 0
    PROMPT=$__gpy_transient
    zle .reset-prompt
    zle -R
}

# Unregister on exit so a restarted agent never signals a recycled PID.
function __gpy_zshexit() {
    emulate -L zsh
    local bin
    __gpy_bin_into bin && "$bin" prompt zsh --exit --pid $$ &>/dev/null
}

add-zsh-hook preexec __gpy_preexec
add-zsh-hook precmd __gpy_precmd
zle -N __gpy_line_init
zle -N __gpy_line_finish
add-zle-hook-widget zle-line-init __gpy_line_init
add-zle-hook-widget zle-line-finish __gpy_line_finish
# Declared first: under `nounset` an unset array cannot be subscripted.
typeset -ga zshexit_functions
(( ${zshexit_functions[(I)__gpy_zshexit]} )) || zshexit_functions+=(__gpy_zshexit)

# `_gpy` (generated, optional) and `_gpy-dynamic` (dynamic values) are
# best-effort. Our rc block runs after the user's compinit, so fpath alone
# would never take effect; autoload + compdef explicitly instead.
if [[ -d $__gpy_zsh_root/completions ]]; then
    # -U: re-sourcing must not grow fpath.
    typeset -gU fpath
    fpath=("$__gpy_zsh_root/completions" $fpath)
    if [[ -f $__gpy_zsh_root/completions/_gpy-dynamic ]]; then
        autoload -Uz _gpy-dynamic
        [[ -f $__gpy_zsh_root/completions/_gpy ]] && autoload -Uz _gpy
        whence compdef >/dev/null 2>&1 && compdef _gpy-dynamic gpy
    fi
fi
unset __gpy_zsh_root
