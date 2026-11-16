# Shell-drawn segments (Bash twin of fish's gpy_section_standalone). SGR is
# wrapped in `\[ \]` so readline counts it zero-width. `FN ... OUT_VAR`
# helpers assign with `printf -v` to avoid forking.

__GPY_SGR_ESC=$'\033'

# Colour depth, mirroring the agent's `ColorDepth`: GPY_COLOR_DEPTH
# (truecolor|24bit|256|16|none, case-insensitive, surrounding blanks ignored)
# wins; otherwise 256 on Apple_Terminal / TERM=linux, else truecolor. Unknown
# values are ignored. Result: truecolor, 256, 16 or none.
__gpy_color_depth() {
    local __v="${GPY_COLOR_DEPTH-}"
    __v=${__v#"${__v%%[![:space:]]*}"}
    __v=${__v%"${__v##*[![:space:]]}"}
    case "$__v" in
        [Tt][Rr][Uu][Ee][Cc][Oo][Ll][Oo][Rr] | 24[Bb][Ii][Tt]) printf -v "$1" '%s' truecolor ;;
        256) printf -v "$1" '%s' 256 ;;
        16) printf -v "$1" '%s' 16 ;;
        [Nn][Oo][Nn][Ee]) printf -v "$1" '%s' none ;;
        *)
            if [[ "${TERM_PROGRAM-}" == Apple_Terminal || "${TERM-}" == linux ]]; then
                printf -v "$1" '%s' 256
            else
                printf -v "$1" '%s' truecolor
            fi
            ;;
    esac
}

# The xterm 6x6x6 cube level (0-5) nearest to channel $1, into $2.
__gpy_cube_level() {
    local __c=$1 __l
    if ((__c < 48)); then __l=0
    elif ((__c < 115)); then __l=1
    elif ((__c < 155)); then __l=2
    elif ((__c < 195)); then __l=3
    elif ((__c < 235)); then __l=4
    else __l=5; fi
    printf -v "$2" '%s' "$__l"
}

# The 16 ANSI colours as xterm draws them (r g b triples, SGR order).
__GPY_ANSI16_RGB=(0 0 0 205 0 0 0 205 0 205 205 0 0 0 238 205 0 205 0 205 205 229 229 229
    127 127 127 255 0 0 0 255 0 255 255 0 92 92 255 255 0 255 0 255 255 255 255 255)

# Foreground code (30-37, 90-97) of the ANSI colour nearest to R=$1 G=$2 B=$3, into $4.
__gpy_rgb_to_16() {
    local __i __best=0 __bestd=-1 __d __dr __dg __db
    for ((__i = 0; __i < 16; __i++)); do
        __dr=$(($1 - __GPY_ANSI16_RGB[__i * 3]))
        __dg=$(($2 - __GPY_ANSI16_RGB[__i * 3 + 1]))
        __db=$(($3 - __GPY_ANSI16_RGB[__i * 3 + 2]))
        __d=$((__dr * __dr + __dg * __dg + __db * __db))
        if ((__bestd < 0 || __d < __bestd)); then __bestd=$__d __best=$__i; fi
    done
    if ((__best < 8)); then
        printf -v "$4" '%s' $((30 + __best))
    else
        printf -v "$4" '%s' $((90 + __best - 8))
    fi
}

# Foreground code (30-37, 90-97) nearest to 256-colour index $1, into $2.
__gpy_index_to_16() {
    local __xn=$1 __xc __xr __xg __xb __xgy
    if ((__xn < 8)); then
        printf -v "$2" '%s' $((30 + __xn))
    elif ((__xn < 16)); then
        printf -v "$2" '%s' $((90 + __xn - 8))
    elif ((__xn < 232)); then
        __xc=$((__xn - 16))
        __xr=$((__xc / 36)) __xg=$((__xc / 6 % 6)) __xb=$((__xc % 6))
        ((__xr == 0)) || __xr=$((55 + 40 * __xr))
        ((__xg == 0)) || __xg=$((55 + 40 * __xg))
        ((__xb == 0)) || __xb=$((55 + 40 * __xb))
        __gpy_rgb_to_16 "$__xr" "$__xg" "$__xb" "$2"
    else
        __xgy=$((8 + 10 * (__xn - 232)))
        __gpy_rgb_to_16 "$__xgy" "$__xgy" "$__xgy" "$2"
    fi
}

