# bash/core/ipc.bash
# IPC communication with gpy-agent

# True unless GPY_AGENT_ENABLED is set to something other than 1 (agent-free
# mode, #841). Every path that would start, register with, or talk to the
# agent consults this; prompt rendering never does, so it keeps working
# through oneshot.
__gpy_agent_enabled() {
    [[ -z "${GPY_AGENT_ENABLED:-}" || "${GPY_AGENT_ENABLED:-}" == "1" ]]
}

# Escape JSON string. Order matters: backslashes first (so escapes introduced
# by later steps aren't re-escaped), then quotes, then control characters --
# mirrors fish/core/ipc.fish's __gpy_json_escape exactly (#614), which also
# escapes \n/\r/\t: a branch name, commit message, or cwd containing a raw
# newline/CR/tab would otherwise produce malformed JSON the agent rejects,
# silently dropping the segment (same failure mode #302 fixed for backslash).
__gpy_escape_json() {
    local s="$1"
    # Escape backslashes first
    s="${s//\\/\\\\}"
    # Escape double quotes
    s="${s//\"/\\\"}"
    # Escape control characters the JSON spec forbids raw inside a string.
    s="${s//$'\n'/\\n}"
    s="${s//$'\r'/\\r}"
    s="${s//$'\t'/\\t}"
    echo "$s"
}

# The virtualenv to forward on `lang` requests: $VIRTUAL_ENV, else
# $CONDA_PREFIX when $CONDA_DEFAULT_ENV is set and not `base` (#729; pinned by
# tests/fixtures/venv_forwarding_vectors.tsv). Prints nothing when none applies.
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

