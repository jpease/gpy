# shellcheck shell=bash
# shellcheck disable=SC1007  # the `local s= p` form is intended
# shellcheck disable=SC2034  # read discards fields into unused names on purpose
# shellcheck disable=SC2209  # s=exit assigns the word exit, not a command
# g_runner.bash - Runner for script (G) segments: runs the scripts and the
# trigger scan under deadlines (ADR-0011 item 6, spike #1006 P2 and P5).
#
# Bash 3.2 code only. Bash sources this file and calls both entry points
# inside `$( )`, so the interactive shell keeps its `$!`, jobs, traps, options
# and descriptors. zsh/core and fish/core hold byte-identical copies, which
# run in a Bash child (`bash -p -c "$src" gpy ARGS`); keep the copies in sync.
# No comment in this file may hold an apostrophe: Bash 3.2 mis-parses one
# inside a process substitution.
#
# __gpy_g_batch SH [ID T_MS CWD CMD REQ N NAME...]...
#   Runs every given segment at once (at most 40 per call; more are ignored);
#   each has its own deadline T_MS (1-500). SH is the interpreter, executed
#   with argv0 sh. REQ (may be empty) runs first, with stdout discarded; CMD
#   runs only when REQ exits 0. Both run in one process group under the same
#   deadline, stdin /dev/null, stderr discarded, in CWD, or in the inherited
#   working directory when CWD is empty (so PWD never has to be in argv).
#   Environment: the base set below, then each NAME whose value is set in
#   the variable __gpy_g_v<K>_<NAME> (K is the 0-based segment index in this
#   call). Values reach the script through its environment, never through
#   argv: exec -c, then a prologue reads them on fd 3. With a clock (Bash 5)
#   the deadlines count from __gpy_g_t0 when set (epoch microseconds: when
#   the caller started the prompt), at most 200 ms back, so the start-up of
#   a runner child counts against T.
#   Prints per segment "ID US STATUS US TEXT US VARIANT RS" (US 0x1f, RS 0x1e).
#   STATUS: ok, exit (non-zero exit), req (REQ failed), timeout, cap (over
#   4096 bytes), c1 (C1 control in the text), intr (Ctrl-C). TEXT and VARIANT
#   only with ok: TEXT is line 1 with C0 and DEL stripped, not escaped;
#   VARIANT is line 2 when it matches ^[a-z0-9_]{1,32}$, else empty.
#
# __gpy_g_scan [ID NF ND NE NAME...]...
#   One child checks every segment: NF names that must exist as files and ND
#   as folders in the working directory or an ancestor (cwd-relative paths;
#   on Linux the search stops at the mount holding it), and NE extensions
#   that must match a file in the working directory. The child gets 30 ms,
#   then SIGKILL. Prints per segment "ID US hit|miss|timeout RS". The
#   working directory is named by __gpy_g_pwd when set, else PWD, so a caller
#   can start Bash without PWD in its environment or argv.
#
# Process rules per run (batch): the reader takes the segments in deadline
# order, so each script group gets TERM and CONT at its own deadline (or on
# Ctrl-C), and KILL after a 50 ms grace or once it exited. The prompt never
# waits for that grace on a timeout: with Bash 3.2-4.4 the timer group sends
# the KILL; with a clock a detached killer, started at the first TERM, sends
# each group its KILL 50 ms after its deadline (after its TERM when that came
# first), so a group the reader reaches late gets KILL sooner. After a
# normal exit, members left in the group get TERM and CONT, then KILL within
# the grace. Records travel in-band ("RS TOKEN kind RS"); TOKEN is random
# per batch, so script output cannot forge one. Bash 5 reads against a
# clock; Bash 3.2-4.4 start one timer group per segment (absolute sleep
# path: a PATH search stats the working directory first, which can hang).

