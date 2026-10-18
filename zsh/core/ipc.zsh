# zsh/core/ipc.zsh

# For $EPOCHSECONDS when sourced standalone.
zmodload zsh/datetime 2>/dev/null

# Gates every start/register/talk path; rendering still works via oneshot.
function __gpy_agent_enabled() {
    [[ "${GPY_AGENT_ENABLED:-1}" == "1" ]]
}

# Backslashes first so later escapes aren't re-escaped.
function __gpy_escape_json() {
    local s=$1
    s=${s//\\/\\\\}
    s=${s//\"/\\\"}
    s=${s//$'\n'/\\n}
    s=${s//$'\r'/\\r}
    s=${s//$'\t'/\\t}
    # -r: echo-style escape handling would collapse the doubled backslashes.
    print -r -- "$s"
}

# $VIRTUAL_ENV, else $CONDA_PREFIX unless the conda env is `base`.
function __gpy_forwarded_venv() {
    if [[ -n "${VIRTUAL_ENV:-}" ]]; then
        printf '%s' "$VIRTUAL_ENV"
    elif [[ -n "${CONDA_PREFIX:-}" && -n "${CONDA_DEFAULT_ENV:-}" && "$CONDA_DEFAULT_ENV" != "base" ]]; then
        printf '%s' "$CONDA_PREFIX"
    fi
}

function __gpy_lang_venv_json() {
    local venv
    venv="$(__gpy_forwarded_venv)"
    [[ -n "$venv" ]] && printf ',"virtual_env":"%s"' "$(__gpy_escape_json "$venv")"
    return 0
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

# Mirrors the agent's paths::runtime_root_for.
function __gpy_runtime_root() {
    local home
    if [[ -n "${XDG_RUNTIME_DIR:-}" && "${XDG_RUNTIME_DIR:-}" == /* ]]; then
        print -r -- "$XDG_RUNTIME_DIR/gpy"
        return
    fi
    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        print -r -- "$XDG_CACHE_HOME/gpy"
        return
    fi
    if __gpy_home home; then
        print -r -- "$home/.cache/gpy"
        return
    fi
    print -r -- "/tmp/gpy"
}

# The file name is the PID the agent signals; doorbell flags live beside it.
function __gpy_shell_registry_dir() {
    print -r -- "$(__gpy_runtime_root)/shells"
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
    throttle_dir="$(__gpy_runtime_root)/refresh-throttle"
    rm -f "$throttle_dir"/$$.*(N) 2>/dev/null || true
}

# Instant-prompt cache dir; no output and non-zero status when unresolvable.
function __gpy_instant_cache_dir() {
    local home

    if [[ -n "${XDG_CACHE_HOME:-}" && "${XDG_CACHE_HOME:-}" == /* ]]; then
        print -r -- "$XDG_CACHE_HOME/gpy/instant-prompts"
        return 0
    fi

    if __gpy_home home; then
        print -r -- "$home/.cache/gpy/instant-prompts"
        return 0
    fi

    return 1
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
    # One reply budget for every transport.
    __gpy_ms_to_secs "${GPY_IPC_TIMEOUT_MS:-}" timeout_secs

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
            print -r -- "$response"
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
        print -r -- "$response"
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

function __gpy_find_git_root() {
    local start_dir=${1:-$PWD}

    # Keep the realpath fork: symlink resolution is part of the cache-key
    # contract with the agent's path_to_cache_key.
    local dir
    dir=$(realpath "$start_dir" 2>/dev/null)
    [[ -z "$dir" ]] && return 1

    while [[ "$dir" != "/" ]]; do
        if [[ -e "$dir/.git" ]]; then
            print -r -- "$dir"
            return 0
        fi
        # `${dir:h}` avoids a dirname fork per level; `/a` yields `/`.
        dir=${dir:h}
    done

    return 1
}

# Must match the agent's injective encoding; escape _ first.
function __gpy_path_to_cache_key() {
    # `local path` would shadow zsh's PATH-tied array.
    local raw_path=$1
    local key="${raw_path//_/__}"
    key="${key//\//_s}"
    key="${key//\\/_b}"
    key="${key//:/_c}"
    key="${key// /_w}"
    print -r -- "$key"
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

# MUST match the agent's `prev_bg_token`.
function __gpy_prev_bg_token() {
    local prev_bg=$1
    if [[ -z "$prev_bg" ]]; then
        print -r -- "none"
        return
    fi
    print -r -- "${prev_bg//[^a-zA-Z0-9]/_}"
}

# MUST match the agent's `variant_suffix`.
function __gpy_cache_variant_suffix() {
    local base=$1 is_last=$2 is_first=$3
    local suffix="$base"
    [[ "$is_first" == "true" ]] && suffix="${suffix}_first"
    [[ "$is_last" == "true" ]] && suffix="${suffix}_last"
    print -r -- "$suffix"
}

# Git caches key by repo root; language caches outside Git key by the
# canonical cwd, as the agent does. Status 1 when there is none.
function __gpy_instant_cache_key_path() {
    local suffix=$1 cwd=$2 __gpy_kp
    __gpy_kp=$(__gpy_find_git_root "$cwd")
    if [[ -z "$__gpy_kp" && "$suffix" == lang* ]]; then
        __gpy_kp=$(realpath "$cwd" 2>/dev/null)
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
    cache_dir=$(__gpy_instant_cache_dir) || return 1
    cache_key=$(__gpy_path_to_cache_key "$key_path")
    __gpy_chunk_cache_key cache_key
    local -a hits=("$cache_dir/$cache_key.$base."*.zsh(N) "$cache_dir/$cache_key.${base}_"*.zsh(N))
    (( ${#hits} > 0 ))
}

# Serve-stale. Status: 0 fresh, 1 miss, +2 stale, +4 served via `.none` fallback.
function __gpy_read_instant_cache() {
    local suffix=${1:-git}
    local cwd=${2:-$PWD}
    local prev_bg=${3:-}

    local key_path
    __gpy_instant_cache_key_path "$suffix" "$cwd" key_path || return 1

    local cache_dir
    cache_dir=$(__gpy_instant_cache_dir) || return 1

    # The ANSI bakes in the fg:prev_bg chevron, so the key includes prev_bg;
    # fall back to `.none` and let the caller trigger a refresh.
    local cache_key token
    cache_key=$(__gpy_path_to_cache_key "$key_path")
    __gpy_chunk_cache_key cache_key
    token=$(__gpy_prev_bg_token "$prev_bg")
    # `.ansi` files are fish's and must never reach PROMPT.
    local cache_file="$cache_dir/$cache_key.$suffix.$token.zsh"
    local used_variant_fallback=0
    if [[ ! -f "$cache_file" && "$token" != "none" ]]; then
        cache_file="$cache_dir/$cache_key.$suffix.none.zsh"
        used_variant_fallback=1
    fi

    if [[ -f "$cache_file" ]]; then
        local now mtime age
        now=$EPOCHSECONDS
        # GNU stat first: on Linux `stat -f` succeeds with non-numeric output.
        mtime=$(stat -c %Y "$cache_file" 2>/dev/null || stat -f %m "$cache_file" 2>/dev/null)

        local exit_code=0
        if [[ "$now" =~ ^[0-9]+$ && "$mtime" =~ ^[0-9]+$ ]]; then
            age=$((now - mtime))
            local ttl=5 default_ttl=5
            if [[ "$suffix" == git* ]]; then
                ttl=${GPY_GIT_INSTANT_CACHE_TTL_SECONDS:-5}
            elif [[ "$suffix" == lang* ]]; then
                default_ttl=30
                ttl=${GPY_LANGUAGE_CACHE_TTL_SECONDS:-30}
            fi
            [[ "$ttl" =~ ^[0-9]+$ ]] || ttl=$default_ttl
            if (( age >= ttl )); then
                exit_code=$((exit_code + 2))
            fi
        fi
        if [[ "$used_variant_fallback" -eq 1 ]]; then
            exit_code=$((exit_code + 4))
        fi

        # `$(<file)` avoids a cat fork; cache files have no trailing newline.
        printf '%s' "$(<"$cache_file")"
        return "$exit_code"
    fi

    return 1
}

# Absent flags are omitted (the agent defaults them to false); callers pass
# "true" or "".
function __gpy_json_flags_tail() {
    local is_last=$1 is_first=$2 prev_bg=$3
    local tail=""
    [[ "$is_last" == "true" ]] && tail="${tail},\"is_last\":true"
    [[ "$is_first" == "true" ]] && tail="${tail},\"is_first\":true"
    if [[ -n "$prev_bg" ]]; then
        tail="${tail},\"prev_bg\":\"$(__gpy_escape_json "$prev_bg")\""
    fi
    printf '%s' "$tail"
}

# Shared by every git/lang request so the request shape stays in one place.
function __gpy_build_data_request() {
    local op=$1 cwd=$2 format=${3:-zsh-prompt} is_last=${4:-} prev_bg=${5:-} is_first=${6:-}
    local json_cwd flags_tail venv_json=""
    json_cwd=$(__gpy_escape_json "$cwd")
    flags_tail=$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")
    [[ "$op" == "lang" ]] && venv_json="$(__gpy_lang_venv_json)"
    printf '%s' "{\"op\":\"$op\",\"cwd\":\"$json_cwd\",\"format\":\"$format\"${flags_tail}${venv_json}}"
}

# Background refresh after serving stale cache. No oneshot fallback: without
# a daemon there is no repaint to deliver.
function __gpy_trigger_data_refresh() {
    local op=$1
    local cwd=${2:-$PWD}
    local is_last=${3:-false}
    local prev_bg=${4:-}

    __gpy_send_json "$(__gpy_build_data_request "$op" "$cwd" zsh-prompt "$is_last" "$prev_bg" "")" >/dev/null 2>&1
}

# One bounded IPC round trip, never oneshot (unbounded). The agent updates
# its cache and notifies other shells itself.
function __gpy_sync_data_request() {
    local op=$1 cwd=$2 is_last=${3:-} prev_bg=${4:-} is_first=${5:-}
    local response
    response=$(__gpy_send_json "$(__gpy_build_data_request "$op" "$cwd" zsh-prompt "$is_last" "$prev_bg" "$is_first")" 2>/dev/null) || return 1
    [[ -n "$response" ]] || return 1
    printf '%s' "$response"
}

# Bit tests for __gpy_read_instant_cache's status.
function __gpy_cache_status_stale() { (( ($1 & 2) != 0 )); }
function __gpy_cache_status_variant() { (( ($1 & 4) != 0 )); }

function __gpy_now_ms() {
    integer __gpy_ms=$(( EPOCHREALTIME * 1000 ))
    print -r -- $__gpy_ms
}

# On disk because segments run in subshells that can't keep in-memory state.
function __gpy_refresh_throttle_dir() {
    print -r -- "$(__gpy_runtime_root)/refresh-throttle"
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

    local dir bucket stamp_name stamp stamped="" last_ms=0 now_ms
    dir=$(__gpy_refresh_throttle_dir)
    # The dot-free fields lead, so the bucket stays injective; the encoded
    # path has no spaces, so it is everything after the stamp's first space.
    bucket="$op.$cache_suffix.$throttle_key_extra.$(__gpy_path_to_cache_key "$root")"
    __gpy_refresh_stamp_name "$bucket" stamp_name
    stamp="$dir/$stamp_name"
    now_ms=$(__gpy_now_ms)
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
        cache_suffix=$(__gpy_cache_variant_suffix git "$is_last" "$is_first")
        local cached_result
        cached_result=$(__gpy_read_instant_cache "$cache_suffix" "$context_path" "$prev_bg")
        if [[ -n "$cached_result" ]]; then
            print -r -- "$cached_result"
            return 0
        fi
    elif [[ "$op" == "lang" ]]; then
        local cache_suffix
        cache_suffix=$(__gpy_cache_variant_suffix lang "$is_last" "$is_first")
        local cached_result
        cached_result=$(__gpy_read_instant_cache "$cache_suffix" "$context_path" "$prev_bg")
        if [[ -n "$cached_result" ]]; then
            print -r -- "$cached_result"
            return 0
        fi
    fi

    local request
    request=$(__gpy_build_data_request "$op" "$context_path" "$format" "$is_last" "$prev_bg" "$is_first")

    __gpy_send_json "$request"
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
    flags_tail=$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")
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
    flags_tail=$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")
    local request="{\"op\":\"clock\",\"shell\":\"zsh\",\"format\":\"zsh-prompt\"${flags_tail}}"

    local result
    result=$(__gpy_send_json "$request")

    [[ -n "$result" ]] || return 1
    print -r -- "$result"
    return 0
}

# Only for a Starship-style `__hostname_format`. No oneshot fallback by
# design: a dead daemon yields no segment rather than a per-prompt fork.
function __gpy_request_hostname() {
    local hostname=$1
    local is_last=$2
    local prev_bg=${3:-}
    local is_ssh=${4:-}

    local json_hostname flags_tail ssh_json=""
    json_hostname=$(__gpy_escape_json "$hostname")
    flags_tail=$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")
    [[ "$is_ssh" == "1" ]] && ssh_json=',"is_ssh":true'
    local request="{\"op\":\"hostname\",\"hostname\":\"${json_hostname}\",\"format\":\"zsh-prompt\"${flags_tail}${ssh_json}}"

    local result
    result=$(__gpy_send_json "$request")

    if [[ -n "$result" ]]; then
        print -r -- "$result"
        return 0
    fi
    return 1
}

# No oneshot fallback, as for hostname.
function __gpy_request_username() {
    local username=$1
    local is_last=$2
    local prev_bg=${3:-}

    local json_username flags_tail
    json_username=$(__gpy_escape_json "$username")
    flags_tail=$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")
    local request="{\"op\":\"username\",\"username\":\"${json_username}\",\"format\":\"zsh-prompt\"${flags_tail}}"

    local result
    result=$(__gpy_send_json "$request")

    if [[ -n "$result" ]]; then
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
    flags_tail=$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")
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

    if instant_dir=$(__gpy_instant_cache_dir) && [[ -n "$instant_dir" ]]; then
        cache_root="${instant_dir%/*}"
    else
        instant_dir="<unresolved>"
        cache_root="<unresolved>"
    fi

    print -r -- "runtime_root=$(__gpy_runtime_root)"
    print -r -- "socket=$socket"
    print -r -- "shell_registry_dir=$(__gpy_shell_registry_dir)"
    print -r -- "cache_root=$cache_root"
    print -r -- "instant_prompts_dir=$instant_dir"
    print -r -- "theme_export_file=<unimplemented>"
    print -r -- "config_path=<unimplemented>"
    print -r -- "config_candidates=<unimplemented>"
    print -r -- "theme_dir=<unimplemented>"
}
