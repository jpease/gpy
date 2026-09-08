# SPDX-License-Identifier: GPL-3.0-or-later
#
# Dynamic-value completion glue for the `gpy` CLI.
#
# `gpy completions bash` (clap_complete) is regenerated at install time into
# `completions/gpy.bash` and covers the STRUCTURAL surface: it defines a
# `_gpy` function (subcommand/flag dispatch driven entirely by
# `COMP_WORDS`/`COMP_CWORD`) and registers it with
# `complete -F _gpy -o nosort -o bashdefault -o default gpy`. It has no way
# to know the live set of themes, palettes, or segments installed on this
# machine -- those come from the hidden `gpy __complete <kind>` command,
# which prints plain newline-delimited names (#328).
#
# This file supplies exactly those dynamic value lists. Unlike gpy.bash it is
# hand-authored and checked into the repo, so it ships and loads unchanged
# regardless of `gpy`'s clap definitions.
#
# Coexistence with the generated file: bash allows only ONE `complete -F`
# registration to win per command (unlike fish, which merges `complete -c`
# rules from every sourced file). So `_gpy_dynamic` below re-implements
# nothing structural -- for any position that isn't one of the six dynamic
# value slots, it DELEGATES to the generated `_gpy` function directly. This
# file's `complete -F _gpy_dynamic gpy` registration therefore MUST be
# sourced AFTER the generated structural completions (`completions/gpy.bash`)
# so it is the one that ends up registered; if `_gpy` isn't defined yet (or
# the structural file failed to install), delegation degrades to an empty
# COMPREPLY instead of erroring.
#
# Bash invokes the registered completion function as:
#   funcname <command-name> <current-word> <previous-word>
# with COMP_WORDS/COMP_CWORD already set as globals describing the full
# command line. The six dynamic positions this file recognizes:
#   gpy theme use <TAB>      / gpy theme validate <TAB>    -> `gpy __complete theme`
#   gpy palette use <TAB>    / gpy palette validate <TAB>  -> `gpy __complete palette`
#   gpy enable <TAB>         / gpy disable <TAB>           -> `gpy __complete segment`
#
# Per-session caching (#346): forking `gpy __complete <kind>` on every single
# TAB press is wasted work once the answer is already known for this shell
# session, so `__gpy_complete_cached` memoizes each kind's result in a
# session-global variable (`__gpy_complete_cache_<kind>`) the first time it's
# asked for and just replays it after that -- warm reads never fork `gpy`.
# Bash 3.2 has no associative arrays, so this uses one plain variable per
# kind plus indirect expansion (`${!name}`, supported since bash 2.x) rather
# than a `declare -A` cache keyed by kind.
#
# Only a NON-EMPTY result is cached: if `gpy __complete <kind>` fails or
# prints nothing (e.g. `gpy` missing from PATH), the cache variable is left
# unset so the next TAB retries instead of permanently caching the failure.
#
# Accepted staleness: a theme/palette/plugin installed mid-session will not
# show up in completions until a new shell is started (matches the general
# shell-session-cache tradeoff -- refresh by opening a new shell).
#
# IMPORTANT: call this as a plain statement, never via `$(...)`. Command
# substitution always forks a subshell, and `printf -v "$cache_var"` inside
# a subshell can't write back to the interactive shell's variables -- the
# cache would silently never stick. Callers read the result back from
# `__gpy_complete_result` (set here, in the caller's own shell) instead.
__gpy_complete_cached() {
    local kind="$1"
    local cache_var="__gpy_complete_cache_$kind"
    local cache_val="${!cache_var}"

    if [[ -z "$cache_val" ]]; then
        cache_val="$(gpy __complete "$kind" 2>/dev/null)"
        if [[ -z "$cache_val" ]]; then
            __gpy_complete_result=""
            return 1
        fi
        printf -v "$cache_var" '%s' "$cache_val"
    fi

    __gpy_complete_result="$cache_val"
}

_gpy_dynamic() {
    local cur
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi

    COMPREPLY=()

    local sub1="${COMP_WORDS[1]:-}"
    local sub2="${COMP_WORDS[2]:-}"

    case "$sub1" in
        theme)
            case "$sub2" in
                use | validate)
                    if [[ "$COMP_CWORD" -eq 3 ]]; then
                        __gpy_complete_cached theme
                        COMPREPLY=($(compgen -W "$__gpy_complete_result" -- "$cur"))
                        return 0
                    fi
                    ;;
            esac
            ;;
        palette)
            case "$sub2" in
                use | validate)
                    if [[ "$COMP_CWORD" -eq 3 ]]; then
                        __gpy_complete_cached palette
                        COMPREPLY=($(compgen -W "$__gpy_complete_result" -- "$cur"))
                        return 0
                    fi
                    ;;
            esac
            ;;
        enable | disable)
            if [[ "$COMP_CWORD" -eq 2 ]]; then
                __gpy_complete_cached segment
                COMPREPLY=($(compgen -W "$__gpy_complete_result" -- "$cur"))
                return 0
            fi
            ;;
    esac

    # Not a dynamic-value position: delegate to the generated structural
    # completion function so subcommand/flag completion still works. Degrade
    # gracefully (empty COMPREPLY, no error) if it isn't defined -- e.g. the
    # structural completions failed to generate/install.
    if declare -F _gpy >/dev/null 2>&1; then
        _gpy "$@"
    fi
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _gpy_dynamic -o nosort -o bashdefault -o default gpy
else
    complete -F _gpy_dynamic -o bashdefault -o default gpy
fi