__gpy_g_cap=4096
__gpy_g_grace=0.05
__gpy_g_grace_us=50000
__gpy_g_lag_us=200000
__gpy_g_scan_t=0.03
__gpy_g_sleep=/bin/sleep
[[ -x $__gpy_g_sleep ]] || __gpy_g_sleep=/usr/bin/sleep
__gpy_g_clock=
[[ -n ${EPOCHREALTIME-} ]] && __gpy_g_clock=1
# Runs in the script process as `sh -c PRO SCRIPT`: reads the exports on fd 3,
# then sources the script, so $0 is the script path as with `sh SCRIPT`.
# shellcheck disable=SC2016  # the script text must stay literal for the child sh
__gpy_g_pro='__gpy_s=
while IFS= read -r __gpy_l; do __gpy_s="$__gpy_s$__gpy_l
"; done <&3
exec 3<&-
eval "$__gpy_s"
unset __gpy_s __gpy_l
. "$0"'

# Single-quote $1 for sh into __gpy_g_qv.
__gpy_g_quote() {
    local q="'" r="'\\''" v
    v=${1//"$q"/"$r"}
    __gpy_g_qv="'$v'"
}

# PATH $1 without empty, dot or relative entries into __gpy_g_qv, quoted for sh.
# Both the base PATH and a declared PATH go through here, so no declared value
# brings a relative entry back.
__gpy_g_path() {
    local p=$1: e out=
    while [[ -n $p ]]; do
        e=${p%%:*}
        p=${p#*:}
        [[ $e == /* ]] && out=${out:+$out:}$e
    done
    __gpy_g_quote "$out"
}

# The clean base environment as sh code into __gpy_g_basev: the cleaned PATH,
# then these names and every LC_* when set.
# Runs before the caller pins LC_ALL=C, so it forwards the real locale.
__gpy_g_base() {
    local s= n
    if [[ -n ${PATH+x} ]]; then
        __gpy_g_path "$PATH"
        s="export PATH=$__gpy_g_qv"$'\n'
    fi
    for n in HOME USER LOGNAME LANG TERM TZ TMPDIR "${!LC_@}"; do
        [[ -n ${!n+x} ]] || continue
        __gpy_g_quote "${!n}"
        s+="export $n=$__gpy_g_qv"$'\n'
    done
    __gpy_g_basev=$s
}

# Producer for segment $1, inside <( ): starts the group and its timer,
# then writes the records. Reads id cwd cmd req pay t tok sh of the batch.
__gpy_g_prod() {
    local c w= i s
    for ((i = 3; i < 3 + $1; i++)); do
        eval "exec $i<&-"
    done
    set -m
    {
        set +m
        if [[ -n $cwd && $cwd != "$PWD" ]]; then
            cd -- "$cwd" || exit 126
        fi
        if [[ -n $req ]]; then
            (exec -c -a sh "$sh" -c "$__gpy_g_pro" "$req" 3< <(printf '%s' "$pay") >/dev/null)
            # shellcheck disable=SC2181  # status of the subshell above
            if [[ $? != 0 ]]; then
                printf '\036%s q\036' "$tok"
                exit 1
            fi
        fi
        exec -c -a sh "$sh" -c "$__gpy_g_pro" "$cmd" 3< <(printf '%s' "$pay")
    } </dev/null 2>/dev/null &
    c=$!
    if [[ -z $__gpy_g_clock ]]; then
        {
            set +m
            trap '' INT PIPE
            "$__gpy_g_sleep" "$t"
            kill -TERM -"$c"
            kill -CONT -"$c"
            printf '\036%s t\036' "$tok"
            "$__gpy_g_sleep" "$__gpy_g_grace"
            kill -KILL -"$c"
        } </dev/null 2>/dev/null &
        w=$!
    fi
    set +m
    trap 'printf "\036%s i\036" "$tok"; exit 130' INT
    printf '\036%s p %s %s\036' "$tok" "$c" "$w"
    wait "$c"
    s=$?
    [[ -n $w ]] && kill -TERM -"$w"
    # Leftovers of a normal exit: TERM and CONT, then KILL within the grace.
    if kill -0 -"$c"; then
        kill -TERM -"$c"
        kill -CONT -"$c"
        for i in 1 2 3 4 5; do
            "$__gpy_g_sleep" 0.01
            kill -0 -"$c" || break
        done
        kill -KILL -"$c"
    fi
    printf '\036%s r %s\036' "$tok" "$s"
}

# Killer of a clock reader, inside >( ): reads "TIME PGID" lines (TIME in
# epoch microseconds, in rising order) and sends each group KILL at its TIME.
# Closes the $1 producer descriptors, so they still see their reader go.
__gpy_g_kill() {
    local i x c
    trap '' INT HUP PIPE
    for ((i = 3; i < 3 + $1; i++)); do
        eval "exec $i<&-"
    done
    exec >/dev/null
    while read -r x c; do
        i=${EPOCHREALTIME/[.,]/}
        i=$((x - 10#$i))
        if ((i > 0)); then
            printf -v x '%d.%06d' $((i / 1000000)) $((i % 1000000))
            "$__gpy_g_sleep" "$x"
        fi
        kill -KILL -"$c"
    done
}

__gpy_g_batch() {
    set +euf
    exec 2>/dev/null
    shopt -u nocasematch
    local IFS=$' \t\n'
    local sh=$1 tok=g$RANDOM$RANDOM$RANDOM
    shift
    __gpy_g_base
    local LC_ALL=C
    local k=0 m n i j id ms cwd cmd req pay ref name t start= left rem ch
    local c w f r q o sep intr= late= kf= v x y s
    local -a ids ts ord ds cs ws fs rs qs os
    if [[ -n $__gpy_g_clock ]]; then
        start=${EPOCHREALTIME/[.,]/}
        start=$((10#$start))
        # The caller start, when it is a plausible past instant.
        x=${__gpy_g_t0-}
        if [[ $x == [1-9]* && $x != *[!0-9]* && ${#x} -le 17 ]]; then
            x=$((start - x))
            ((x > __gpy_g_lag_us)) && x=$__gpy_g_lag_us
            ((x > 0)) && start=$((start - x))
        fi
    fi
    # Launch every producer first (one shared start), then read them in
    # deadline order, so no read outlasts the deadline of a segment still unread.
    while (($# >= 6 && k < 40)); do
        id=$1 ms=$2 cwd=$3 cmd=$4 req=$5 n=$6
        shift 6
        if [[ -n $n && $n != *[!0-9]* ]] && ((10#$n <= $#)); then :; else break; fi
        n=$((10#$n))
        pay=$__gpy_g_basev
        for ((i = 0; i < n; i++)); do
            name=$1
            shift
            [[ -n $name && $name != [0-9]* && $name != *[!A-Za-z0-9_]* ]] || continue
            ref=__gpy_g_v${k}_$name
            [[ -n ${!ref+x} ]] || continue
            if [[ $name == PATH ]]; then
                __gpy_g_path "${!ref}"
            else
                __gpy_g_quote "${!ref}"
            fi
            pay+="export $name=$__gpy_g_qv"$'\n'
        done
        ids[k]=$id
        ts[k]=0
        fs[k]=
        if [[ -n $id && $id != *[!a-z0-9_]* && -n $ms && $ms != *[!0-9]* && ${#ms} -le 3 ]] &&
            ((10#$ms >= 1 && 10#$ms <= 500)) && [[ $sh == /* && $cmd == /* ]] &&
            [[ -z $cwd || $cwd == /* ]] &&
            [[ -z $req || $req == /* ]]; then
            ts[k]=$((10#$ms))
            printf -v t '0.%03d' "${ts[k]}"
            [[ $ms == 500 ]] && t=0.5
            [[ -n $start ]] && ds[k]=$((start + ts[k] * 1000))
            eval "exec $((3 + k))< <(__gpy_g_prod $k)"
        else
            fs[k]=x
        fi
        # ord lists the segments by deadline, equal ones in batch order.
        i=$k
        while ((i > 0)); do
            ((ts[ord[i - 1]] > ts[k])) || break
            ord[i]=${ord[i - 1]}
            i=$((i - 1))
        done
        ord[i]=$k
        k=$((k + 1))
    done
    # From here the reader notes INT itself. Bash 3.2 as a non-interactive
    # child starts <( ) with SIGINT ignored and runs this trap only after
    # read -t returns, so there a Ctrl-C ends the run at its deadline.
    trap 'intr=1' INT TERM HUP
    for ((j = 0; j < k; j++)); do
        m=${ord[j]}
        c= w= f=${fs[m]} r= q= o= sep=
        if [[ -z $f ]]; then
            rem=2 # Bash < 5: safety net only; the timer record ends the wait
            while :; do
                if [[ -n $__gpy_g_clock ]]; then
                    left=${EPOCHREALTIME/[.,]/}
                    left=$((ds[m] - 10#$left))
                    # Past the deadline: still take what is already in the pipe.
                    if ((left > 0)); then
                        printf -v rem '%d.%06d' $((left / 1000000)) $((left % 1000000))
                    else
                        rem=0.001
                    fi
                fi
                [[ -n $intr && -n $__gpy_g_clock ]] && rem=0.001
                if IFS= read -r -u $((3 + m)) -d $'\036' -n $((__gpy_g_cap + 64)) -t "$rem" ch; then
                    case $ch in
                        "$tok p "*)
                            c=${ch#"$tok p "}
                            w=${c#* }
                            c=${c%% *}
                            sep=
                            ;;
                        "$tok r "*)
                            r=${ch#"$tok r "}
                            break
                            ;;
                        "$tok q") q=1 ;;
                        "$tok t")
                            f=t
                            break
                            ;;
                        "$tok i")
                            f=i
                            break
                            ;;
                        *)
                            o+=$sep$ch
                            sep=$'\036'
                            if ((${#o} > __gpy_g_cap)); then
                                f=c
                                break
                            fi
                            ;;
                    esac
                else
                    if [[ -n $intr ]]; then f=i; else f=t; fi
                    break
                fi
            done
            [[ -n $c && $c != *[!0-9]* ]] || { c= && f=${f:-x}; }
            # Bash 3.2 runs the INT trap only once the timer record ended the read.
            [[ -n $intr && $f == t ]] && f=i
        fi
        cs[m]=$c ws[m]=$w fs[m]=$f rs[m]=$r qs[m]=$q os[m]=$o
        [[ -n $c ]] || continue
        if [[ -n $f && -n $__gpy_g_clock ]]; then
            kill -TERM -"$c"
            kill -CONT -"$c"
            # KILL when the grace from the deadline (or from now, when
            # earlier) ends, from a detached killer, so the prompt does not
            # wait for it. The times reach it in rising order.
            x=${EPOCHREALTIME/[.,]/}
            x=$((10#$x))
            ((x > ds[m])) && x=${ds[m]}
            if [[ -z $kf ]]; then
                kf=$((3 + k))
                eval "exec $kf> >(__gpy_g_kill $k)"
            fi
            printf '%s %s\n' "$((x + __gpy_g_grace_us))" "$c" >&"$kf"
        elif [[ -n $f && $f != t ]]; then
            # The timer KILLs a timed-out group itself.
            kill -TERM -"$c"
            kill -CONT -"$c"
            [[ -n $w ]] && kill -TERM -"$w"
            late=1
        fi
    done
    if [[ -n $late ]]; then
        "$__gpy_g_sleep" "$__gpy_g_grace"
        for ((m = 0; m < k; m++)); do
            [[ -n ${fs[m]} && -n ${cs[m]} && ${fs[m]} != t ]] && kill -KILL -"${cs[m]}"
        done
    fi
    for ((m = 0; m < k; m++)); do
        eval "exec $((3 + m))<&-"
    done 2>/dev/null
    [[ -n $kf ]] && eval "exec $kf>&-"
    # Sanitize once: cap (above), split, strip C0 and DEL, then hide on C1.
    for ((m = 0; m < k; m++)); do
        r=${rs[m]} s= x= v=
        case ${fs[m]} in
            t) s=timeout ;;
            i) s=intr ;;
            c) s=cap ;;
            x) s=exit ;;
            *)
                if [[ -n ${qs[m]} ]]; then
                    s=req
                elif [[ $r != 0 ]]; then
                    s=exit
                else
                    s=ok
                    o=${os[m]}
                    x=${o%%$'\n'*}
                    if [[ $o == *$'\n'* ]]; then
                        v=${o#*$'\n'}
                        v=${v%%$'\n'*}
                        [[ -n $v && ${#v} -le 32 && $v != *[!a-z0-9_]* ]] || v=
                    fi
                    y=
                    n=${#x}
                    for ((i = 0; i < n; i += 64)); do
                        ch=${x:i:64}
                        y+=${ch//[$'\001'-$'\037'$'\177']/}
                    done
                    x=$y
                    if [[ $x == *$'\302'[$'\200'-$'\237']* ]]; then
                        s=c1 x= v=
                    fi
                fi
                ;;
        esac
        printf '%s\037%s\037%s\037%s\036' "${ids[m]}" "$s" "$x" "$v"
    done
}

# The scan itself, in the child: one "ID US hit|miss RS" per segment, then
# "Z" and GS once complete. Paths are relative to the working directory.
__gpy_g_scan_body() {
    local pwd=$1 mr=/ rest a b c d mp e id nf nd ne i res n f r
    shift
    if [[ -r /proc/self/mountinfo ]]; then
        while read -r a b c d mp e; do
            case $mp in *\\*) printf -v mp '%b' "$mp" ;; esac
            case $pwd/ in "$mp"/*) ((${#mp} > ${#mr})) && mr=$mp ;; esac
        done </proc/self/mountinfo
    fi
    rest=${pwd#"${mr%/}"}
    shopt -s nullglob
    shopt -u failglob nocaseglob dotglob
    unset GLOBIGNORE
    while (($# >= 4)); do
        id=$1 nf=$2 nd=$3 ne=$4
        shift 4
        res=hit
        for ((i = 0; i < nf + nd; i++)); do
            n=$1
            shift
            [[ $res == hit ]] || continue
            r= f=$rest
            while :; do
                if ((i < nf)); then
                    [[ -f $r$n ]] && break
                else
                    [[ -d $r$n ]] && break
                fi
                [[ -n $f ]] || {
                    res=miss
                    break
                }
                f=${f%/*}
                r=../$r
            done
        done
        for ((i = 0; i < ne; i++)); do
            n=$1
            shift
            [[ $res == hit ]] || continue
            r=
            for f in *."$n"; do
                if [[ -f $f ]]; then
                    r=1
                    break
                fi
            done
            [[ -n $r ]] || res=miss
        done
        printf '%s\037%s\036' "$id" "$res"
    done
    printf 'Z\035'
}

__gpy_g_scan() {
    set +euf
    exec 2>/dev/null
    local IFS=$' \t\n'
    local LC_ALL=C
    local pwd=${__gpy_g_pwd:-$PWD} body= pid= c w nf nd ne id
    if ((BASH_VERSINFO[0] >= 4)); then
        # The child names itself, so a stale $! is never signalled.
        exec 3< <(
            printf '%s\n' "$BASHPID"
            __gpy_g_scan_body "$pwd" "$@"
        )
        IFS= read -r -t "$__gpy_g_scan_t" -u 3 pid
        if ! IFS= read -r -d $'\035' -t "$__gpy_g_scan_t" -u 3 body || [[ $body != *Z ]]; then
            [[ -n $pid && $pid != *[!0-9]* ]] && kill -KILL "$pid"
            body=
        fi
    else
        # Bash 3.2 has only whole-second read -t: the child races the scan
        # against an absolute sleep; read -t 1 is the safety net.
        exec 3< <(
            __gpy_g_scan_body "$pwd" "$@" &
            c=$!
            (
                "$__gpy_g_sleep" "$__gpy_g_scan_t"
                kill -KILL "$c"
            ) &
            w=$!
            wait "$c"
            kill "$w"
            printf '\035'
        )
        IFS= read -r -d $'\035' -t 1 -u 3 body || body=
        [[ $body == *Z ]] || body=
    fi
    exec 3<&-
    if [[ -n $body ]]; then
        printf '%s' "${body%Z}"
        return 0
    fi
    while (($# >= 4)); do
        id=$1 nf=$2 nd=$3 ne=$4
        shift 4
        [[ $nf$nd$ne != *[!0-9]* ]] || return 0
        shift $((nf + nd + ne))
        printf '%s\037timeout\036' "$id"
    done
}
