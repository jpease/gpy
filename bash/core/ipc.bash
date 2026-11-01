# The boolean table shared with Fish, Zsh and the agent (tests/fixtures/
# shell_env_flag_vectors.tsv): `1/true/yes/on` is on, `0/false/no/off` is off,
# case-insensitive, no surrounding whitespace. Assigns `on`, `off`, or DEFAULT
# for anything else to OUTVAR. Explicit spellings, not `${v,,}` (Bash 3.2);
# `printf -v`, so it never forks. Silent, and the writer of OUTVAR only.
# Usage: __gpy_flag_value VALUE DEFAULT OUTVAR
__gpy_flag_value() {
    case "$1" in
        1 | [Tt][Rr][Uu][Ee] | [Yy][Ee][Ss] | [Oo][Nn]) printf -v "$3" '%s' on ;;
        0 | [Ff][Aa][Ll][Ss][Ee] | [Nn][Oo] | [Oo][Ff][Ff]) printf -v "$3" '%s' off ;;
        *) printf -v "$3" '%s' "$2" ;;
    esac
}

# Precedence: a recognised user value, then the theme export's __gpy_cfg_*
# mirror (off only for `0`), then on. Silent; __gpy_warn_flag_values reports
# once. False in agent-free mode. Rendering never checks it, so oneshot still
# works.
__gpy_agent_enabled() {
    local __gpy_ae_state
    __gpy_flag_value "${GPY_AGENT_ENABLED-}" cfg __gpy_ae_state
    case "$__gpy_ae_state" in
        on) return 0 ;;
        off) return 1 ;;
    esac
    [[ "${__gpy_cfg_agent_enabled-}" != 0 ]]
}

# Governs restarts only; agent start and registration use __gpy_agent_enabled.
__gpy_supervisor_enabled() {
    local __gpy_se_state
    __gpy_flag_value "${GPY_AGENT_SUPERVISOR_ENABLED-}" cfg __gpy_se_state
    case "$__gpy_se_state" in
        on) return 0 ;;
        off) return 1 ;;
    esac
    [[ "${__gpy_cfg_supervisor_enabled-}" != 0 ]]
}

# Runs once per shell, after the theme loads: names the flag values whose
# meaning changed, and logs unrecognised ones under GPY_DEBUG.
__gpy_warn_flag_values() {
    local __gpy_wf_name __gpy_wf_value __gpy_wf_state
    for __gpy_wf_name in GPY_AGENT_ENABLED GPY_AGENT_SUPERVISOR_ENABLED GPY_SHOW_STATUS; do
        __gpy_wf_value="${!__gpy_wf_name-}"
        [[ -n "$__gpy_wf_value" ]] || continue
        if [[ "$__gpy_wf_name" == GPY_SHOW_STATUS ]]; then
            case "$__gpy_wf_value" in
                [Tt][Rr][Uu][Ee] | [Yy][Ee][Ss] | [Oo][Nn])
                    echo "gpy: GPY_SHOW_STATUS=$__gpy_wf_value now shows the status indicator (it used to hide it); set GPY_SHOW_STATUS=1 or unset it" >&2
                    ;;
            esac
        else
            case "$__gpy_wf_value" in
                [Nn][Oo] | [Oo][Ff][Ff])
                    echo "gpy: $__gpy_wf_name=$__gpy_wf_value now disables it (it used to leave it on); set $__gpy_wf_name=0 to silence this" >&2
                    ;;
            esac
        fi
        __gpy_flag_value "$__gpy_wf_value" unrecognised __gpy_wf_state
        if [[ "$__gpy_wf_state" == unrecognised && "${GPY_DEBUG-}" == 1 ]]; then
            echo "gpy[flags]: debug: $__gpy_wf_name=$__gpy_wf_value is not a boolean; ignoring it" >&2
        fi
    done
}