# The home directory, with the passwd-database fallback the agent and the
# other two shells apply (#626).
#
# Bash, unlike Fish and Zsh, does not re-derive $HOME when the environment does
# not carry it, so every `$HOME/...` resolver here used to build a
# filesystem-root path (`/.cache/gpy/gpy.sock`) -- a third answer, agreeing
# with neither the shells nor the agent, in exactly the HOME-less environments
# (cron, `env -i`, some systemd units) where the shell most needs to find the
# running agent. `~user` is the derivation that consults the passwd database:
# a bare `~` returns $HOME whenever it is set, including when it is set to the
# empty string, which `std::env::home_dir` (and therefore the agent) treats as
# unset.
#
# Writes into the caller-named variable via `printf -v` -- the same indirect
# pattern as __gpy_ms_to_secs below -- so the common case (HOME set) costs no
# fork on the prompt path. Returns non-zero, leaving the variable untouched,
# when no absolute home can be derived at all; callers fall through to their
# own last resort exactly as the agent falls through to /tmp/gpy.
# The local is named `__gpy_home_value`, not `home`: shell locals are
# dynamically scoped, so a local named after the variable a caller passes as
# OUT_VAR would shadow the caller's variable and `printf -v` would write into
# this function's copy, which dies with the call.
__gpy_home() {
    local out_var="$1" __gpy_home_value="${HOME:-}" __gpy_home_user
    if [[ -z "$__gpy_home_value" ]]; then
        __gpy_home_user="$(id -un 2>/dev/null)" || return 1
        [[ -n "$__gpy_home_user" ]] || return 1
        # Assignment context, so bash tilde-expands "~user" from passwd; an
        # unknown user leaves the literal "~user", which the guard below
        # rejects.
        eval "__gpy_home_value=~$__gpy_home_user" 2>/dev/null
    fi
    [[ "$__gpy_home_value" == /* ]] || return 1
    printf -v "$out_var" '%s' "$__gpy_home_value"
}

# Determine socket path following XDG standards.
#
# An $XDG_* variable counts as set only when it is non-empty AND absolute, per
# the XDG Base Directory spec and matching crate::paths::xdg_value and the
# other two shells (#626).
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

    # Last resort, matching paths::runtime_root_for's final branch.
    echo "/tmp/gpy/gpy.sock"
}

# The agent's runtime root (crate::paths::runtime_root_for): where the
# socket lives by default and where the agent looks for the shell PIDs it
# nudges after a restart (#638). Mirrors fish's
# __gpy_runtime_root, including the same fallbacks.
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

# Directory of shell PIDs the agent wakes when it (re)starts, and this
# shell's own entry in it. The file name IS the PID the agent signals, so it
# must be numeric. The agent's `<pid>.reload` / `<pid>.reregister` doorbell
# flags (#674) live next to it.
__gpy_shell_registry_dir() {
    echo "$(__gpy_runtime_root)/shells"
}
__gpy_shell_registry_file() {
    echo "$(__gpy_shell_registry_dir)/$$"
}

# Record this shell so an agent restart can nudge it to re-register (#638):
# the agent only signals PIDs it finds here, and until this landed Bash never
# wrote one, so an open Bash shell stopped receiving agent notifications for
# the rest of its life after any agent restart. Also caches the doorbell flag
# base path so __gpy_consume_shell_flags checks flags without forking.
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

# Directory holding the agent's pre-rendered instant-prompt cache files.
#
# Extracted from __gpy_read_instant_cache verbatim (same `-n` guards, same
# precedence, same "no directory" signal: no output, non-zero status) so the
# instant-cache read path and __gpy_debug_paths resolve it through one
# implementation rather than two. The parity harness
# (tests/fish/path_parity.test.fish) diffs this against the agent's own
# resolver.
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

# Path to the cached Bash-format theme export. Mirrors Fish's
# __gpy_theme_export_cache_path (fish/core/util.fish) exactly, one directory
# level up (same XDG_CACHE_HOME/HOME precedence as __gpy_instant_cache_dir
# above, not the "instant-prompts" subdirectory). The agent writes this file
# atomically on startup and after every config/theme change
# (write_theme_export_to_dir, gpy-agent/src/cache/theme_export.rs, #614), so
# __gpy_load_theme (init.bash) can source it instead of forking `gpy-agent
# theme export` on every shell start.
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

# Convert a millisecond count to whole.fractional seconds (e.g. 150 -> "0.150")
# for the `timeout` invocation below. Pure integer arithmetic assigned into
# the *named* variable OUT_VAR via indirect `printf -v` (see
# __gpy_epoch_diff_ms in init.bash for the same pattern) rather than
# `echo`+command-substitution, so this never forks -- safe to call on every
# send rather than needing a version gate or a once-per-session precompute
# (#341). GPY_IPC_TIMEOUT_MS is a session constant (constants.bash), but since
# the conversion itself is already fork-free there is nothing a precompute
# would save.
__gpy_ms_to_secs() {
    local ms="$1" out_var="$2"
    # Guard against a non-numeric override so `$(( ))` below can't error out
    # and silently break the IPC send path.
    [[ "$ms" =~ ^[0-9]+$ ]] || ms=150
    printf -v "$out_var" '%d.%03d' "$(( ms / 1000 ))" "$(( ms % 1000 ))"
}

# Does nc(1) take -U (Unix sockets)? Probed once with `nc -h` and cached, the
# way Fish does (#850): netcat-traditional has no -U, and treating it as a
# client would fail every request after forking it. The cache lives in the
# shell, but prompt segments run in `$(...)` subshells that cannot write it
# back, so the probe is also run once at source time when nc is the client
# that would be used (below).
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

# True when some Unix-socket client exists: socat, or an nc that takes -U.
__gpy_ipc_client_available() {
    command -v socat &>/dev/null && return 0
    command -v nc &>/dev/null || return 1
    __gpy_nc_probe_capabilities
    [[ "$__gpy_nc_supports_unix" == 1 ]]
}

# Send raw JSON message to agent. Prints the reply and returns:
#   0  a complete reply arrived within GPY_IPC_TIMEOUT_MS;
#   1  the agent could not be reached (no socket, connection refused, no
#      usable client, agent-free mode);
#   2  the agent accepted the request but sent no complete reply in the
#      budget. The request is already on the wire, so the caller must not
#      recompute the segment with a oneshot fork (#757); it omits the segment
#      this render. Zsh and Fish apply the same three outcomes (#845).
#
# Uses `IFS= read -r response < <(...)` rather than `response=$(...)`: command
# substitution strips the trailing newline unconditionally, so it cannot tell
# a complete response from a truncated one. `read` returns nonzero when EOF is
# hit before the newline delimiter is found — that's how a partial write (the
# agent stalled or died mid-response, e.g. wrote `{"status":"o` and went
# silent) is distinguished from a complete one. A bare non-empty check would
# happily hand that truncated string to callers that printf raw ANSI straight
# into the prompt (#300). Process substitution keeps `read` in the current
# shell (not a pipe subshell) so `response` and `$?` are both visible here.
#
# The client's exit status rides the same stream, as `\037<status>\n` after
# whatever the client printed: `read` has no other way to learn whether socat
# or nc connected (a late agent: exit 0, or 124 when `timeout` killed nc) or
# never did (a refused or vanished socket: exit 1). A complete reply is read
# before the trailer ever matters; a partial or absent one leaves the marker in
# the line. \037 (unit separator) is not emitted by any renderer.
#
# `exec 2>/dev/null` opens each substitution: `read` closes its end as soon as
# the reply line arrives, so the trailer `printf` (and, for a client that
# exits early, the `echo`) can write to a closed pipe. Where SIGPIPE is
# ignored -- a shell spawned by Python, Node, .NET (every GitHub Actions
# step) or many IDE terminals inherits that -- the write fails with EPIPE and
# bash prints "printf: write error: Broken pipe" into the prompt, once per
# request. Nothing in the substitution's stderr is wanted.
__gpy_send_json() {
    # Agent-free mode (GPY_AGENT_ENABLED=0) never talks to a daemon, even one
    # another shell started: callers fall back to oneshot (#841).
    __gpy_agent_enabled || return 1

    local json="$1"
    local socket_path
    socket_path="$(__gpy_ipc_endpoint)"
    local response="" read_status=1 timeout_secs client_status="" connected_codes=" 0 "
    # One reply budget for every transport (#757).
    __gpy_ms_to_secs "${GPY_IPC_TIMEOUT_MS:-150}" timeout_secs

    # Check if socket exists
    [[ -S "$socket_path" ]] || return 1

    # Try socat first (most reliable)
    if command -v socat &>/dev/null; then
        IFS= read -r response < <(exec 2>/dev/null; echo "$json" | socat -t "$timeout_secs" - "UNIX-CONNECT:$socket_path" 2>/dev/null; printf '\037%s\n' "$?")
        read_status=$?
    # Try nc with -U flag (Unix socket). nc's -w only accepts whole seconds, so
    # a hung-but-connected agent stalls the prompt for a full second rather
    # than the ~150ms IPC target; wrap with `timeout` for a tighter bound when
    # it's available (#324). Falls back to the coarser -w 1 bound otherwise.
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

    # No usable reply. A client that connected, or a complete-but-empty reply
    # line, means the agent is alive: report it as late, not as unreachable.
    if [[ -n "$client_status" ]]; then
        [[ "$connected_codes" == *" $client_status "* ]] && return 2
    elif [[ $read_status -eq 0 ]]; then
        return 2
    fi
    return 1
}

# Expected protocol version (must match gpy-agent/src/ipc/protocol.rs::PROTOCOL_VERSION)
GPY_EXPECTED_PROTOCOL_VERSION=2

# Check protocol version compatibility with the running agent.
#
# Returns 0 if versions match, or if the agent's status can't be queried
# (nothing to compare against yet -- not our problem to report here).
# Returns 1 on a confirmed mismatch, so callers (the supervisor) can treat
# a responsive-but-mismatched agent as not running and restart it (#307).
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

# Find git repository root
__gpy_find_git_root() {
    local start_dir="${1:-$PWD}"

    # Convert to absolute path. Kept as a real `realpath` fork (not replaced
    # with a builtin) (#341): the result feeds __gpy_path_to_cache_key below,
    # which MUST stay byte-for-byte identical to the Rust writer's
    # `path_to_cache_key` (gpy-agent/src/cache/instant_prompt.rs). realpath's
    # symlink resolution is part of that contract -- dropping it would
    # diverge the cache key (and the git-root result) for symlinked repos. One
    # fork here is cheaper than a broken instant cache.
    local dir
    dir=$(realpath "$start_dir" 2>/dev/null)
    [[ -z "$dir" ]] && return 1

    while [[ "$dir" != "/" ]]; do
        if [[ -e "$dir/.git" ]]; then
            echo "$dir"
            return 0
        fi
        # Parameter expansion instead of a `dirname` fork per directory level
        # (#341) -- the biggest fork win here, since this loop runs once per
        # ancestor directory. Mirrors the pattern already used in
        # bash/segments/language.bash. `${dir%/*}` yields "" (not "/") when
        # `dir` is a top-level entry like "/a" -- normalize back to "/" so the
        # loop condition above terminates on the next check, exactly like the
        # old `dirname "/a"` -> "/" step (neither version checks "/.git"
        # itself; both stop as soon as `dir` becomes "/").
        dir="${dir%/*}"
        [[ -z "$dir" ]] && dir="/"
    done

    return 1
}

# Convert path to cache key (matches Rust/fish injective encoding)
# Order matters: escape _ first so subsequent replacements don't double-encode.
__gpy_path_to_cache_key() {
    local path="$1"
    local key="${path//_/__}"
    key="${key//\//_s}"
    key="${key//\\/_b}"
    key="${key//:/_c}"
    key="${key// /_w}"
    echo "$key"
}

# Turn the cache key in the variable named by $1 into its on-disk stem, in
# place (#771): unchanged up to 200 characters (existing names stay
# byte-for-byte), else 50-character chunks joined by `/` so no filename
# component exceeds NAME_MAX. MUST match `cache_key_path_for` in
# gpy-agent/src/cache/instant_prompt.rs (pinned by
# tests/fixtures/cache_key_vectors.tsv). Builtins only, no subshell. Lengths
# count characters under a UTF-8 locale but bytes under LC_ALL=C, where a long
# non-ASCII key resolves elsewhere and simply misses the cache.
__gpy_chunk_cache_key() {
    local __gpy_ck_key="${!1}" __gpy_ck_stem="" __gpy_ck_i
    (( ${#__gpy_ck_key} > 200 )) || return 0
    for (( __gpy_ck_i = 0; __gpy_ck_i < ${#__gpy_ck_key}; __gpy_ck_i += 50 )); do
        __gpy_ck_stem+="${__gpy_ck_stem:+/}${__gpy_ck_key:__gpy_ck_i:50}"
    done
    printf -v "$1" '%s' "$__gpy_ck_stem"
}

# Filesystem-safe token identifying the previous-segment background a cache entry
# was rendered with. Empty -> "none" (context-free). Every non-alphanumeric byte
# becomes "_". MUST match `prev_bg_token` in
# gpy-agent/src/cache/instant_prompt.rs so both sides resolve the same cache file.
__gpy_prev_bg_token() {
    local prev_bg="$1"
    [[ -z "$prev_bg" ]] && {
        echo "none"
        return
    }
    echo "${prev_bg//[^a-zA-Z0-9]/_}"
}

# Cache-filename suffix for a given (is_last, is_first) pair, e.g.
# `__gpy_cache_variant_suffix git true false` -> "git_last". MUST match
# `variant_suffix` in gpy-agent/src/cache/instant_prompt.rs so both sides
# resolve the same cache file (#401).
__gpy_cache_variant_suffix() {
    local base="$1" is_last="$2" is_first="$3"
    local suffix="$base"
    [[ "$is_first" == "true" ]] && suffix="${suffix}_first"
    [[ "$is_last" == "true" ]] && suffix="${suffix}_last"
    echo "$suffix"
}

# Resolve the path an instant-cache entry for `suffix` (git* or lang*) under
# `cwd` is keyed by into the variable named by $3 (empty, status 1, when there
# is none). Git caches are keyed by the repository root. Language caches
# additionally cover non-Git project directories (detected by package.json,
# Cargo.toml, etc.), which the agent keys by the canonical request path.
# Mirror that fallback so warm language caches are consumed outside Git
# repositories instead of cold-missing forever (#173). Out-var, not `$(...)`,
# so callers pay no extra subshell on the prompt path.
__gpy_instant_cache_key_path() {
    local suffix="$1" cwd="$2" __gpy_kp
    __gpy_kp="$(__gpy_find_git_root "$cwd")"
    if [[ -z "$__gpy_kp" && "$suffix" == lang* ]]; then
        # No Git root: language cache is keyed by the canonical request path.
        __gpy_kp="$(realpath "$cwd" 2>/dev/null)"
    fi
    printf -v "$3" '%s' "$__gpy_kp"
    [[ -n "$__gpy_kp" ]]
}

# Return 0 iff any instant-cache entry exists for `base` (git|lang) under
# `cwd`, in any position variant or prev_bg context (#766). The agent writes
# every is_last/is_first variant at once, but the context-free `.none` token
# only when some request carried no prev_bg, so a probe of `.none` alone
# would miss a cache populated only by contextual requests. No resolvable
# cache directory counts as no entry, matching __gpy_read_instant_cache.
__gpy_instant_cache_present() {
    local base="$1" cwd="${2:-$PWD}" key_path cache_dir cache_key hit found=1 had_failglob=0
    __gpy_instant_cache_key_path "$base" "$cwd" key_path || return 1
    cache_dir="$(__gpy_instant_cache_dir)" || return 1
    cache_key="$(__gpy_path_to_cache_key "$key_path")"
    # `<key>.<base>.<token>.bash` and `<key>.<base>_<position>.<token>.bash`.
    # An unmatched glob stays literal and fails the -e test; a user's
    # failglob would instead abort the loop with an error, so lift it here.
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

# Read instant-prompt cache if available (serve-stale model: always serve).
# Status is carried ENTIRELY by the exit code (#614, mirrors the contract
# #612 gave Fish -- see fish/core/ipc.fish's __gpy_read_instant_cache doc
# comment):
#   0 = fresh, exact-token hit
#   1 = miss (no cache file at all; nothing printed)
#   2 = stale, exact-token hit
#   4 = fresh, served via the `.none` variant fallback
#   6 = stale, served via the `.none` variant fallback
# (bit 2 = stale, bit 4 = variant fallback). Callers read `$?` off the plain
# `out=$(__gpy_read_instant_cache ..)` assignment -- no more `_stale_$$` temp
# file for callers to `rm -f`/`-e` check.
__gpy_read_instant_cache() {
    local suffix="${1:-git}"
    local cwd="${2:-$PWD}"
    local prev_bg="${3:-}"

    # Resolve the cache-key path (git root, or the canonical project path for
    # language caches outside Git).
    local key_path
    __gpy_instant_cache_key_path "$suffix" "$cwd" key_path || return 1

    # Compute cache directory
    local cache_dir
    cache_dir="$(__gpy_instant_cache_dir)" || return 1

    # Convert resolved path to cache key and build cache file path. The rendered
    # ANSI bakes in the fg:prev_bg opening chevron, so the cache is keyed by the
    # previous-segment background too. Fall back to the context-free ("none")
    # render when no context-specific entry exists yet — the caller then triggers
    # a refresh that populates the correct token file and repaints via the doorbell.
    local cache_key token
    cache_key="$(__gpy_path_to_cache_key "$key_path")"
    __gpy_chunk_cache_key cache_key
    token="$(__gpy_prev_bg_token "$prev_bg")"
    # `.bash`: the agent's bash-prompt dialect, escaped for PS1 (#677). The
    # `.ansi` files are fish's and must never reach PS1.
    local cache_file="$cache_dir/$cache_key.$suffix.$token.bash"
    # Track whether this read is about to fall back to the `.none` variant
    # instead of serving the requested token's own file (bit 4 below): the
    # content is still correct, but its opening chevron was rendered without
    # this render's real prev_bg context.
    local used_variant_fallback=0
    if [[ ! -f "$cache_file" && "$token" != "none" ]]; then
        cache_file="$cache_dir/$cache_key.$suffix.none.bash"
        used_variant_fallback=1
    fi

    # Serve stale-or-fresh cache; staleness/variant-fallback are folded into
    # the exit code (see contract above) instead of a side-effect temp file.
    if [[ -f "$cache_file" ]]; then
        local now mtime age
        # `printf -v` epoch time (no `date` fork) on bash 4.2+ (#341); older
        # bash falls back to the original `date +%s` fork.
        if [[ "${__gpy_have_printf_epoch:-0}" == "1" ]]; then
            printf -v now '%(%s)T' -1
        else
            now=$(date +%s 2>/dev/null)
        fi
        # GNU coreutils (stat -c) first, then BSD/macOS (stat -f). The reverse
        # order breaks on Linux: there `stat -f` means --file-system and returns
        # non-numeric text *successfully*, so the fallback never runs and the
        # arithmetic below crashes with "syntax error in expression". No bash
        # builtin computes mtime, so `stat` stays a real fork -- the one
        # intentional remaining per-read fork in this function (#341).
        mtime=$(stat -c %Y "$cache_file" 2>/dev/null || stat -f %m "$cache_file" 2>/dev/null)

        local exit_code=0
        # Require numeric operands: a non-numeric mtime (e.g. a stat quirk) must
        # not crash the prompt via a bad $((...)).
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

        # `$(<file)` is bash's built-in fast-path for reading a whole file --
        # it reads directly in-process instead of forking `cat` (#341), and
        # has been supported since early bash 2.x, so no version gate is
        # needed. Verified byte-identical to the old `cat` here: every
        # instant-cache file is written by the Rust prompt formatter
        # (gpy-agent/src/formatter/fish_ansi.rs), which never appends a trailing
        # newline, so there is nothing for `$(<file)`'s newline-stripping to
        # change (confirmed against real on-disk cache files -- none end in
        # 0x0a). Even if a writer ever changed that, both call sites capture
        # this function's own output via `$(...)`, which strips a trailing
        # newline unconditionally regardless of how the file was read.
        printf '%s' "$(<"$cache_file")"
        return "$exit_code"
    fi

    return 1
}

# Shared JSON tail for is_last/is_first/prev_bg (#613). Every IPC request
# builder below appends this SAME sequence -- one implementation instead of
# several hand-built copies, so a bug fixed once is fixed everywhere. Mirrors
# __gpy_json_flags_tail in fish/core/ipc.fish and zsh/core/ipc.zsh exactly.
# Convention (first-party, all three shells): a caller passes is_last/is_first
# as "true" or "" (never "false") -- see init.bash's render-loop dispatch.
# Emits nothing for a flag that isn't literally "true" (an absent flag is
# OMITTED from the JSON, not sent as `false`; the agent defaults it to false
# via `#[serde(default)]`, gpy-agent/src/ipc/protocol.rs) and nothing for an
# empty prev_bg. No trailing newline -- callers concatenate this directly
# onto their request string.
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

# Build the JSON request for a git/lang data op (format, is_last/is_first/
# prev_bg tail, and the activated virtualenv for language detection -- see
# fish/core/ipc.fish's __gpy_build_data_payload). One builder shared by
# __gpy_request, __gpy_trigger_data_refresh and __gpy_sync_data_request so a
# request-shape fix lands everywhere.
__gpy_build_data_request() {
    local op="$1" cwd="$2" format="${3:-bash-prompt}" is_last="${4:-}" prev_bg="${5:-}" is_first="${6:-}"
    local json_cwd flags_tail venv_json=""
    json_cwd="$(__gpy_escape_json "$cwd")"
    flags_tail="$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")"
    [[ "$op" == "lang" ]] && venv_json="$(__gpy_lang_venv_json)"
    printf '%s' "{\"op\":\"$op\",\"cwd\":\"$json_cwd\",\"format\":\"$format\"${flags_tail}${venv_json}}"
}

# Send a data op directly to the agent via IPC, bypassing the instant cache.
# Used for background refreshes after serving a stale cache entry. No oneshot
# fallback: without a running daemon there is no agent repaint, so the
# already-served stale prompt simply remains until the agent returns.
__gpy_trigger_data_refresh() {
    local op="$1"
    local cwd="${2:-$PWD}"
    local is_last="${3:-false}"
    local prev_bg="${4:-}"

    __gpy_send_json "$(__gpy_build_data_request "$op" "$cwd" bash-prompt "$is_last" "$prev_bg" "")" >/dev/null 2>&1
}

# Bounded synchronous IPC query (#434/#436): one round-trip, IPC only -- never
# the oneshot fallback, which is unbounded and would break the "graceful omit
# within budget" contract. __gpy_send_json is gated on the socket existing (a
# DOWN agent is an instant failure, no connect, no hang) and bounded by
# GPY_IPC_TIMEOUT_MS. Prints the agent's rendered prompt text; non-zero on any miss.
# The agent writes its own instant cache and notifies other shells while
# handling the request, so a success needs no extra refresh. Mirrors the
# inline `__gpy_ipc_send` block in fish/segments/git.fish.
__gpy_sync_data_request() {
    local op="$1" cwd="$2" is_last="${3:-}" prev_bg="${4:-}" is_first="${5:-}"
    local response
    response="$(__gpy_send_json "$(__gpy_build_data_request "$op" "$cwd" bash-prompt "$is_last" "$prev_bg" "$is_first")" 2>/dev/null)" || return 1
    [[ -n "$response" ]] || return 1
    printf '%s' "$response"
}

# Predicates over __gpy_read_instant_cache's exit-code status contract (0 fresh
# hit, 1 miss, 2 stale hit, 4 fresh variant-fallback hit, 6 stale variant-
# fallback hit -- bit 2 = stale, bit 4 = variant fallback). Mirror fish's
# __gpy_cache_status_stale/_variant.
__gpy_cache_status_stale() { (( ($1 & 2) != 0 )); }
__gpy_cache_status_variant() { (( ($1 & 4) != 0 )); }

# Epoch milliseconds. Fork-free via EPOCHREALTIME (bash 5+, whose radix
# follows the locale -- strip either separator); older bash has only whole
# seconds, which coarsens the 500ms throttle to 1s but never loosens it.
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

# Directory of per-shell refresh-throttle stamps. Segments render inside
# `$(...)` subshells, so an in-memory throttle variable (Fish's approach)
# would be lost on return; the stamp lives on disk under the private runtime
# root instead, keyed by this shell's PID.
__gpy_refresh_throttle_dir() {
    echo "$(__gpy_runtime_root)/refresh-throttle"
}

# Shared throttled background refresh dispatcher (port of Fish's
# __gpy_maybe_refresh, #612). At most one refresh per 500ms per
# op+suffix+path (+ optional prev_bg token for the variant-fallback path, so a
# recent refresh for a different context cannot starve this one). Never
# blocks: the request is backgrounded and its output discarded -- the goal is
# the agent's side effects (recompute, instant-cache write, doorbell-on-change).
# Agent-free mode never refreshes; a DOWN agent (no socket) is a cheap failure
# inside __gpy_send_json, as before.
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

    # Cheap no-op once registered (#419); must run before the fork because a
    # backgrounded child could not propagate __gpy_registered back.
    __gpy_register_with_agent >/dev/null 2>&1

    __gpy_trigger_data_refresh "$op" "$root" "$is_last" "$prev_bg" >/dev/null 2>&1 &
    disown $! 2>/dev/null
}

# High-level request wrapper
# For git operations, tries instant-prompt cache first for 0ms latency
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
    # 2: connected but the reply was late (#757). The agent is computing the
    # same segment, so omit it this render rather than fork a oneshot.
    (( send_status == 2 )) && return 1
    if (( send_status != 0 )); then
        # oneshot fallback has no --prev-bg flag; the opening chevron color is
        # skipped until the daemon connects (mirrors fish). is_first IS passed
        # (--first, #401), so the opening cap's presence/absence is correct.
        # __gpy_fallback_oneshot's own exit code (0/1/GPY_SEG_STATUS_ONESHOT)
        # becomes __gpy_request's return status, since it's the last command
        # run in this branch.
        __gpy_fallback_oneshot "$op" "$context_path" "$format" "$is_last" "$is_first"
    fi
}

# Per-render budget limiting oneshot-fallback forks to one per prompt render
# (#614). Without this, a dead/hung daemon costs one `gpy-agent oneshot` fork
# per segment (git, lang, directory, duration, character -- up to 5 per
# prompt), each with its own process-start latency (#324).
#
# Segments run inside `$(...)` command-substitution subshells and cannot
# write back to __gpy_render_prompt's scope, so this is no longer a
# predictable `${TMPDIR:-/tmp}/.gpy_oneshot_used_$$` marker file (#342's
# rationale for a file, #614's reason to remove it): __gpy_render_prompt
# resets the plain variable __gpy_oneshot_used=0 at the start of each render;
# a request wrapper that actually claims the budget and forks oneshot
# returns GPY_SEG_STATUS_ONESHOT, which the render loop reads off `$?` after
# every `segment_output=$($segment_fn ..)` call and folds into
# __gpy_oneshot_used=1 in ITS OWN scope -- later segments' subshells inherit
# that 1 by fork, so __gpy_oneshot_claim (below, running inside one of those
# subshells) denies a second fork for the rest of this render.
__gpy_oneshot_claim() {
    [[ "${__gpy_oneshot_used:-0}" == "1" ]] && return 1
    return 0
}

# Fallback to oneshot mode. Returns GPY_SEG_STATUS_ONESHOT whenever it
# actually claimed the budget and forked `gpy-agent oneshot` -- regardless of
# whether that oneshot call itself produced output -- so the render loop can
# track the spent budget; 1 otherwise (binary missing, or budget already
# spent this render).
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

# Send a clock render request to the agent via IPC. Used when a theme sets
# `[segments.clock].format`; the response embeds bash's own `\D{…}` prompt
# token (see gpy-agent/src/formatter/clock_resolver.rs), so the clock still
# ticks between draws off a single render.
#
# No oneshot fallback: clock.bash falls back to its own pure-bash renderer, so
# an unreachable agent degrades the segment to an uncapped clock rather than
# costing a fork per prompt.
#
# is_last/is_first: "true" or "" (#613).
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

# Send a duration render request to the agent via IPC, with oneshot fallback.
# Unlike git/lang/directory there is no cwd — only duration_ms is required.
# Returns GPY_SEG_STATUS_ONESHOT when the oneshot fallback was actually
# claimed+forked this call (see __gpy_fallback_oneshot above), 0 on a normal
# IPC hit with output, 1 otherwise.
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
    # 2 = connected but the reply was late: omit rather than recompute (#757).
    if (( send_status != 0 && send_status != 2 )); then
        # Fallback to oneshot, capped to one fork per prompt render (#324).
        # NOTE: oneshot fallback does not pass prev_bg — the CLI subcommand
        # has no --prev-bg flag (mirrors fish). is_first IS passed (--first,
        # #401).
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

# Send a character (prompt symbol) render request to the agent via IPC, with
# oneshot fallback. Input is a 0/1 success flag (1 == exit status 0); there is
# no cwd. The character is always the last element, and every caller (init.bash)
# passes is_last="true" explicitly, so no default is needed here.
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
    # 2 = connected but the reply was late: omit rather than recompute (#757).
    if (( send_status != 0 && send_status != 2 )); then
        # Fallback to oneshot, capped to one fork per prompt render (#324).
        # Convert the boolean back to an exit-code integer.
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

# Send a hostname render request to the agent via IPC. No oneshot fallback:
# a `oneshot hostname` subcommand does exist (added for the starship-parity
# test harness), but this segment intentionally does not call it — under the
# starship theme with a dead daemon, the hostname segment should render
# nothing until the daemon comes back up rather than fall back to a
# per-prompt subprocess. A failed/empty IPC round-trip simply renders
# nothing for that prompt.
#
# is_last: "true" or "" (#613 -- the same convention every segment now uses).
# The caller (hostname.bash) always passes it explicitly, so no default here.
# is_ssh: "1" in an SSH session ($__gpy_is_ssh). Sent as "is_ssh":true so the
# agent draws the theme icon only over SSH (#826); omitted otherwise.
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

# Send a username render request to the agent via IPC. No oneshot fallback, for
# the same reason as __gpy_request_hostname above: under the starship theme with
# a dead daemon the username segment renders nothing until the daemon returns.
#
# is_last: "true" or "" (#613 -- the same convention every segment now uses).
# The caller (username.bash) always passes it explicitly, so no default here.
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

# Emit every path this shell resolves from the environment, as `key=value`
# lines in the agent's key order.
#
# Counterpart to `gpy debug paths --format kv`.
# `tests/fish/path_parity.test.fish` runs both under a matrix of synthetic
# environments and diffs the full maps, so path resolution cannot drift
# between the agent and this shell (#476).
#
# Every value comes from the resolver the prompt itself calls; nothing here
# re-derives a precedence rule, because a reimplementation would agree with
# itself and prove nothing. `cache_root` is the lexical parent of the
# instant-prompt cache directory — a derivation of a real resolver's output,
# not a second copy of the precedence.
#
# Two sentinels carry the cases a path string cannot: `<unimplemented>` for a
# key this shell has no resolver for, and `<unresolved>` for a resolver that
# ran and produced nothing. Bash reads the instant-prompt cache, talks to the
# socket, and (since #638) records its PID under the runtime root's shells/
# directory for the agent's restart nudge; it never reads a theme export by
# path of its own or locates config.toml — those keys are the agent's and
# Fish's, and the harness holds this shell to exactly that split.
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
