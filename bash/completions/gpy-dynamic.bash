# SPDX-License-Identifier: GPL-3.0-or-later
#
# Dynamic-value completion glue for the `gpy` CLI: live theme, palette and
# segment names via `gpy __complete <kind>`; every other position delegates to
# the generated `_gpy`, so source this after `completions/gpy.bash`. Design
# notes: docs/dev/completions.md.
#
# Call __gpy_complete_cached as a plain statement (never `$(...)`) and read
# `__gpy_complete_result`.
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

# Fill COMPREPLY with the words in $__gpy_complete_result that match $1, one
# per line, without word-splitting or globbing them (bash 3.2 has no mapfile).
__gpy_complete_reply() {
    local word
    COMPREPLY=()
    while IFS= read -r word; do
        COMPREPLY+=("$word")
    done < <(compgen -W "$__gpy_complete_result" -- "$1")
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
                        __gpy_complete_reply "$cur"
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
                        __gpy_complete_reply "$cur"
                        return 0
                    fi
                    ;;
            esac
            ;;
        enable | disable)
            if [[ "$COMP_CWORD" -eq 2 ]]; then
                __gpy_complete_cached segment
                __gpy_complete_reply "$cur"
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
