# zsh/core/script.zsh
# Script ("G") segments: plugin scripts named by the theme export
# (`__gpy_script_*`, contract in docs/dev/plugins.md "Theme export"), run once
# per command from precmd in this interactive shell, never from the
# `$(__gpy_render_prompt ...)` subshell or a repaint (TRAPURG, reset-prompt).
#
# - Triggers first: `require_env` (no fork), then one scan child for
#   `require_files`/`require_folders`/`require_extensions` (30 ms, then
#   SIGKILL), then `require_command` inside the runner.
# - Cache: shell variables only, keyed by the exported static key plus the
#   declared inputs and `env` values, reused for `ttl` seconds (0 = never).
# - Runner: bash/core/g_runner.bash (`gpy setup` installs it as
#   core/g_runner.bash; a checkout uses the repository copy) in one Bash child
#   per due prompt, with one shared deadline. The child is a Bash 4 or later
#   when one is installed, else /bin/bash; with Bash 3.2 a Ctrl-C waits for the
#   timeout (the run still reports it and the process groups are killed).
#   Declared values reach the child through its environment, never argv.
# - Output: the runner caps, strips C0/DEL and rejects C1; this file escapes
#   the text for the prompt once and stores only that render-ready value.
# - No files: no cache, relay or marker file; the descriptors used here are
#   closed before the prompt.

zmodload zsh/datetime zsh/system

# A parent environment can pre-set any of these names (an IDE, `sudo -E`,
# ssh SendEnv): start from nothing, before the theme export is sourced, so a
# failed export never leaves a stale `__gpy_script_segments` behind.
unset -m '__gpy_g_*' '__gpy_script_*' 2>/dev/null

typeset -ga __gpy_g_toks=() __gpy_g_names=()
# Per token (tokens are checked against ^[a-z0-9_]+$ before they are keys).
typeset -gA __gpy_g_nm=() __gpy_g_cmd=() __gpy_g_req=() __gpy_g_root=() \
    __gpy_g_ttl=() __gpy_g_tmo=() __gpy_g_folder=() __gpy_g_skey=()
typeset -gA __gpy_g_text=() __gpy_g_variant=() __gpy_g_key=() __gpy_g_at=() \
    __gpy_g_runfails=() __gpy_g_scanfails=() __gpy_g_off=()
typeset -g __gpy_g_due=1 __gpy_g_pwd="" __gpy_g_cmd_seen=""
typeset -g __gpy_g_sh=/bin/sh __gpy_g_bash="" __gpy_g_bash_resolved=""
typeset -g __gpy_g_src=""
# Consecutive timeouts or over-cap outputs that disable a segment for this shell.
typeset -gi __gpy_g_strikes_max=2
typeset -g __gpy_g_core_dir=${${(%):-%x}:a:h}
typeset -g __gpy_g_runner_file=$__gpy_g_core_dir/g_runner.bash
[[ -r $__gpy_g_runner_file ]] || __gpy_g_runner_file=${__gpy_g_core_dir:h:h}/bash/core/g_runner.bash
unset __gpy_g_core_dir
if [[ -r $__gpy_g_runner_file ]]; then
    __gpy_g_src="$(<$__gpy_g_runner_file)"$'\n''__gpy_g_batch "$@"'
elif [[ ${GPY_DEBUG-} == 1 || ${GPY_VERBOSE-} == 1 ]]; then
    print -ru2 -- "gpy[script]: warning: Runner $__gpy_g_runner_file missing - script segments disabled"
fi

# Every variable name built from exported data is checked before ${(P)...}.
function __gpy_g_name_ok() {
    local LC_ALL=C
    [[ -n $1 && $1 != [0-9]* && $1 != *[^A-Za-z0-9_]* ]]
}

