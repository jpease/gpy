# shellcheck shell=bash
# SPDX-License-Identifier: GPL-3.0-or-later
# gpy.bash - GPY for Bash 3.2+.
#
# `gpy prompt bash` draws PS1, runs the script segments and keeps this
# shell's lifecycle (agent start, registration, supervisor). This file only
# times commands, counts them and hands the binary what it cannot see: the
# exit status, the duration, the command count and the PID.

# Sourcing it again (after an upgrade) redefines the functions and leaves
# the hooks in place.

__gpy_bash_root="${BASH_SOURCE[0]}"
if [[ $__gpy_bash_root == */* ]]; then
    __gpy_bash_root="${__gpy_bash_root%/*}"
else
    __gpy_bash_root=.
fi

# Duration clock: EPOCHREALTIME on Bash 5+; on 4, `date +%s%N` only when
# date is GNU (BSD date prints a literal `N`), else whole seconds; 3: none.
if ((BASH_VERSINFO[0] >= 5)); then
    __gpy_clock="epochrealtime"
elif ((BASH_VERSINFO[0] == 4)); then
    if [[ "$(date +%s%N 2>/dev/null)" =~ ^[0-9]{12,}$ ]]; then
        __gpy_clock="date"
    else
        __gpy_clock="seconds"
    fi
else
    __gpy_clock="none"
fi

__gpy_cmd_start=""
__gpy_cmd_count=0
__gpy_armed=""
# This shell, as `gpy prompt` tells it from an earlier one at the same PID
# (`exec bash`, a recycled PID): drawn once per load, never inherited.
__gpy_instance="$RANDOM$RANDOM${EPOCHREALTIME-}"

# The `gpy` command into variable $1: PATH first, then ~/.local/bin.
__gpy_bin_into() {
    local __gpy_b
    __gpy_b="$(type -P gpy 2>/dev/null)" ||
        { [[ -n ${HOME-} && -x $HOME/.local/bin/gpy ]] && __gpy_b="$HOME/.local/bin/gpy"; }
    [[ -n $__gpy_b ]] || return 1
    printf -v "$1" '%s' "$__gpy_b"
}

__gpy_now_into() {
    case $__gpy_clock in
        epochrealtime) printf -v "$1" '%s' "$EPOCHREALTIME" ;;
        date) printf -v "$1" '%s' "$(date +%s%N)" ;;
        seconds) printf -v "$1" '%s' "$SECONDS" ;;
    esac
}

# Milliseconds from $1 to $2 (as __gpy_now_into wrote them) into variable $3.
# EPOCHREALTIME's separator is locale-dependent and a leading-zero fraction
# would parse as octal, so every operand is forced with `10#`.
__gpy_elapsed_into() {
    local __gpy_e
    case $__gpy_clock in
        epochrealtime)
            local s_frac="${1##*[.,]}" e_frac="${2##*[.,]}"
            [[ ${#s_frac} -eq 6 && ${#e_frac} -eq 6 ]] || return 1
            __gpy_e=$(((10#${2%%[.,]*} * 1000000 + 10#$e_frac - 10#${1%%[.,]*} * 1000000 - 10#$s_frac) / 1000))
            ;;
        date) __gpy_e=$((($2 - $1) / 1000000)) ;;
        seconds) __gpy_e=$((($2 - $1) * 1000)) ;;
        *) return 1 ;;
    esac
    printf -v "$3" '%s' "$__gpy_e"
}

# Locals are __gpy_pc_*: a script segment's declared env names are read by
# the binary from the environment, never from here.
__gpy_precmd() {
    local __gpy_pc_status=$? __gpy_pc_bin __gpy_pc_end __gpy_pc_ms=""
    # Drop the dynamic-completion caches (completions/gpy-dynamic.bash).
    unset __gpy_complete_cache_theme __gpy_complete_cache_palette __gpy_complete_cache_segment
    if [[ -n $__gpy_cmd_start ]]; then
        __gpy_now_into __gpy_pc_end
        __gpy_elapsed_into "$__gpy_cmd_start" "$__gpy_pc_end" __gpy_pc_ms || __gpy_pc_ms=""
        __gpy_cmd_start=""
    fi
    if __gpy_bin_into __gpy_pc_bin; then
        local -a __gpy_pc_args=(prompt bash --status "$__gpy_pc_status" --pid "$$"
            --cmd "$__gpy_cmd_count" --instance "$__gpy_instance" --shell-version "$BASH_VERSION")
        [[ -n $__gpy_pc_ms ]] && __gpy_pc_args+=(--duration-ms "$__gpy_pc_ms")
        # stderr stays the terminal: a tripped script segment says so there.
        local __gpy_pc_out
        if __gpy_pc_out="$("$__gpy_pc_bin" "${__gpy_pc_args[@]}")" && [[ -n $__gpy_pc_out ]]; then
            # The binary escapes for promptvars; re-assert it every prompt.
            shopt -s promptvars
            PS1="$__gpy_pc_out"
        else
            PS1='❯ '
        fi
    else
        PS1='❯ '
    fi
    __gpy_keep_precmd_first
    __gpy_keep_arm_last
    return "$__gpy_pc_status"
}

# Armed last in PROMPT_COMMAND (the bash-preexec technique), so the first
# DEBUG firing after it is the user's command. Preserves `$?`.
__gpy_arm() {
    local s=$?
    __gpy_armed=1
    return $s
}

# Installed as `__gpy_debug_trap "$_"`: the trap's own command resets `$_`,
# so it must end on the user's `$_` (passed as $1) to leave it unchanged.
__gpy_debug_trap() {
    [[ -n ${COMP_LINE-} ]] && return
    # A `bind -x` key binding (READLINE_POINT is set only while one runs) is
    # not the user's command.
    [[ -n ${READLINE_POINT+x} ]] && return
    if [[ -n $__gpy_armed ]]; then
        __gpy_armed=""
        # The first DEBUG after arming is the user's command or, on an empty
        # Enter or Ctrl-C, the first PROMPT_COMMAND command (up to `;` or the
        # newline the arm hook joins with).
        local pc="${PROMPT_COMMAND-}"
        pc="${pc%%;*}"
        if [[ $BASH_COMMAND != "${pc%%$'\n'*}" ]]; then
            ((__gpy_cmd_count += 1))
            __gpy_now_into __gpy_cmd_start
        fi
    fi
    # `if`, not `&&`, so a missing prior trap does not leak status 1.
    if [[ -n $__gpy_prev_debug_trap ]]; then
        : "${1-}"
        eval "$__gpy_prev_debug_trap"
    fi
}

# Prints the body of `trap -p` output; eval undoes its quoting.
__gpy_trap_body() {
    [[ -n $1 ]] || return 0
    eval "set -- $1"
    printf '%s' "${3-}"
}

# bash hides the DEBUG trap from functions and sourced files (no functrace),
# so a prior DEBUG trap is captured from a PROMPT_COMMAND string, at top
# level, on the first prompt. Passes `$?` through.
: "${__gpy_prev_debug_trap=}"
# shellcheck disable=SC2016
__gpy_debug_oneshot='__gpy_dbg_s=$?; __gpy_dbg_seen="$(trap -p DEBUG)"; __gpy_install_debug_trap "$__gpy_dbg_s"'
__gpy_install_debug_trap() {
    if [[ -n $__gpy_dbg_seen && $__gpy_dbg_seen != *__gpy_debug_trap* ]]; then
        __gpy_prev_debug_trap="$(__gpy_trap_body "$__gpy_dbg_seen")"
    fi
    trap '__gpy_debug_trap "$_"' DEBUG
    __gpy_pc_remove "$__gpy_debug_oneshot; "
    __gpy_pc_remove "$__gpy_debug_oneshot"
    __gpy_keep_arm_last
    return "${1:-0}"
}

# PROMPT_COMMAND is an array of commands on Bash >= 5.1 when the user made it
# one.
__gpy_pc_is_array() {
    ((BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1))) &&
        [[ ${PROMPT_COMMAND@a} == *a* ]]
}

# Remove every occurrence of $1 from PROMPT_COMMAND.
__gpy_pc_remove() {
    if __gpy_pc_is_array; then
        local -a kept=()
        local x
        for x in "${PROMPT_COMMAND[@]}"; do
            x="${x//"$1"/}"
            [[ -n $x ]] && kept+=("$x")
        done
        PROMPT_COMMAND=("${kept[@]}")
    else
        # shellcheck disable=SC2178
        PROMPT_COMMAND="${PROMPT_COMMAND//"$1"/}"
    fi
}

# Keep __gpy_arm the last PROMPT_COMMAND entry, else idle time before the
# prompt is timed. Idempotent and fork-free.
__gpy_keep_arm_last() {
    local hook=__gpy_arm
    if __gpy_pc_is_array; then
        [[ ${PROMPT_COMMAND[-1]} == "$hook" ]] && return 0
        local -a kept=()
        local entry
        for entry in "${PROMPT_COMMAND[@]}"; do
            [[ $entry == "$hook" ]] || kept+=("$entry")
        done
        PROMPT_COMMAND=("${kept[@]}" "$hook")
        return 0
    fi
    local pc="${PROMPT_COMMAND:-}"
    while [[ $pc == *[\;[:space:]] ]]; do pc="${pc%?}"; done
    [[ $pc == "$hook" || $pc == *[\;[:space:]]"$hook" ]] && return 0
    pc="${pc//; $hook/}"
    pc="${pc//$'\n'$hook/}"
    while [[ $pc == *[\;[:space:]] ]]; do pc="${pc%?}"; done
    # Newline, not ';': the user's string may end in '&'.
    # shellcheck disable=SC2178
    PROMPT_COMMAND="${pc:+$pc$'\n'}$hook"
}

# Command string $1 with `__gpy_precmd` moved to its front (after a leading
# DEBUG-trap oneshot, which passes `$?` through), into variable $2. The rest
# keeps its order and separators. Status 1 when $1 holds no `__gpy_precmd`.
__gpy_precmd_first_into() {
    local __gpy_pf_s="$1" __gpy_pf_h=__gpy_precmd __gpy_pf_lead="" __gpy_pf_t __gpy_pf_l __gpy_pf_r
    if [[ $__gpy_pf_s == "$__gpy_debug_oneshot; "* ]]; then
        __gpy_pf_lead="$__gpy_debug_oneshot; "
        __gpy_pf_s="${__gpy_pf_s#"$__gpy_pf_lead"}"
    fi
    case "$__gpy_pf_s" in
        "$__gpy_pf_h" | "$__gpy_pf_h;"* | "$__gpy_pf_h"$'\n'*)
            printf -v "$2" '%s' "$1"
            return 0
            ;;
    esac
    # Padded with `;` so the hook always has a separator on both sides; the
    # left one stays, so `cmd &` before it never becomes `cmd &;`.
    __gpy_pf_t=";$__gpy_pf_s;"
    for __gpy_pf_l in ';' '; ' $'\n'; do
        for __gpy_pf_r in ';' $'\n'; do
            # Unquoted RHS: bash 3.2 keeps the quotes of a quoted replacement.
            __gpy_pf_t=${__gpy_pf_t/"$__gpy_pf_l$__gpy_pf_h$__gpy_pf_r"/"$__gpy_pf_l"}
        done
    done
    [[ $__gpy_pf_t != ";$__gpy_pf_s;" ]] || return 1
    __gpy_pf_t="${__gpy_pf_t#;}"
    while [[ $__gpy_pf_t == *[\;[:space:]] ]]; do __gpy_pf_t="${__gpy_pf_t%?}"; done
    __gpy_pf_t="${__gpy_pf_t#"${__gpy_pf_t%%[![:space:]]*}"}"
    printf -v "$2" '%s' "$__gpy_pf_lead$__gpy_pf_h${__gpy_pf_t:+; $__gpy_pf_t}"
}

# Keep __gpy_precmd the first command of its PROMPT_COMMAND entry: a command
# before it, e.g. a later `PROMPT_COMMAND="history -a; $PROMPT_COMMAND"`,
# would replace the `$?` it reads. bash restores `$?` between array elements,
# so only the hook's own element matters. Fork-free.
__gpy_keep_precmd_first() {
    local fixed i
    if __gpy_pc_is_array; then
        for i in "${!PROMPT_COMMAND[@]}"; do
            __gpy_precmd_first_into "${PROMPT_COMMAND[i]}" fixed || continue
            [[ $fixed == "${PROMPT_COMMAND[i]}" ]] || PROMPT_COMMAND[i]="$fixed"
            return 0
        done
        return 0
    fi
    __gpy_precmd_first_into "${PROMPT_COMMAND:-}" fixed || return 0
    # shellcheck disable=SC2128,SC2178
    [[ $fixed == "${PROMPT_COMMAND:-}" ]] || PROMPT_COMMAND="$fixed"
    return 0
}

# Unregister on exit so a restarted agent never signals a recycled PID. A
# pre-existing EXIT trap is chained, never clobbered.
: "${__gpy_prev_exit_trap=}"
# Return "$1" so the saved trap's eval starts with `$?` = "$1".
__gpy_replay_status() {
    return "$1"
}
__gpy_handle_exit() {
    local status=$? bin
    __gpy_bin_into bin && "$bin" prompt bash --exit --pid "$$" &>/dev/null
    if [[ -n $__gpy_prev_exit_trap ]]; then
        __gpy_replay_status "$status"
        eval "$__gpy_prev_exit_trap"
    fi
}

__gpy_existing_exit="$(trap -p EXIT)"
if [[ -n $__gpy_existing_exit && $__gpy_existing_exit != *__gpy_handle_exit* ]]; then
    __gpy_prev_exit_trap="$(__gpy_trap_body "$__gpy_existing_exit")"
fi
trap '__gpy_handle_exit' EXIT
unset __gpy_existing_exit

if [[ ! ${PROMPT_COMMAND:-} =~ __gpy_precmd ]]; then
    # Bash >= 5.1 array form: the hook is prepended to element 0.
    # shellcheck disable=SC2128,SC2178
    PROMPT_COMMAND="__gpy_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
fi
# Once: the oneshot removes itself when it has installed the trap.
# shellcheck disable=SC2128
if [[ ${PROMPT_COMMAND:-} != *"$__gpy_debug_oneshot"* && "$(trap -p DEBUG)" != *__gpy_debug_trap* ]]; then
    # shellcheck disable=SC2178
    PROMPT_COMMAND="$__gpy_debug_oneshot; $PROMPT_COMMAND"
fi
__gpy_keep_arm_last

# Bash honours only the last `complete -F`, and the dynamic file must win (it
# delegates to `_gpy`), so the generated structural completion loads first.
# shellcheck source=/dev/null
[[ -f $__gpy_bash_root/completions/gpy.bash ]] && source "$__gpy_bash_root/completions/gpy.bash"
# shellcheck source=/dev/null
[[ -f $__gpy_bash_root/completions/gpy-dynamic.bash ]] && source "$__gpy_bash_root/completions/gpy-dynamic.bash"
unset __gpy_bash_root
