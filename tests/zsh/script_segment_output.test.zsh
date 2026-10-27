#!/usr/bin/env zsh
# tests/zsh/script_segment_output.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Script (G) segment output in Zsh (docs/dev/adr/adr-0011-addendum-script-
# segment-security.md, "Output: sanitized once"):
#   - script_segment_hostile_pwd: a newline and ESC in $PWD reach the script
#     literally, and the text is shown without them, the same on a run and on
#     a cache hit; folders named `*`, `?` and `[a-z]*` never hit another
#     folder's entry;
#   - script_segment_c1_locale: every vector of the step 4 table under
#     LC_ALL=C, C.UTF-8 and en_US.UTF-8 (C0/DEL stripped, C1 hides);
#   - script_segment_output_escaped: `$(...)`, backticks and `%` show
#     literally, and the stored value is the escaped one;
#   - the 4096-byte cap, line 2 as a colour variant, a non-zero exit.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
source "$ROOT/tests/lib/script_segment_fixture.zsh"

hex() { print -rn -- "$1" | od -An -tx1 | tr -s ' \n' ' ' | sed 's/^ //;s/ $//'; }

# --- hostile $PWD ------------------------------------------------------------
g_segment where 'printf "%s" "$GPY_PWD"'
__gpy_script_where_inputs=(pwd)
__gpy_script_where_scope=folder
g_enable where
# Line 1 is the text, so the ESC goes on line 1 and the newline after it.
hostile=$G_SANDBOX/work/$'x\e]0;PWNED\a\ny'
mkdir -p "$hostile"
cd "$hostile" || exit 1
g_command
expected="$G_SANDBOX/work/x]0;PWNED"
check "newline and ESC in \$PWD: the script got \$PWD, the text drops the C0 bytes" \
    "$expected" "$(g_text where)"
check "the stored text holds no control byte" "" "$(print -rn -- "${__gpy_g_text[where]}" | LC_ALL=C tr -d '[:print:]\200-\377')"
runs=$(g_runs)
g_command
check "a cache hit in the same folder does not run the script" "$runs" "$(g_runs)"
check "a cache hit shows the same text" "$expected" "$(g_text where)"
check "the prompt draws it with no ESC" "" "$(g_shown | LC_ALL=C tr -cd '\033')"

for d in '*' '?' '[a-z]*' a; do mkdir -p "$G_SANDBOX/work/$d"; done
cd "$G_SANDBOX/work/a" && g_command
cd "$G_SANDBOX/work/*" && g_command
check "a folder named * gets its own entry" "$G_SANDBOX/work/*" "$(g_text where)"
cd "$G_SANDBOX/work/?" && g_command
check "a folder named ? gets its own entry" "$G_SANDBOX/work/?" "$(g_text where)"
cd "$G_SANDBOX/work/[a-z]*" && g_command
check "a folder named [a-z]* gets its own entry" "$G_SANDBOX/work/[a-z]*" "$(g_text where)"
cd "$G_SANDBOX/work/a" && g_command
check "back in a: its own text, not the glob folder's" "$G_SANDBOX/work/a" "$(g_text where)"
cd "$G_SANDBOX/work" || exit 1

