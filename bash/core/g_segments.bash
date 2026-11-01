# shellcheck shell=bash
# g_segments.bash - Script (G) segments in Bash: the once-per-command guard,
# triggers, the in-memory cache, the circuit breaker and the pill (ADR-0011,
# docs/dev/plugins.md "Script segments"). The runner is core/g_runner.bash.
#
# Input: the theme export's __gpy_script_* variables (contract in
# docs/dev/plugins.md "Theme export"). __gpy_g_sync reads them after every
# theme load; only tokens in __gpy_script_segments count, and every name
# built from one is checked before indirection. gpy.bash unsets the
# __gpy_g_* and __gpy_script_* namespaces before loading, so nothing seeded
# by the parent environment is read.
#
# State, per token <tok> (no files; nothing keyed by a PID):
#   __gpy_g_out_<tok>  text drawn this prompt, already escaped ("" hides)
#   __gpy_g_vnt_<tok>  colour variant drawn this prompt
#   __gpy_g_ck_<tok> __gpy_g_ct_<tok> __gpy_g_cv_<tok> __gpy_g_cn_<tok>
#                      cache: full key, time stored, text, variant
#   __gpy_g_rs_<tok> __gpy_g_ss_<tok>
#                      consecutive run / scan strikes (timeout or over cap)
#   __gpy_g_off_<tok>  set once the circuit breaker tripped

# Bash 4.4+ reads the command number (`\#`) to tell a command from an empty
# Enter; older Bash learns it from the DEBUG trap (__gpy_debug_trap sets
# __gpy_g_ran), which init.bash installs only while a script segment is
# listed (__gpy_needs_debug_trap).
__gpy_g_cmdno=""
if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4))); then
    __gpy_g_cmdno=1
fi
__gpy_g_toks=()
__gpy_g_words=()
__gpy_g_wtoks=()
__gpy_g_first=1
__gpy_g_cmd=""
__gpy_g_ran=""
__gpy_g_lastpwd=""
__gpy_g_strikes_max=2
# Explicit sets, not ranges: they hold in every locale.
__gpy_g_idc=ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_
__gpy_g_tokc=abcdefghijklmnopqrstuvwxyz0123456789_

__gpy_g_isname() {
    [[ -n $1 && $1 != [0123456789]* && $1 != *[!$__gpy_g_idc]* ]]
}

