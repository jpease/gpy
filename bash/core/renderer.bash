# Shell-drawn segments (Bash twin of fish's gpy_section_standalone). SGR is
# wrapped in `\[ \]` so readline counts it zero-width. `FN ... OUT_VAR`
# helpers assign with `printf -v` to avoid forking.

__GPY_SGR_ESC=$'\033'

# Unknown names, `transparent`, `normal` and empty map to the default (39/49).
__gpy_sgr_param() {
    local __kind="$1" __color="$2" __base=30 __code=""
    [[ "$__kind" == bg ]] && __base=40
    case "$__color" in
        black) __code=$((__base + 0)) ;;
        red) __code=$((__base + 1)) ;;
        green) __code=$((__base + 2)) ;;
        yellow) __code=$((__base + 3)) ;;
        blue) __code=$((__base + 4)) ;;
        magenta | purple) __code=$((__base + 5)) ;;
        cyan) __code=$((__base + 6)) ;;
        white) __code=$((__base + 7)) ;;
        brblack | bright_black | bright-black | gray | grey) __code=$((__base + 60)) ;;
        brred | bright_red | bright-red) __code=$((__base + 61)) ;;
        brgreen | bright_green | bright-green) __code=$((__base + 62)) ;;
        bryellow | bright_yellow | bright-yellow) __code=$((__base + 63)) ;;
        brblue | bright_blue | bright-blue) __code=$((__base + 64)) ;;
        brmagenta | bright_magenta | bright-purple) __code=$((__base + 65)) ;;
        brcyan | bright_cyan | bright-cyan) __code=$((__base + 66)) ;;
        brwhite | bright_white | bright-white) __code=$((__base + 67)) ;;
        \#[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F])
            __code="$((__base + 8));2;$((16#${__color:1:2}));$((16#${__color:3:2}));$((16#${__color:5:2}))"
            ;;
        [0-9] | [0-9][0-9] | [0-9][0-9][0-9])
            if ((10#$__color <= 255)); then
                __code="$((__base + 8));5;$((10#$__color))"
            else
                __code=$((__base + 9))
            fi
            ;;
        *) __code=$((__base + 9)) ;;
    esac
    printf -v "$3" '%s' "$__code"
}