# Nearest xterm 256 index to R=$1 G=$2 B=$3, into $4.
__gpy_rgb_to_256() {
    local __lr __lg __lb
    if (($1 == $2 && $2 == $3)); then
        if (($1 < 8)); then printf -v "$4" '%s' 16
        elif (($1 >= 248)); then printf -v "$4" '%s' 231
        else
            __lr=$((($1 - 3) / 10))
            ((__lr > 23)) && __lr=23
            printf -v "$4" '%s' $((232 + __lr))
        fi
        return 0
    fi
    __gpy_cube_level "$1" __lr
    __gpy_cube_level "$2" __lg
    __gpy_cube_level "$3" __lb
    printf -v "$4" '%s' $((16 + 36 * __lr + 6 * __lg + __lb))
}

# GPY_COLOR_DEPTH=none yields empty parameters (no colour codes).
# Unknown names, `transparent`, `normal` and empty map to the default (39/49).
__gpy_sgr_param() {
    local __kind="$1" __color="$2" __base=30 __code="" __spd __spn __spr __spg __spb
    [[ "$__kind" == bg ]] && __base=40
    __gpy_color_depth __spd
    if [[ "$__spd" == none ]]; then
        printf -v "$3" '%s' ""
        return 0
    fi
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
            __spr=$((16#${__color:1:2})) __spg=$((16#${__color:3:2})) __spb=$((16#${__color:5:2}))
            case "$__spd" in
                truecolor) __code="$((__base + 8));2;${__spr};${__spg};${__spb}" ;;
                256)
                    __gpy_rgb_to_256 "$__spr" "$__spg" "$__spb" __spn
                    __code="$((__base + 8));5;${__spn}"
                    ;;
                *)
                    __gpy_rgb_to_16 "$__spr" "$__spg" "$__spb" __spn
                    __code=$((__spn + __base - 30))
                    ;;
            esac
            ;;
        [0-9] | [0-9][0-9] | [0-9][0-9][0-9])
            if ((10#$__color <= 255)); then
                if [[ "$__spd" == 16 ]]; then
                    __gpy_index_to_16 "$((10#$__color))" __spn
                    __code=$((__spn + __base - 30))
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
    if [[ -z "$__f" && -z "$__b" ]]; then
        printf -v "$3" '%s' ""
        return 0
    fi
    printf -v "$3" '%s' "\\[${__GPY_SGR_ESC}[${__f};${__b}m\\]"
}

# One capped segment. CONTENT is PS1 source (escape data first). Optional
# CAPS ($6) holds `o` and/or `c`: the segment's own open cap ($7 text, $8 fg,
# $9 bg) or close cap ($10-$12) replaces the global delimiter on that side.
# `d` (plugin pills) draws a last segment's theme close cap on the terminal
# default background instead of the prompt_close bg.
__gpy_section_standalone() {
    local bg="${1:-}" fg="${2:-}" content="${3-}" is_last="${4:-}" is_first="${5:-}" caps="${6-}"
    # `${var-default}` without colon: the default theme exports an empty first delimiter.
    local start_delim start_fg start_bg end_delim end_fg end_bg

    if [[ "$caps" == *o* ]]; then
        start_delim="${7-}" start_fg="${8-}" start_bg="${9-}"
    elif [[ "$is_first" == true ]]; then
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
    if [[ "$caps" == *c* ]]; then
        end_delim="${10-}" end_fg="${11-}" end_bg="${12-}"
    elif ((last)); then
        end_delim="${__segment_delim_last-)}"
        end_fg="${__prompt_close_color-}"
        if [[ "$caps" == *d* ]]; then end_bg=""; else end_bg="${__prompt_close_bg-}"; fi
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
    local __exit="${1:-0}" __out="" __sgr __reset __icon __color __show_status
    __reset="\\[${__GPY_SGR_ESC}[0m\\]"

    __gpy_flag_value "${GPY_SHOW_STATUS-}" on __show_status
    if [[ "$__show_status" == on ]]; then
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
