# zsh/core/ipc.zsh

# Needed for $EPOCHSECONDS (__gpy_read_instant_cache, #614): idempotent and a
# no-op if zsh/core/init.zsh's own `zmodload zsh/datetime` already ran, so
# this file also works when sourced standalone (e.g. a test that only needs
# ipc.zsh) without paying a `date` fork either way.
zmodload zsh/datetime 2>/dev/null

# Escape JSON string. Order matters: backslashes first (so escapes introduced
# by later steps aren't re-escaped), then quotes, then control characters --
# mirrors fish/core/ipc.fish's __gpy_json_escape exactly (#614), which also
# escapes \n/\r/\t: a branch name, commit message, or cwd containing a raw
# newline/CR/tab would otherwise produce malformed JSON the agent rejects,
# silently dropping the segment (same failure mode #302 fixed for backslash).
function __gpy_escape_json() {
    local s=$1
    # Escape backslashes first
    s=${s//\\/\\\\}
    # Escape double quotes
    s=${s//\"/\\\"}
    # Escape control characters the JSON spec forbids raw inside a string.
    s=${s//$'\n'/\\n}
    s=${s//$'\r'/\\r}
    s=${s//$'\t'/\\t}
    # print -r avoids echo's default interpretation of backslash escapes,
    # which would otherwise collapse the doubled backslashes above.
    print -r -- "$s"
}

# The home directory, with the passwd-database fallback the agent applies
# (#626).
#
# Zsh re-derives $HOME from passwd when the environment does not carry it, so
# this only differs from a bare "$HOME" for the one case zsh keeps verbatim:
# HOME set to the empty string, where `std::env::home_dir` (and therefore the
# agent, and Fish, and Bash's __gpy_home) uses the passwd home. `~user`
# consults passwd directly; a bare `~` would give back the empty $HOME.
# Writes into the caller-named variable via `printf -v` (the same indirect,
# fork-free pattern as __gpy_ms_to_secs below) because the instant-cache
# resolver runs on every prompt; a `$(...)` here would add a fork per render.
# The local is named `__gpy_home_value`, not `home`: shell locals are
# dynamically scoped, so a local named after the variable a caller passes as
# OUT_VAR would shadow the caller's variable and `printf -v` would write into
# this function's copy, which dies with the call.
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

# Determine socket path following XDG standards.
#
# An $XDG_* variable counts as set only when it is non-empty AND absolute, per
# the XDG Base Directory spec and matching crate::paths::xdg_value and the
# other two shells (#626).
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

    # Last resort, matching paths::runtime_root_for's final branch.
    print -r -- "/tmp/gpy/gpy.sock"
}

# The agent's runtime root (crate::paths::runtime_root_for): where the
# socket lives by default and where the agent looks for the shell PIDs it
# nudges after a restart (#638). Mirrors fish's
# __gpy_runtime_root, including the same fallbacks.
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

# Directory of shell PIDs the agent wakes when it (re)starts, and this
# shell's own entry in it. The file name IS the PID the agent signals, so it
# must be numeric. The agent's `<pid>.reload` / `<pid>.reregister` doorbell
# flags (#674) live next to it.
function __gpy_shell_registry_dir() {
    print -r -- "$(__gpy_runtime_root)/shells"
}
function __gpy_shell_registry_file() {
    print -r -- "$(__gpy_shell_registry_dir)/$$"
}

# Record this shell so an agent restart can nudge it to re-register (#638):
# the agent only signals PIDs it finds here, and until this landed Zsh never
# wrote one, so an open Zsh shell stopped receiving agent notifications for
# the rest of its life after any agent restart. Also caches the doorbell flag
# base path so TRAPURG checks flags without forking.
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
}

# Directory holding the agent's pre-rendered instant-prompt cache files.
#
# Extracted from __gpy_read_instant_cache verbatim (same `-n` guards, same
# precedence, same "no directory" signal: no output, non-zero status) so the
# instant-cache read path and __gpy_debug_paths resolve it through one
# implementation rather than two. The parity harness
# (tests/fish/path_parity.test.fish) diffs this against the agent's own
# resolver.
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

# Path to the cached Zsh-format theme export. Mirrors Fish's
# __gpy_theme_export_cache_path (fish/core/util.fish) exactly, one directory
# level up (same XDG_CACHE_HOME/HOME precedence as __gpy_instant_cache_dir
# above, not the "instant-prompts" subdirectory). The agent writes this file
# atomically on startup and after every config/theme change
# (write_theme_export_to_dir, gpy-agent/src/cache/theme_export.rs, #614), so
# __gpy_load_theme (init.zsh) can source it instead of forking `gpy-agent
# theme export` on every shell start.
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

# Convert a millisecond count to whole.fractional seconds (e.g. 150 ->
# "0.150") for the `timeout` invocation below. Pure integer arithmetic
# assigned into the *named* variable OUT_VAR via `printf -v` (mirrors
# __gpy_ms_to_secs in bash/core/ipc.bash exactly) rather than an `awk` fork
# (#614) -- safe to call on every send rather than needing a once-per-session
# precompute, since the conversion itself is already fork-free.
function __gpy_ms_to_secs() {
    local ms=$1 out_var=$2
    # Guard against a non-numeric override so `$(( ))` below can't error out
    # and silently break the IPC send path.
    [[ "$ms" =~ ^[0-9]+$ ]] || ms=150
    printf -v "$out_var" '%d.%03d' "$(( ms / 1000 ))" "$(( ms % 1000 ))"
}

# Send raw JSON message to agent
#
# Every read here checks its own exit status rather than just `-n "$response"`:
# `read` (both the zsocket fd read and the `IFS= read -r < <(...)` process-
# substitution reads below) returns nonzero when its timeout/EOF fires before
# the newline delimiter is found — that's how a partial write (the agent
# stalled or died mid-response, e.g. wrote `{"status":"o` and went silent) is
# distinguished from a complete one. A bare non-empty check would happily hand
# that truncated string to callers that print raw ANSI straight into the
# prompt (#300). Command substitution (`response=$(...)`) can't make this
# distinction — it strips the trailing newline unconditionally either way —
# so the socat/nc branches use process substitution instead, which keeps
# `read` in the current shell (not a pipe subshell) so `response` and `$?` are
# both visible here.
function __gpy_send_json() {
    local json=$1
    local socket_path=$(__gpy_ipc_endpoint)
    local response="" read_status=1
    local zsocket_connected=0

    # Check if socket exists
    if [[ ! -S "$socket_path" ]]; then
        return 1
    fi

    # 1. Try zsh/net/socket (Fastest, builtin)
    if zmodload zsh/net/socket 2>/dev/null; then
        local fd
        if zsocket "$socket_path" 2>/dev/null; then
            zsocket_connected=1
            fd=$REPLY
            # -r: plain `print` (like zsh's `echo`) interprets backslash
            # escapes, which would undo __gpy_escape_json's doubling and
            # corrupt the request (#676).
            print -r -u $fd -- "$json"
            # `IFS= read -r`, not a bare `read`: the default IFS makes `read`
            # strip leading and trailing whitespace, which silently ate the
            # trailing space the character segment's template emits after `❯`
            # (so zsh rendered `❯cd foo` where fish rendered `❯ cd foo`), and
            # without -r it would mangle the backslashes in the rendered ANSI.
            IFS= read -r -u $fd -t 0.1 response
            read_status=$?
            exec {fd}>&-
        fi
    fi

    # Once zsocket has connected, $json has already gone out over that
    # connection -- a slow-but-alive agent (read_status nonzero from the
    # 0.1s timeout above) is not the same as "never sent". Falling through
    # to socat/nc here would resend the same payload on a second connection
    # while the agent may still be working the first one (#575). So this
    # branch always returns once connected, whether or not the read landed
    # in time, and only an unconnected zsocket (module missing, or the
    # connect itself failed) reaches the socat/nc fallbacks below.
    if (( zsocket_connected )); then
        if [[ $read_status -eq 0 && -n "$response" ]]; then
            print -r -- "$response"
            return 0
        fi
        return 1
    fi

    # 2. Try socat
    read_status=1
    if (( $+commands[socat] )); then
        IFS= read -r response < <(print -r -- "$json" | socat -t 0.1 - UNIX-CONNECT:"$socket_path" 2>/dev/null)
        read_status=$?

    # 3. Try nc (BSD/macOS style with -U). nc's -w only accepts whole
    # seconds, so a hung-but-connected agent stalls the prompt for a full
    # second rather than the ~150ms IPC target ($GPY_IPC_TIMEOUT_MS); wrap
    # with `timeout` for a tighter bound when it's available (#324). Falls
    # back to the coarser -w 1 bound otherwise.
    elif (( $+commands[nc] )); then
        if (( $+commands[timeout] )); then
            local timeout_secs
            __gpy_ms_to_secs "${GPY_IPC_TIMEOUT_MS:-150}" timeout_secs
            IFS= read -r response < <(print -r -- "$json" | timeout "$timeout_secs" nc -U "$socket_path" -w 1 2>/dev/null)
        else
            IFS= read -r response < <(print -r -- "$json" | nc -U "$socket_path" -w 1 2>/dev/null)
        fi
        read_status=$?
    else
        return 1
    fi

    if [[ $read_status -eq 0 && -n "$response" ]]; then
        print -r -- "$response"
        return 0
    fi

    return 1
}

# Expected protocol version (must match gpy-agent/src/ipc/protocol.rs::PROTOCOL_VERSION)
typeset -g GPY_EXPECTED_PROTOCOL_VERSION=2

# Check protocol version compatibility with the running agent.
#
# Returns 0 if versions match, or if the agent's status can't be queried
# (nothing to compare against yet -- not our problem to report here).
# Returns 1 on a confirmed mismatch, so callers (the supervisor) can treat
# a responsive-but-mismatched agent as not running and restart it (#307).
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

# Find git repository root
function __gpy_find_git_root() {
    local start_dir=${1:-$PWD}

    # Convert to absolute path. Kept as a real `realpath` fork (not replaced
    # with a builtin, #614): the result feeds __gpy_path_to_cache_key below,
    # which MUST stay byte-for-byte identical to the Rust writer's
    # `path_to_cache_key` (gpy-agent/src/cache/instant_prompt.rs). realpath's
    # symlink resolution is part of that contract -- dropping it would
    # diverge the cache key (and the git-root result) for symlinked repos.
    # Mirrors bash/core/ipc.bash's __gpy_find_git_root exactly.
    local dir
    dir=$(realpath "$start_dir" 2>/dev/null)
    [[ -z "$dir" ]] && return 1

    while [[ "$dir" != "/" ]]; do
        if [[ -e "$dir/.git" ]]; then
            print -r -- "$dir"
            return 0
        fi
        # zsh history-modifier parameter expansion instead of a `dirname`
        # fork per directory level (#614) -- the biggest fork win here, since
        # this loop runs once per ancestor directory. `${dir:h}` is zsh's
        # built-in "head" (dirname-equivalent) modifier: `${dir:h}` on `/a`
        # yields `/` (not `/a`, and not empty), matching dirname's behavior
        # exactly, so the loop condition above terminates the same way.
        dir=${dir:h}
    done

    return 1
}

# Convert path to cache key (matches Rust/fish injective encoding)
# Order matters: escape _ first so subsequent replacements don't double-encode.
function __gpy_path_to_cache_key() {
    # `local path` would shadow zsh's PATH-tied array for the function's
    # duration; harmless here (no command runs) but avoided on principle.
    local raw_path=$1
    local key="${raw_path//_/__}"
    key="${key//\//_s}"
    key="${key//\\/_b}"
    key="${key//:/_c}"
    key="${key// /_w}"
    print -r -- "$key"
}

# Turn the cache key in the parameter named by $1 into its on-disk stem, in
# place (#771): unchanged up to 200 characters (existing names stay
# byte-for-byte), else 50-character chunks joined by `/` so no filename
# component exceeds NAME_MAX. MUST match `cache_key_path_for` in
# gpy-agent/src/cache/instant_prompt.rs (pinned by
# tests/fixtures/cache_key_vectors.tsv). Builtins only, no subshell.
# `${key:i:50}` is 0-based whatever KSH_ARRAYS says. Lengths count characters
# under a UTF-8 locale but bytes under LC_ALL=C, where a long non-ASCII key
# resolves elsewhere and simply misses the cache.
function __gpy_chunk_cache_key() {
    local __gpy_ck_key=${(P)1} __gpy_ck_stem= __gpy_ck_i
    (( ${#__gpy_ck_key} > 200 )) || return 0
    for (( __gpy_ck_i = 0; __gpy_ck_i < ${#__gpy_ck_key}; __gpy_ck_i += 50 )); do
        __gpy_ck_stem+="${__gpy_ck_stem:+/}${__gpy_ck_key:$__gpy_ck_i:50}"
    done
    : ${(P)1::=$__gpy_ck_stem}
}

# Filesystem-safe token identifying the previous-segment background a cache entry
# was rendered with. Empty -> "none" (context-free). Every non-alphanumeric byte
# becomes "_". MUST match `prev_bg_token` in
# gpy-agent/src/cache/instant_prompt.rs so both sides resolve the same cache file.
function __gpy_prev_bg_token() {
    local prev_bg=$1
    if [[ -z "$prev_bg" ]]; then
        print -r -- "none"
        return
    fi
    print -r -- "${prev_bg//[^a-zA-Z0-9]/_}"
}

# Cache-filename suffix for a given (is_last, is_first) pair, e.g.
# `__gpy_cache_variant_suffix git true false` -> "git_last". MUST match
# `variant_suffix` in gpy-agent/src/cache/instant_prompt.rs so both sides
# resolve the same cache file (#401).
function __gpy_cache_variant_suffix() {
    local base=$1 is_last=$2 is_first=$3
    local suffix="$base"
    [[ "$is_first" == "true" ]] && suffix="${suffix}_first"
    [[ "$is_last" == "true" ]] && suffix="${suffix}_last"
    print -r -- "$suffix"
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
function __gpy_read_instant_cache() {
    local suffix=${1:-git}
    local cwd=${2:-$PWD}
    local prev_bg=${3:-}

    # Resolve the cache-key path. Git caches are keyed by the repository root.
    # Language caches additionally cover non-Git project directories (detected by
    # package.json, Cargo.toml, etc.), which the agent keys by the canonical
    # request path. Mirror that fallback so warm language caches are consumed
    # outside Git repositories instead of cold-missing forever (#173).
    local key_path
    key_path=$(__gpy_find_git_root "$cwd")
    if [[ -z "$key_path" && "$suffix" == lang* ]]; then
        # No Git root: language cache is keyed by the canonical request path.
        key_path=$(realpath "$cwd" 2>/dev/null)
    fi
    if [[ -z "$key_path" ]]; then
        return 1
    fi

    # Compute cache directory
    local cache_dir
    cache_dir=$(__gpy_instant_cache_dir) || return 1

    # Convert resolved path to cache key and build cache file path. The rendered
    # ANSI bakes in the fg:prev_bg opening chevron, so the cache is keyed by the
    # previous-segment background too. Fall back to the context-free ("none")
    # render when no context-specific entry exists yet — the caller then triggers
    # a refresh that populates the correct token file and repaints via the doorbell.
    local cache_key token
    cache_key=$(__gpy_path_to_cache_key "$key_path")
    __gpy_chunk_cache_key cache_key
    token=$(__gpy_prev_bg_token "$prev_bg")
    # `.zsh`: the agent's zsh-prompt dialect, escaped for PROMPT (#677). The
    # `.ansi` files are fish's and must never reach PROMPT.
    local cache_file="$cache_dir/$cache_key.$suffix.$token.zsh"
    # Track whether this read is about to fall back to the `.none` variant
    # instead of serving the requested token's own file (bit 4 below): the
    # content is still correct, but its opening chevron was rendered without
    # this render's real prev_bg context.
    local used_variant_fallback=0
    if [[ ! -f "$cache_file" && "$token" != "none" ]]; then
        cache_file="$cache_dir/$cache_key.$suffix.none.zsh"
        used_variant_fallback=1
    fi

    # Serve stale-or-fresh cache; staleness/variant-fallback are folded into
    # the exit code (see contract above) instead of a side-effect temp file.
    if [[ -f "$cache_file" ]]; then
        local now mtime age
        # zsh/datetime (loaded in zsh/core/init.zsh) exposes $EPOCHSECONDS --
        # no `date` fork needed (#614).
        now=$EPOCHSECONDS
        # GNU coreutils (stat -c) first, then BSD/macOS (stat -f). The reverse
        # order breaks on Linux: there `stat -f` means --file-system and returns
        # non-numeric text *successfully*, so the fallback never runs and the
        # arithmetic below crashes with "syntax error in expression". No zsh
        # builtin computes mtime, so `stat` stays a real fork.
        mtime=$(stat -c %Y "$cache_file" 2>/dev/null || stat -f %m "$cache_file" 2>/dev/null)

        local exit_code=0
        # Require numeric operands: a non-numeric mtime (e.g. a stat quirk) must
        # not crash the prompt via a bad $((...)).
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

        # `$(<file)` is zsh's built-in fast-path for reading a whole file --
        # reads directly in-process instead of forking `cat` (#614). Every
        # instant-cache file is written by the Rust prompt formatter
        # (gpy-agent/src/formatter/fish_ansi.rs), which never appends a trailing
        # newline, so there is nothing for the newline-stripping command
        # substitution below to change (mirrors bash/core/ipc.bash).
        printf '%s' "$(<"$cache_file")"
        return "$exit_code"
    fi

    return 1
}

# Shared JSON tail for is_last/is_first/prev_bg (#613). Every IPC request
# builder below appends this SAME sequence -- one implementation instead of
# several hand-built copies, so a bug fixed once is fixed everywhere. Mirrors
# __gpy_json_flags_tail in fish/core/ipc.fish and bash/core/ipc.bash exactly.
# Convention (first-party, all three shells): a caller passes is_last/is_first
# as "true" or "" (never "false") -- see init.zsh's render-loop dispatch.
# Emits nothing for a flag that isn't literally "true" (an absent flag is
# OMITTED from the JSON, not sent as `false`; the agent defaults it to false
# via `#[serde(default)]`, gpy-agent/src/ipc/protocol.rs) and nothing for an
# empty prev_bg. No trailing newline -- callers concatenate this directly
# onto their request string.
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

# Send a data op directly to the agent via IPC, bypassing the instant cache.
# Used for background refreshes after serving a stale cache entry. No oneshot
# fallback: without a running daemon there is no agent repaint, so the
# already-served stale prompt simply remains until the agent returns.
function __gpy_trigger_data_refresh() {
    local op=$1
    local cwd=${2:-$PWD}
    local is_last=${3:-false}
    local prev_bg=${4:-}

    local json_cwd flags_tail venv_json=""
    json_cwd=$(__gpy_escape_json "$cwd")
    flags_tail=$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")
    # Forward the activated virtualenv for language detection (see fish/core/ipc.fish).
    [[ "$op" == "lang" && -n "${VIRTUAL_ENV:-}" ]] \
        && venv_json=",\"virtual_env\":\"$(__gpy_escape_json "$VIRTUAL_ENV")\""
    local request="{\"op\":\"$op\",\"cwd\":\"$json_cwd\",\"format\":\"zsh-prompt\"${flags_tail}${venv_json}}"
    __gpy_send_json "$request" >/dev/null 2>&1
}

# High-level request wrapper
# For git operations, tries instant-prompt cache first for 0ms latency
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

    # For git/lang operations, try instant cache first (0ms latency)
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

    local json_cwd flags_tail venv_json=""
    json_cwd=$(__gpy_escape_json "$context_path")
    flags_tail=$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")
    # Forward the activated virtualenv for language detection (see fish/core/ipc.fish).
    [[ "$op" == "lang" && -n "${VIRTUAL_ENV:-}" ]] \
        && venv_json=",\"virtual_env\":\"$(__gpy_escape_json "$VIRTUAL_ENV")\""

    # Use "cwd" as per protocol, not "path"
    local request="{\"op\":\"$op\",\"cwd\":\"$json_cwd\",\"format\":\"$format\"${flags_tail}${venv_json}}"

    if ! __gpy_send_json "$request"; then
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
# __gpy_render_prompt runs entirely inside the `$(...)` command substitution
# __gpy_precmd uses to capture $PROMPT, and each segment is itself invoked
# via a further `$(...)`, so no segment can write back to a parent scope --
# this is no longer a predictable `${TMPDIR:-/tmp}/.gpy_oneshot_used_$$`
# marker file (#342's rationale for a file, #614's reason to remove it):
# __gpy_render_prompt resets the plain variable __gpy_oneshot_used=0 at the
# start of each render; a request wrapper that actually claims the budget and
# forks oneshot returns GPY_SEG_STATUS_ONESHOT, which the render loop reads
# off `$?` after every `seg_out=$(__gpy_segment_$seg ..)` call and folds into
# __gpy_oneshot_used=1 in ITS OWN scope -- later segments' subshells inherit
# that 1 by fork, so __gpy_oneshot_claim (below, running inside one of those
# subshells) denies a second fork for the rest of this render.
function __gpy_oneshot_claim() {
    [[ "${__gpy_oneshot_used:-0}" == "1" ]] && return 1
    return 0
}

# Fallback to oneshot mode. Returns GPY_SEG_STATUS_ONESHOT whenever it
# actually claimed the budget and forked `gpy-agent oneshot` -- regardless of
# whether that oneshot call itself produced output -- so the render loop can
# track the spent budget; 1 otherwise (binary missing, or budget already
# spent this render).
function __gpy_fallback_oneshot() {
    local op=$1
    # Not `path`: in zsh that name is the array tied to $PATH, and a
    # `local path=<dir>` here replaced the shell's PATH with the repository
    # path for the rest of the function, so `gpy-agent` could never be found
    # and the oneshot fallback silently never ran (#639).
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

# Send a duration render request to the agent via IPC, with oneshot fallback.
# Unlike git/lang/directory there is no cwd — only duration_ms is required.
# Returns GPY_SEG_STATUS_ONESHOT when the oneshot fallback was actually
# claimed+forked this call (see __gpy_fallback_oneshot above), 0 on a normal
# IPC hit with output, 1 otherwise.
function __gpy_request_duration() {
    local duration_ms=$1
    local is_last=${2:-false}
    local prev_bg=${3:-}
    local is_first=${4:-false}

    local flags_tail
    flags_tail=$(__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg")
    local request="{\"op\":\"duration\",\"duration_ms\":${duration_ms},\"format\":\"zsh-prompt\"${flags_tail}}"

    local result
    result=$(__gpy_send_json "$request")

    # NOTE: oneshot fallback does not pass prev_bg — the CLI subcommand has no
    # --prev-bg flag (mirrors fish). is_first IS passed (--first, #401).
    local used_oneshot=0
    if [[ -z "$result" ]] && (( $+commands[gpy-agent] )) && __gpy_oneshot_claim; then
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

# Send a clock render request to the agent via IPC. Used when a theme sets
# `[segments.clock].format`; the response embeds zsh's own `%D{…}` prompt token
# (see gpy-agent/src/formatter/clock_resolver.rs), so the clock still ticks
# between draws off a single render.
#
# There is intentionally NO oneshot fallback, and unlike `__gpy_request_hostname`
# an unreachable agent is not the end of it: clock.zsh falls back to its own
# pure-zsh renderer, so the segment degrades to an uncapped clock rather than
# disappearing.
#
# is_last/is_first: "true" or "" (#613).
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

# Send a hostname render request to the agent via IPC. Used only when a theme
# sets a Starship-compatible `__hostname_format`; the pure-zsh render path in
# hostname.zsh handles the common case without any IPC call. There is
# intentionally NO oneshot fallback here. A `oneshot hostname` subcommand does
# exist (added for the starship-parity test harness), but this segment
# deliberately skips it: under the starship theme with a dead daemon, the
# hostname segment should render nothing until the daemon comes back up
# rather than fall back to a per-prompt subprocess — an unreachable agent
# simply yields no hostname segment.
#
# is_last: "true" or "" (#613 -- the same convention every segment now uses).
# The caller (hostname.zsh) always passes it explicitly, so no default here.
function __gpy_request_hostname() {
    local hostname=$1
    local is_last=$2
    local prev_bg=${3:-}

    local json_hostname flags_tail
    json_hostname=$(__gpy_escape_json "$hostname")
    flags_tail=$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")
    local request="{\"op\":\"hostname\",\"hostname\":\"${json_hostname}\",\"format\":\"zsh-prompt\"${flags_tail}}"

    local result
    result=$(__gpy_send_json "$request")

    if [[ -n "$result" ]]; then
        print -r -- "$result"
        return 0
    fi
    return 1
}

# Send a username render request to the agent via IPC. No oneshot fallback, for
# the same reason as __gpy_request_hostname above: under the starship theme with
# a dead daemon the username segment renders nothing until the daemon returns.
#
# is_last: "true" or "" (#613 -- the same convention every segment now uses).
# The caller (username.zsh) always passes it explicitly, so no default here.
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

# Send a character (prompt symbol) render request to the agent via IPC, with
# oneshot fallback. Input is a 0/1 success flag (1 == exit status 0); there is
# no cwd. The character is always the last element, and every caller (init.zsh)
# passes is_last="true" explicitly, so no default is needed here.
function __gpy_request_character() {
    local success=$1
    local is_last=$2
    local prev_bg=${3:-}

    local success_val=false
    [[ "$success" == "1" ]] && success_val=true

    local flags_tail
    flags_tail=$(__gpy_json_flags_tail "$is_last" "" "$prev_bg")
    local request="{\"op\":\"character\",\"success\":${success_val},\"format\":\"zsh-prompt\"${flags_tail}}"

    local result used_oneshot=0
    result=$(__gpy_send_json "$request")

    if [[ -z "$result" ]] && (( $+commands[gpy-agent] )) && __gpy_oneshot_claim; then
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
# ran and produced nothing. Zsh reads the instant-prompt cache and talks to
# the socket; it never resolves a runtime root of its own, registers a shell
# PID for the restart nudge (#638), but never reads a theme export by a path
# of its own or locates config.toml — those keys are the agent's and Fish's,
# and the harness holds this shell to exactly that split.
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
