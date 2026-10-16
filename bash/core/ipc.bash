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
__gpy_runtime_root() {
    local home
    if [[ -n "${XDG_RUNTIME_DIR:-}" && "${XDG_RUNTIME_DIR:-}" == /* ]]; then
        echo "$XDG_RUNTIME_DIR/gpy"
        return
    fi
    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        echo "$XDG_CACHE_HOME/gpy"
        return
    fi
    if __gpy_home home; then
        echo "$home/.cache/gpy"
        return
    fi
    echo "/tmp/gpy"
}

# The file name is the PID the agent signals, so it must be numeric.
__gpy_shell_registry_dir() {
    echo "$(__gpy_runtime_root)/shells"
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
    local file
    file="$(__gpy_shell_registry_file)"
    rm -f "$file" "$file.reload" "$file.reregister" 2>/dev/null || true
    rm -f "$(__gpy_runtime_root)/refresh-throttle/$$".* 2>/dev/null || true
}

# No output and non-zero status when there is no cache directory.
__gpy_instant_cache_dir() {
    local home

    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        echo "$XDG_CACHE_HOME/gpy/instant-prompts"
        return 0
    fi

    if __gpy_home home; then
        echo "$home/.cache/gpy/instant-prompts"
        return 0
    fi

    return 1
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
        echo "$response"
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

__gpy_find_git_root() {
    local start_dir="${1:-$PWD}"

    # Keep the realpath fork: the cache key must match the Rust writer's
    # `path_to_cache_key` byte for byte, including symlink resolution.
    local dir
    dir=$(realpath "$start_dir" 2>/dev/null)
    [[ -z "$dir" ]] && return 1

    while [[ "$dir" != "/" ]]; do
        if [[ -e "$dir/.git" ]]; then
            echo "$dir"
            return 0
        fi
        # `${dir%/*}` avoids a dirname fork per level; "" means "/".
        dir="${dir%/*}"
        [[ -z "$dir" ]] && dir="/"
    done

    return 1
}

# Must match the Rust injective encoding; escape `_` first to avoid double-encoding.
__gpy_path_to_cache_key() {
    local path="$1"
    local key="${path//_/__}"
    key="${key//\//_s}"
    key="${key//\\/_b}"
    key="${key//:/_c}"
    key="${key// /_w}"
    echo "$key"
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

# MUST match `prev_bg_token` in the agent. Empty -> "none".
__gpy_prev_bg_token() {
    local prev_bg="$1"
    [[ -z "$prev_bg" ]] && {
        echo "none"
        return
    }
    echo "${prev_bg//[^a-zA-Z0-9]/_}"
}

# MUST match `variant_suffix` in the agent, e.g. git true false -> "git_last".
__gpy_cache_variant_suffix() {
    local base="$1" is_last="$2" is_first="$3"
    local suffix="$base"
    [[ "$is_first" == "true" ]] && suffix="${suffix}_first"
    [[ "$is_last" == "true" ]] && suffix="${suffix}_last"
    echo "$suffix"
}

# Cache key path into $3: the git root, or for language caches outside Git the
# canonical request path, matching how the agent keys them.
__gpy_instant_cache_key_path() {
    local suffix="$1" cwd="$2" __gpy_kp
    __gpy_kp="$(__gpy_find_git_root "$cwd")"
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
    cache_dir="$(__gpy_instant_cache_dir)" || return 1
    cache_key="$(__gpy_path_to_cache_key "$key_path")"
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

# Serve-stale read. Exit code: 0 fresh, 1 miss, 2 stale, 4 fresh `.none`
# fallback, 6 stale `.none` fallback (bit 2 = stale, bit 4 = fallback).
__gpy_read_instant_cache() {
    local suffix="${1:-git}"
    local cwd="${2:-$PWD}"
    local prev_bg="${3:-}"

    local key_path
    __gpy_instant_cache_key_path "$suffix" "$cwd" key_path || return 1

    local cache_dir
    cache_dir="$(__gpy_instant_cache_dir)" || return 1

    # The ANSI bakes in the fg:prev_bg chevron, so the key includes prev_bg.
    # Fall back to `.none`; the caller's refresh fills the right token file.
    local cache_key token
    cache_key="$(__gpy_path_to_cache_key "$key_path")"
    __gpy_chunk_cache_key cache_key
    token="$(__gpy_prev_bg_token "$prev_bg")"
    # `.bash` only: the `.ansi` files are fish's and must never reach PS1.
    local cache_file="$cache_dir/$cache_key.$suffix.$token.bash"
    local used_variant_fallback=0
    if [[ ! -f "$cache_file" && "$token" != "none" ]]; then
        cache_file="$cache_dir/$cache_key.$suffix.none.bash"
        used_variant_fallback=1
    fi

    if [[ -f "$cache_file" ]]; then
        local now mtime age
        if [[ "${__gpy_have_printf_epoch:-0}" == "1" ]]; then
            printf -v now '%(%s)T' -1
        else
            now=$(date +%s 2>/dev/null)
        fi
        # GNU stat -c first: on Linux `stat -f` means --file-system and
        # succeeds with non-numeric text, so the BSD fallback would never run.
        mtime=$(stat -c %Y "$cache_file" 2>/dev/null || stat -f %m "$cache_file" 2>/dev/null)

        local exit_code=0
        if [[ "$now" =~ ^[0-9]+$ && "$mtime" =~ ^[0-9]+$ ]]; then
            age=$((now - mtime))
            local ttl=5 default_ttl=5
            if [[ "$suffix" == git* ]]; then
                ttl="${GPY_GIT_INSTANT_CACHE_TTL_SECONDS:-5}"
            elif [[ "$suffix" == lang* ]]; then
                default_ttl=30
                ttl="${GPY_LANGUAGE_CACHE_TTL_SECONDS:-30}"
            fi
            [[ "$ttl" =~ ^[0-9]+$ ]] || ttl="$default_ttl"
            if (( age >= ttl )); then
                exit_code=$((exit_code + 2))
            fi
        fi
        if [[ "$used_variant_fallback" -eq 1 ]]; then
            exit_code=$((exit_code + 4))
        fi

        # `$(<file)` reads without forking `cat`.
        printf '%s' "$(<"$cache_file")"
        return "$exit_code"
    fi

    return 1
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

# Epoch ms. EPOCHREALTIME's radix follows the locale; older bash has whole
# seconds only, coarsening the throttle without loosening it.
__gpy_now_ms() {
    if [[ -n "${EPOCHREALTIME:-}" ]]; then
        local t="${EPOCHREALTIME//[.,]/}"
        echo "${t:0:${#t}-3}"
    elif [[ "${__gpy_have_printf_epoch:-0}" == "1" ]]; then
        local s
        printf -v s '%(%s)T' -1
        echo "${s}000"
    else
        echo "$(date +%s)000"
    fi
}

# Throttle stamps live on disk because segments run in `$(...)` subshells.
__gpy_refresh_throttle_dir() {
    echo "$(__gpy_runtime_root)/refresh-throttle"
}

# Throttled background refresh: at most one per 500ms per op+suffix+path
# (+prev_bg token, so another context cannot starve this one). Never blocks.
__gpy_maybe_refresh() {
    local op="$1" root="$2" cache_suffix="$3" is_last="${4:-}" prev_bg="${5:-}" is_first="${6:-}" throttle_key_extra="${7:-}"
    __gpy_agent_enabled || return 0

    local dir stamp last_ms=0 now_ms
    dir="$(__gpy_refresh_throttle_dir)"
    stamp="$dir/$$.$op.$cache_suffix.$(__gpy_path_to_cache_key "$root")${throttle_key_extra:+.$throttle_key_extra}"
    now_ms="$(__gpy_now_ms)"
    [[ -r "$stamp" ]] && last_ms="$(<"$stamp")"
    [[ "$last_ms" =~ ^[0-9]+$ ]] || last_ms=0
    (( now_ms - last_ms >= 500 )) || return 0
    mkdir -p "$dir" 2>/dev/null && echo "$now_ms" >"$stamp" 2>/dev/null

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
        cache_suffix="$(__gpy_cache_variant_suffix git "$is_last" "$is_first")"
        local cached_result
        cached_result="$(__gpy_read_instant_cache "$cache_suffix" "$context_path" "$prev_bg")"
        if [[ -n "$cached_result" ]]; then
            echo "$cached_result"
            return 0
        fi
    elif [[ "$op" == "lang" ]]; then
        local cache_suffix
        cache_suffix="$(__gpy_cache_variant_suffix lang "$is_last" "$is_first")"
        local cached_result
        cached_result="$(__gpy_read_instant_cache "$cache_suffix" "$context_path" "$prev_bg")"
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

    if instant_dir="$(__gpy_instant_cache_dir)" && [[ -n "$instant_dir" ]]; then
        cache_root="${instant_dir%/*}"
    else
        instant_dir="<unresolved>"
        cache_root="<unresolved>"
    fi

    echo "runtime_root=$(__gpy_runtime_root)"
    echo "socket=$socket"
    echo "shell_registry_dir=$(__gpy_shell_registry_dir)"
    echo "cache_root=$cache_root"
    echo "instant_prompts_dir=$instant_dir"
    echo "theme_export_file=<unimplemented>"
    echo "config_path=<unimplemented>"
    echo "config_candidates=<unimplemented>"
    echo "theme_dir=<unimplemented>"
}
