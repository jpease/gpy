# zsh/core/renderer.zsh
# Shell-drawn segment renderer: the Zsh twin of `gpy_section_standalone` in
# fish/core/renderer.fish.
#
# Every segment the shell draws itself (the clock without a template, the
# status pill, the hostname/username fallback) goes through
# `__gpy_section_standalone`, so the same theme draws the same opening cap,
# colors, gap space and closing cap here as in Fish (#844). It emits the
# agent's own Zsh dialect: raw SGR sequences, each wrapped in `%{ %}` so ZLE
# counts it as zero-width (#679), and text escaped for `PROMPT` with
# `prompt_subst` and `prompt_percent` on (#677).
#
# Out-var convention (as __gpy_segment_bg): `FN ... OUT_VAR` assigns with
# `printf -v`, so none of the helpers fork a subshell.

# The escape byte, so the SGR sequences below carry the same raw bytes as the
# agent's zsh-prompt output.
typeset -g __GPY_SGR_ESC=$'\033'

# __gpy_sgr_param KIND COLOR OUT_VAR
# KIND is `fg` or `bg`. Writes the SGR parameter that selects COLOR. Named
# colors (and their bright forms), `#RRGGBB` and 0-255 map to the codes the
# agent emits; `transparent`, `normal`, `default`, an empty value, or a name
# this table does not know map to the terminal default (39/49).
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

# __gpy_prompt_escape TEXT OUT_VAR
# Escape TEXT so PROMPT displays it literally with `prompt_subst` and
# `prompt_percent` on: backslash-quote `\`, `$` and backtick, and double `%`
# (the same rules as the agent's zsh-prompt encoder).
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

# __gpy_sgr_pair FG BG OUT_VAR: one SGR sequence setting both colors, wrapped
# in `%{ %}`.
function __gpy_sgr_pair() {
    local __f __b
    __gpy_sgr_param fg "$1" __f
    __gpy_sgr_param bg "$2" __b
    printf -v "$3" '%s' "%{${__GPY_SGR_ESC}[${__f};${__b}m%}"
}

# __gpy_section_standalone BG FG CONTENT IS_LAST IS_FIRST
# Prints one capped segment: opening delimiter, CONTENT on BG/FG, a
# same-background gap space before a non-last closing delimiter, closing
# delimiter, reset. CONTENT is PROMPT source (use __gpy_prompt_escape for
# data). IS_LAST / IS_FIRST are "true" or "" (#613); `last` is accepted for
# IS_LAST like fish. The delimiter and color rules are those of fish's
# gpy_section_standalone: `match_bg` / `match_text` borrow the segment's own
# colors, `transparent` and empty mean the terminal default.
function __gpy_section_standalone() {
    local bg=${1:-} fg=${2:-} content=${3-} is_last=${4:-} is_first=${5:-}
    # Delimiter fallbacks match fish/core/renderer.fish (a theme export
    # overrides them). `${var-default}` has no colon on purpose: the default
    # theme exports an empty first delimiter.
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

# __gpy_status_icon ok|fail OUT_VAR
# The exit-status icon for the current theme, unescaped. The fallbacks mirror
# fish/core/init.fish: nerd glyphs, or ASCII words when the theme asks for
# ASCII icons. `${var-default}` (no colon) as `set -q`: an icon the theme
# exports as an empty string stays empty.
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

# __gpy_prompt_tail EXIT_CODE OUT_VAR
# The prompt symbol drawn by the shell itself: the root prompt, and the
# fallback when the agent rendered no character. The same output as the
# non-agent branch of fish_prompt (#844):
#   - the exit-status indicator (green/red icon) unless GPY_SHOW_STATUS=0;
#   - then `__icon_root_prompt` in `__root_prompt_color` for root, otherwise
#     `__icon_prompt` in `__prompt_color`, each followed by a space.
# Root never asks the agent for a character, so the caller does not either.
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