# Decimal digits, short enough for arithmetic.
__gpy_g_isnum() {
    [[ -n $1 && ${#1} -le 12 && $1 != *[!0123456789]* ]]
}

# Scalar __gpy_script_<tok>_<field> into __gpy_g_s; list into __gpy_g_l.
# <tok> comes from __gpy_g_toks (checked by __gpy_g_sync), <field> is literal.
__gpy_g_get() {
    local n="__gpy_script_${1}_$2"
    __gpy_g_s=${!n-}
}
__gpy_g_list() {
    local n="__gpy_script_${1}_$2[@]"
    __gpy_g_l=("${!n}")
}

# Seconds into __gpy_g_t from a clock of this shell (SECONDS on Bash 3.2-4.1,
# which has no fork-free epoch).
__gpy_g_now() {
    if [[ -n ${EPOCHSECONDS-} ]]; then
        __gpy_g_t=$EPOCHSECONDS
    elif [[ ${__gpy_have_printf_epoch:-0} == 1 ]]; then
        printf -v __gpy_g_t '%(%s)T' -1
    else
        __gpy_g_t=$SECONDS
    fi
    __gpy_g_isnum "$__gpy_g_t" || __gpy_g_t=0
}

__gpy_g_drop() {
    unset "__gpy_g_out_$1" "__gpy_g_vnt_$1" "__gpy_g_ck_$1" "__gpy_g_ct_$1" \
        "__gpy_g_cv_$1" "__gpy_g_cn_$1" "__gpy_g_rs_$1" "__gpy_g_ss_$1" "__gpy_g_off_$1"
}

# Before a theme (re)load: unset the style variables of the current plugin
# segments, so a field the next export drops does not linger. Tokens were
# checked against __gpy_g_tokc when listed, so each eval name is literal.
__gpy_g_clear_styles() {
    local tok n
    local -a names
    for tok in ${__gpy_g_toks[@]+"${__gpy_g_toks[@]}"}; do
        [[ -n $tok && $tok != *[!$__gpy_g_tokc]* ]] || continue
        eval "names=(\${!__gpy_segment_${tok}_@} \${!__color_${tok}_@} \${!__icon_${tok}_@})"
        unset "__icon_$tok"
        for n in ${names[@]+"${names[@]}"}; do
            unset "$n"
        done
    done
}

# After every theme load: take the listed tokens, drop the state of tokens
# no longer listed, reset every breaker, map __enabled_segments words to
# tokens and make the next prompt due. A changed `key` is a cache miss by
# itself, since the full key starts with it.
__gpy_g_sync() {
    local tok old w name n keep f=
    local IFS=$' \t\n'
    local -a prev
    prev=("${__gpy_g_toks[@]}")
    __gpy_g_toks=()
    if [[ -n ${__gpy_script_segments+x} ]]; then
        for tok in "${__gpy_script_segments[@]}"; do
            [[ -n $tok && $tok != *[!$__gpy_g_tokc]* ]] || continue
            [[ " ${__gpy_g_toks[*]} " == *" $tok "* ]] && continue
            __gpy_g_toks+=("$tok")
        done
    fi
    for old in "${prev[@]}"; do
        keep=""
        for tok in "${__gpy_g_toks[@]}"; do
            [[ $tok == "$old" ]] && keep=1
        done
        [[ -n $keep ]] || __gpy_g_drop "$old"
    done
    for tok in "${__gpy_g_toks[@]}"; do
        n="__gpy_g_out_$tok"
        [[ -n ${!n+x} ]] || printf -v "$n" '%s' ""
        n="__gpy_g_vnt_$tok"
        [[ -n ${!n+x} ]] || printf -v "$n" '%s' ""
        printf -v "__gpy_g_rs_$tok" '%s' 0
        printf -v "__gpy_g_ss_$tok" '%s' 0
        unset "__gpy_g_off_$tok"
    done
    # Enabled-segment words name segments case-insensitively, as export does.
    __gpy_g_words=()
    __gpy_g_wtoks=()
    if ((${#__gpy_g_toks[@]})); then
        local ncm=""
        shopt -q nocasematch && ncm=1
        shopt -s nocasematch
        [[ $- == *f* ]] || {
            f=1
            set -f
        }
        # shellcheck disable=SC2154 # set by the theme export
        for w in ${__enabled_segments-}; do
            for tok in "${__gpy_g_toks[@]}"; do
                __gpy_g_get "$tok" name
                name=$__gpy_g_s
                [[ -n $name && $w == "$name" ]] || continue
                __gpy_g_words+=("$w")
                __gpy_g_wtoks+=("$tok")
                break
            done
        done
        [[ -n $f ]] && set +f
        [[ -n $ncm ]] || shopt -u nocasematch
    fi
    __gpy_g_first=1
    return 0
}

# From __gpy_precmd, before the render. Due on the first prompt, after a
# command (not an empty Enter or Ctrl-C at the prompt), when PWD changed and
# after a theme load; never on a redraw. The guard is cleared before the run:
# Ctrl-C makes Bash abandon PROMPT_COMMAND and run it again, which must draw
# the prompt, not start the scripts again.
__gpy_g_maybe_run() {
    ((${#__gpy_g_toks[@]})) || return 0
    local due=$__gpy_g_first c
    if [[ -n $__gpy_g_cmdno ]]; then
        c='\#'
        c=${c@P}
        [[ $c != "$__gpy_g_cmd" ]] && due=1
        __gpy_g_cmd=$c
    elif [[ -n $__gpy_g_ran ]]; then
        due=1
    fi
    [[ $PWD != "$__gpy_g_lastpwd" ]] && due=1
    __gpy_g_first=""
    __gpy_g_ran=""
    __gpy_g_lastpwd=$PWD
    [[ -n $due ]] || return 0
    # The cache compares keys exactly and lists may be empty.
    local opts=$- ncm=""
    shopt -q nocasematch && ncm=1
    shopt -u nocasematch
    set +u
    __gpy_g_run_due
    [[ -n $ncm ]] && shopt -s nocasematch
    [[ $opts == *u* ]] && set -u
    return 0
}

# One strike for a timeout or an over-cap output; the second in a row
# disables the segment for this shell, with one line on stderr.
__gpy_g_strike() {
    local tok=$1 kind=$2 n v
    n="__gpy_g_${kind}_$tok"
    v=${!n-0}
    __gpy_g_isnum "$v" || v=0
    v=$((10#$v + 1))
    printf -v "$n" '%s' "$v"
    ((v >= __gpy_g_strikes_max)) || return 0
    printf -v "__gpy_g_off_$tok" '%s' 1
    printf -v "__gpy_g_out_$tok" '%s' ""
    __gpy_g_get "$tok" name
    printf 'gpy: script segment %s disabled for this shell after %s failures (timeout or output over 4096 bytes)\n' \
        "$__gpy_g_s" "$__gpy_g_strikes_max" >&2
}

# Full cache key and the environment of one segment: the static key, then
# every declared input and env name with its value, length-framed. Sets
# __gpy_g_k, __gpy_g_names and __gpy_g_vals; returns 1 on an unknown input
# or a bad name.
__gpy_g_key() {
    local tok=$1 w n v e
    __gpy_g_names=()
    __gpy_g_vals=()
    __gpy_g_get "$tok" key
    __gpy_g_k=$__gpy_g_s
    __gpy_g_list "$tok" env
    for n in "${__gpy_g_l[@]}"; do
        __gpy_g_isname "$n" && [[ $n != GPY_* ]] || return 1
        if [[ -n ${!n+x} ]]; then
            v=${!n}
            __gpy_g_k+="|e${#n}:$n${#v}:$v"
            __gpy_g_names+=("$n")
            __gpy_g_vals+=("$v")
        else
            __gpy_g_k+="|u${#n}:$n"
        fi
    done
    __gpy_g_list "$tok" inputs
    for w in "${__gpy_g_l[@]}"; do
        case $w in
            pwd) n=GPY_PWD v=$PWD ;;
            is_root) n=GPY_IS_ROOT v=${__gpy_is_root-0} ;;
            is_sudo) n=GPY_IS_SUDO v=${__gpy_is_sudo-0} ;;
            is_ssh) n=GPY_IS_SSH v=${__gpy_is_ssh-0} ;;
            cmd_duration) n=GPY_CMD_DURATION v=${__gpy_cmd_duration-} ;;
            prompt_icons) n=GPY_PROMPT_ICONS v=${__prompt_icons-} ;;
            *) return 1 ;;
        esac
        __gpy_g_k+="|i${#w}:$w${#v}:$v"
        __gpy_g_names+=("$n")
        __gpy_g_vals+=("$v")
    done
    # Settings are in the static key already; the runner sets them last, so
    # they override any inherited GPY_OPT_*.
    __gpy_g_list "$tok" opts
    for e in "${__gpy_g_l[@]}"; do
        [[ $e == GPY_OPT_*=* ]] || return 1
        n=${e%%=*}
        __gpy_g_isname "$n" && ((${#n} <= 72)) || return 1
        __gpy_g_names+=("$n")
        __gpy_g_vals+=("${e#*=}")
    done
    return 0
}

# A failed trigger drops the segment's cache entry, so a later prompt whose
# trigger holds again runs the command instead of reusing stale output.
__gpy_g_uncache() {
    unset "__gpy_g_ck_$1" "__gpy_g_ct_$1" "__gpy_g_cv_$1" "__gpy_g_cn_$1"
}

__gpy_g_run_due() {
    local sh=${__gpy_script_sh-} tok n v ok i k res rec id st tx vn now
    local nf nd ne scanned=" " hits=" " ttl ms scope cwd cmd req root
    local -a cand scan fl dl el batch btoks bkeys bttls
    __gpy_g_now
    now=$__gpy_g_t
    # Emptied first: a run cut short (Ctrl-C) leaves its segment hidden.
    for tok in "${__gpy_g_toks[@]}"; do
        printf -v "__gpy_g_out_$tok" '%s' ""
        printf -v "__gpy_g_vnt_$tok" '%s' ""
    done
    [[ $sh == /* ]] || return 0

    # Triggers: require_env here (fork-free), files, folders and extensions
    # in one scan child for every segment that has any.
    for tok in "${__gpy_g_toks[@]}"; do
        n="__gpy_g_off_$tok"
        [[ -n ${!n-} ]] && continue
        __gpy_g_list "$tok" reqenv
        ok=1
        for n in "${__gpy_g_l[@]}"; do
            if ! __gpy_g_isname "$n" || [[ -z ${!n-} ]]; then
                ok=""
                break
            fi
        done
        [[ -n $ok ]] || { __gpy_g_uncache "$tok"; continue; }
        __gpy_g_list "$tok" reqfiles
        fl=("${__gpy_g_l[@]}")
        __gpy_g_list "$tok" reqfolders
        dl=("${__gpy_g_l[@]}")
        __gpy_g_list "$tok" reqexts
        el=("${__gpy_g_l[@]}")
        for n in "${fl[@]}" "${dl[@]}" "${el[@]}"; do
            [[ -n $n && $n != */* && $n != . && $n != .. ]] || ok=""
        done
        [[ -n $ok ]] || { __gpy_g_uncache "$tok"; continue; }
        cand+=("$tok")
        nf=${#fl[@]} nd=${#dl[@]} ne=${#el[@]}
        if ((nf + nd + ne)); then
            scan+=("$tok" "$nf" "$nd" "$ne" "${fl[@]}" "${dl[@]}" "${el[@]}")
            scanned+="$tok "
        fi
    done
    if ((${#scan[@]})); then
        res=$(__gpy_g_scan "${scan[@]}")
        while [[ $res == *$'\036'* ]]; do
            rec=${res%%$'\036'*}
            res=${res#*$'\036'}
            id=${rec%%$'\037'*}
            st=${rec#*$'\037'}
            [[ $scanned == *" $id "* ]] || continue
            case $st in
                hit)
                    hits+="$id "
                    printf -v "__gpy_g_ss_$id" '%s' 0
                    ;;
                miss) printf -v "__gpy_g_ss_$id" '%s' 0 ;;
                *) __gpy_g_strike "$id" ss ;;
            esac
        done
    fi

    # Cache, then one batch for every miss.
    k=0
    for tok in "${cand[@]}"; do
        if [[ $scanned == *" $tok "* && $hits != *" $tok "* ]]; then
            __gpy_g_uncache "$tok"
            continue
        fi
        __gpy_g_get "$tok" ttl
        ttl=$__gpy_g_s
        __gpy_g_get "$tok" timeout
        ms=$__gpy_g_s
        if __gpy_g_isnum "$ttl" && __gpy_g_isnum "$ms"; then :; else continue; fi
        __gpy_g_key "$tok" || continue
        if ((10#$ttl > 0)); then
            n="__gpy_g_ck_$tok"
            v="__gpy_g_ct_$tok"
            if [[ -n ${!n+x} && ${!n} == "$__gpy_g_k" ]] && __gpy_g_isnum "${!v-}" &&
                ((10#${!v} <= now && now - 10#${!v} < 10#$ttl)); then
                n="__gpy_g_cv_$tok"
                printf -v "__gpy_g_out_$tok" '%s' "${!n-}"
                n="__gpy_g_cn_$tok"
                printf -v "__gpy_g_vnt_$tok" '%s' "${!n-}"
                continue
            fi
        fi
        __gpy_g_get "$tok" command
        cmd=$__gpy_g_s
        __gpy_g_get "$tok" reqcommand
        req=$__gpy_g_s
        __gpy_g_get "$tok" root
        root=$__gpy_g_s
        __gpy_g_get "$tok" scope
        scope=$__gpy_g_s
        cwd=$root
        # The inherited working directory, so $PWD is not in any argv.
        [[ $scope == folder ]] && cwd=""
        for ((i = 0; i < ${#__gpy_g_names[@]}; i++)); do
            n="__gpy_g_v${k}_${__gpy_g_names[i]}"
            local "$n"
            printf -v "$n" '%s' "${__gpy_g_vals[i]}"
        done
        batch+=("$tok" "$ms" "$cwd" "$cmd" "$req" "${#__gpy_g_names[@]}" "${__gpy_g_names[@]}")
        btoks[k]=$tok
        bkeys[k]=$__gpy_g_k
        bttls[k]=$ttl
        k=$((k + 1))
        ((k < 40)) || break
    done
    ((k)) || return 0

    res=$(__gpy_g_batch "$sh" "${batch[@]}")
    i=0
    while [[ $res == *$'\036'* ]] && ((i < k)); do
        rec=${res%%$'\036'*}
        res=${res#*$'\036'}
        id=${rec%%$'\037'*}
        rec=${rec#*$'\037'}
        st=${rec%%$'\037'*}
        rec=${rec#*$'\037'}
        tx=${rec%%$'\037'*}
        vn=${rec#*$'\037'}
        tok=${btoks[i]}
        i=$((i + 1))
        [[ $id == "$tok" ]] || continue
        case $st in
            ok)
                # Stored escaped: a cache hit or redraw costs no escape.
                __gpy_prompt_escape "$tx" tx
                [[ $vn != *[!$__gpy_g_tokc]* && ${#vn} -le 32 ]] || vn=""
                printf -v "__gpy_g_rs_$tok" '%s' 0
                ;;
            exit | req | c1) tx="" vn="" ;;
            timeout | cap)
                __gpy_g_strike "$tok" rs
                continue
                ;;
            *) continue ;;
        esac
        printf -v "__gpy_g_out_$tok" '%s' "$tx"
        printf -v "__gpy_g_vnt_$tok" '%s' "$vn"
        if ((10#${bttls[i - 1]} > 0)); then
            printf -v "__gpy_g_ck_$tok" '%s' "${bkeys[i - 1]}"
            printf -v "__gpy_g_ct_$tok" '%s' "$now"
            printf -v "__gpy_g_cv_$tok" '%s' "$tx"
            printf -v "__gpy_g_cn_$tok" '%s' "$vn"
        fi
    done
    return 0
}

# The token of enabled-segment word $1 into __gpy_g_wt; 1 if not a script
# segment.
__gpy_g_word_tok() {
    local i
    for ((i = 0; i < ${#__gpy_g_words[@]}; i++)); do
        if [[ ${__gpy_g_words[i]} == "$1" ]]; then
            __gpy_g_wt=${__gpy_g_wtoks[i]}
            return 0
        fi
    done
    return 1
}

# Decided before positions are fixed, so no neighbour keeps a dangling cap.
__gpy_g_visible() {
    local n="__gpy_g_out_$1" off="__gpy_g_off_$1"
    [[ -n ${!n-} && -z ${!off-} ]]
}

# The pill of token $1 (is_last $2, is_first $3) into __gpy_g_rout, its
# background into __gpy_g_rbg. Styles: __color_<tok>_{bg,fg} and __icon_<tok>,
# each replaced by the variant's __color_<tok>_<variant>_{bg,fg} or
# __icon_<tok>_<variant> when set and non-empty. A set
# __gpy_segment_<tok>_{open,close}_icon (with _icon_color, _bg_color)
# replaces the global delimiter on that side.
__gpy_g_render() {
    local tok=$1 n text vnt bg fg icon label caps="" od="" ofg="" obg="" cl="" cfg="" cbg=""
    n="__gpy_g_out_$tok"
    text=${!n-}
    n="__gpy_g_vnt_$tok"
    vnt=${!n-}
    n="__color_${tok}_bg"
    bg=${!n-}
    n="__color_${tok}_fg"
    fg=${!n-}
    n="__icon_$tok"
    icon=${!n-}
    if [[ -n $vnt && $vnt != *[!$__gpy_g_tokc]* ]]; then
        n="__color_${tok}_${vnt}_bg"
        [[ -n ${!n-} ]] && bg=${!n}
        n="__color_${tok}_${vnt}_fg"
        [[ -n ${!n-} ]] && fg=${!n}
        n="__icon_${tok}_${vnt}"
        [[ -n ${!n-} ]] && icon=${!n}
    fi
    label=" $text"
    if [[ -n $icon ]]; then
        __gpy_prompt_escape "$icon" icon
        label=" $icon $text"
    fi
    n="__gpy_segment_${tok}_open_icon"
    if [[ -n ${!n+x} ]]; then
        caps+=o od=${!n}
        n="__gpy_segment_${tok}_open_icon_color"
        ofg=${!n-}
        n="__gpy_segment_${tok}_open_bg_color"
        obg=${!n-}
    fi
    n="__gpy_segment_${tok}_close_icon"
    if [[ -n ${!n+x} ]]; then
        caps+=c cl=${!n}
        n="__gpy_segment_${tok}_close_icon_color"
        cfg=${!n-}
        n="__gpy_segment_${tok}_close_bg_color"
        cbg=${!n-}
    fi
    [[ $caps == *c* ]] || caps+=d
    __gpy_g_rout=$(__gpy_section_standalone "$bg" "$fg" "$label" "$2" "$3" "$caps" \
        "$od" "$ofg" "$obg" "$cl" "$cfg" "$cbg")
    __gpy_g_rbg=$bg
}
