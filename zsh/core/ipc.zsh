# zsh/core/ipc.zsh

# For $EPOCHSECONDS when sourced standalone.
zmodload zsh/datetime 2>/dev/null
# zstat only: plain zsh/stat would shadow the external `stat`.
zmodload -F zsh/stat b:zstat 2>/dev/null

# A flag is off only when set to `0` or `false`; unset, empty and any other
# value leave it on. Fish and Bash apply the same rule.
# Gates every start/register/talk path; rendering still works via oneshot.
function __gpy_agent_enabled() {
    [[ "${GPY_AGENT_ENABLED:-}" != (0|false) ]]
}

# Governs restarts only; agent start and registration use __gpy_agent_enabled.
function __gpy_supervisor_enabled() {
    [[ "${GPY_AGENT_SUPERVISOR_ENABLED:-}" != (0|false) ]]
}

# Escape $2 for a JSON string into variable $1 without a fork. Backslashes
# first so later escapes aren't re-escaped. Locals are __gpy_ej_* so a
# caller's OUTVAR is not shadowed.
function __gpy_escape_json_into() {
    local __gpy_ej_s=$2
    __gpy_ej_s=${__gpy_ej_s//\\/\\\\}
    __gpy_ej_s=${__gpy_ej_s//\"/\\\"}
    __gpy_ej_s=${__gpy_ej_s//$'\n'/\\n}
    __gpy_ej_s=${__gpy_ej_s//$'\r'/\\r}
    __gpy_ej_s=${__gpy_ej_s//$'\t'/\\t}
    printf -v "$1" '%s' "$__gpy_ej_s"
}

# Printing form, kept for the vector tests.
function __gpy_escape_json() {
    local __gpy_ej_out
    __gpy_escape_json_into __gpy_ej_out "$1"
    # -r: echo-style escape handling would collapse the doubled backslashes.
    print -r -- "$__gpy_ej_out"
}

# $VIRTUAL_ENV, else $CONDA_PREFIX unless the conda env is `base`, into
# variable $1 (empty when neither applies). Writes via `printf -v`: no fork.
function __gpy_forwarded_venv_into() {
    if [[ -n "${VIRTUAL_ENV:-}" ]]; then
        printf -v "$1" '%s' "$VIRTUAL_ENV"
    elif [[ -n "${CONDA_PREFIX:-}" && -n "${CONDA_DEFAULT_ENV:-}" && "$CONDA_DEFAULT_ENV" != "base" ]]; then
        printf -v "$1" '%s' "$CONDA_PREFIX"
    else
        printf -v "$1" '%s' ""
    fi
}

# `,"virtual_env":"<venv>"` for a `lang` request, or nothing, into variable
# $1. Locals are __gpy_lvj_* so a caller's OUTVAR is not shadowed.
function __gpy_lang_venv_json_into() {
    local __gpy_lvj_venv
    __gpy_forwarded_venv_into __gpy_lvj_venv
    if [[ -z "$__gpy_lvj_venv" ]]; then
        printf -v "$1" '%s' ""
        return 0
    fi
    __gpy_escape_json_into __gpy_lvj_venv "$__gpy_lvj_venv"
    printf -v "$1" ',"virtual_env":"%s"' "$__gpy_lvj_venv"
}

# HOME with the agent's passwd fallback: zsh keeps an empty $HOME verbatim,
# where the agent uses passwd; `~user` consults passwd directly. Writes via
# `printf -v` to avoid a per-render fork. The local is not named `home` because
# dynamic scoping would shadow a caller's OUT_VAR of that name.
function __gpy_home() {
    local out_var=$1 __gpy_home_value=${HOME:-} __gpy_home_user
    if [[ -z "$__gpy_home_value" ]]; then
        __gpy_home_user=${USERNAME:-}
        [[ -n "$__gpy_home_user" ]] || __gpy_home_user="$(id -un 2>/dev/null)"
        [[ -n "$__gpy_home_user" ]] || return 1
        __gpy_home_value=~$__gpy_home_user
    fi
    [[ "$__gpy_home_value" == /* ]] || return 1
    printf -v "$out_var" '%s' "$__gpy_home_value"
}

# XDG vars count only when non-empty and absolute, matching the agent.
function __gpy_ipc_endpoint() {
    local home
    if [[ -n "${GPY_AGENT_SOCKET_PATH:-}" ]]; then
        print -r -- "$GPY_AGENT_SOCKET_PATH"
        return
    fi

    if [[ -n "${XDG_RUNTIME_DIR:-}" && "${XDG_RUNTIME_DIR:-}" == /* ]]; then
        print -r -- "$XDG_RUNTIME_DIR/gpy/gpy.sock"
        return
    fi

    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        print -r -- "$XDG_CACHE_HOME/gpy/gpy.sock"
        return
    fi

    if __gpy_home home; then
        print -r -- "$home/.cache/gpy/gpy.sock"
        return
    fi

    print -r -- "/tmp/gpy/gpy.sock"
}

# Mirrors the agent's paths::runtime_root_for. Writes into variable $1 so hot
# paths need no `$(...)` fork.
function __gpy_runtime_root() {
    local __gpy_rr_home
    if [[ -n "${XDG_RUNTIME_DIR:-}" && "${XDG_RUNTIME_DIR:-}" == /* ]]; then
        printf -v "$1" '%s' "$XDG_RUNTIME_DIR/gpy"
    elif [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        printf -v "$1" '%s' "$XDG_CACHE_HOME/gpy"
    elif __gpy_home __gpy_rr_home; then
        printf -v "$1" '%s' "$__gpy_rr_home/.cache/gpy"
    else
        printf -v "$1" '%s' "/tmp/gpy"
    fi
}

# The file name is the PID the agent signals; doorbell flags live beside it.
function __gpy_shell_registry_dir() {
    local root
    __gpy_runtime_root root
    print -r -- "$root/shells"
}
function __gpy_shell_registry_file() {
    print -r -- "$(__gpy_shell_registry_dir)/$$"
}

# The agent only nudges PIDs listed here. Caches the flag base so TRAPURG
# needs no fork.
typeset -g __gpy_shell_flag_base=""
function __gpy_track_shell_for_agent_recovery() {
    local dir
    dir=$(__gpy_shell_registry_dir)
    __gpy_shell_flag_base="$dir/$$"
    mkdir -p "$dir" 2>/dev/null || return 0
    print -r -- "$$" >"$dir/$$" 2>/dev/null || true
}

function __gpy_untrack_shell_for_agent_recovery() {
    local file
    file=$(__gpy_shell_registry_file)
    rm -f "$file" "$file.reload" "$file.reregister" 2>/dev/null || true
    # (N): an unmatched glob must not print "no matches found" on exit.
    local throttle_dir
    __gpy_runtime_root throttle_dir
    throttle_dir+="/refresh-throttle"
    rm -f "$throttle_dir"/$$.*(N) 2>/dev/null || true
}

# Instant-prompt cache dir into variable $1; non-zero status when unresolvable.
function __gpy_instant_cache_dir() {
    local __gpy_icd_home
    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        printf -v "$1" '%s' "$XDG_CACHE_HOME/gpy/instant-prompts"
    elif __gpy_home __gpy_icd_home; then
        printf -v "$1" '%s' "$__gpy_icd_home/.cache/gpy/instant-prompts"
    else
        return 1
    fi
}

# Cached Zsh theme export, written atomically by the agent.
function __gpy_theme_export_cache_path() {
    local home

    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        print -r -- "$XDG_CACHE_HOME/gpy/theme-export.zsh"
        return 0
    fi

    if __gpy_home home; then
        print -r -- "$home/.cache/gpy/theme-export.zsh"
        return 0
    fi

    return 1
}

# 150 -> "0.150" without an awk fork.
function __gpy_ms_to_secs() {
    local ms=$1 out_var=$2
    [[ "$ms" =~ ^[0-9]+$ ]] || ms=150
    printf -v "$out_var" '%d.%03d' "$(( ms / 1000 ))" "$(( ms % 1000 ))"
}

# The budget for a segment the prompt has nothing to show for, into variable
# $1: GPY_IPC_COLD_TIMEOUT_MS, never below GPY_IPC_TIMEOUT_MS. A non-numeric
# value falls back to its default. Locals are __gpy_ct_* so a caller's OUTVAR
# is not shadowed.
function __gpy_cold_timeout_ms_into() {
    local __gpy_ct_cold=${GPY_IPC_COLD_TIMEOUT_MS:-} __gpy_ct_warm=${GPY_IPC_TIMEOUT_MS:-}
    [[ "$__gpy_ct_cold" =~ ^[0-9]+$ ]] || __gpy_ct_cold=500
    [[ "$__gpy_ct_warm" =~ ^[0-9]+$ ]] || __gpy_ct_warm=150
    (( 10#$__gpy_ct_cold < 10#$__gpy_ct_warm )) && __gpy_ct_cold=$__gpy_ct_warm
    printf -v "$1" '%d' "$(( 10#$__gpy_ct_cold ))"
}

# netcat-traditional lacks -U. Probed at source time too, because segment
# subshells cannot write the cache back.
typeset -g __gpy_nc_supports_unix=""
function __gpy_nc_probe_capabilities() {
    [[ -n "$__gpy_nc_supports_unix" ]] && return 0
    local help
    __gpy_nc_supports_unix=0
    if (( $+commands[nc] )); then
        help="$(nc -h 2>&1)"
        [[ "$help" == *-U* ]] && __gpy_nc_supports_unix=1
    fi
    return 0
}
if __gpy_agent_enabled && ! (( $+commands[socat] )); then
    __gpy_nc_probe_capabilities
fi

function __gpy_ipc_client_available() {
    (( $+commands[socat] )) && return 0
    (( $+commands[nc] )) || return 1
    __gpy_nc_probe_capabilities
    [[ "$__gpy_nc_supports_unix" == 1 ]]
}

# Embedded reply newline: the agent sends `\n` as RS so a reply stays one line.
typeset -g __gpy_nl=$'\n'

# Prints the reply. Returns 0 on a complete reply, 1 when unreachable, 2 when
# the request was sent but no complete reply arrived in budget (the caller
# must not recompute via oneshot; it omits the segment).
#
# `read` fails when timeout/EOF hits before the newline, which is how a
# truncated reply is rejected; `$(...)` strips the newline either way, so
# socat/nc use process substitution to keep `read` in this shell.
# The client's exit status trails as `\037<status>\n` so `read` can tell a
# refused connect (1) from a late agent (0, or 124 from `timeout`).
# `exec 2>/dev/null`: with SIGPIPE ignored (inherited from Python/Node/CI),
# writes after `read` closes its end print "Broken pipe" into the prompt.
function __gpy_send_json() {
    # Agent-free mode never uses another shell's daemon.
    __gpy_agent_enabled || return 1

    local json=$1
    local socket_path=$(__gpy_ipc_endpoint)
    local response="" read_status=1 client_status="" connected_codes=" 0 "
    local zsocket_connected=0 timeout_secs
    # One reply budget for every transport: $2 in ms, else GPY_IPC_TIMEOUT_MS.
    __gpy_ms_to_secs "${2:-${GPY_IPC_TIMEOUT_MS:-}}" timeout_secs

    if [[ ! -S "$socket_path" ]]; then
        return 1
    fi

    # zsh/net/socket: builtin, fastest.
    if zmodload zsh/net/socket 2>/dev/null; then
        local fd
        if zsocket "$socket_path" 2>/dev/null; then
            zsocket_connected=1
            fd=$REPLY
            # -r: escape interpretation would undo __gpy_escape_json.
            print -r -u $fd -- "$json"
            # `IFS=`: default IFS strips the trailing space after `❯`; -r keeps ANSI backslashes.
            IFS= read -r -u $fd -t $timeout_secs response
            read_status=$?
            exec {fd}>&-
        fi
    fi

    # Once connected the request is on the wire; falling through would resend
    # it on a second connection.
    if (( zsocket_connected )); then
        if [[ $read_status -eq 0 && -n "$response" ]]; then
            # The agent sends embedded newlines as RS (\036) to keep one reply line.
            print -r -- "${response//$'\036'/${__gpy_nl}}"
            return 0
        fi
        return 2
    fi

    read_status=1
    if (( $+commands[socat] )); then
        IFS= read -r response < <(exec 2>/dev/null; print -r -- "$json" | socat -t $timeout_secs - UNIX-CONNECT:"$socket_path" 2>/dev/null; printf '\037%s\n' "$?")
        read_status=$?

    # nc -w only takes whole seconds; `timeout` gives a tighter bound.
    elif __gpy_ipc_client_available; then
        if (( $+commands[timeout] )); then
            IFS= read -r response < <(exec 2>/dev/null; print -r -- "$json" | timeout "$timeout_secs" nc -U "$socket_path" -w 1 2>/dev/null; printf '\037%s\n' "$?")
            # 124: `timeout` killed an nc that had connected and was waiting.
            connected_codes=" 0 124 "
        else
            IFS= read -r response < <(exec 2>/dev/null; print -r -- "$json" | nc -U "$socket_path" -w 1 2>/dev/null; printf '\037%s\n' "$?")
        fi
        read_status=$?
    else
        return 1
    fi

    if [[ "$response" == *$'\037'* ]]; then
        client_status=${response##*$'\037'}
        response=""
    fi

    if [[ $read_status -eq 0 && -n "$response" ]]; then
        print -r -- "${response//$'\036'/${__gpy_nl}}"
        return 0
    fi

    # A connected client or empty reply line means the agent is alive but late.
    if [[ -n "$client_status" ]]; then
        [[ "$connected_codes" == *" $client_status "* ]] && return 2
    elif [[ $read_status -eq 0 ]]; then
        return 2
    fi
    return 1
}

# Must match PROTOCOL_VERSION in gpy-agent/src/ipc/protocol/mod.rs.
typeset -g GPY_EXPECTED_PROTOCOL_VERSION=2

# 1 only on a confirmed mismatch; an unqueryable agent passes.
function __gpy_check_protocol_version() {
    local response
    response=$(__gpy_send_json '{"op":"status"}' 2>/dev/null)
    [[ -n "$response" ]] || return 0

    local protocol_version=0
    if [[ $response =~ '"protocol_version":([0-9]+)' ]]; then
        protocol_version="${match[1]}"
    fi

    if [[ "$protocol_version" != "$GPY_EXPECTED_PROTOCOL_VERSION" ]]; then
        print -r -- "gpy: WARNING: agent protocol version $protocol_version does not match expected $GPY_EXPECTED_PROTOCOL_VERSION; restart the agent (gpy-agent stop && gpy-agent start)" >&2
        return 1
    fi

    return 0
}

# Git root above $1 into variable $2; status 1 when there is none.
function __gpy_find_git_root() {
    local __gpy_fgr_dir
    __gpy_canonical_path "${1:-$PWD}" __gpy_fgr_dir || return 1

    while [[ "$__gpy_fgr_dir" != "/" ]]; do
        if [[ -e "$__gpy_fgr_dir/.git" ]]; then
            printf -v "$2" '%s' "$__gpy_fgr_dir"
            return 0
        fi
        # `${dir:h}` avoids a dirname fork per level; `/a` yields `/`.
        __gpy_fgr_dir=${__gpy_fgr_dir:h}
    done

    return 1
}

# Canonical path of existing path $1 into variable $2. Symlink resolution is
# part of the cache-key contract with the agent's canonicalize; `:A` resolves
# through realpath(3) like realpath(1), without a fork.
function __gpy_canonical_path() {
    [[ -e "$1" ]] || return 1
    printf -v "$2" '%s' "${1:A}"
}

# Cache key for path $1 into variable $2. Must match the agent's injective
# encoding; escape _ first.
function __gpy_path_to_cache_key() {
    local __gpy_pk_key="${1//_/__}"
    __gpy_pk_key="${__gpy_pk_key//\//_s}"
    __gpy_pk_key="${__gpy_pk_key//\\/_b}"
    __gpy_pk_key="${__gpy_pk_key//:/_c}"
    __gpy_pk_key="${__gpy_pk_key// /_w}"
    printf -v "$2" '%s' "$__gpy_pk_key"
}

# Rewrites the key in place: unchanged up to 200 chars, else 50-char chunks
# joined by `/` to stay under NAME_MAX. MUST match the agent's
# cache_key_path_for. Under LC_ALL=C lengths count bytes, so a long non-ASCII
# key just misses the cache.
function __gpy_chunk_cache_key() {
    local __gpy_ck_key=${(P)1} __gpy_ck_stem= __gpy_ck_i
    (( ${#__gpy_ck_key} > 200 )) || return 0
    for (( __gpy_ck_i = 0; __gpy_ck_i < ${#__gpy_ck_key}; __gpy_ck_i += 50 )); do
        __gpy_ck_stem+="${__gpy_ck_stem:+/}${__gpy_ck_key:$__gpy_ck_i:50}"
    done
    : ${(P)1::=$__gpy_ck_stem}
}

# Token for prev_bg $1 into variable $2. MUST match the agent's `prev_bg_token`.
function __gpy_prev_bg_token() {
    if [[ -z "$1" ]]; then
        printf -v "$2" '%s' "none"
    else
        printf -v "$2" '%s' "${1//[^a-zA-Z0-9]/_}"
    fi
}

# Suffix for base $1, is_last $2, is_first $3 into variable $4. MUST match
# the agent's cache variant names (`CacheFileName`).
function __gpy_cache_variant_suffix() {
    local __gpy_cvs_suffix=$1
    [[ "$3" == "true" ]] && __gpy_cvs_suffix+="_first"
    [[ "$2" == "true" ]] && __gpy_cvs_suffix+="_last"
    printf -v "$4" '%s' "$__gpy_cvs_suffix"
}

# Git caches key by repo root; language caches outside Git key by the
# canonical cwd, as the agent does. Status 1 when there is none.
function __gpy_instant_cache_key_path() {
    local suffix=$1 cwd=$2 __gpy_kp=""
    __gpy_find_git_root "$cwd" __gpy_kp
    if [[ -z "$__gpy_kp" && "$suffix" == lang* ]]; then
        __gpy_canonical_path "$cwd" __gpy_kp
    fi
    printf -v "$3" '%s' "$__gpy_kp"
    [[ -n "$__gpy_kp" ]]
}

# Any variant or prev_bg token counts: `.none` exists only if some request
# carried no prev_bg.
function __gpy_instant_cache_present() {
    # `(N)`: an unmatched pattern is an empty list, not a NOMATCH error.
    emulate -L zsh
    local base=$1 cwd=${2:-$PWD} key_path cache_dir cache_key
    __gpy_instant_cache_key_path "$base" "$cwd" key_path || return 1
    __gpy_instant_cache_dir cache_dir || return 1
    __gpy_path_to_cache_key "$key_path" cache_key
    __gpy_chunk_cache_key cache_key
    local -a hits=("$cache_dir/$cache_key.$base."*.zsh(N) "$cache_dir/$cache_key.${base}_"*.zsh(N))
    (( ${#hits} > 0 ))
}

# Serve-stale read of the entry for suffix $1, cwd $2, prev_bg $3 into
# variable $4 ("" on a miss). Status: 0 fresh, 1 miss, +2 stale, +4 served
# via `.none` fallback.
function __gpy_read_instant_cache() {
    local __gpy_ric_suffix="${1:-git}" __gpy_ric_cwd="${2:-$PWD}" __gpy_ric_prev_bg="${3:-}"
    local __gpy_ric_key_path __gpy_ric_dir __gpy_ric_key __gpy_ric_token __gpy_ric_file __gpy_ric_text=""
    local __gpy_ric_now __gpy_ric_mtime __gpy_ric_status=0 __gpy_ric_ttl=5 __gpy_ric_default_ttl=5
    printf -v "$4" '%s' ""
    __gpy_instant_cache_key_path "$__gpy_ric_suffix" "$__gpy_ric_cwd" __gpy_ric_key_path || return 1
    __gpy_instant_cache_dir __gpy_ric_dir || return 1

    # The ANSI bakes in the fg:prev_bg chevron, so the key includes prev_bg.
    # Fall back to `.none`; the caller's refresh fills the right token file.
    __gpy_path_to_cache_key "$__gpy_ric_key_path" __gpy_ric_key
    __gpy_chunk_cache_key __gpy_ric_key
    __gpy_prev_bg_token "$__gpy_ric_prev_bg" __gpy_ric_token
    # `.ansi` files are fish's and must never reach PROMPT.
    __gpy_ric_file="$__gpy_ric_dir/$__gpy_ric_key.$__gpy_ric_suffix.$__gpy_ric_token.zsh"
    if [[ ! -f "$__gpy_ric_file" && "$__gpy_ric_token" != "none" ]]; then
        __gpy_ric_file="$__gpy_ric_dir/$__gpy_ric_key.$__gpy_ric_suffix.none.zsh"
        __gpy_ric_status=4
    fi
    [[ -f "$__gpy_ric_file" ]] || return 1

    local -a __gpy_ric_stat
    __gpy_ric_now=$EPOCHSECONDS
    # zsh/stat's zstat when available; else GNU stat first: on Linux `stat -f`
    # succeeds with non-numeric output.
    if zstat -A __gpy_ric_stat +mtime -- "$__gpy_ric_file" 2>/dev/null; then
        __gpy_ric_mtime=${__gpy_ric_stat[1]}
    else
        __gpy_ric_mtime=$(stat -c %Y "$__gpy_ric_file" 2>/dev/null || stat -f %m "$__gpy_ric_file" 2>/dev/null)
    fi
    if [[ "$__gpy_ric_now" =~ ^[0-9]+$ && "$__gpy_ric_mtime" =~ ^[0-9]+$ ]]; then
        if [[ "$__gpy_ric_suffix" == git* ]]; then
            __gpy_ric_ttl="${GPY_GIT_INSTANT_CACHE_TTL_SECONDS:-5}"
        elif [[ "$__gpy_ric_suffix" == lang* ]]; then
            __gpy_ric_default_ttl=30
            __gpy_ric_ttl="${GPY_LANGUAGE_CACHE_TTL_SECONDS:-30}"
        fi
        [[ "$__gpy_ric_ttl" =~ ^[0-9]+$ ]] || __gpy_ric_ttl="$__gpy_ric_default_ttl"
        (( __gpy_ric_now - __gpy_ric_mtime >= __gpy_ric_ttl )) && __gpy_ric_status=$((__gpy_ric_status + 2))
    fi

    # `read` in this shell, not `$(<file)` in a subshell; trailing newlines
    # are dropped as command substitution would.
    IFS= read -r -d '' __gpy_ric_text <"$__gpy_ric_file"
    while [[ "$__gpy_ric_text" == *$'\n' ]]; do __gpy_ric_text="${__gpy_ric_text%$'\n'}"; done
    printf -v "$4" '%s' "$__gpy_ric_text"
    return "$__gpy_ric_status"
}

# Shared is_last/is_first/prev_bg JSON tail into variable $1. Absent flags are
# omitted (the agent defaults them to false); callers pass "true" or "".
# Locals are __gpy_jft_* so a caller's OUTVAR is not shadowed.
function __gpy_json_flags_tail_into() {
    local __gpy_jft_tail="" __gpy_jft_prev_bg
    [[ "$2" == "true" ]] && __gpy_jft_tail="${__gpy_jft_tail},\"is_last\":true"
    [[ "$3" == "true" ]] && __gpy_jft_tail="${__gpy_jft_tail},\"is_first\":true"
    if [[ -n "$4" ]]; then
        __gpy_escape_json_into __gpy_jft_prev_bg "$4"
        __gpy_jft_tail="${__gpy_jft_tail},\"prev_bg\":\"${__gpy_jft_prev_bg}\""
    fi
    printf -v "$1" '%s' "$__gpy_jft_tail"
}

# Shared by every git/lang request so the request shape stays in one place.
# Writes into variable $1; locals are __gpy_bdr_* so a caller's OUTVAR is not
# shadowed.
function __gpy_build_data_request_into() {
    local __gpy_bdr_op=$2 __gpy_bdr_format=${4:-zsh-prompt}
    local __gpy_bdr_cwd __gpy_bdr_tail __gpy_bdr_venv=""
    __gpy_escape_json_into __gpy_bdr_cwd "$3"
    __gpy_json_flags_tail_into __gpy_bdr_tail "${5:-}" "${7:-}" "${6:-}"
    [[ "$__gpy_bdr_op" == "lang" ]] && __gpy_lang_venv_json_into __gpy_bdr_venv
    printf -v "$1" '%s' "{\"op\":\"$__gpy_bdr_op\",\"cwd\":\"$__gpy_bdr_cwd\",\"format\":\"$__gpy_bdr_format\"${__gpy_bdr_tail}${__gpy_bdr_venv}}"
}

# Background refresh after serving stale cache. No oneshot fallback: without
# a daemon there is no repaint to deliver.
function __gpy_trigger_data_refresh() {
    local op=$1
    local cwd=${2:-$PWD}
    local is_last=${3:-false}
    local prev_bg=${4:-}
    local request

    __gpy_build_data_request_into request "$op" "$cwd" zsh-prompt "$is_last" "$prev_bg" ""
    __gpy_send_json "$request" >/dev/null 2>&1
}

# One bounded IPC round trip, never oneshot (unbounded). The agent updates
# its cache and notifies other shells itself. $6 is the reply budget in ms
# (default GPY_IPC_TIMEOUT_MS).
function __gpy_sync_data_request() {
    local op=$1 cwd=$2 is_last=${3:-} prev_bg=${4:-} is_first=${5:-} timeout_ms=${6:-}
    local request response
    __gpy_build_data_request_into request "$op" "$cwd" zsh-prompt "$is_last" "$prev_bg" "$is_first"
    response=$(__gpy_send_json "$request" "$timeout_ms" 2>/dev/null) || return 1
    [[ -n "$response" ]] || return 1
    printf '%s' "$response"
}

# Bit tests for __gpy_read_instant_cache's status.
function __gpy_cache_status_stale() { (( ($1 & 2) != 0 )); }
function __gpy_cache_status_variant() { (( ($1 & 4) != 0 )); }

# Epoch ms into variable $1.
function __gpy_now_ms() {
    integer __gpy_ms=$(( EPOCHREALTIME * 1000 ))
    printf -v "$1" '%s' "$__gpy_ms"
}

# Stamp filename for throttle bucket $1 into $2. A bucket of up to 200 bytes
# names the stamp itself; a longer one gets its 32-bit FNV-1a hash, which
# keeps the name under NAME_MAX for any path. The stamp stores the bucket, so
# a hash collision reads as no stamp rather than throttling another bucket.
# The last long bucket's name is memoized: the byte loop costs ~0.5 ms.
typeset -g __gpy_refresh_stamp_memo_bucket=""
typeset -g __gpy_refresh_stamp_memo_name=""
function __gpy_refresh_stamp_name() {
    # Byte semantics: NAME_MAX counts bytes, and so does the hash.
    emulate -L zsh
    setopt no_multibyte
    local __gpy_rs_bucket=$1 __gpy_rs_byte
    integer __gpy_rs_i __gpy_rs_hash=2166136261
    if (( ${#__gpy_rs_bucket} <= 200 )); then
        printf -v "$2" '%s.%s' "$$" "$__gpy_rs_bucket"
        return
    fi
    if [[ "$__gpy_rs_bucket" != "$__gpy_refresh_stamp_memo_bucket" ]]; then
        for (( __gpy_rs_i = 1; __gpy_rs_i <= ${#__gpy_rs_bucket}; __gpy_rs_i++ )); do
            __gpy_rs_byte=${__gpy_rs_bucket[__gpy_rs_i]}
            (( __gpy_rs_hash = ((__gpy_rs_hash ^ #__gpy_rs_byte) * 16777619) & 0xffffffff ))
        done
        printf -v __gpy_refresh_stamp_memo_name '%s.#%08x' "$$" "$__gpy_rs_hash"
        __gpy_refresh_stamp_memo_bucket=$__gpy_rs_bucket
    fi
    printf -v "$2" '%s' "$__gpy_refresh_stamp_memo_name"
}

# At most one background refresh per 500ms per op+suffix+path(+extra), run for
# the agent's side effects (cache write, doorbell).
function __gpy_maybe_refresh() {
    local op=$1 root=$2 cache_suffix=$3 is_last=${4:-} prev_bg=${5:-} is_first=${6:-} throttle_key_extra=${7:-}
    __gpy_agent_enabled || return 0

    local dir bucket stamp_name stamp stamped="" last_ms=0 now_ms path_key
    # On disk because segments run in subshells that can't keep in-memory state.
    __gpy_runtime_root dir
    dir+="/refresh-throttle"
    # The dot-free fields lead, so the bucket stays injective; the encoded
    # path has no spaces, so it is everything after the stamp's first space.
    __gpy_path_to_cache_key "$root" path_key
    bucket="$op.$cache_suffix.$throttle_key_extra.$path_key"
    __gpy_refresh_stamp_name "$bucket" stamp_name
    stamp="$dir/$stamp_name"
    __gpy_now_ms now_ms
    [[ -r "$stamp" ]] && IFS= read -r -d '' stamped <"$stamp"
    [[ "${stamped#* }" == "$bucket" ]] && last_ms=${stamped%% *}
    [[ "$last_ms" == <-> ]] || last_ms=0
    (( now_ms - last_ms >= 500 )) || return 0
    # The stamp is the only limit across renders: without one, skip this
    # refresh rather than send one per render. `>|` overrides NO_CLOBBER.
    if ! { print -rn -- "$now_ms $bucket" >|"$stamp"; } 2>/dev/null; then
        mkdir -p "$dir" 2>/dev/null && { print -rn -- "$now_ms $bucket" >|"$stamp"; } 2>/dev/null || return 0
    fi

    # Before the fork: a background child can't propagate __gpy_registered.
    __gpy_register_with_agent >/dev/null 2>&1

    __gpy_trigger_data_refresh "$op" "$root" "$is_last" "$prev_bg" >/dev/null 2>&1 &!
}

# Instant cache first for git/lang.
function __gpy_request() {
    local op=$1
    local context_path=$2
    local format=${3:-json}
    local is_last=${4:-false}
    local prev_bg=${5:-}
    local is_first=${6:-false}

    if [[ -z "$context_path" ]]; then
        context_path="$PWD"
    fi

    if [[ "$op" == "git" ]]; then
        local cache_suffix
        __gpy_cache_variant_suffix git "$is_last" "$is_first" cache_suffix
        local cached_result
        __gpy_read_instant_cache "$cache_suffix" "$context_path" "$prev_bg" cached_result
        if [[ -n "$cached_result" ]]; then
            print -r -- "$cached_result"
            return 0
        fi
    elif [[ "$op" == "lang" ]]; then
        local cache_suffix
        __gpy_cache_variant_suffix lang "$is_last" "$is_first" cache_suffix
        local cached_result
        __gpy_read_instant_cache "$cache_suffix" "$context_path" "$prev_bg" cached_result
        if [[ -n "$cached_result" ]]; then
            print -r -- "$cached_result"
            return 0
        fi
    fi

    local request timeout_ms=""
    __gpy_build_data_request_into request "$op" "$context_path" "$format" "$is_last" "$prev_bg" "$is_first"
    # A directory request means the memo had nothing to show.
    [[ "$op" == "directory" ]] && __gpy_cold_timeout_ms_into timeout_ms

    __gpy_send_json "$request" "$timeout_ms"
    local send_status=$?
    # 2: the agent is already computing it; omit rather than fork oneshot.
    (( send_status == 2 )) && return 1
    if (( send_status != 0 )); then
        # oneshot has no --prev-bg; its status becomes ours.
        __gpy_fallback_oneshot "$op" "$context_path" "$format" "$is_last" "$is_first"
    fi
}

# One oneshot fork per render: segments run in nested subshells, so the
# render loop sets __gpy_oneshot_used and later subshells inherit it by fork.
function __gpy_oneshot_claim() {
    [[ "${__gpy_oneshot_used:-0}" == "1" ]] && return 1
    return 0
}

# Returns GPY_SEG_STATUS_ONESHOT whenever it forked, output or not.
function __gpy_fallback_oneshot() {
    local op=$1
    # Not `path`: that local would replace $PATH and hide gpy-agent.
    local cwd=$2
    local format=$3
    local is_last=${4:-false}
    local is_first=${5:-false}

    (( $+commands[gpy-agent] )) || return 1
    __gpy_oneshot_claim || return 1

    local -a args
    args=("$op" --cwd "$cwd" --format "$format")
    if [[ "$is_last" != "true" ]]; then
        args+=(--not-last)
    fi
    if [[ "$is_first" == "true" ]]; then
        args+=(--first)
    fi

    gpy-agent oneshot $args 2>/dev/null
    return "$GPY_SEG_STATUS_ONESHOT"
}

function __gpy_request_duration() {
    local duration_ms=$1
    local is_last=${2:-false}
    local prev_bg=${3:-}
    local is_first=${4:-false}

    local flags_tail
    __gpy_json_flags_tail_into flags_tail "$is_last" "$is_first" "$prev_bg"
    local request="{\"op\":\"duration\",\"duration_ms\":${duration_ms},\"format\":\"zsh-prompt\"${flags_tail}}"

    local result send_status
    result=$(__gpy_send_json "$request")
    send_status=$?

    # Status 2 never forks oneshot: the agent is already computing it.
    local used_oneshot=0
    if [[ -z "$result" ]] && (( send_status != 2 )) && (( $+commands[gpy-agent] )) && __gpy_oneshot_claim; then
        used_oneshot=1
        local -a args
        args=(duration --duration-ms "$duration_ms" --format zsh-prompt)
        if [[ "$is_last" != "true" ]]; then
            args+=(--not-last)
        fi
        if [[ "$is_first" == "true" ]]; then
            args+=(--first)
        fi
        result=$(gpy-agent oneshot $args 2>/dev/null)
    fi

    [[ -n "$result" ]] && print -r -- "$result"
    if [[ "$used_oneshot" -eq 1 ]]; then
        return "$GPY_SEG_STATUS_ONESHOT"
    fi
    [[ -n "$result" ]] && return 0
    return 1
}

# The reply embeds zsh's `%D{…}` token so the clock ticks between draws. No
# oneshot: clock.zsh has its own pure-zsh fallback.
function __gpy_request_clock() {
    local is_last=${1:-}
    local prev_bg=${2:-}
    local is_first=${3:-}

    local flags_tail
    __gpy_json_flags_tail_into flags_tail "$is_last" "$is_first" "$prev_bg"
    local request="{\"op\":\"clock\",\"shell\":\"zsh\",\"format\":\"zsh-prompt\"${flags_tail}}"

    local result
    result=$(__gpy_send_json "$request")

    [[ -n "$result" ]] || return 1
    print -r -- "$result"
    return 0
}

# Only for a Starship-style `__hostname_format`. No oneshot fallback by
# design: with a dead daemon (or an error reply) this returns 1 and the
# segment draws its local pill rather than fork per prompt.
function __gpy_request_hostname() {
    local hostname=$1
    local is_last=$2
    local prev_bg=${3:-}
    local is_ssh=${4:-}
    local is_first=${5:-}

    local json_hostname flags_tail ssh_json=""
    __gpy_escape_json_into json_hostname "$hostname"
    __gpy_json_flags_tail_into flags_tail "$is_last" "$is_first" "$prev_bg"
    [[ "$is_ssh" == "1" ]] && ssh_json=',"is_ssh":true'
    local request="{\"op\":\"hostname\",\"hostname\":\"${json_hostname}\",\"format\":\"zsh-prompt\"${flags_tail}${ssh_json}}"

    local result
    result=$(__gpy_send_json "$request")

    if [[ -n "$result" && "$result" != '{"error"'* ]]; then
        print -r -- "$result"
        return 0
    fi
    return 1
}

# No oneshot fallback, as for hostname. is_root "1" sends "is_root":true so the
# agent picks the root style; omitted (agent default: user style) otherwise.
function __gpy_request_username() {
    local username=$1
    local is_last=$2
    local prev_bg=${3:-}
    local is_root=${4:-}
    local is_first=${5:-}

    local json_username flags_tail root_json=""
    __gpy_escape_json_into json_username "$username"
    __gpy_json_flags_tail_into flags_tail "$is_last" "$is_first" "$prev_bg"
    [[ "$is_root" == "1" ]] && root_json=',"is_root":true'
    local request="{\"op\":\"username\",\"username\":\"${json_username}\",\"format\":\"zsh-prompt\"${flags_tail}${root_json}}"

    local result
    result=$(__gpy_send_json "$request")

    if [[ -n "$result" && "$result" != '{"error"'* ]]; then
        print -r -- "$result"
        return 0
    fi
    return 1
}

# Exit-status pill request ("exit_status": the bare "status" op is the agent
# health check). success is a 0/1 flag (1 == exit status 0). No oneshot
# fallback: the status segment draws the pill locally instead. An error reply
# (an older agent without the op) counts as no reply.
function __gpy_request_exit_status() {
    local success=$1
    local is_last=${2:-}
    local prev_bg=${3:-}
    local is_first=${4:-}

    local success_val=false
    [[ "$success" == "1" ]] && success_val=true

    local flags_tail
    __gpy_json_flags_tail_into flags_tail "$is_last" "$is_first" "$prev_bg"
    local request="{\"op\":\"exit_status\",\"success\":${success_val},\"format\":\"zsh-prompt\"${flags_tail}}"

    local result
    result=$(__gpy_send_json "$request")

    if [[ -n "$result" && "$result" != '{"error"'* ]]; then
        print -r -- "$result"
        return 0
    fi
    return 1
}

# Success is a 0/1 flag (1 == exit status 0).
function __gpy_request_character() {
    local success=$1
    local is_last=$2
    local prev_bg=${3:-}

    local success_val=false
    [[ "$success" == "1" ]] && success_val=true

    local flags_tail
    __gpy_json_flags_tail_into flags_tail "$is_last" "" "$prev_bg"
    local request="{\"op\":\"character\",\"success\":${success_val},\"format\":\"zsh-prompt\"${flags_tail}}"

    local result send_status used_oneshot=0
    result=$(__gpy_send_json "$request")
    send_status=$?

    # Status 2 (connected, reply late) never forks a oneshot.
    if [[ -z "$result" ]] && (( send_status != 2 )) && (( $+commands[gpy-agent] )) && __gpy_oneshot_claim; then
        used_oneshot=1
        local exit_code=1
        [[ "$success" == "1" ]] && exit_code=0
        local -a args
        args=(character --exit-code "$exit_code" --format zsh-prompt)
        if [[ "$is_last" != "true" ]]; then
            args+=(--not-last)
        fi
        result=$(gpy-agent oneshot $args 2>/dev/null)
    fi

    [[ -n "$result" ]] && print -r -- "$result"
    if [[ "$used_oneshot" -eq 1 ]]; then
        return "$GPY_SEG_STATUS_ONESHOT"
    fi
    [[ -n "$result" ]] && return 0
    return 1
}

# `key=value` paths for the parity harness against `gpy debug paths`. Values
# come from the prompt's own resolvers; `<unimplemented>` marks keys zsh has
# no resolver for, `<unresolved>` a resolver that produced nothing.
function __gpy_debug_paths() {
    local socket instant_dir cache_root

    socket=$(__gpy_ipc_endpoint)
    [[ -n "$socket" ]] || socket="<unresolved>"

    if __gpy_instant_cache_dir instant_dir && [[ -n "$instant_dir" ]]; then
        cache_root="${instant_dir%/*}"
    else
        instant_dir="<unresolved>"
        cache_root="<unresolved>"
    fi

    local runtime_root
    __gpy_runtime_root runtime_root
    print -r -- "runtime_root=$runtime_root"
    print -r -- "socket=$socket"
    print -r -- "shell_registry_dir=$(__gpy_shell_registry_dir)"
    print -r -- "cache_root=$cache_root"
    print -r -- "instant_prompts_dir=$instant_dir"
    print -r -- "theme_export_file=<unimplemented>"
    print -r -- "config_path=<unimplemented>"
    print -r -- "config_candidates=<unimplemented>"
    print -r -- "theme_dir=<unimplemented>"
}
