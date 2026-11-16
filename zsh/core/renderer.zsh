# zsh/core/renderer.zsh
# Zsh twin of fish's gpy_section_standalone for shell-drawn segments. SGR is
# wrapped in `%{ %}` so ZLE counts it zero-width. `FN ... OUT_VAR` helpers
# assign with `printf -v` so none fork.

typeset -g __GPY_SGR_ESC=$'\033'

# Colour depth, mirroring the agent's `ColorDepth` (style_encoder.rs):
# `truecolor|256|16|none`. GPY_COLOR_DEPTH wins (case-insensitive, `24bit` =
# truecolor, unknown ignored); else 256 on Apple Terminal / TERM=linux.
typeset -g __GPY_COLOR_DEPTH
function __gpy_color_depth_detect() {
    local __v=${GPY_COLOR_DEPTH:-}
    __v=${__v##[[:space:]]#}
    __v=${__v%%[[:space:]]#}
    case ${(L)__v} in
        truecolor | 24bit) __GPY_COLOR_DEPTH=truecolor; return 0 ;;
        256) __GPY_COLOR_DEPTH=256; return 0 ;;
        16) __GPY_COLOR_DEPTH=16; return 0 ;;
        none) __GPY_COLOR_DEPTH=none; return 0 ;;
    esac
    if [[ ${TERM_PROGRAM:-} == Apple_Terminal || ${TERM:-} == linux ]]; then
        __GPY_COLOR_DEPTH=256
    else
        __GPY_COLOR_DEPTH=truecolor
    fi
}
__gpy_color_depth_detect

# xterm 6x6x6 cube level (0-5) nearest to channel $1 into $2.
function __gpy_cube_level() {
    local -i __c=$1 __l
    if ((__c < 48)); then __l=0
    elif ((__c < 115)); then __l=1
    elif ((__c < 155)); then __l=2
    elif ((__c < 195)); then __l=3
    elif ((__c < 235)); then __l=4
    else __l=5; fi
    printf -v "$2" '%s' "$__l"
}

# Nearest xterm 256 index to r g b ($1-$3) into $4.
function __gpy_rgb_to_256() {
    local -i __r=$1 __g=$2 __b=$3 __n __lr __lg __lb
    if ((__r == __g && __g == __b)); then
        if ((__r <= 7)); then
            __n=16
        elif ((__r >= 248)); then
            __n=231
        else
            __n=$(((__r - 3) / 10))
            ((__n > 23)) && __n=23
            __n=$((232 + __n))
        fi
    else
        __gpy_cube_level $__r __lr
        __gpy_cube_level $__g __lg
        __gpy_cube_level $__b __lb
        __n=$((16 + 36 * __lr + 6 * __lg + __lb))
    fi
    printf -v "$4" '%s' "$__n"
}

typeset -ga __GPY_ANSI16_RGB=(0,0,0 205,0,0 0,205,0 205,205,0 0,0,238 205,0,205 0,205,205 229,229,229
    127,127,127 255,0,0 0,255,0 255,255,0 92,92,255 255,0,255 0,255,255 255,255,255)

# Foreground code (30-37, 90-97) of the ANSI colour nearest r g b ($1-$3) into $4.
function __gpy_rgb_to_16() {
    local -i __r=$1 __g=$2 __b=$3 __i __best=0 __bd=-1 __d
    local -a __p
    for ((__i = 1; __i <= 16; __i++)); do
        __p=("${(@s:,:)__GPY_ANSI16_RGB[__i]}")
        __d=$(((__r - __p[1]) * (__r - __p[1]) + (__g - __p[2]) * (__g - __p[2]) + (__b - __p[3]) * (__b - __p[3])))
        if ((__bd < 0 || __d < __bd)); then
            __bd=$__d
            __best=$((__i - 1))
        fi
    done
    if ((__best < 8)); then __d=$((30 + __best)); else __d=$((90 + __best - 8)); fi
    printf -v "$4" '%s' "$__d"
}

# Foreground 16-colour code nearest 256-colour index $1 into $2.
function __gpy_index_to_16() {
    local -i __x=$1 __c __g __s
    local -a __v
    if ((__x < 8)); then
        printf -v "$2" '%s' "$((30 + __x))"
    elif ((__x < 16)); then
        printf -v "$2" '%s' "$((90 + __x - 8))"
    elif ((__x < 232)); then
        __c=$((__x - 16))
        for __s in $((__c / 36)) $((__c / 6 % 6)) $((__c % 6)); do
            if ((__s == 0)); then __v+=(0); else __v+=($((55 + 40 * __s))); fi
        done
        __gpy_rgb_to_16 "${__v[1]}" "${__v[2]}" "${__v[3]}" "$2"
    else
        __g=$((8 + 10 * (__x - 232)))
        __gpy_rgb_to_16 $__g $__g $__g "$2"
    fi
}