# Escape TEXT for PS1 under promptvars: `\` needs four backslashes, `$` and
# backtick two. Bare `\$` is avoided because it renders `#` for root.
# Bash 3.2 substitution costs the string length per match, so the work runs
# over 64-byte slices in the C locale (the three characters are ASCII and never
# inside a multibyte character). The operands are quoted variables: Bash 3.2
# keeps literal quotes in a double-quoted `${x//p/"r"}`, and an unquoted
# replacement is subject to Bash 5.2's patsub_replacement.
# shellcheck disable=SC1003 # the backslash replacements are literal on purpose
__gpy_prompt_escape() {
    if [[ "$1" != *[\\\$\`]* ]]; then
        printf -v "$2" '%s' "$1"
        return 0
    fi
    local LC_ALL=C
    local __text="$1" __out="" __slice __i __len __bs='\' __dl='$' __bt='`' \
        __rbs='\\\\' __rdl='\\$' __rbt='\\`'
    __len=${#__text}
    for ((__i = 0; __i < __len; __i += 64)); do
        __slice=${__text:__i:64}
        __slice=${__slice//"$__bs"/"$__rbs"}
        __slice=${__slice//"$__dl"/"$__rdl"}
        __slice=${__slice//"$__bt"/"$__rbt"}
        __out+=$__slice
    done
    printf -v "$2" '%s' "$__out"
}

# One SGR sequence setting both colors.
__gpy_sgr_pair() {
    local __f __b
    __gpy_sgr_param fg "$1" __f
    __gpy_sgr_param bg "$2" __b
    printf -v "$3" '%s' "\\[${__GPY_SGR_ESC}[${__f};${__b}m\\]"
}

# One capped segment. CONTENT is PS1 source (escape data first).
__gpy_section_standalone() {
    local bg="${1:-}" fg="${2:-}" content="${3-}" is_last="${4:-}" is_first="${5:-}"
    # `${var-default}` without colon: the default theme exports an empty first delimiter.
    local start_delim start_fg start_bg end_delim end_fg end_bg

    if [[ "$is_first" == true ]]; then
        start_delim="${__segment_delim_first-|}"
        start_fg="${__prompt_open_color-}"
        start_bg="${__prompt_open_bg-}"
    else
        start_delim="${__segment_delim_start-[}"
        start_fg="${__segment_delimiter_color-}"
        start_bg="${__segment_delimiter_bg-}"
    fi
    local last=0
    [[ "$is_last" == true || "$is_last" == last ]] && last=1
    if ((last)); then
        end_delim="${__segment_delim_last-)}"
        end_fg="${__prompt_close_color-}"
        end_bg="${__prompt_close_bg-}"
    else
        end_delim="${__segment_delim_end-]}"
        end_fg="${__segment_delimiter_color-}"
        end_bg="${__segment_delimiter_bg-}"
    fi

    case "$start_fg" in match_bg) start_fg="$bg" ;; match_text) start_fg="$fg" ;; esac
    case "$start_bg" in match_bg) start_bg="$bg" ;; match_text) start_bg="$fg" ;; esac
    case "$end_fg" in match_bg) end_fg="$bg" ;; match_text) end_fg="$fg" ;; esac
    case "$end_bg" in match_bg) end_bg="$bg" ;; match_text) end_bg="$fg" ;; esac

    local sgr_start sgr_body sgr_gap sgr_end start_text end_text
    __gpy_sgr_pair "$start_fg" "$start_bg" sgr_start
    __gpy_sgr_pair "$fg" "$bg" sgr_body
    __gpy_sgr_pair normal "$bg" sgr_gap
    __gpy_sgr_pair "$end_fg" "$end_bg" sgr_end
    __gpy_prompt_escape "$start_delim" start_text
    __gpy_prompt_escape "$end_delim" end_text

    local out="${sgr_start}${start_text}${sgr_body}${content}"
    ((last)) || out+="${sgr_gap} "
    out+="${sgr_end}${end_text}\\[${__GPY_SGR_ESC}[0m\\]"
    printf '%s\n' "$out"
}

# `${var-default}` without colon: a theme-exported empty icon stays empty.
__gpy_status_icon() {
    local __ok="✔" __fail="✖"
    if [[ "${__prompt_icons:-nerd}" != "nerd" ]]; then
        __ok="ok"
        __fail="err"
    fi
    if [[ "$1" == ok ]]; then
        printf -v "$2" '%s' "${__icon_status_ok-$__ok}"
    else
        printf -v "$2" '%s' "${__icon_status_fail-$__fail}"
    fi
}

# Status icon plus prompt symbol, drawn by the shell for root (which never
# asks the agent) and when the agent rendered no character.
__gpy_prompt_tail() {
    local __exit="${1:-0}" __out="" __sgr __reset __icon __color
    __reset="\\[${__GPY_SGR_ESC}[0m\\]"

    if [[ "${GPY_SHOW_STATUS:-1}" == "1" ]]; then
        if ((__exit == 0)); then
            __gpy_status_icon ok __icon
            __color=green
        else
            __gpy_status_icon fail __icon
            __color=red
        fi
        __gpy_prompt_escape "$__icon" __icon
        __gpy_sgr_pair "$__color" normal __sgr
        __out+="${__sgr}${__icon} ${__reset}"
    fi

    if [[ "${__gpy_is_root:-0}" == "1" ]]; then
        __icon="${__icon_root_prompt-!❯!}"
        __color="${__root_prompt_color:-red}"
    else
        __icon="${__icon_prompt-❯}"
        __color="${__prompt_color:-green}"
    fi
    __gpy_prompt_escape "$__icon" __icon
    __gpy_sgr_pair "$__color" normal __sgr
    __out+="${__sgr}${__icon} ${__reset}"
    printf -v "$2" '%s' "$__out"
}