# --- C1 and C0 vectors, three locales ---------------------------------------
# Bytes in, bytes shown ("" = hidden), from the addendum's step 4 table.
typeset -a vectors=(
    '61 c2 9b 62' ''
    '61 c2 85 1b 07 7f 09 62' ''
    'c3 a9 e2 86 92 c3 bc c2 a0' 'c3 a9 e2 86 92 c3 bc c2 a0'
    'c2 c2 9b 9b' ''
    'c2 c2 c2 9b 9b 9b' ''
    'c2 1b 9b' ''
    '78 c2 01 9b 79' ''
    'e2 c2 9b 82' ''
    '61 9b 62' '61 9b 62'
    '61 85 62' '61 85 62'
    '61 1b 5b 33 31 6d 62' '61 5b 33 31 6d 62'
)
g_segment vec 'printf "$GPY_OPT_bytes"'
__gpy_script_vec_ttl=0
lc_all_before=${LC_ALL-unset}
for loc in C C.UTF-8 en_US.UTF-8; do
    for (( i = 1; i <= ${#vectors}; i += 2 )); do
        oct=""
        for b in ${=vectors[i]}; do oct+="\\$(( [##8] 16#$b ))"; done
        __gpy_script_vec_opts=("GPY_OPT_bytes=$oct")
        g_enable vec
        LC_ALL=$loc g_command
        got=$(LC_ALL=C hex "${__gpy_g_text[vec]}")
        check "LC_ALL=$loc: [${vectors[i]}] shows [${vectors[i+1]}]" "${vectors[i+1]}" "$got"
    done
done
LC_ALL=C g_command
check "the shell's own LC_ALL is left as it was" "$lc_all_before" "${LC_ALL-unset}"

# --- prompt expansion in output ---------------------------------------------
g_segment esc 'printf "%s" "$GPY_OPT_text"'
for text in '$(echo GPY_EXPANDED)' 'tick`echo GPY_TICK`' '100%_done' '%F{red}x%f' 'a\b' '${HOME}'; do
    __gpy_script_esc_opts=("GPY_OPT_text=$text")
    g_enable esc
    g_command
    check "output [$text] shows literally" "$text" "$(g_text esc)"
done
__gpy_script_esc_opts=('GPY_OPT_text=$x%')
g_enable esc
g_command
check "the stored value is escaped once (render-ready)" '\$x%%' "${__gpy_g_text[esc]}"
check "the drawn prompt shows it literally" true \
    "$([[ $(g_shown) == *'$x%'* ]] && echo true || echo false)"

# --- cap, variant, exit ------------------------------------------------------
g_segment big 'i=0; while [ $i -lt 4097 ]; do printf a; i=$((i + 1)); done'
g_enable big
g_command
check "4097 bytes of output hide the segment" "" "$(g_text big)"
g_segment fit 'i=0; while [ $i -lt 2048 ]; do printf "é"; i=$((i + 1)); done'
g_enable fit
g_command
check "4096 bytes (2048 x é) are shown" 4096 "$(LC_ALL=C; print -r -- ${#__gpy_g_text[fit]})"
g_segment over 'i=0; while [ $i -lt 2049 ]; do printf "é"; i=$((i + 1)); done'
g_enable over
g_command
check "4098 bytes of multibyte output hide the segment" "" "$(g_text over)"

g_segment var 'printf "hello\n%s\nignored\n" "$GPY_OPT_v"'
typeset -g __gpy_segment_var_variant_warn_bg_color=red
for v in warn 'Bad' 'a[$(touch '"$G_SANDBOX"'/VARF)]' ''; do
    __gpy_script_var_opts=("GPY_OPT_v=$v")
    g_enable var
    g_command
    check "line 1 is the text (variant [$v])" "hello" "$(g_text var)"
    bg="" fg=""
    __gpy_g_colors var bg fg
    if [[ $v == warn ]]; then
        check "a valid styled variant picks its background" red "$bg"
    else
        check "variant [$v] falls back to the segment colour" blue "$bg"
    fi
done
check "a hostile variant created no file" false "$([[ -e $G_SANDBOX/VARF ]] && echo true || echo false)"

# --- last-pill close cap, stale styles on reload ------------------------------
typeset -g __prompt_close_bg=magenta __segment_delim_last=E
out=$(__gpy_g_render var true "" false)
check "a last plugin pill draws its close cap on the default background" true \
    "$([[ $out == *'49m%}E'* && $out != *'45m%}E'* ]] && echo true || echo false)"
out=$(__gpy_section_standalone blue white x true)
check "a built-in last segment keeps the prompt_close background" true \
    "$([[ $out == *'45m%}E'* ]] && echo true || echo false)"
unset __prompt_close_bg __segment_delim_last
typeset -g __color_var_bg=blue __icon_var=K __icon_var_warn=W
__gpy_g_clear_styles
check "a theme reload clears the plugin segment's old style variables" "" \
    "${__gpy_segment_var_variant_warn_bg_color+x}${__color_var_bg+x}${__icon_var+x}${__icon_var_warn+x}"

g_segment fails 'printf shown; exit 3'
g_enable fails
g_command
check "a non-zero exit hides the segment" "" "$(g_text fails)"

test_finish "script segment output"
