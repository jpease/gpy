# False in agent-free mode. Rendering never checks it, so oneshot still works.
__gpy_agent_enabled() {
    [[ -z "${GPY_AGENT_ENABLED:-}" || "${GPY_AGENT_ENABLED:-}" == "1" ]]
}

# Backslashes first so later escapes are not re-escaped. A raw newline/CR/tab
# would make malformed JSON and the agent would drop the segment.
__gpy_escape_json() {
    local s="$1"
    s="${s//\\/\\\\}"
    s="${s//\"/\\\"}"
    s="${s//$'\n'/\\n}"
    s="${s//$'\r'/\\r}"
    s="${s//$'\t'/\\t}"
    echo "$s"
}

# $VIRTUAL_ENV, else $CONDA_PREFIX unless the conda env is `base`.
__gpy_forwarded_venv() {
    if [[ -n "${VIRTUAL_ENV:-}" ]]; then
        printf '%s' "$VIRTUAL_ENV"
    elif [[ -n "${CONDA_PREFIX:-}" && -n "${CONDA_DEFAULT_ENV:-}" && "$CONDA_DEFAULT_ENV" != "base" ]]; then
        printf '%s' "$CONDA_PREFIX"
    fi
}

# `,"virtual_env":"<venv>"` for a `lang` request, or nothing.
__gpy_lang_venv_json() {
    local venv
    venv="$(__gpy_forwarded_venv)"
    [[ -n "$venv" ]] && printf ',"virtual_env":"%s"' "$(__gpy_escape_json "$venv")"
    return 0
}