# Escape $2 for a JSON string into variable $1 without a fork. Backslashes
# first so later escapes are not re-escaped; a raw newline/CR/tab would make
# malformed JSON. Locals are __gpy_ej_* so a caller's OUTVAR is not shadowed.
__gpy_escape_json_into() {
    local __gpy_ej_s="$2"
    __gpy_ej_s="${__gpy_ej_s//\\/\\\\}"
    __gpy_ej_s="${__gpy_ej_s//\"/\\\"}"
    __gpy_ej_s="${__gpy_ej_s//$'\n'/\\n}"
    __gpy_ej_s="${__gpy_ej_s//$'\r'/\\r}"
    __gpy_ej_s="${__gpy_ej_s//$'\t'/\\t}"
    printf -v "$1" '%s' "$__gpy_ej_s"
}

# $VIRTUAL_ENV, else $CONDA_PREFIX unless the conda env is `base`, into
# variable $1 (empty when neither applies). Writes via `printf -v`: no fork.
__gpy_forwarded_venv_into() {
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
__gpy_lang_venv_json_into() {
    local __gpy_lvj_venv
    __gpy_forwarded_venv_into __gpy_lvj_venv
    if [[ -z "$__gpy_lvj_venv" ]]; then
        printf -v "$1" '%s' ""
        return 0
    fi
    __gpy_escape_json_into __gpy_lvj_venv "$__gpy_lvj_venv"
    printf -v "$1" ',"virtual_env":"%s"' "$__gpy_lvj_venv"
}

# $HOME with a passwd fallback. Bash does not re-derive an unset $HOME, and
# a bare `~` returns $HOME even when empty (which the agent treats as unset),
# so `~user` is used. Returns non-zero when no absolute home exists. The local
# is not named `home`: dynamic scoping would shadow a caller's OUT_VAR.
# Rust counterpart: the `home` argument `paths::xdg_path`'s callers pass.
__gpy_home() {
    local out_var="$1" __gpy_home_value="${HOME:-}" __gpy_home_user
    if [[ -z "$__gpy_home_value" ]]; then
        __gpy_home_user="$(id -un 2>/dev/null)" || return 1
        [[ -n "$__gpy_home_user" ]] || return 1
        # Only a plausible username may reach eval; a leading - would make ~- expand to $OLDPWD.
        [[ "$__gpy_home_user" =~ ^[A-Za-z0-9_][A-Za-z0-9._-]*$ ]] || return 1
        # Assignment context tilde-expands "~user"; an unknown user stays
        # literal and the guard rejects it.
        eval "__gpy_home_value=~$__gpy_home_user" 2>/dev/null
    fi
    [[ "$__gpy_home_value" == /* ]] || return 1
    printf -v "$out_var" '%s' "$__gpy_home_value"
}

# An $XDG_* variable counts only when non-empty and absolute (XDG spec).
# Rust counterpart: `SocketPath::resolve`.
__gpy_ipc_endpoint_into() {
    local __gpy_ie_home
    if [[ -n "${GPY_AGENT_SOCKET_PATH:-}" ]]; then
        printf -v "$1" '%s' "$GPY_AGENT_SOCKET_PATH"
    elif [[ -n "${XDG_RUNTIME_DIR:-}" && "${XDG_RUNTIME_DIR:-}" == /* ]]; then
        printf -v "$1" '%s' "$XDG_RUNTIME_DIR/gpy/gpy.sock"
    elif [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        printf -v "$1" '%s' "$XDG_CACHE_HOME/gpy/gpy.sock"
    elif __gpy_home __gpy_ie_home; then
        printf -v "$1" '%s' "$__gpy_ie_home/.cache/gpy/gpy.sock"
    else
        printf -v "$1" '%s' "/tmp/gpy/gpy.sock"
    fi
}

# Printing form for callers that already run in a `$(...)`; every other
# caller uses __gpy_ipc_endpoint_into and forks nothing.
__gpy_ipc_endpoint() {
    local __gpy_ep_path
    __gpy_ipc_endpoint_into __gpy_ep_path
    printf '%s\n' "$__gpy_ep_path"
}

# Where the socket lives by default and where the agent finds shell PIDs.
# Writes into variable $1 so hot paths need no `$(...)` fork.
# Rust counterpart: `paths::runtime_root_for`.
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
# Rust counterpart: `paths::cache_root_for`.
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
# Rust counterpart: `paths::cache_root_for`.
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

# The budget for a segment the prompt has nothing to show for, into variable
# $1: GPY_IPC_COLD_TIMEOUT_MS, never below GPY_IPC_TIMEOUT_MS. A non-numeric
# value falls back to its default. Locals are __gpy_ct_* so a caller's OUTVAR
# is not shadowed.
__gpy_cold_timeout_ms_into() {
    local __gpy_ct_cold="${GPY_IPC_COLD_TIMEOUT_MS:-}" __gpy_ct_warm="${GPY_IPC_TIMEOUT_MS:-}"
    [[ "$__gpy_ct_cold" =~ ^[0-9]+$ ]] || __gpy_ct_cold=500
    [[ "$__gpy_ct_warm" =~ ^[0-9]+$ ]] || __gpy_ct_warm=150
    __gpy_ct_cold=$((10#$__gpy_ct_cold))
    __gpy_ct_warm=$((10#$__gpy_ct_warm))
    (( __gpy_ct_cold < __gpy_ct_warm )) && __gpy_ct_cold=$__gpy_ct_warm
    printf -v "$1" '%s' "$__gpy_ct_cold"
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

# Returns 0 on a complete reply within the budget ($2 in ms, else
# GPY_IPC_TIMEOUT_MS), 1 when the agent is unreachable, 2 when it accepted the
# request but replied late (the caller must omit the segment, not fork
# oneshot). With OUTVAR ($3) the reply is assigned to it (trailing newlines
# stripped, as `$(...)` would) and nothing is printed; OUTVAR is "" on every
# non-0 return. Without OUTVAR the reply is printed. Locals are __gpy_sj_* so
# a caller's OUTVAR is not shadowed.
#
# `IFS= read -r` returns nonzero on EOF before the newline, which separates a
# truncated reply from a complete one; `$(...)` strips the newline and cannot.
# Process substitution keeps `read` out of a pipe subshell. The client's exit
# status follows as `\037<status>\n`, the only way to tell a late agent from a
# refused socket. `exec 2>/dev/null`: when SIGPIPE is ignored (CI, IDEs), the
# trailer write after `read` closes the pipe prints "Broken pipe" into the prompt.
# The request reaches the client through a here-string, so no `printf |`
# subshell; Bash < 5.1 backs it with a temp file, and a failed here-string
# reads as unreachable.
__gpy_send_json() {
    local __gpy_sj_out="${3:-}"
    [[ -z "$__gpy_sj_out" ]] || printf -v "$__gpy_sj_out" '%s' ''

    # Agent-free mode never talks to a daemon, even one another shell started.
    __gpy_agent_enabled || return 1

    local __gpy_sj_json="$1"
    local __gpy_sj_socket_path
    __gpy_ipc_endpoint_into __gpy_sj_socket_path
    local __gpy_sj_response="" __gpy_sj_read_status=1 __gpy_sj_timeout_secs __gpy_sj_client_status="" __gpy_sj_connected_codes=" 0 "
    # One reply budget for every transport.
    __gpy_ms_to_secs "${2:-${GPY_IPC_TIMEOUT_MS:-}}" __gpy_sj_timeout_secs

    [[ -S "$__gpy_sj_socket_path" ]] || return 1

    if command -v socat &>/dev/null; then
        IFS= read -r __gpy_sj_response < <(exec 2>/dev/null; socat -t "$__gpy_sj_timeout_secs" - "UNIX-CONNECT:$__gpy_sj_socket_path" 2>/dev/null <<<"$__gpy_sj_json"; printf '\037%s\n' "$?")
        __gpy_sj_read_status=$?
    # nc -w takes whole seconds only; `timeout` tightens the bound when present.
    elif __gpy_ipc_client_available; then
        if command -v timeout &>/dev/null; then
            IFS= read -r __gpy_sj_response < <(exec 2>/dev/null; timeout "$__gpy_sj_timeout_secs" nc -U "$__gpy_sj_socket_path" -w 1 2>/dev/null <<<"$__gpy_sj_json"; printf '\037%s\n' "$?")
            # 124: `timeout` killed an nc that had connected and was waiting.
            __gpy_sj_connected_codes=" 0 124 "
        else
            IFS= read -r __gpy_sj_response < <(exec 2>/dev/null; nc -U "$__gpy_sj_socket_path" -w 1 2>/dev/null <<<"$__gpy_sj_json"; printf '\037%s\n' "$?")
        fi
        __gpy_sj_read_status=$?
    else
        return 1
    fi

    if [[ "$__gpy_sj_response" == *$'\037'* ]]; then
        __gpy_sj_client_status="${__gpy_sj_response##*$'\037'}"
        __gpy_sj_response=""
    fi

    if [[ $__gpy_sj_read_status -eq 0 && -n "$__gpy_sj_response" ]]; then
        # The agent sends embedded newlines as RS (\036) to keep one reply line.
        __gpy_sj_response="${__gpy_sj_response//$'\036'/$'\n'}"
        if [[ -n "$__gpy_sj_out" ]]; then
            while [[ "$__gpy_sj_response" == *$'\n' ]]; do
                __gpy_sj_response="${__gpy_sj_response%$'\n'}"
            done
            printf -v "$__gpy_sj_out" '%s' "$__gpy_sj_response"
        else
            printf '%s\n' "$__gpy_sj_response"
        fi
        return 0
    fi

    # A client that connected, or an empty complete reply, means late, not unreachable.
    if [[ -n "$__gpy_sj_client_status" ]]; then
        [[ "$__gpy_sj_connected_codes" == *" $__gpy_sj_client_status "* ]] && return 2
    elif [[ $__gpy_sj_read_status -eq 0 ]]; then
        return 2
    fi
    return 1
}

# Must match PROTOCOL_VERSION in gpy-agent/src/ipc/protocol/mod.rs.
__gpy_protocol_version=2

# Returns 1 only on a confirmed mismatch; an unqueryable status passes.
__gpy_check_protocol_version() {
    local response
    __gpy_send_json '{"op":"status"}' "" response 2>/dev/null
    [[ -n "$response" ]] || return 0

    local protocol_version=0
    if [[ $response =~ \"protocol_version\":([0-9]+) ]]; then
        protocol_version="${BASH_REMATCH[1]}"
    fi

    if [[ "$protocol_version" != "$__gpy_protocol_version" ]]; then
        echo "gpy: WARNING: agent protocol version $protocol_version does not match expected $__gpy_protocol_version; restart the agent (gpy-agent restart)" >&2
        return 1
    fi

    return 0
}

# $PWD resolved once per directory change (`__gpy_pwd_memo`, "" when realpath
# fails), so a cache-hit render forks no realpath.
__gpy_resolve_pwd() {
    [[ "$PWD" == "${__gpy_pwd_memo_key-}" ]] && return 0
    __gpy_pwd_memo_key="$PWD"
    __gpy_pwd_memo="$(realpath "$PWD" 2>/dev/null)" || __gpy_pwd_memo=""
}

# Git root above $1 into variable $2; status 1 when there is none.
__gpy_find_git_root() {
    # The cache key must match the Rust writer's `path_to_cache_key` byte for byte,
    # including symlink resolution, and only `realpath` resolves a path without
    # changing directory. $PWD uses the per-`cd` memo; any other path still forks.
    local __gpy_fgr_dir __gpy_fgr_start="${1:-$PWD}"
    if [[ -n "${__gpy_pwd_memo_key-}" && "$__gpy_fgr_start" == "$__gpy_pwd_memo_key" ]]; then
        __gpy_fgr_dir="${__gpy_pwd_memo-}"
    else
        __gpy_fgr_dir=$(realpath "$__gpy_fgr_start" 2>/dev/null)
    fi
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

# Suffix for base $1, is_last $2, is_first $3 into variable $4. MUST match the
# agent's cache variant names (`CacheFileName`), e.g. git true false -> "git_last".
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
        if [[ -n "${__gpy_pwd_memo_key-}" && "$cwd" == "$__gpy_pwd_memo_key" ]]; then
            __gpy_kp="${__gpy_pwd_memo-}"
        else
            __gpy_kp="$(realpath "$cwd" 2>/dev/null)"
        fi
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

# Tail of the agent's git-background file for dialect extension $1 into
# variable $2. MUST match the agent's `CacheKind::GitBg` name.
__gpy_git_bg_file_tail() {
    printf -v "$2" '%s' "git_bg.none.$1"
}

# Background the git pill was drawn with, for the segment after it, into
# variable $1: the colour the agent recorded, else the clean colour. Agent-free
# renders (git segment status __gpy_seg_status_oneshot) never read the file: it may
# be days old. No subshell: `read` in this shell with the per-prompt cwd memo.
__gpy_git_segment_bg() {
    local __gpy_gsb_key_path __gpy_gsb_dir __gpy_gsb_key __gpy_gsb_tail __gpy_gsb_bg=""
    if [[ "${__gpy_git_seg_status:-0}" != "$__gpy_seg_status_oneshot" ]] &&
        __gpy_instant_cache_key_path git "$PWD" __gpy_gsb_key_path &&
        __gpy_instant_cache_dir __gpy_gsb_dir; then
        __gpy_path_to_cache_key "$__gpy_gsb_key_path" __gpy_gsb_key
        __gpy_chunk_cache_key __gpy_gsb_key
        __gpy_git_bg_file_tail bash __gpy_gsb_tail
        # The file has no trailing newline, so `read -d ''` returns 1 yet fills the variable.
        [[ -f "$__gpy_gsb_dir/$__gpy_gsb_key.$__gpy_gsb_tail" ]] &&
            IFS= read -r -d '' __gpy_gsb_bg <"$__gpy_gsb_dir/$__gpy_gsb_key.$__gpy_gsb_tail"
    fi
    printf -v "$1" '%s' "${__gpy_gsb_bg:-${__color_git_clean_bg:-}}"
}

# Shared is_last/is_first/prev_bg JSON tail into variable $1. Flags are "true"
# or ""; absent flags are omitted (the agent defaults them to false). Locals
# are __gpy_jft_* so a caller's OUTVAR is not shadowed.
__gpy_json_flags_tail_into() {
    local __gpy_jft_tail="" __gpy_jft_prev_bg
    [[ "$2" == "true" ]] && __gpy_jft_tail="${__gpy_jft_tail},\"is_last\":true"
    [[ "$3" == "true" ]] && __gpy_jft_tail="${__gpy_jft_tail},\"is_first\":true"
    if [[ -n "$4" ]]; then
        __gpy_escape_json_into __gpy_jft_prev_bg "$4"
        __gpy_jft_tail="${__gpy_jft_tail},\"prev_bg\":\"${__gpy_jft_prev_bg}\""
    fi
    printf -v "$1" '%s' "$__gpy_jft_tail"
}

# Shared git/lang request builder, including the activated virtualenv, into
# variable $1. Locals are __gpy_bdr_* so a caller's OUTVAR is not shadowed.
__gpy_build_data_request_into() {
    local __gpy_bdr_op="$2" __gpy_bdr_format="${4:-bash-prompt}"
    local __gpy_bdr_cwd __gpy_bdr_tail __gpy_bdr_venv=""
    __gpy_escape_json_into __gpy_bdr_cwd "$3"
    __gpy_json_flags_tail_into __gpy_bdr_tail "${5:-}" "${7:-}" "${6:-}"
    [[ "$__gpy_bdr_op" == "lang" ]] && __gpy_lang_venv_json_into __gpy_bdr_venv
    printf -v "$1" '%s' "{\"op\":\"$__gpy_bdr_op\",\"cwd\":\"$__gpy_bdr_cwd\",\"format\":\"$__gpy_bdr_format\"${__gpy_bdr_tail}${__gpy_bdr_venv}}"
}

# Direct IPC data op for background refreshes. No oneshot fallback: without a
# daemon nothing would repaint anyway.
__gpy_trigger_data_refresh() {
    local op="$1"
    local cwd="${2:-$PWD}"
    local is_last="${3:-false}"
    local prev_bg="${4:-}"
    local request

    __gpy_build_data_request_into request "$op" "$cwd" bash-prompt "$is_last" "$prev_bg" ""
    __gpy_send_json "$request" >/dev/null 2>&1
}

# Bounded IPC-only query; oneshot is unbounded and would break the omit
# budget. The agent writes its own cache, so success needs no refresh. $6 is
# the reply budget in ms (default GPY_IPC_TIMEOUT_MS).
__gpy_sync_data_request() {
    local op="$1" cwd="$2" is_last="${3:-}" prev_bg="${4:-}" is_first="${5:-}" timeout_ms="${6:-}"
    local request response
    __gpy_build_data_request_into request "$op" "$cwd" bash-prompt "$is_last" "$prev_bg" "$is_first"
    __gpy_send_json "$request" "$timeout_ms" response 2>/dev/null || return 1
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

    local request timeout_ms=""
    __gpy_build_data_request_into request "$op" "$context_path" "$format" "$is_last" "$prev_bg" "$is_first"
    # A directory request means the memo had nothing to show.
    [[ "$op" == "directory" ]] && __gpy_cold_timeout_ms_into timeout_ms

    __gpy_send_json "$request" "$timeout_ms"
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

# Returns __gpy_seg_status_oneshot whenever it forked oneshot, even with no
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
    return "$__gpy_seg_status_oneshot"
}

# The response embeds bash's `\D{…}` token, so the clock ticks between
# renders. No oneshot fallback: clock.bash has a pure-bash renderer.
__gpy_request_clock() {
    local is_last="${1:-}"
    local prev_bg="${2:-}"
    local is_first="${3:-}"

    local flags_tail
    __gpy_json_flags_tail_into flags_tail "$is_last" "$is_first" "$prev_bg"
    local request="{\"op\":\"clock\",\"shell\":\"bash\",\"format\":\"bash-prompt\"${flags_tail}}"

    local result
    __gpy_send_json "$request" "" result || return 1

    [[ -n "$result" ]] || return 1
    echo "$result"
    return 0
}

# Duration request with oneshot fallback; returns __gpy_seg_status_oneshot when
# it forked.
__gpy_request_duration() {
    local duration_ms="$1"
    local is_last="${2:-false}"
    local prev_bg="${3:-}"
    local is_first="${4:-false}"

    local flags_tail
    __gpy_json_flags_tail_into flags_tail "$is_last" "$is_first" "$prev_bg"
    local request="{\"op\":\"duration\",\"duration_ms\":${duration_ms},\"format\":\"bash-prompt\"${flags_tail}}"

    local result send_status used_oneshot=0
    __gpy_send_json "$request" "" result
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
        return "$__gpy_seg_status_oneshot"
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
    __gpy_json_flags_tail_into flags_tail "$is_last" "" "$prev_bg"
    local request="{\"op\":\"character\",\"success\":${success_val},\"format\":\"bash-prompt\"${flags_tail}}"

    local result send_status used_oneshot=0
    __gpy_send_json "$request" "" result
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
        return "$__gpy_seg_status_oneshot"
    fi
    [[ -n "$result" ]] && return 0
    return 1
}

# Exit-status pill request ("exit_status": the bare "status" op is the agent
# health check). success is a 0/1 flag (1 == exit status 0). No oneshot
# fallback: the status segment draws the pill locally instead. An error reply
# (an older agent without the op) counts as no reply.
__gpy_request_exit_status() {
    local success="$1"
    local is_last="${2:-}"
    local prev_bg="${3:-}"
    local is_first="${4:-}"

    local success_val=false
    [[ "$success" == "1" ]] && success_val=true

    local flags_tail
    __gpy_json_flags_tail_into flags_tail "$is_last" "$is_first" "$prev_bg"
    local request="{\"op\":\"exit_status\",\"success\":${success_val},\"format\":\"bash-prompt\"${flags_tail}}"

    local result
    if __gpy_send_json "$request" "" result && [[ -n "$result" && "$result" != '{"error"'* ]]; then
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

    __gpy_ipc_endpoint_into socket
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
