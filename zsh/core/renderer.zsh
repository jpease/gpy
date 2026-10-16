# zsh/core/renderer.zsh
# Zsh twin of fish's gpy_section_standalone for shell-drawn segments. SGR is
# wrapped in `%{ %}` so ZLE counts it zero-width. `FN ... OUT_VAR` helpers
# assign with `printf -v` so none fork.

typeset -g __GPY_SGR_ESC=$'\033'

# Unknown/empty/`transparent` colors map to the terminal default (39/49).
function __gpy_sgr_param() {
    local __kind=$1 __color=$2 __code=""
    local -i __base=30
    [[ $__kind == bg ]] && __base=40
    case $__color in
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
            __code="$((__base + 8));2;$((16#${__color[2,3]}));$((16#${__color[4,5]}));$((16#${__color[6,7]}))"
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

# Same escaping rules as the agent's zsh-prompt encoder.
function __gpy_prompt_escape() {
    local __text=$1 __out="" __ch
    local -i __i
    if [[ $__text != *[\\\$\`%]* ]]; then
        printf -v "$2" '%s' "$__text"
        return 0
    fi
    for ((__i = 1; __i <= ${#__text}; __i++)); do
        __ch=${__text[__i]}
        case $__ch in
            '\') __out+='\\' ;;
            '$') __out+='\$' ;;
            '`') __out+='\`' ;;
            '%') __out+='%%' ;;
            *) __out+=$__ch ;;
        esac
    done
    printf -v "$2" '%s' "$__out"
}

function __gpy_sgr_pair() {
    local __f __b
    __gpy_sgr_param fg "$1" __f
    __gpy_sgr_param bg "$2" __b
    printf -v "$3" '%s' "%{${__GPY_SGR_ESC}[${__f};${__b}m%}"
}

# CONTENT is PROMPT source; escape data with __gpy_prompt_escape first.
function __gpy_section_standalone() {
    local bg=${1:-} fg=${2:-} content=${3-} is_last=${4:-} is_first=${5:-}
    # `${var-default}` without colon: the default theme exports an empty first delimiter.
    local start_delim start_fg start_bg end_delim end_fg end_bg

    if [[ $is_first == true ]]; then
        start_delim=${__segment_delim_first-|}
        start_fg=${__prompt_open_color-}
        start_bg=${__prompt_open_bg-}
    else
        start_delim=${__segment_delim_start-[}
        start_fg=${__segment_delimiter_color-}
        start_bg=${__segment_delimiter_bg-}
    fi
    local -i last=0
    [[ $is_last == true || $is_last == last ]] && last=1
    if ((last)); then
        end_delim=${__segment_delim_last-)}
        end_fg=${__prompt_close_color-}
        end_bg=${__prompt_close_bg-}
    else
        end_delim=${__segment_delim_end-]}
        end_fg=${__segment_delimiter_color-}
        end_bg=${__segment_delimiter_bg-}
    fi

    case $start_fg in match_bg) start_fg=$bg ;; match_text) start_fg=$fg ;; esac
    case $start_bg in match_bg) start_bg=$bg ;; match_text) start_bg=$fg ;; esac
    case $end_fg in match_bg) end_fg=$bg ;; match_text) end_fg=$fg ;; esac
    case $end_bg in match_bg) end_bg=$bg ;; match_text) end_bg=$fg ;; esac

    local sgr_start sgr_body sgr_gap sgr_end start_text end_text
    __gpy_sgr_pair "$start_fg" "$start_bg" sgr_start
    __gpy_sgr_pair "$fg" "$bg" sgr_body
    __gpy_sgr_pair normal "$bg" sgr_gap
    __gpy_sgr_pair "$end_fg" "$end_bg" sgr_end
    __gpy_prompt_escape "$start_delim" start_text
    __gpy_prompt_escape "$end_delim" end_text

    local out="${sgr_start}${start_text}${sgr_body}${content}"
    ((last)) || out+="${sgr_gap} "
    out+="${sgr_end}${end_text}%{${__GPY_SGR_ESC}[0m%}"
    print -r -- "$out"
}

# `${var-default}` without colon: an icon exported as empty stays empty.
function __gpy_status_icon() {
    local __ok="✔" __fail="✖"
    if [[ ${__prompt_icons:-nerd} != nerd ]]; then
        __ok="ok"
        __fail="err"
    fi
    if [[ $1 == ok ]]; then
        printf -v "$2" '%s' "${__icon_status_ok-$__ok}"
    else
        printf -v "$2" '%s' "${__icon_status_fail-$__fail}"
    fi
}

# Shell-drawn prompt symbol: root, or the fallback when the agent rendered none.
function __gpy_prompt_tail() {
    local __exit=${1:-0} __out="" __sgr __reset __icon __color
    __reset="%{${__GPY_SGR_ESC}[0m%}"

    if [[ ${GPY_SHOW_STATUS:-1} == 1 ]]; then
        if (( __exit == 0 )); then
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

    if [[ ${__gpy_is_root:-0} == 1 ]]; then
        __icon=${__icon_root_prompt-!❯!}
        __color=${__root_prompt_color:-red}
    else
        __icon=${__icon_prompt-❯}
        __color=${__prompt_color:-green}
    fi
    __gpy_prompt_escape "$__icon" __icon
    __gpy_sgr_pair "$__color" normal __sgr
    __out+="${__sgr}${__icon} ${__reset}"
    printf -v "$2" '%s' "$__out"
}