function __gpy_g_digits_ok() {
    local LC_ALL=C
    [[ -n $1 && $1 != *[^0-9]* && ${#1} -le 9 ]]
}

# The runner child: the first Bash 4 or later on PATH, else /bin/bash.
# Resolved once per shell (one short `bash -c` per candidate checked). None
# found: a warning under GPY_DEBUG/GPY_VERBOSE only, as Fish logs it.
function __gpy_g_resolve_bash() {
    __gpy_g_bash_resolved=1
    local d b
    for d in "${path[@]}"; do
        b=$d/bash
        [[ $d == /* && -f $b && -x $b ]] || continue
        if "$b" -p -c '[ "${BASH_VERSINFO[0]}" -ge 4 ]' </dev/null >/dev/null 2>&1; then
            __gpy_g_bash=$b
            return 0
        fi
    done
    if [[ -x /bin/bash ]]; then
        __gpy_g_bash=/bin/bash
    elif [[ ${GPY_DEBUG-} == 1 || ${GPY_VERBOSE-} == 1 ]]; then
        print -ru2 -- "gpy[script]: warning: No bash found - script segments disabled"
    fi
    return 0
}

# Snapshot one exported segment; 1 = skip it (contract §3 checks).
function __gpy_g_load_one() {
    local tok=$1 n v w f
    local LC_ALL=C
    local -A got
    for f in name root command reqcommand ttl timeout scope key; do
        n=__gpy_script_${tok}_$f
        __gpy_g_name_ok "$n" || return 1
        (( ${(P)+n} )) || return 1
        got[$f]=${(P)n}
    done
    [[ -n ${got[name]} && ${got[name]} != *[^a-z0-9_-]* ]] || return 1
    [[ ${got[name]//-/_} == "$tok" ]] || return 1
    [[ ${got[root]} == /* && ${got[command]} == /* ]] || return 1
    [[ -z ${got[reqcommand]} || ${got[reqcommand]} == /* ]] || return 1
    __gpy_g_digits_ok "${got[ttl]}" || return 1
    __gpy_g_digits_ok "${got[timeout]}" || return 1
    (( 10#${got[timeout]} >= 1 && 10#${got[timeout]} <= 500 )) || return 1
    [[ ${got[scope]} == (session|folder) ]] || return 1
    n=__gpy_script_${tok}_inputs
    for w in "${(@P)n}"; do
        [[ $w == (pwd|is_root|is_sudo|is_ssh|cmd_duration|prompt_icons) ]] || return 1
    done
    __gpy_g_nm[$tok]=${got[name]}
    __gpy_g_root[$tok]=${got[root]}
    __gpy_g_cmd[$tok]=${got[command]}
    __gpy_g_req[$tok]=${got[reqcommand]}
    __gpy_g_ttl[$tok]=$(( 10#${got[ttl]} ))
    __gpy_g_tmo[$tok]=$(( 10#${got[timeout]} ))
    __gpy_g_folder[$tok]=${${(M)got[scope]:#folder}:+1}
    __gpy_g_skey[$tok]=${got[key]}
    return 0
}

# Before a theme (re)load: unset the style variables of the current plugin
# segments, so a field the next export drops does not linger.
function __gpy_g_clear_styles() {
    emulate -L zsh
    local tok
    for tok in "${__gpy_g_toks[@]}"; do
        [[ -n $tok && $tok != *[^a-z0-9_]* ]] || continue
        unset -m "__gpy_segment_${tok}_*" "__color_${tok}_*" "__icon_${tok}_*" 2>/dev/null
        unset "__icon_$tok" 2>/dev/null
    done
    return 0
}

# After every theme (re)load: the listed tokens become the active set, the
# breakers reset, state of tokens no longer listed goes, and the next prompt
# is due (a changed static key misses the cache by itself).
function __gpy_g_reload() {
    emulate -L zsh
    local tok
    local -a toks names
    if [[ ${(t)__gpy_script_segments-} == array* ]]; then
        for tok in "${__gpy_script_segments[@]}"; do
            [[ -n $tok && $tok != *[^a-z0-9_]* ]] || continue
            (( ${toks[(Ie)$tok]} )) && continue
            __gpy_g_load_one "$tok" || continue
            toks+=("$tok")
            names+=("${__gpy_g_nm[$tok]}")
        done
    fi
    [[ ${(t)__gpy_script_sh-} == scalar* && $__gpy_script_sh == /* ]] \
        && __gpy_g_sh=$__gpy_script_sh || __gpy_g_sh=/bin/sh
    for tok in ${(k)__gpy_g_text} ${(k)__gpy_g_key} ${(k)__gpy_g_off}; do
        (( ${toks[(Ie)$tok]} )) && continue
        unset "__gpy_g_text[$tok]" "__gpy_g_variant[$tok]" "__gpy_g_key[$tok]" "__gpy_g_at[$tok]"
    done
    __gpy_g_toks=("${toks[@]}")
    __gpy_g_names=("${names[@]}")
    __gpy_g_runfails=() __gpy_g_scanfails=() __gpy_g_off=()
    __gpy_g_due=1
    if (( ${#toks} )) && [[ -z $__gpy_g_bash_resolved ]]; then
        __gpy_g_resolve_bash
    fi
    return 0
}

# Value of input word $1 into REPLY; 1 for an unknown word.
function __gpy_g_input() {
    case $1 in
        pwd) REPLY=$PWD ;;
        is_root) REPLY=${__gpy_is_root:-0} ;;
        is_sudo) REPLY=${__gpy_is_sudo:-0} ;;
        is_ssh) REPLY=${__gpy_is_ssh:-0} ;;
        cmd_duration) REPLY=${__gpy_g_cmd_seen:+${__gpy_cmd_duration:-0}} ;;
        prompt_icons) REPLY=${__prompt_icons-} ;;
        *) return 1 ;;
    esac
}

# Environment name for input word $1 into REPLY.
function __gpy_g_input_env() {
    case $1 in
        pwd) REPLY=GPY_PWD ;;
        is_root) REPLY=GPY_IS_ROOT ;;
        is_sudo) REPLY=GPY_IS_SUDO ;;
        is_ssh) REPLY=GPY_IS_SSH ;;
        cmd_duration) REPLY=GPY_CMD_DURATION ;;
        prompt_icons) REPLY=GPY_PROMPT_ICONS ;;
        *) return 1 ;;
    esac
}

# Full cache key of token $1 into REPLY: static key, then each declared input
# and env value framed as <bytes>:<value> (`u` for an unset env name).
function __gpy_g_full_key() {
    local tok=$1 n w v k
    local LC_ALL=C
    k=${__gpy_g_skey[$tok]}
    n=__gpy_script_${tok}_inputs
    for w in "${(@P)n}"; do
        __gpy_g_input "$w" || return 1
        k+="|${#REPLY}:$REPLY"
    done
    n=__gpy_script_${tok}_env
    for w in "${(@P)n}"; do
        __gpy_g_name_ok "$w" && [[ $w != GPY_* ]] || return 1
        if (( ${(P)+w} )); then
            v=${(P)w}
            k+="|${#v}:$v"
        else
            k+="|u"
        fi
    done
    REPLY=$k
}

# One timeout or over-cap result (kind $2: run or scan) for token $1; the
# second in a row disables the segment for this shell, with one stderr line.
function __gpy_g_strike() {
    local tok=$1 c
    if [[ $2 == scan ]]; then
        c=$(( ${__gpy_g_scanfails[$tok]:-0} + 1 ))
        __gpy_g_scanfails[$tok]=$c
    else
        c=$(( ${__gpy_g_runfails[$tok]:-0} + 1 ))
        __gpy_g_runfails[$tok]=$c
    fi
    if (( c >= __gpy_g_strikes_max )) && [[ -z ${__gpy_g_off[$tok]-} ]]; then
        __gpy_g_off[$tok]=1
        print -ru2 -- "gpy: script segment ${__gpy_g_nm[$tok]} disabled for this shell after $__gpy_g_strikes_max failures (timeout or output over 4096 bytes)"
    fi
}

# Runs only in the scan child (behind a process substitution): prints its
# own PID, then one 1 (all conditions hold) or 0 per token. Paths are
# relative to the working directory, so its name never matters; the upward
# search stops at the mount holding it on Linux, and at / elsewhere.
function __gpy_g_scan_child() {
    emulate -L zsh
    exec 2>/dev/null
    print -r -- $sysparams[pid]
    local tok n r rest res="" here=${PWD:A} mr=/ mp _a _b _c _e _r
    local -i hit
    local -a files dirs exts m
    if [[ -r /proc/self/mountinfo ]]; then
        while read -r _a _b _c _e mp _r; do
            mp=${(g:o:)mp}
            [[ $here/ == "${mp%/}"/* ]] && (( ${#mp} > ${#mr} )) && mr=$mp
        done </proc/self/mountinfo
    fi
    for tok in "$@"; do
        n=__gpy_script_${tok}_reqfiles; files=("${(@P)n}")
        n=__gpy_script_${tok}_reqfolders; dirs=("${(@P)n}")
        n=__gpy_script_${tok}_reqexts; exts=("${(@P)n}")
        r="" rest=${here#"${mr%/}"}
        while (( ${#files} + ${#dirs} )); do
            m=()
            for n in "${files[@]}"; do [[ -f $r$n ]] || m+=("$n"); done
            files=("${m[@]}")
            m=()
            for n in "${dirs[@]}"; do [[ -d $r$n ]] || m+=("$n"); done
            dirs=("${m[@]}")
            [[ -n $rest ]] || break
            rest=${rest%/*}
            r=../$r
        done
        hit=1
        (( ${#files} + ${#dirs} )) && hit=0
        for n in "${exts[@]}"; do
            (( hit )) || break
            m=(*."$n"(ND-.Y1))
            (( ${#m} )) || hit=0
        done
        res+=$hit
    done
    print -r -- $res
}

# Scan tokens $@ in one child within 30 ms; sets reply to one 1/0 per token.
# 1 when the child timed out (it is SIGKILLed) or could not start.
function __gpy_g_scan() {
    local fd p line rem dl=$(( EPOCHREALTIME + 0.03 ))
    reply=()
    exec {fd}< <(__gpy_g_scan_child "$@") || return 1
    # The child prints its PID before touching the file system.
    if ! read -r -t 1 -u $fd p || [[ $p != <-> ]]; then
        exec {fd}<&-
        return 1
    fi
    rem=$(( dl - EPOCHREALTIME ))
    (( rem > 0 )) || rem=0.001
    if read -r -t $rem -u $fd line && [[ ${#line} == $# && $line != *[^01]* ]]; then
        exec {fd}<&-
        reply=("${(@s::)line}")
        return 0
    fi
    kill -KILL $p 2>/dev/null
    exec {fd}<&-
    return 1
}

# Hide token $1 until it runs again: a failed trigger or a run without a
# record also drops the cache entry, so a later prompt never reuses it.
function __gpy_g_hide() {
    __gpy_g_text[$1]="" __gpy_g_variant[$1]=""
    unset "__gpy_g_key[$1]" "__gpy_g_at[$1]"
}

# Due on the first prompt, after a command (preexec), when $PWD changed
# without one (a widget that cds and runs precmd) and after a theme reload.
function __gpy_g_maybe_run() {
    (( ${#__gpy_g_toks} )) || return 0
    [[ $PWD != "$__gpy_g_pwd" ]] && __gpy_g_due=1
    __gpy_g_pwd=$PWD
    [[ $__gpy_g_due == 1 ]] || return 0
    # Cleared before the run, so an interrupted run is not started again.
    __gpy_g_due=""
    emulate -L zsh
    # The run start in epoch microseconds: the runner child counts its
    # deadlines from here, so its own start-up counts against T.
    local -a et=($epochtime)
    local -i t0=$(( et[1] * 1000000 + et[2] / 1000 ))
    local -i cancel=0
    # Ctrl-C ends the run, not precmd: unhandled, zsh abandons precmd and the
    # transient prompt zle-line-finish swapped in stays on screen. The runner
    # child shares this process group and gets the INT itself.
    TRAPINT() { cancel=1; return 0 }
    local tok n w v k i now=$EPOCHSECONDS
    local -a cand scan run args pay_n pay_v names f
    local -A keys

    for tok in "${__gpy_g_toks[@]}"; do
        if [[ -n ${__gpy_g_off[$tok]-} ]]; then
            __gpy_g_hide "$tok"
            continue
        fi
        n=__gpy_script_${tok}_reqenv
        for w in "${(@P)n}"; do
            if ! __gpy_g_name_ok "$w" || [[ -z ${(P)w-} ]]; then
                __gpy_g_hide "$tok"
                continue 2
            fi
        done
        k=0
        for n in reqfiles reqfolders reqexts; do
            n=__gpy_script_${tok}_$n
            (( ${#${(@P)n}} )) && k=1
        done
        if (( k )); then scan+=("$tok"); else cand+=("$tok"); fi
    done

    if (( ${#scan} )); then
        if __gpy_g_scan "${scan[@]}"; then
            for (( i = 1; i <= ${#scan}; i++ )); do
                tok=${scan[i]}
                __gpy_g_scanfails[$tok]=0
                if [[ ${reply[i]} == 1 ]]; then
                    cand+=("$tok")
                else
                    __gpy_g_hide "$tok"
                fi
            done
        else
            for tok in "${scan[@]}"; do
                __gpy_g_hide "$tok"
                (( cancel )) || __gpy_g_strike "$tok" scan
            done
        fi
    fi
    (( cancel )) && return 0

    for tok in "${cand[@]}"; do
        __gpy_g_full_key "$tok" || { __gpy_g_hide "$tok"; continue; }
        k=$REPLY
        if (( __gpy_g_ttl[$tok] > 0 )) && [[ ${+__gpy_g_key[$tok]} == 1 && ${__gpy_g_key[$tok]} == "$k" ]] \
            && (( now - ${__gpy_g_at[$tok]:-0} < __gpy_g_ttl[$tok] )); then
            continue
        fi
        keys[$tok]=$k
        run+=("$tok")
    done
    (( ${#run} )) || return 0
    # The runner takes at most 40 segments per call.
    run=("${(@)run[1,40]}")
    # Emptied first, so a run that ends without a record hides its segment.
    for tok in "${run[@]}"; do __gpy_g_hide "$tok"; done
    [[ -n $__gpy_g_bash && -n $__gpy_g_src ]] || return 0

    # Arguments: ID T_MS CWD CMD REQ N NAME... per segment; the value of NAME
    # for the k-th segment travels as __gpy_g_v<k>_<NAME> in the child's
    # environment only.
    k=0
    for tok in "${run[@]}"; do
        names=()
        n=__gpy_script_${tok}_inputs
        for w in "${(@P)n}"; do
            __gpy_g_input "$w" || continue
            v=$REPLY
            __gpy_g_input_env "$w" || continue
            names+=("$REPLY"); pay_n+=("__gpy_g_v${k}_$REPLY"); pay_v+=("$v")
        done
        n=__gpy_script_${tok}_env
        for w in "${(@P)n}"; do
            __gpy_g_name_ok "$w" && [[ $w != GPY_* ]] && (( ${(P)+w} )) || continue
            names+=("$w"); pay_n+=("__gpy_g_v${k}_$w"); pay_v+=("${(P)w}")
        done
        n=__gpy_script_${tok}_opts
        for w in "${(@P)n}"; do
            v=${w%%=*}
            __gpy_g_name_ok "$v" && [[ $v == GPY_OPT_* && $w == *=* ]] || continue
            names+=("$v"); pay_n+=("__gpy_g_v${k}_$v"); pay_v+=("${w#*=}")
        done
        # "" keeps the child's working directory, this shell's $PWD, so the
        # directory name never reaches argv.
        v=${__gpy_g_root[$tok]}
        [[ -n ${__gpy_g_folder[$tok]} ]] && v=""
        args+=("$tok" "${__gpy_g_tmo[$tok]}" "$v" "${__gpy_g_cmd[$tok]}" "${__gpy_g_req[$tok]}" ${#names} "${names[@]}")
        (( k++ ))
    done

    local res
    res=$(
        for (( i = 1; i <= ${#pay_n}; i++ )); do
            export "${pay_n[i]}=${pay_v[i]}"
        done
        export __gpy_g_t0=$t0
        exec "$__gpy_g_bash" -p -c "$__gpy_g_src" gpy "$__gpy_g_sh" "${args[@]}" </dev/null 2>/dev/null
    )

    local rec rs=$'\x1e' us=$'\x1f' esc
    local -A seen
    for rec in "${(@ps:$rs:)res}"; do
        [[ -n $rec ]] || continue
        f=("${(@ps:$us:)rec}")
        tok=${f[1]}
        [[ -n $tok && $tok != *[^a-z0-9_]* ]] && (( ${run[(Ie)$tok]} )) || continue
        [[ -z ${seen[$tok]-} ]] || continue
        seen[$tok]=1
        case ${f[2]} in
            ok)
                __gpy_prompt_escape "${f[3]}" esc
                __gpy_g_text[$tok]=$esc
                v=${f[4]}
                [[ -n $v && ${#v} -le 32 && $v != *[^a-z0-9_]* ]] || v=""
                __gpy_g_variant[$tok]=$v
                __gpy_g_runfails[$tok]=0
                ;|
            ok|exit|req|c1)
                __gpy_g_key[$tok]=${keys[$tok]}
                __gpy_g_at[$tok]=$now
                ;;
            timeout|cap)
                # With a Bash 3.2 child a Ctrl-C ends the run at its deadline
                # and can come back as a timeout; Ctrl-C never counts.
                [[ ${f[2]} == timeout ]] && (( cancel )) && continue
                __gpy_g_strike "$tok" run
                ;;
        esac
    done
    return 0
}

# Script segment token of enabled segment name $1 into $2; 1 when $1 is not
# an active script segment.
function __gpy_g_token() {
    local -i i=${__gpy_g_names[(Ie)$1]}
    (( i )) || return 1
    printf -v "$2" '%s' "${__gpy_g_toks[i]}"
}

# Background ($2) and text ($3) colours of token $1: the variant's when
# styled, else the segment's. Out-var locals are prefixed, as in renderer.zsh.
function __gpy_g_colors() {
    local __gt=$1 __gv=${__gpy_g_variant[$1]-} __gn __gb __gf
    __gn=__gpy_segment_${__gt}_bg_color; __gb=${(P)__gn-}
    __gn=__gpy_segment_${__gt}_text_color; __gf=${(P)__gn-}
    if [[ -n $__gv && $__gv != *[^a-z0-9_]* ]]; then
        __gn=__gpy_segment_${__gt}_variant_${__gv}_bg_color
        __gpy_g_name_ok "$__gn" && [[ -n ${(P)__gn-} ]] && __gb=${(P)__gn}
        __gn=__gpy_segment_${__gt}_variant_${__gv}_text_color
        __gpy_g_name_ok "$__gn" && [[ -n ${(P)__gn-} ]] && __gf=${(P)__gn}
    fi
    printf -v "$2" '%s' "$__gb"
    printf -v "$3" '%s' "$__gf"
}

# Render token $1 as a pill: $2 is_last, $3 prev_bg, $4 is_first. The stored
# text is already escaped; the theme's icon and caps are escaped here. The
# variant's icon, when set and non-empty, replaces the segment icon.
function __gpy_g_render() {
    emulate -L zsh
    local tok=$1 text=${__gpy_g_text[$1]-} bg fg icon n caps="" vnt=${__gpy_g_variant[$1]-}
    local od="" ofg="" obg="" cl="" cfg="" cbg=""
    [[ -n $text ]] || return 0
    __gpy_g_colors "$tok" bg fg
    n=__gpy_segment_${tok}_icon; icon=${(P)n-}
    if [[ -n $vnt && $vnt != *[^a-z0-9_]* ]]; then
        n=__gpy_segment_${tok}_variant_${vnt}_icon
        __gpy_g_name_ok "$n" && [[ -n ${(P)n-} ]] && icon=${(P)n}
    fi
    if [[ -n $icon ]]; then
        __gpy_prompt_escape "$icon" icon
        text="$icon $text"
    fi
    n=__gpy_segment_${tok}_open_icon
    if (( ${(P)+n} )); then
        caps+=o od=${(P)n}
        n=__gpy_segment_${tok}_open_icon_color; ofg=${(P)n-}
        n=__gpy_segment_${tok}_open_bg_color; obg=${(P)n-}
    fi
    n=__gpy_segment_${tok}_close_icon
    if (( ${(P)+n} )); then
        caps+=c cl=${(P)n}
        n=__gpy_segment_${tok}_close_icon_color; cfg=${(P)n-}
        n=__gpy_segment_${tok}_close_bg_color; cbg=${(P)n-}
    fi
    [[ $caps == *c* ]] || caps+=d
    __gpy_section_standalone "$bg" "$fg" " $text" "$2" "$4" "$caps" \
        "$od" "$ofg" "$obg" "$cl" "$cfg" "$cbg"
}