# Unknown/empty/`transparent` colors map to the terminal default (39/49). At
# depth `none` every parameter is empty (no colour codes at all).
function __gpy_sgr_param() {
    local __kind=$1 __color=$2 __code=""
    if [[ $__GPY_COLOR_DEPTH == none ]]; then
        printf -v "$3" '%s' ""
        return 0
    fi
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
            local -i __cr=$((16#${__color[2,3]})) __cg=$((16#${__color[4,5]})) __cb=$((16#${__color[6,7]}))
            case $__GPY_COLOR_DEPTH in
                256)
                    __gpy_rgb_to_256 $__cr $__cg $__cb __code
                    __code="$((__base + 8));5;$__code"
                    ;;
                16)
                    __gpy_rgb_to_16 $__cr $__cg $__cb __code
                    [[ $__kind == bg ]] && __code=$((__code + 10))
                    ;;
                *) __code="$((__base + 8));2;$__cr;$__cg;$__cb" ;;
            esac
            ;;
        [0-9] | [0-9][0-9] | [0-9][0-9][0-9])
            if ((10#$__color <= 255)); then
                if [[ $__GPY_COLOR_DEPTH == 16 ]]; then
                    __gpy_index_to_16 "$((10#$__color))" __code
                    [[ $__kind == bg ]] && __code=$((__code + 10))
                else
                    __code="$((__base + 8));5;$((10#$__color))"
                fi
            else
                __code=$((__base + 9))
            fi
            ;;
        *) __code=$((__base + 9)) ;;
    esac
    printf -v "$3" '%s' "$__code"
}

# Same escaping rules as the agent's zsh-prompt encoder. A whole-string
# `${x//p/r}` costs the string length per match, so the text is split into
# bytes (C locale; the four characters are ASCII and never inside a multibyte
# character) and each element is substituted, which stays linear.
function __gpy_prompt_escape() {
    if [[ $1 != *[\\\$\`%]* ]]; then
        printf -v "$2" '%s' "$1"
        return 0
    fi
    local LC_ALL=C
    local -a __bytes
    __bytes=("${(@s::)1}")
    __bytes=("${(@)__bytes[@]//\\/\\\\}")
    __bytes=("${(@)__bytes[@]//\$/\\\$}")
    __bytes=("${(@)__bytes[@]//\`/\\\`}")
    __bytes=("${(@)__bytes[@]//\%/%%}")
    printf -v "$2" '%s' "${(j::)__bytes[@]}"
}

function __gpy_sgr_pair() {
    local __f __b
    __gpy_sgr_param fg "$1" __f
    __gpy_sgr_param bg "$2" __b
    if [[ -z $__f && -z $__b ]]; then
        printf -v "$3" '%s' ""
        return 0
    fi
    printf -v "$3" '%s' "%{${__GPY_SGR_ESC}[${__f};${__b}m%}"
}

# CONTENT is PROMPT source; escape data with __gpy_prompt_escape first.
# Optional $6 names per-segment caps that replace the theme's delimiters at
# every position: `o` takes the opening cap from $7 (icon), $8 (fg), $9 (bg),
# `c` the closing cap from $10, $11, $12 (script segments' `open`/`close`).
# `d` (plugin pills) draws a last segment's theme close cap on the terminal
# default background instead of the prompt_close bg. Prints the section;
# __gpy_section_standalone_into OUT_VAR ARGS... assigns it without a fork
# (OUT_VAR must be a `__gpy_` name: the locals there are not prefixed).
function __gpy_section_standalone() {
    local __gpy_ss_out
    __gpy_section_standalone_into __gpy_ss_out "$@"
    print -r -- "$__gpy_ss_out"
}

function __gpy_section_standalone_into() {
    local __gpy_ss_var=$1
    shift
    local bg=${1:-} fg=${2:-} content=${3-} is_last=${4:-} is_first=${5:-} caps=${6-}
    # `${var-default}` without colon: the default theme exports an empty first delimiter.
    local start_delim start_fg start_bg end_delim end_fg end_bg

    if [[ $caps == *o* ]]; then
        start_delim=${7-} start_fg=${8-} start_bg=${9-}
    elif [[ $is_first == true ]]; then
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
    if [[ $caps == *c* ]]; then
        end_delim=${10-} end_fg=${11-} end_bg=${12-}
    elif ((last)); then
        end_delim=${__segment_delim_last-)}
        end_fg=${__prompt_close_color-}
        if [[ $caps == *d* ]]; then end_bg=""; else end_bg=${__prompt_close_bg-}; fi
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
    printf -v "$__gpy_ss_var" '%s' "$out"
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
    local __exit=${1:-0} __out="" __sgr __reset __icon __color __show_status
    __reset="%{${__GPY_SGR_ESC}[0m%}"

    __gpy_flag_value "${GPY_SHOW_STATUS-}" on __show_status
    if [[ -z ${__gpy_hide_status_tail-} && $__show_status == on ]]; then
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