# $HOME with a passwd fallback. Bash does not re-derive an unset $HOME, and
# a bare `~` returns $HOME even when empty (which the agent treats as unset),
# so `~user` is used. Returns non-zero when no absolute home exists. The local
# is not named `home`: dynamic scoping would shadow a caller's OUT_VAR.
__gpy_home() {
    local out_var="$1" __gpy_home_value="${HOME:-}" __gpy_home_user
    if [[ -z "$__gpy_home_value" ]]; then
        __gpy_home_user="$(id -un 2>/dev/null)" || return 1
        [[ -n "$__gpy_home_user" ]] || return 1
        # Assignment context tilde-expands "~user"; an unknown user stays
        # literal and the guard rejects it.
        eval "__gpy_home_value=~$__gpy_home_user" 2>/dev/null
    fi
    [[ "$__gpy_home_value" == /* ]] || return 1
    printf -v "$out_var" '%s' "$__gpy_home_value"
}

# An $XDG_* variable counts only when non-empty and absolute (XDG spec).
__gpy_ipc_endpoint() {
    local home
    if [[ -n "${GPY_AGENT_SOCKET_PATH:-}" ]]; then
        echo "$GPY_AGENT_SOCKET_PATH"
        return
    fi

    if [[ -n "${XDG_RUNTIME_DIR:-}" && "${XDG_RUNTIME_DIR:-}" == /* ]]; then
        echo "$XDG_RUNTIME_DIR/gpy/gpy.sock"
        return
    fi

    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        echo "$XDG_CACHE_HOME/gpy/gpy.sock"
        return
    fi

    if __gpy_home home; then
        echo "$home/.cache/gpy/gpy.sock"
        return
    fi

    echo "/tmp/gpy/gpy.sock"
}

# Where the socket lives by default and where the agent finds shell PIDs.
# Writes into variable $1 so hot paths need no `$(...)` fork.
__gpy_runtime_root() {
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

# The file name is the PID the agent signals, so it must be numeric.
__gpy_shell_registry_dir() {
    local root
    __gpy_runtime_root root
    echo "$root/shells"
}
__gpy_shell_registry_file() {
    echo "$(__gpy_shell_registry_dir)/$$"
}

# Record this shell so a restarted agent can nudge it, and cache the flag
# base so __gpy_consume_shell_flags never forks.
__gpy_shell_flag_base=""
__gpy_track_shell_for_agent_recovery() {
    local dir
    dir="$(__gpy_shell_registry_dir)"
    __gpy_shell_flag_base="$dir/$$"
    mkdir -p "$dir" 2>/dev/null || return 0
    echo "$$" >"$dir/$$" 2>/dev/null || true
}

__gpy_untrack_shell_for_agent_recovery() {
    local file root
    file="$(__gpy_shell_registry_file)"
    rm -f "$file" "$file.reload" "$file.reregister" 2>/dev/null || true
    __gpy_runtime_root root
    rm -f "$root/refresh-throttle/$$".* 2>/dev/null || true
}

# Instant-prompt cache dir into variable $1; non-zero status when there is
# no cache directory.
__gpy_instant_cache_dir() {
    local __gpy_icd_home
    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        printf -v "$1" '%s' "$XDG_CACHE_HOME/gpy/instant-prompts"
    elif __gpy_home __gpy_icd_home; then
        printf -v "$1" '%s' "$__gpy_icd_home/.cache/gpy/instant-prompts"
    else
        return 1
    fi
}

# Cached Bash theme export, written atomically by the agent.
__gpy_theme_export_cache_path() {
    local home

    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        echo "$XDG_CACHE_HOME/gpy/theme-export.bash"
        return 0
    fi

    if __gpy_home home; then
        echo "$home/.cache/gpy/theme-export.bash"
        return 0
    fi

    return 1
}

# Milliseconds to seconds for `timeout` (150 -> "0.150"), fork-free.
__gpy_ms_to_secs() {
    local ms="$1" out_var="$2"
    # A non-numeric override must not break `$(( ))`; `10#` stops octal.
    [[ "$ms" =~ ^[0-9]+$ ]] || ms=150
    ms=$((10#$ms))
    printf -v "$out_var" '%d.%03d' "$(( ms / 1000 ))" "$(( ms % 1000 ))"
}

# netcat-traditional has no -U, and using it would fail every request after a
# fork. Prompt segments run in `$(...)` and cannot write the cache back, so
# the probe also runs once at source time.
__gpy_nc_supports_unix=""
__gpy_nc_probe_capabilities() {
    [[ -n "$__gpy_nc_supports_unix" ]] && return 0
    local help
    __gpy_nc_supports_unix=0
    if command -v nc &>/dev/null; then
        help="$(nc -h 2>&1)"
        [[ "$help" == *-U* ]] && __gpy_nc_supports_unix=1
    fi
    return 0
}
if __gpy_agent_enabled && ! command -v socat &>/dev/null; then
    __gpy_nc_probe_capabilities
fi

__gpy_ipc_client_available() {
    command -v socat &>/dev/null && return 0
    command -v nc &>/dev/null || return 1
    __gpy_nc_probe_capabilities
    [[ "$__gpy_nc_supports_unix" == 1 ]]
}

# Prints the reply and returns 0 on a complete reply within GPY_IPC_TIMEOUT_MS,
# 1 when the agent is unreachable, 2 when it accepted the request but replied
# late (the caller must omit the segment, not fork oneshot).
#
# `IFS= read -r` returns nonzero on EOF before the newline, which separates a
# truncated reply from a complete one; `$(...)` strips the newline and cannot.
# Process substitution keeps `read` out of a pipe subshell. The client's exit
# status follows as `\037<status>\n`, the only way to tell a late agent from a
# refused socket. `exec 2>/dev/null`: when SIGPIPE is ignored (CI, IDEs), the
# trailer write after `read` closes the pipe prints "Broken pipe" into the prompt.
__gpy_send_json() {
    # Agent-free mode never talks to a daemon, even one another shell started.
    __gpy_agent_enabled || return 1

    local json="$1"
    local socket_path
    socket_path="$(__gpy_ipc_endpoint)"
    local response="" read_status=1 timeout_secs client_status="" connected_codes=" 0 "
    # One reply budget for every transport.
    __gpy_ms_to_secs "${GPY_IPC_TIMEOUT_MS:-}" timeout_secs

    [[ -S "$socket_path" ]] || return 1

    if command -v socat &>/dev/null; then
        IFS= read -r response < <(exec 2>/dev/null; echo "$json" | socat -t "$timeout_secs" - "UNIX-CONNECT:$socket_path" 2>/dev/null; printf '\037%s\n' "$?")
        read_status=$?
    # nc -w takes whole seconds only; `timeout` tightens the bound when present.
    elif __gpy_ipc_client_available; then
        if command -v timeout &>/dev/null; then
            IFS= read -r response < <(exec 2>/dev/null; echo "$json" | timeout "$timeout_secs" nc -U "$socket_path" -w 1 2>/dev/null; printf '\037%s\n' "$?")
            # 124: `timeout` killed an nc that had connected and was waiting.
            connected_codes=" 0 124 "
        else
            IFS= read -r response < <(exec 2>/dev/null; echo "$json" | nc -U "$socket_path" -w 1 2>/dev/null; printf '\037%s\n' "$?")
        fi
        read_status=$?
    else
        return 1
    fi

    if [[ "$response" == *$'\037'* ]]; then
        client_status="${response##*$'\037'}"
        response=""
    fi

    if [[ $read_status -eq 0 && -n "$response" ]]; then
        # The agent sends embedded newlines as RS (\036) to keep one reply line.
        echo "${response//$'\036'/$'\n'}"
        return 0
    fi

    # A client that connected, or an empty complete reply, means late, not unreachable.
    if [[ -n "$client_status" ]]; then
        [[ "$connected_codes" == *" $client_status "* ]] && return 2
    elif [[ $read_status -eq 0 ]]; then
        return 2
    fi
    return 1
}

# Must match PROTOCOL_VERSION in gpy-agent/src/ipc/protocol/mod.rs.
GPY_EXPECTED_PROTOCOL_VERSION=2

# Returns 1 only on a confirmed mismatch; an unqueryable status passes.
__gpy_check_protocol_version() {
    local response
    response=$(__gpy_send_json '{"op":"status"}' 2>/dev/null)
    [[ -n "$response" ]] || return 0

    local protocol_version=0
    if [[ $response =~ \"protocol_version\":([0-9]+) ]]; then
        protocol_version="${BASH_REMATCH[1]}"
    fi

    if [[ "$protocol_version" != "$GPY_EXPECTED_PROTOCOL_VERSION" ]]; then
        echo "gpy: WARNING: agent protocol version $protocol_version does not match expected $GPY_EXPECTED_PROTOCOL_VERSION; restart the agent (gpy-agent stop && gpy-agent start)" >&2
        return 1
    fi

    return 0
}

# Git root above $1 into variable $2; status 1 when there is none.
__gpy_find_git_root() {
    # Keep the realpath fork: the cache key must match the Rust writer's
    # `path_to_cache_key` byte for byte, including symlink resolution, and
    # bash has no builtin that resolves a path without changing directory.
    local __gpy_fgr_dir
    __gpy_fgr_dir=$(realpath "${1:-$PWD}" 2>/dev/null)
    [[ -z "$__gpy_fgr_dir" ]] && return 1

    while [[ "$__gpy_fgr_dir" != "/" ]]; do
        if [[ -e "$__gpy_fgr_dir/.git" ]]; then
            printf -v "$2" '%s' "$__gpy_fgr_dir"
            return 0
        fi
        # `${dir%/*}` avoids a dirname fork per level; "" means "/".
        __gpy_fgr_dir="${__gpy_fgr_dir%/*}"
        [[ -z "$__gpy_fgr_dir" ]] && __gpy_fgr_dir="/"
    done

    return 1
}

# Cache key for path $1 into variable $2. Must match the Rust injective
# encoding; escape `_` first to avoid double-encoding.
__gpy_path_to_cache_key() {
    local __gpy_pk_key="${1//_/__}"
    __gpy_pk_key="${__gpy_pk_key//\//_s}"
    __gpy_pk_key="${__gpy_pk_key//\\/_b}"
    __gpy_pk_key="${__gpy_pk_key//:/_c}"
    __gpy_pk_key="${__gpy_pk_key// /_w}"
    printf -v "$2" '%s' "$__gpy_pk_key"
}

# Cache key to on-disk stem in place: unchanged up to 200 characters, else
# 50-character chunks joined by `/` to stay under NAME_MAX. MUST match
# `cache_key_path_for`. Under LC_ALL=C lengths count bytes, so a long
# non-ASCII key just misses the cache.
__gpy_chunk_cache_key() {
    local __gpy_ck_key="${!1}" __gpy_ck_stem="" __gpy_ck_i
    (( ${#__gpy_ck_key} > 200 )) || return 0
    for (( __gpy_ck_i = 0; __gpy_ck_i < ${#__gpy_ck_key}; __gpy_ck_i += 50 )); do
        __gpy_ck_stem+="${__gpy_ck_stem:+/}${__gpy_ck_key:__gpy_ck_i:50}"
    done
    printf -v "$1" '%s' "$__gpy_ck_stem"
}

# Token for prev_bg $1 into variable $2. MUST match `prev_bg_token` in the
# agent. Empty -> "none".
__gpy_prev_bg_token() {
    if [[ -z "$1" ]]; then
        printf -v "$2" '%s' "none"
    else
        printf -v "$2" '%s' "${1//[^a-zA-Z0-9]/_}"
    fi
}

# Suffix for base $1, is_last $2, is_first $3 into variable $4. MUST match
# `variant_suffix` in the agent, e.g. git true false -> "git_last".
__gpy_cache_variant_suffix() {
    local __gpy_cvs_suffix="$1"
    [[ "$3" == "true" ]] && __gpy_cvs_suffix+="_first"
    [[ "$2" == "true" ]] && __gpy_cvs_suffix+="_last"
    printf -v "$4" '%s' "$__gpy_cvs_suffix"
}

# Cache key path into $3: the git root, or for language caches outside Git the
# canonical request path, matching how the agent keys them.
__gpy_instant_cache_key_path() {
    local suffix="$1" cwd="$2" __gpy_kp=""
    __gpy_find_git_root "$cwd" __gpy_kp
    if [[ -z "$__gpy_kp" && "$suffix" == lang* ]]; then
        __gpy_kp="$(realpath "$cwd" 2>/dev/null)"
    fi
    printf -v "$3" '%s' "$__gpy_kp"
    [[ -n "$__gpy_kp" ]]
}

# Any variant or prev_bg token counts: the agent writes `.none` only when some
# request had no prev_bg.
__gpy_instant_cache_present() {
    local base="$1" cwd="${2:-$PWD}" key_path cache_dir cache_key hit found=1 had_failglob=0
    __gpy_instant_cache_key_path "$base" "$cwd" key_path || return 1
    __gpy_instant_cache_dir cache_dir || return 1
    __gpy_path_to_cache_key "$key_path" cache_key
    __gpy_chunk_cache_key cache_key
    # An unmatched glob stays literal and fails -e; failglob would abort, so lift it.
    shopt -q failglob && had_failglob=1 && shopt -u failglob
    for hit in "$cache_dir/$cache_key.$base."*.bash "$cache_dir/$cache_key.${base}_"*.bash; do
        if [[ -e "$hit" ]]; then
            found=0
            break
        fi
    done
    ((had_failglob)) && shopt -s failglob
    return "$found"
}

# Serve-stale read of the entry for suffix $1, cwd $2, prev_bg $3 into
# variable $4 ("" on a miss). Exit code: 0 fresh, 1 miss, 2 stale, 4 fresh
# `.none` fallback, 6 stale `.none` fallback (bit 2 = stale, bit 4 = fallback).
__gpy_read_instant_cache() {
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
    # `.bash` only: the `.ansi` files are fish's and must never reach PS1.
    __gpy_ric_file="$__gpy_ric_dir/$__gpy_ric_key.$__gpy_ric_suffix.$__gpy_ric_token.bash"
    if [[ ! -f "$__gpy_ric_file" && "$__gpy_ric_token" != "none" ]]; then
        __gpy_ric_file="$__gpy_ric_dir/$__gpy_ric_key.$__gpy_ric_suffix.none.bash"
        __gpy_ric_status=4
    fi
    [[ -f "$__gpy_ric_file" ]] || return 1

    if [[ "${__gpy_have_printf_epoch:-0}" == "1" ]]; then
        printf -v __gpy_ric_now '%(%s)T' -1
    else
        __gpy_ric_now=$(date +%s 2>/dev/null)
    fi
    # No builtin reads an mtime. GNU stat -c first: on Linux `stat -f` means
    # --file-system and succeeds with non-numeric text, so the BSD fallback
    # would never run.
    __gpy_ric_mtime=$(stat -c %Y "$__gpy_ric_file" 2>/dev/null || stat -f %m "$__gpy_ric_file" 2>/dev/null)
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

# Shared is_last/is_first/prev_bg JSON tail. Flags are "true" or ""; absent
# flags are omitted (the agent defaults them to false).
__gpy_json_flags_tail() {
    local is_last="$1" is_first="$2" prev_bg="$3"
    local tail=""
    [[ "$is_last" == "true" ]] && tail="${tail},\"is_last\":true"
    [[ "$is_first" == "true" ]] && tail="${tail},\"is_first\":true"
    if [[ -n "$prev_bg" ]]; then
        tail="${tail},\"prev_bg\":\"$(__gpy_escape_json "$prev_bg")\""
    fi
    printf '%s' "$tail"
}

# Shared git/lang request builder, including the activated virtualenv.
__gpy_build_data_request() {
    local op="$1" cwd="$2" format="${3:-bash-prompt}" is_last="${4:-}" prev_bg="${5:-}" is_first="${6:-}"
    local json_cwd flags_tail venv_json=""
    json_cwd="$(__gpy_escape_json "$cwd")"
    flags_tail="$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")"
    [[ "$op" == "lang" ]] && venv_json="$(__gpy_lang_venv_json)"
    printf '%s' "{\"op\":\"$op\",\"cwd\":\"$json_cwd\",\"format\":\"$format\"${flags_tail}${venv_json}}"
}

# Direct IPC data op for background refreshes. No oneshot fallback: without a
# daemon nothing would repaint anyway.
__gpy_trigger_data_refresh() {
    local op="$1"
    local cwd="${2:-$PWD}"
    local is_last="${3:-false}"
    local prev_bg="${4:-}"

    __gpy_send_json "$(__gpy_build_data_request "$op" "$cwd" bash-prompt "$is_last" "$prev_bg" "")" >/dev/null 2>&1
}

# Bounded IPC-only query; oneshot is unbounded and would break the omit
# budget. The agent writes its own cache, so success needs no refresh.
__gpy_sync_data_request() {
    local op="$1" cwd="$2" is_last="${3:-}" prev_bg="${4:-}" is_first="${5:-}"
    local response
    response="$(__gpy_send_json "$(__gpy_build_data_request "$op" "$cwd" bash-prompt "$is_last" "$prev_bg" "$is_first")" 2>/dev/null)" || return 1
    [[ -n "$response" ]] || return 1
    printf '%s' "$response"
}

# Predicates over __gpy_read_instant_cache's exit-code bits.
__gpy_cache_status_stale() { (( ($1 & 2) != 0 )); }
__gpy_cache_status_variant() { (( ($1 & 4) != 0 )); }

# Epoch ms into variable $1. EPOCHREALTIME's radix follows the locale; older
# bash has whole seconds only, coarsening the throttle without loosening it.
__gpy_now_ms() {
    local __gpy_nm_t
    if [[ -n "${EPOCHREALTIME:-}" ]]; then
        __gpy_nm_t="${EPOCHREALTIME//[.,]/}"
        printf -v "$1" '%s' "${__gpy_nm_t:0:${#__gpy_nm_t}-3}"
    else
        __gpy_now_seconds __gpy_nm_t
        printf -v "$1" '%s000' "$__gpy_nm_t"
    fi
}

# Stamp filename for throttle bucket $1 into $2. A bucket of up to 200 bytes
# names the stamp itself; a longer one gets its 32-bit FNV-1a hash, which
# keeps the name under NAME_MAX for any path. The stamp stores the bucket, so
# a hash collision reads as no stamp rather than throttling another bucket.
# The last long bucket's name is memoized: the byte loop costs ~1 ms.
__gpy_refresh_stamp_memo_bucket=""
__gpy_refresh_stamp_memo_name=""
__gpy_refresh_stamp_name() {
    # Byte semantics: NAME_MAX counts bytes, and so does the hash.
    local LC_ALL=C __gpy_rs_bucket="$1" __gpy_rs_i __gpy_rs_byte __gpy_rs_hash=2166136261
    if (( ${#__gpy_rs_bucket} <= 200 )); then
        printf -v "$2" '%s.%s' "$$" "$__gpy_rs_bucket"
        return
    fi
    if [[ "$__gpy_rs_bucket" != "$__gpy_refresh_stamp_memo_bucket" ]]; then
        for (( __gpy_rs_i = 0; __gpy_rs_i < ${#__gpy_rs_bucket}; __gpy_rs_i++ )); do
            # bash 3.2 reports bytes over 0x7f as negative; the mask fixes that.
            printf -v __gpy_rs_byte '%d' "'${__gpy_rs_bucket:__gpy_rs_i:1}"
            (( __gpy_rs_hash = ((__gpy_rs_hash ^ (__gpy_rs_byte & 255)) * 16777619) & 0xffffffff ))
        done
        printf -v __gpy_refresh_stamp_memo_name '%s.#%08x' "$$" "$__gpy_rs_hash"
        __gpy_refresh_stamp_memo_bucket="$__gpy_rs_bucket"
    fi
    printf -v "$2" '%s' "$__gpy_refresh_stamp_memo_name"
}

# Throttled background refresh: at most one per 500ms per op+suffix+path
# (+prev_bg token, so another context cannot starve this one). Never blocks.
__gpy_maybe_refresh() {
    local op="$1" root="$2" cache_suffix="$3" is_last="${4:-}" prev_bg="${5:-}" is_first="${6:-}" throttle_key_extra="${7:-}"
    __gpy_agent_enabled || return 0

    local dir bucket stamp_name stamp stamped="" last_ms=0 now_ms path_key
    # On disk because segments run in `$(...)` subshells.
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
    [[ "${stamped#* }" == "$bucket" ]] && last_ms="${stamped%% *}"
    [[ "$last_ms" =~ ^[0-9]+$ ]] || last_ms=0
    (( now_ms - last_ms >= 500 )) || return 0
    # The stamp is the only limit across renders: without one, skip this
    # refresh rather than send one per render. `>|` overrides noclobber.
    if ! { printf '%s' "$now_ms $bucket" >|"$stamp"; } 2>/dev/null; then
        mkdir -p "$dir" 2>/dev/null && { printf '%s' "$now_ms $bucket" >|"$stamp"; } 2>/dev/null || return 0
    fi

    # Before the fork: a backgrounded child cannot propagate __gpy_registered.
    __gpy_register_with_agent >/dev/null 2>&1

    __gpy_trigger_data_refresh "$op" "$root" "$is_last" "$prev_bg" >/dev/null 2>&1 &
    disown $! 2>/dev/null
}

# Instant cache first for git/lang, then IPC, then oneshot.
__gpy_request() {
    local op="$1"
    local context_path="${2:-$PWD}"
    local format="${3:-json}"
    local is_last="${4:-false}"
    local prev_bg="${5:-}"
    local is_first="${6:-false}"

    # For git/lang operations, try instant cache first (0ms latency)
    if [[ "$op" == "git" ]]; then
        local cache_suffix
        __gpy_cache_variant_suffix git "$is_last" "$is_first" cache_suffix
        local cached_result
        __gpy_read_instant_cache "$cache_suffix" "$context_path" "$prev_bg" cached_result
        if [[ -n "$cached_result" ]]; then
            echo "$cached_result"
            return 0
        fi
    elif [[ "$op" == "lang" ]]; then
        local cache_suffix
        __gpy_cache_variant_suffix lang "$is_last" "$is_first" cache_suffix
        local cached_result
        __gpy_read_instant_cache "$cache_suffix" "$context_path" "$prev_bg" cached_result
        if [[ -n "$cached_result" ]]; then
            echo "$cached_result"
            return 0
        fi
    fi

    local request
    request="$(__gpy_build_data_request "$op" "$context_path" "$format" "$is_last" "$prev_bg" "$is_first")"

    __gpy_send_json "$request"
    local send_status=$?
    # 2: late reply; the agent is computing it, so omit rather than fork oneshot.
    (( send_status == 2 )) && return 1
    if (( send_status != 0 )); then
        # oneshot has no --prev-bg, so the chevron color is skipped until the
        # daemon connects; --first keeps the opening cap correct.
        __gpy_fallback_oneshot "$op" "$context_path" "$format" "$is_last" "$is_first"
    fi
}

# Caps oneshot forks to one per render instead of one per segment. Reset by
# __gpy_render_prompt; subshells inherit the folded value by fork.
__gpy_oneshot_claim() {
    [[ "${__gpy_oneshot_used:-0}" == "1" ]] && return 1
    return 0
}

# Returns GPY_SEG_STATUS_ONESHOT whenever it forked oneshot, even with no
# output, so the budget is tracked; 1 otherwise.
__gpy_fallback_oneshot() {
    local op="$1"
    local path="$2"
    local format="$3"
    local is_last="${4:-false}"
    local is_first="${5:-false}"

    command -v gpy-agent &>/dev/null || return 1
    __gpy_oneshot_claim || return 1

    local args=("$op" --cwd "$path" --format "$format")
    if [[ "$is_last" != "true" ]]; then
        args+=(--not-last)
    fi
    if [[ "$is_first" == "true" ]]; then
        args+=(--first)
    fi

    gpy-agent oneshot "${args[@]}" 2>/dev/null
    return "$GPY_SEG_STATUS_ONESHOT"
}

# The response embeds bash's `\D{…}` token, so the clock ticks between
# renders. No oneshot fallback: clock.bash has a pure-bash renderer.
__gpy_request_clock() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"

    local flags_tail
    flags_tail="$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")"
    local request="{\"op\":\"clock\",\"shell\":\"bash\",\"format\":\"bash-prompt\"${flags_tail}}"

    local result
    result="$(__gpy_send_json "$request")" || return 1

    [[ -n "$result" ]] || return 1
    echo "$result"
    return 0
}

# Duration request with oneshot fallback; returns GPY_SEG_STATUS_ONESHOT when
# it forked.
__gpy_request_duration() {
    local duration_ms="$1"
    local is_last="${2:-false}"
    local prev_bg="${3:-}"
    local is_first="${4:-false}"

    local flags_tail
    flags_tail="$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")"
    local request="{\"op\":\"duration\",\"duration_ms\":${duration_ms},\"format\":\"bash-prompt\"${flags_tail}}"

    local result send_status used_oneshot=0
    result="$(__gpy_send_json "$request")"
    send_status=$?
    # 2 = connected but the reply was late: omit rather than recompute.
    if (( send_status != 0 && send_status != 2 )); then
        # Capped to one oneshot fork per render; oneshot has no --prev-bg.
        if command -v gpy-agent &>/dev/null && __gpy_oneshot_claim; then
            used_oneshot=1
            local args=(duration --duration-ms "$duration_ms" --format bash-prompt)
            if [[ "$is_last" != "true" ]]; then
                args+=(--not-last)
            fi
            if [[ "$is_first" == "true" ]]; then
                args+=(--first)
            fi
            result="$(gpy-agent oneshot "${args[@]}" 2>/dev/null)"
        fi
    fi

    [[ -n "$result" ]] && echo "$result"
    if [[ "$used_oneshot" -eq 1 ]]; then
        return "$GPY_SEG_STATUS_ONESHOT"
    fi
    [[ -n "$result" ]] && return 0
    return 1
}

# Input is a 0/1 success flag; the character is always last.
__gpy_request_character() {
    local success="$1"
    local is_last="$2"
    local prev_bg="${3:-}"

    local success_val=false
    [[ "$success" == "1" ]] && success_val=true

    local flags_tail
    flags_tail="$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")"
    local request="{\"op\":\"character\",\"success\":${success_val},\"format\":\"bash-prompt\"${flags_tail}}"

    local result send_status used_oneshot=0
    result="$(__gpy_send_json "$request")"
    send_status=$?
    # 2 = connected but the reply was late: omit rather than recompute.
    if (( send_status != 0 && send_status != 2 )); then
        if command -v gpy-agent &>/dev/null && __gpy_oneshot_claim; then
            used_oneshot=1
            local exit_code=1
            [[ "$success" == "1" ]] && exit_code=0
            local args=(character --exit-code "$exit_code" --format bash-prompt)
            if [[ "$is_last" != "true" ]]; then
                args+=(--not-last)
            fi
            result="$(gpy-agent oneshot "${args[@]}" 2>/dev/null)"
        fi
    fi

    [[ -n "$result" ]] && echo "$result"
    if [[ "$used_oneshot" -eq 1 ]]; then
        return "$GPY_SEG_STATUS_ONESHOT"
    fi
    [[ -n "$result" ]] && return 0
    return 1
}

# No oneshot fallback: with a dead daemon the segment renders nothing rather
# than fork per prompt. is_ssh "1" lets the agent draw the icon over SSH only.
__gpy_request_hostname() {
    local hostname="$1"
    local is_last="$2"
    local prev_bg="${3:-}"
    local is_ssh="${4:-}"

    local json_hostname flags_tail ssh_json=""
    json_hostname="$(__gpy_escape_json "$hostname")"
    flags_tail="$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")"
    [[ "$is_ssh" == "1" ]] && ssh_json=',"is_ssh":true'
    local request="{\"op\":\"hostname\",\"hostname\":\"${json_hostname}\",\"format\":\"bash-prompt\"${flags_tail}${ssh_json}}"

    local result
    if result="$(__gpy_send_json "$request")" && [[ -n "$result" ]]; then
        echo "$result"
        return 0
    fi
    return 1
}

# No oneshot fallback, as for hostname.
__gpy_request_username() {
    local username="$1"
    local is_last="$2"
    local prev_bg="${3:-}"

    local json_username flags_tail
    json_username="$(__gpy_escape_json "$username")"
    flags_tail="$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")"
    local request="{\"op\":\"username\",\"username\":\"${json_username}\",\"format\":\"bash-prompt\"${flags_tail}}"

    local result
    if result="$(__gpy_send_json "$request")" && [[ -n "$result" ]]; then
        echo "$result"
        return 0
    fi
    return 1
}

# Every resolved path as `key=value`, counterpart to `gpy debug paths --format
# kv` for the parity harness. Values come from the real resolvers, never
# re-derived. `<unimplemented>`: no resolver in this shell; `<unresolved>`: a
# resolver produced nothing.
__gpy_debug_paths() {
    local socket instant_dir cache_root

    socket="$(__gpy_ipc_endpoint)"
    [[ -n "$socket" ]] || socket="<unresolved>"

    if __gpy_instant_cache_dir instant_dir && [[ -n "$instant_dir" ]]; then
        cache_root="${instant_dir%/*}"
    else
        instant_dir="<unresolved>"
        cache_root="<unresolved>"
    fi

    local runtime_root
    __gpy_runtime_root runtime_root
    echo "runtime_root=$runtime_root"
    echo "socket=$socket"
    echo "shell_registry_dir=$(__gpy_shell_registry_dir)"
    echo "cache_root=$cache_root"
    echo "instant_prompts_dir=$instant_dir"
    echo "theme_export_file=<unimplemented>"
    echo "config_path=<unimplemented>"
    echo "config_candidates=<unimplemented>"
    echo "theme_dir=<unimplemented>"
}
