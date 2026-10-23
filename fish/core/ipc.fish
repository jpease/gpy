# Home directory with the agent's passwd fallback: an empty $HOME counts as
# unset, else a blanked HOME resolves /.cache/gpy while the agent uses passwd.
function __gpy_home --description 'Home directory with the passwd fallback'
    set -l home "$HOME"
    if test -z "$home"
        set -l user (id -un 2>/dev/null)
        string match -qr '^[A-Za-z0-9._-]+$' -- "$user"; or return 1
        set home ~$user
    end
    string match -q '/*' -- "$home"; or return 1
    printf '%s' "$home"
end

# Runtime root, agent precedence. Per the XDG spec, empty or relative values
# count as unset.
function __gpy_runtime_root --description 'XDG runtime root for GPY'
    if set -q XDG_RUNTIME_DIR; and test -n "$XDG_RUNTIME_DIR"; and string match -q '/*' -- "$XDG_RUNTIME_DIR"
        echo "$XDG_RUNTIME_DIR/gpy"
        return
    else if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"; and string match -q '/*' -- "$XDG_CACHE_HOME"
        echo "$XDG_CACHE_HOME/gpy"
        return
    end
    set -l home (__gpy_home)
    if test -n "$home"
        echo "$home/.cache/gpy"
    else
        echo /tmp/gpy
    end
end

function __gpy_instant_cache_dir --description 'Instant-prompt cache directory'
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"; and string match -q '/*' -- "$XDG_CACHE_HOME"
        echo "$XDG_CACHE_HOME/gpy/instant-prompts"
    else
        set -l home (__gpy_home)
        test -n "$home"; and echo "$home/.cache/gpy/instant-prompts"
    end
end

function __gpy_shell_registry_dir --description 'Directory of Fish shell PIDs for agent recovery nudges'
    echo (__gpy_runtime_root)/shells
end

function __gpy_shell_registry_file --description 'Fish shell PID file for agent recovery nudges'
    # $fish_pid, not %self: %-expansion only happens at argument start, and the
    # agent parses this filename as the PID to signal.
    echo (__gpy_shell_registry_dir)/$fish_pid
end

function __gpy_ipc_endpoint --description 'Returns socket path or pipe hint'
    # Empty counts as unset, matching Bash and Zsh.
    if set -q GPY_AGENT_SOCKET_PATH; and test -n "$GPY_AGENT_SOCKET_PATH"
        echo "$GPY_AGENT_SOCKET_PATH"
        return
    end

    set -l root (__gpy_runtime_root)
    echo "$root/gpy.sock"
end

# Cache whether nc supports -U; netcat-traditional must read as "no client".
function __gpy_nc_probe_capabilities
    set -q __gpy_nc_probed; and return
    set -g __gpy_nc_probed 1
    set -g __gpy_nc_supports_unix 0
    if command -q nc
        set -l help (nc -h 2>&1)
        if string match -q '*-U*' -- $help
            set -g __gpy_nc_supports_unix 1
        end
    end
end

function __gpy_ipc_client_available
    command -q socat; and return 0
    command -q nc; or return 1
    __gpy_nc_probe_capabilities
    test "$__gpy_nc_supports_unix" -eq 1
end

# Ping within one second over the real client: stock macOS lacks `timeout` and
# its `nc -z` rejects -U. With no client, socket existence is the best answer.
function __gpy_socket_ready --argument-names socket_path
    test -S "$socket_path"; or return 1
    __gpy_ipc_client_available; or return 0
    __gpy_ipc_send '{"op":"ping"}' 1000 "$socket_path" >/dev/null
end

function __gpy_wait_for_socket --argument-names socket_path
    set -l max_attempts $GPY_SOCKET_WAIT_MAX_ATTEMPTS
    set -l attempt 1

    while test $attempt -le $max_attempts
        if __gpy_socket_ready "$socket_path"
            return 0
        end

        set -l delay_ms (math "min(50 * (2 ^ min($attempt - 1, 3)), 200)")
        sleep (math "$delay_ms / 1000")

        set attempt (math "$attempt + 1")
    end

    return 1
end

# Print the line only if its trailing newline arrived; `head -n1` passed
# truncated writes straight into the prompt. Must be the last pipe stage.
function __gpy_ipc_read_complete_line --description 'Validate newline framing on an IPC response'
    set -l raw
    read -lz raw
    if test -n "$raw"; and string match -qr '\n$' -- "$raw"
        # The agent sends embedded newlines as RS (\x1e) to keep one reply line.
        printf '%s\n' (string trim --right --chars \n -- "$raw" | string replace -a \x1e \n | string collect)
        return 0
    end
    return 1
end

# The budget for a segment the prompt has nothing to show for:
# GPY_IPC_COLD_TIMEOUT_MS, never below GPY_IPC_TIMEOUT_MS.
function __gpy_ipc_cold_timeout_ms --description 'Reply budget (ms) when a segment has nothing to show'
    set -l cold (__gpy_uint_or_default "$GPY_IPC_COLD_TIMEOUT_MS" 500)
    set -l warm (__gpy_uint_or_default "$GPY_IPC_TIMEOUT_MS" 150)
    math --scale=0 "max($cold, $warm)"
end

# Returns 0 on a complete reply, 1 if unreachable, 2 if the request was sent
# but no complete reply arrived in time; on 2 callers must not resend or fork
# oneshot for the same segment.
function __gpy_ipc_send --argument-names payload timeout_ms sock
    set -q timeout_ms[1]; or set timeout_ms $GPY_IPC_TIMEOUT_MS
    test -n "$sock"; or set sock (__gpy_ipc_endpoint)

    # The real connect fast-fails if nothing listens.
    if not test -S "$sock"
        return 1
    end

    set -l reply
    set -l ps
    # Client exit codes meaning "connected, but no reply" vs "never connected".
    set -l client_status
    set -l connected_codes 0
    if command -q socat
        # -t bounds the post-EOF reply wait (-T does not; default 0.5s).
        set -l secs (math "$timeout_ms / 1000")
        set reply (echo $payload | socat -t $secs - UNIX-CONNECT:$sock 2>/dev/null | __gpy_ipc_read_complete_line)
        set ps $pipestatus
    else if __gpy_ipc_client_available
        if command -q timeout
            # Floor at 0.05s so short budgets don't round to zero and hang.
            set -l timeout_secs (math "max(0.05, $timeout_ms / 1000)")
            set reply (echo $payload | timeout --foreground --kill-after=0.5s --signal=TERM $timeout_secs"s" nc -U $sock 2>/dev/null | __gpy_ipc_read_complete_line)
            set ps $pipestatus
            # 124: `timeout` killed a nc that had connected and was waiting.
            set connected_codes 0 124
        else
            # nc -w takes whole seconds only.
            set -l nc_w_secs (math "max(1, ceil($timeout_ms / 1000))")
            set reply (echo $payload | nc -U $sock -w $nc_w_secs 2>/dev/null | __gpy_ipc_read_complete_line)
            set ps $pipestatus
        end
    else
        __gpy_log_warn ipc "no suitable IPC client found (socat or nc -U)"
        return 1
    end

    # $ps is [echo, client, reader]; a truncated reply fails the reader even
    # when the client exits 0.
    set client_status $ps[2]
    if test "$ps[3]" = 0; and test -n "$reply"
        printf '%s\n' $reply
        return 0
    end
    if contains -- "$client_status" $connected_codes
        return 2
    end
    return 1
end

# Fire-and-forget send. Fish runs functions in the foreground, so only a pipeline
# ending in an external `command` becomes a real background job.
function __gpy_ipc_send_detached --argument-names payload timeout_ms --description 'Send an IPC payload in a detached background job, discarding the reply'
    set -q timeout_ms[1]; or set timeout_ms $GPY_IPC_TIMEOUT_MS
    set -l sock (__gpy_ipc_endpoint)
    test -S "$sock"; or return 1

    if command -q socat
        set -l secs (math "$timeout_ms / 1000")
        printf '%s\n' $payload | command socat -t $secs - UNIX-CONNECT:$sock >/dev/null 2>&1 &
    else if command -q nc
        __gpy_nc_probe_capabilities
        test "$__gpy_nc_supports_unix" -eq 1; or return 1
        set -l nc_w_secs (math "max(1, ceil($timeout_ms / 1000))")
        printf '%s\n' $payload | command nc -U $sock -w $nc_w_secs >/dev/null 2>&1 &
    else
        return 1
    end
    # Without disown an interactive fish prints "Job ... has ended" later.
    disown $last_pid 2>/dev/null
    return 0
end

function __gpy_json_escape --argument-names str --description 'Escape a string for safe JSON inclusion'
    set -l escaped (string replace -a -- '\\' '\\\\' "$str" | string collect)
    set escaped (string replace -a -- '"' '\\"' "$escaped" | string collect)
    set escaped (string replace -a -- \n '\\n' "$escaped" | string collect)
    set escaped (string replace -a -- \r '\\r' "$escaped" | string collect)
    set escaped (string replace -a -- \t '\\t' "$escaped" | string collect)
    printf '%s' "$escaped"
end

# Fork-free JSON field extraction from a flat blob.
function __gpy_json_extract_string --argument-names field json --description 'Fork-free "field":"value" extraction from a flat JSON blob'
    set -l matches (string match -r '"'"$field"'":"([^"]*)"' -- $json)
    set -q matches[2]; or return 1
    printf '%s' $matches[2]
end

function __gpy_json_extract_int --argument-names field json --description 'Fork-free "field":123 extraction from a flat JSON blob'
    set -l matches (string match -r '"'"$field"'":([0-9]+)' -- $json)
    set -q matches[2]; or return 1
    printf '%s' $matches[2]
end

# Shared is_last/is_first/prev_bg JSON tail. Absent flags are omitted; the
# agent defaults them to false.
function __gpy_json_flags_tail --argument-names is_last is_first prev_bg --description 'Build the shared ,"is_last":true,"is_first":true,"prev_bg":"..." JSON tail'
    set -l tail ''
    if test "$is_last" = true
        set tail (string join '' $tail ',"is_last":true')
    end
    if test "$is_first" = true
        set tail (string join '' $tail ',"is_first":true')
    end
    if test -n "$prev_bg"
        set -l escaped_prev_bg (__gpy_json_escape "$prev_bg")
        set tail (string join '' $tail ',"prev_bg":"' $escaped_prev_bg '"')
    end
    printf '%s' $tail
end

# Virtualenv for `lang` requests: $VIRTUAL_ENV, else a non-base conda prefix.
function __gpy_forwarded_venv --description 'Virtualenv path to forward on lang requests'
    if test -n "$VIRTUAL_ENV"
        printf '%s' "$VIRTUAL_ENV"
    else if test -n "$CONDA_PREFIX"; and test -n "$CONDA_DEFAULT_ENV"; and test "$CONDA_DEFAULT_ENV" != base
        printf '%s' "$CONDA_PREFIX"
    end
end

function __gpy_build_data_payload --argument-names op cwd format is_last prev_bg is_first --description 'Build JSON payload for data requests (git/lang)'
    set -q cwd[1]; or set cwd $PWD
    set -l escaped_cwd (__gpy_json_escape "$cwd")

    set -l payload (string join '' '{"op":"' $op '","cwd":"' $escaped_cwd '","format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg"))
    # The daemon never inherits the shell's VIRTUAL_ENV.
    if test "$op" = lang
        set -l venv (__gpy_forwarded_venv)
        if test -n "$venv"
            set -l escaped_venv (__gpy_json_escape "$venv")
            set payload (string join '' $payload ',"virtual_env":"' $escaped_venv '"')
        end
    end
    echo (string join '' $payload '}')
end

# Allow one oneshot fork per render so a dead daemon doesn't cost a fork per
# segment. A plain global works because fish runs `(...)` in-process; a TMPDIR
# marker could be pre-created by another user.
function __gpy_oneshot_claim --description 'Claim the one oneshot-fallback fork allowed for this prompt render'
    set -q __gpy_oneshot_used; and return 1
    set -g __gpy_oneshot_used 1
    return 0
end

function __gpy_oneshot_fallback --argument-names op cwd format is_last is_first --description 'Execute oneshot command with proper flags'
    set -q cwd[1]; or set cwd $PWD
    set -l agent_binary (__gpy_resolve_agent_binary)
    if test -z "$agent_binary"
        return 1
    end
    __gpy_oneshot_claim; or return 1

    set -l args $op --cwd "$cwd" --format $format
    if not test "$is_last" = true
        set args $args --not-last
    end
    if test "$is_first" = true
        set args $args --first
    end
    "$agent_binary" oneshot $args 2>/dev/null
end

# Duration request; no cwd, so the payload is built inline.
function __gpy_request_duration --argument-names duration_ms is_last prev_bg is_first --description 'Send duration render request to agent'
    set -l format ansi

    set -l payload (string join '' '{"op":"duration","duration_ms":' $duration_ms ',"format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg"))
    set payload (string join '' $payload '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)
    set -l send_status $status

    # Oneshot fallback when no daemon is running. Not when the daemon is
    # running but late (send status 2): it is computing this very segment
    #, so the segment is omitted this render.
    # NOTE: oneshot fallback does not pass prev_bg — the CLI subcommand has no
    # --prev-bg flag, so the powerline opening chevron color is skipped on
    # first-shell startup until the daemon connects. is_first IS passed
    # (--first), so the opening cap's presence/absence is still correct.
    if test -z "$result"; and test $send_status -ne 2; and __gpy_resolve_agent_binary >/dev/null; and __gpy_oneshot_claim
        set -l agent_binary (__gpy_resolve_agent_binary)
        set -l args duration --duration-ms "$duration_ms" --format $format
        if not test "$is_last" = true
            set args $args --not-last
        end
        if test "$is_first" = true
            set args $args --first
        end
        set result ("$agent_binary" oneshot $args 2>/dev/null)
    end

    if test -z "$result"
        return 1
    end
    printf '%s\n' $result
end

# Send a character render request to the agent via IPC, with oneshot fallback.
#
# Arguments:
#   success  — 1 if the last command exited 0, 0 otherwise
#   is_last  — pass "true" if this is the last prompt element
#
# Outputs the ANSI-rendered character symbol, or returns 1 on failure so the
# caller can fall back to the legacy set_color/printf path.
function __gpy_request_character --argument-names success is_last prev_bg --description 'Send character render request to agent'
    set -l format ansi

    # Convert the 0/1 integer to the JSON boolean the agent expects.
    set -l success_val false
    if test "$success" = 1
        set success_val true
    end

    set -l payload (string join '' '{"op":"character","success":' $success_val ',"format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" '' "$prev_bg"))
    set payload (string join '' $payload '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)
    set -l send_status $status

    # Oneshot fallback when no daemon is running; a late daemon (status 2) is
    # computing this very segment, so it is omitted rather than recomputed.
    # prev_bg is not passed (see __gpy_request_duration).
    if test -z "$result"; and test $send_status -ne 2; and __gpy_resolve_agent_binary >/dev/null; and __gpy_oneshot_claim
        set -l agent_binary (__gpy_resolve_agent_binary)
        # Convert boolean back to exit-code integer for the CLI flag.
        set -l exit_code 1
        if test "$success" = 1
            set exit_code 0
        end
        if test -n "$is_last"; and test "$is_last" = true
            set result ("$agent_binary" oneshot character --exit-code "$exit_code" --format $format 2>/dev/null)
        else
            set result ("$agent_binary" oneshot character --exit-code "$exit_code" --format $format --not-last 2>/dev/null)
        end
    end

    if test -z "$result"
        return 1
    end
    printf '%s\n' $result
end

# Send a hostname render request to the agent via IPC.
#
# Unlike character/duration, there is NO oneshot fallback here. A `oneshot
# hostname` CLI subcommand does exist (added for the starship-parity test
# harness), but this segment intentionally does not call it: under the
# starship theme with a dead daemon, the hostname segment draws its local pill
# until the daemon comes back up, rather than paying a per-prompt subprocess
# cost as a fallback. If IPC is unavailable (or the agent replies with an
# error) this returns 1 and the caller draws the local pill.
#
# Arguments:
#   host     — the hostname string ($hostname, fish's builtin variable). Named
#              `host` rather than `hostname` because `$hostname` is a
#              read-only electric variable in fish — using it as an
#              --argument-names binding is a hard error.
#   is_last  — "true" if this is the last prompt segment, "" otherwise.
#   prev_bg  — the previous segment's background, for the powerline chevron.
#   is_ssh   — "1" in an SSH session ($__gpy_is_ssh), anything else otherwise.
#              Sent as "is_ssh":true so the agent draws the theme icon only
#              over SSH; omitted (agent default false) otherwise.
#   is_first — "true" if this is the first prompt segment, "" otherwise (no opening cap).
function __gpy_request_hostname --argument-names host is_last prev_bg is_ssh is_first --description 'Send hostname render request to agent'
    set -l format ansi
    set -l escaped_host (__gpy_json_escape "$host")

    set -l payload (string join '' '{"op":"hostname","hostname":"' $escaped_host '","format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg"))
    test "$is_ssh" = 1; and set payload (string join '' $payload ',"is_ssh":true')
    set payload (string join '' $payload '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    if test -z "$result"; or string match -q '{"error"*' -- "$result[1]"
        return 1
    end
    printf '%s\n' $result
end

# is_root "1" ($__gpy_is_root) is sent as "is_root":true so the agent picks the
# root style; omitted (agent default false, the user style) otherwise.
# No oneshot fallback: no reply (or an error) returns 1 and the caller draws the local pill.
function __gpy_request_username --argument-names user is_last prev_bg is_root is_first --description 'Send username render request to agent'
    set -l format ansi
    set -l escaped_user (__gpy_json_escape "$user")

    set -l payload (string join '' '{"op":"username","username":"' $escaped_user '","format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg"))
    test "$is_root" = 1; and set payload (string join '' $payload ',"is_root":true')
    set payload (string join '' $payload '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    if test -z "$result"; or string match -q '{"error"*' -- "$result[1]"
        return 1
    end
    printf '%s\n' $result
end

# Clock request; the reply carries the bare strftime spec, which
# segment_clock_render fills. No oneshot fallback: it renders locally instead.
function __gpy_request_clock --argument-names is_last is_first prev_bg --description 'Send clock render request to agent'
    set -l payload (string join '' '{"op":"clock","shell":"fish","format":"ansi"' (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg") '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    if test -z "$result"
        return 1
    end
    printf '%s\n' $result
end

# Exit-status pill request ("exit_status": the bare "status" op is the agent health
# check). success is 1 when the last command exited 0. No oneshot fallback:
# segment_status_render draws the pill locally instead. An error reply (an older
# agent without the op) counts as no reply.
function __gpy_request_exit_status --argument-names success is_last is_first prev_bg --description 'Send status render request to agent'
    set -l success_val false
    test "$success" = 1; and set success_val true

    set -l payload (string join '' '{"op":"exit_status","success":' $success_val ',"format":"ansi"' (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg") '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    if test -z "$result"; or string match -q '{"error"*' -- "$result[1]"
        return 1
    end
    printf '%s\n' $result
end

function __gpy_find_git_root --argument-names start_dir --description 'Find git repository root from directory'
    set -q start_dir[1]; or set start_dir $PWD

    # Require an existing directory so a malformed path can't match relative to CWD.
    set -l dir (path resolve -- "$start_dir" 2>/dev/null)
    if test -z "$dir"
        set dir $start_dir
    end
    if not test -d "$dir"
        return 1
    end

    while test -n "$dir"
        if test -e "$dir/.git"
            echo $dir
            return 0
        end
        set -l parent (path dirname -- "$dir")
        if test "$parent" = "$dir"
            break
        end
        set dir $parent
    end

    return 1
end

# Per-render git root memo, keyed by directory. fish_prompt clears it each
# render; it MUST NOT persist across renders. An empty result is cached too.
function __gpy_memoized_git_root --argument-names start_dir --description 'Per-render memoized git root lookup'
    set -q start_dir[1]; or set start_dir $PWD
    if not set -q __gpy_git_root_this_render_dir; or test "$__gpy_git_root_this_render_dir" != "$start_dir"
        set -g __gpy_git_root_this_render_dir $start_dir
        set -g __gpy_git_root_this_render (__gpy_find_git_root $start_dir)
    end
    printf '%s' "$__gpy_git_root_this_render"
end

# Reversible escape (`_` doubled first). Order MUST match path_to_cache_key in
# gpy-agent/src/cache/instant_prompt/mod.rs.
function __gpy_path_to_cache_key --argument-names path --description 'Convert path to cache key'
    string replace -a -- _ __ "$path" | string replace -a -- / _s | string replace -a -- '\\' _b | string replace -a -- ':' _c | string replace -a -- ' ' _w
end

# Keys over 200 chars split into 50-char `/` chunks to stay under NAME_MAX.
# MUST match `cache_key_path_for` in the agent.
function __gpy_chunk_cache_key --argument-names key --description 'On-disk stem of a cache key'
    if test (string length -- "$key") -le 200
        printf '%s\n' "$key"
        return
    end
    string match -ra -- '(?s).{1,50}' "$key" | string join /
end

# MUST match `prev_bg_token` in the agent.
function __gpy_prev_bg_token --argument-names prev_bg --description 'Cache-key token for prev_bg'
    if test -z "$prev_bg"
        echo none
        return
    end
    string replace -ra -- '[^a-zA-Z0-9]' _ "$prev_bg"
end

# MUST match `variant_suffix` in the agent.
function __gpy_cache_variant_suffix --argument-names base is_last is_first --description 'Cache-filename suffix for an (is_last, is_first) pair'
    set -l suffix $base
    if test "$is_first" = true
        set suffix "$suffix"_first
    end
    if test "$is_last" = true
        set suffix "$suffix"_last
    end
    printf '%s' $suffix
end

# Path an instant-cache entry is keyed by: the git root, or for lang outside
# git, the canonical cwd as the agent keys it.
function __gpy_instant_cache_key_path --argument-names suffix cwd --description 'Path an instant-cache entry is keyed by'
    set -l key_path (__gpy_memoized_git_root $cwd)
    if test -z "$key_path"; and string match -q 'lang*' -- $suffix
        # `path resolve` resolves symlinks like the agent's `canonicalize`.
        set key_path (path resolve -- "$cwd" 2>/dev/null)
    end
    test -n "$key_path"; or return 1
    printf '%s' "$key_path"
end

# Whether any variant/prev_bg entry exists; the agent writes `.none` only for
# context-free requests, so probing `.none` alone would miss.
function __gpy_instant_cache_present --argument-names base cwd --description 'Whether any instant-cache entry exists for git|lang under cwd'
    set -l key_path (__gpy_instant_cache_key_path $base $cwd); or return 1
    set -l cache_dir (__gpy_instant_cache_dir)
    test -n "$cache_dir"; or return 1
    set -l cache_key (__gpy_chunk_cache_key (__gpy_path_to_cache_key $key_path))
    set -l hits $cache_dir/$cache_key.$base.*.ansi $cache_dir/$cache_key.$base"_"*.ansi
    set -q hits[1]
end

# Exit status: 1 = miss; else bit 2 = stale, bit 4 = served via `.none` fallback.
function __gpy_read_instant_cache --argument-names suffix cwd prev_bg --description 'Read instant-prompt cache; status via exit code (see comment above)'
    set -q cwd[1]; or set cwd $PWD
    if test -z "$suffix"
        set suffix git
    end

    set -l key_path (__gpy_instant_cache_key_path $suffix $cwd); or return 1

    set -l cache_dir (__gpy_instant_cache_dir)
    if test -z "$cache_dir"
        return 1
    end

    # Rendered ANSI bakes in the prev_bg chevron, so the key includes it; fall
    # back to `.none` until a refresh writes the exact token.
    set -l cache_key (__gpy_chunk_cache_key (__gpy_path_to_cache_key $key_path))
    set -l token (__gpy_prev_bg_token "$prev_bg")
    set -l cache_file "$cache_dir/$cache_key.$suffix.$token.ansi"
    set -l used_variant_fallback 0
    if not test -f "$cache_file"; and test "$token" != none
        set cache_file "$cache_dir/$cache_key.$suffix.none.ansi"
        set used_variant_fallback 1
    end

    if not test -f "$cache_file"
        return 1
    end

    # Both caches are served stale; only the TTL differs.
    set -l ttl $GPY_GIT_INSTANT_CACHE_TTL_SECONDS
    if string match -q 'lang*' -- $suffix
        set ttl $GPY_LANGUAGE_CACHE_TTL_SECONDS
        test -z "$ttl"; and set ttl 30
    else
        test -z "$ttl"; and set ttl 5
    end

    set -l now $__gpy_prompt_now
    if test -z "$now"
        set now (date +%s 2>/dev/null)
    end
    set -l mtime (__gpy_file_mtime "$cache_file")

    set -l exit_code 0
    if test (__gpy_cache_freshness "$now" "$mtime" "$ttl") = stale
        set exit_code (math "$exit_code + 2")
    end
    if test "$used_variant_fallback" = 1
        set exit_code (math "$exit_code + 4")
    end

    __gpy_read_file "$cache_file"
    return $exit_code
end

# Send an op, trying the instant cache first for git/lang and oneshot last.
function __gpy_request --argument-names op cwd is_last prev_bg is_first --description 'Send a JSON op to the agent with oneshot fallback'
    if test "$op" = git
        set -l git_suffix (__gpy_cache_variant_suffix git $is_last $is_first)
        set -l cached_result (__gpy_read_instant_cache $git_suffix $cwd $prev_bg)
        if test $status -ne 1
            printf '%s\n' $cached_result
            return 0
        end
    else if test "$op" = lang
        set -l lang_suffix (__gpy_cache_variant_suffix lang $is_last $is_first)
        set -l cached_result (__gpy_read_instant_cache $lang_suffix $cwd $prev_bg)
        if test $status -ne 1
            printf '%s\n' $cached_result
            return 0
        end
    end

    set -l payload
    switch $op
        case ping
            set payload '{"op":"ping"}'
        case register
            set -l pid %self
            set -l register_cwd (pwd)
            set -l escaped_cwd (__gpy_json_escape "$register_cwd")
            set payload (string join '' '{"op":"register","pid":' $pid ',"cwd":"' $escaped_cwd '"}' )
        case unregister
            set -l pid %self
            set payload (string join '' '{"op":"unregister","pid":' $pid '}' )
        case git lang directory
            set payload (__gpy_build_data_payload $op $cwd ansi $is_last $prev_bg $is_first)
        case workspace
            set -q cwd[1]; or set cwd $PWD
            set -l escaped_cwd (__gpy_json_escape "$cwd")
            set -l pid %self
            set payload (string join '' '{"op":"workspace","pid":' $pid ',"cwd":"' $escaped_cwd '"}' )
        case '*'
            return 1
    end

    # A directory request means the memo had nothing to show.
    set -l timeout_ms $GPY_IPC_TIMEOUT_MS
    test "$op" = directory; and set timeout_ms (__gpy_ipc_cold_timeout_ms)
    set -l result (__gpy_ipc_send $payload $timeout_ms)
    set -l send_status $status

    # Status 2: the daemon is computing this segment; omit it rather than fork.
    # Oneshot has no --prev-bg, so the opening chevron color is skipped.
    if test -z "$result"; and test $send_status -ne 2; and __gpy_resolve_agent_binary >/dev/null
        switch $op
            case git lang directory
                set result (__gpy_oneshot_fallback $op $cwd ansi $is_last $is_first)
        end
    end

    if test -z "$result"
        return 1
    end
    printf '%s\n' $result
end

# Throttled refresh straight to the agent (bypassing a stale cache entry); the
# agent repaints via SIGURG. No oneshot: it would block the prompt and spend the
# render's oneshot budget. throttle_key_extra separates per-prev_bg buckets.
function __gpy_maybe_refresh --argument-names op root cache_suffix is_last prev_bg is_first throttle_key_extra --description 'Shared throttled background refresh dispatcher for git/lang segments'
    __gpy_resolve_agent_binary >/dev/null; or return

    set -q is_last[1]; or set is_last ""
    set -q prev_bg[1]; or set prev_bg ""
    set -q is_first[1]; or set is_first ""
    set -q throttle_key_extra[1]; or set throttle_key_extra ""

    # One bucket per (op, suffix, token, root). The suffix and token never hold
    # `/`, so with root last the joined key is unambiguous, and var-style
    # escaping is injective.
    set -l throttle_var "__gpy_"$op"_refresh_ms_"(string escape --style=var -- "$cache_suffix/$throttle_key_extra/$root")

    set -l now_ms (__gpy_get_time_ms)
    set -l last_ms 0
    set -q $throttle_var; and set last_ms $$throttle_var

    __gpy_throttle_elapsed "$last_ms" "$now_ms" 500; or return
    set -g $throttle_var $now_ms

    __gpy_register_with_agent >/dev/null 2>&1

    set -l payload (__gpy_build_data_payload $op "$root" ansi "$is_last" "$prev_bg" "$is_first")
    __gpy_ipc_send_detached $payload $GPY_IPC_TIMEOUT_MS
end

# Prints the agent version; sets __gpy_agent_version/_protocol_version.
function __gpy_get_agent_version --description 'Get the version and protocol version from the agent'
    set -q GPY_AGENT_ENABLED; or set -g GPY_AGENT_ENABLED 1
    if test "$GPY_AGENT_ENABLED" -ne 1
        return 1
    end

    set -l payload '{"op":"status"}'
    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    if test -z "$result"
        return 1
    end

    # Not `version`: that is fish's read-only special variable.
    set -l agent_version_str (__gpy_json_extract_string version $result)
    if test $status -ne 0
        return 1
    end
    set -g __gpy_agent_version $agent_version_str

    set -l protocol_version (__gpy_json_extract_int protocol_version $result)
    if test $status -eq 0
        set -g __gpy_agent_protocol_version $protocol_version
    else
        set -g __gpy_agent_protocol_version 0
    end

    echo $__gpy_agent_version
    return 0
end

function __gpy_check_protocol_version --description 'Check and warn on protocol version mismatch'
    # MUST match PROTOCOL_VERSION in gpy-agent/src/ipc/protocol/mod.rs.
    set -l EXPECTED_PROTOCOL_VERSION 2

    if not __gpy_get_agent_version >/dev/null
        return 0
    end

    if test "$__gpy_agent_protocol_version" -ne "$EXPECTED_PROTOCOL_VERSION"
        echo "" >&2
        echo "warning: protocol version mismatch" >&2
        echo "================================================" >&2
        echo "Running agent protocol version: $__gpy_agent_protocol_version" >&2
        echo "Expected protocol version: $EXPECTED_PROTOCOL_VERSION" >&2
        echo "" >&2
        echo "This may cause compatibility issues. Recommended actions:" >&2
        echo "  1. Restart the agent: gpy-agent stop && gpy-agent start" >&2
        echo "  2. If issues persist, reinstall: ./install-dev.fish" >&2
        echo "" >&2
        echo "Fallback: Use oneshot mode if IPC fails:" >&2
        echo "  gpy-agent oneshot git --cwd . --format ansi" >&2
        echo "================================================" >&2
        echo "" >&2
        return 1
    end

    return 0
end

# Live supervisor, not merely a live (possibly recycled) PID.
function __gpy_is_supervisor_pid --argument-names pid
    string match -qr '^\d+$' -- "$pid"; or return 1
    ps -o command= -p $pid 2>/dev/null | string match -q '*__gpy_agent_supervisor_loop*'
end

function __gpy_agent_supervisor_start --description 'Start agent supervisor for automatic restart'
    if test "$GPY_AGENT_SUPERVISOR_ENABLED" = 0; or test "$GPY_AGENT_ENABLED" = 0
        return 0
    end

    if not __gpy_resolve_agent_binary >/dev/null
        __gpy_log_warn ipc "Agent binary not found in PATH; skipping supervisor"
        return 1
    end

    set -l runtime_root (__gpy_runtime_root)
    mkdir -p "$runtime_root"
    set -l supervisor_pidfile "$runtime_root/supervisor.pid"

    # Never remove the file here: only the child recovers it, under the claim.
    if __gpy_is_supervisor_pid (cat "$supervisor_pidfile" 2>/dev/null)
        return 0
    end

    set -l config_root (__gpy_config_root)
    # fds are redirected at spawn: a redirect-only `exec` in fish does not
    # reopen the shell's fds. The child claims the pidfile with no-clobber `>?`
    # (O_EXCL); an empty read means the winner hasn't written its PID yet.
    set -l supervisor_script '
        cd /
        set -gx GPY_SUPERVISOR_CHILD 1
        set -l pidfile $argv[1]
        set -l cfgroot $argv[2]
        source "$cfgroot/gpy/core/constants.fish"; or exit 1
        source "$cfgroot/gpy/core/util.fish"; or exit 1
        source "$cfgroot/gpy/core/ipc.fish"; or exit 1
        if not echo $fish_pid >?"$pidfile"
            set -l owner (cat "$pidfile" 2>/dev/null)
            if test -z "$owner"
                sleep 0.2
                set owner (cat "$pidfile" 2>/dev/null)
            end
            __gpy_is_supervisor_pid $owner; and exit 0
            rm -f "$pidfile"
            echo $fish_pid >?"$pidfile"; or exit 0
        end
        __gpy_agent_supervisor_loop "$pidfile"
        test "$(cat "$pidfile" 2>/dev/null)" = "$fish_pid"; and rm -f "$pidfile"
    '

    fish -c "$supervisor_script" -- "$supervisor_pidfile" "$config_root" </dev/null >/dev/null 2>&1 &
    set -l spawned_pid $last_pid
    disown $spawned_pid 2>/dev/null

    if test -n "$spawned_pid"
        __gpy_log_debug ipc "Agent supervisor started with PID $spawned_pid"
        return 0
    end

    __gpy_log_error ipc "Failed to start agent supervisor loop"
    return 1
end

function __gpy_track_shell_for_agent_recovery --description 'Track this Fish PID so agent restarts can wake existing shells'
    if set -q GPY_SUPERVISOR_CHILD
        return 0
    end

    set -l registry_dir (__gpy_shell_registry_dir)
    mkdir -p "$registry_dir" 2>/dev/null
    echo $fish_pid >(__gpy_shell_registry_file) 2>/dev/null
end

function __gpy_untrack_shell_for_agent_recovery --description 'Remove this Fish PID from agent recovery tracking'
    set -l registry_file (__gpy_shell_registry_file)
    rm -f $registry_file $registry_file.reload $registry_file.reregister 2>/dev/null
end

function __gpy_refresh_registration_after_restart --description 'Forget this shell registration and register again (agent .reregister doorbell)'
    if set -q GPY_SUPERVISOR_CHILD
        return 1
    end

    if test "$GPY_AGENT_ENABLED" = 0
        return 1
    end

    # Forget the stale registration; on failure the agent's re-nudge retries.
    # The agent may have restarted with another config: drop its renders too.
    set -e __gpy_registered
    set -e __gpy_registered_pid
    set -e __gpy_last_workspace
    __gpy_clear_render_memos
    __gpy_log_debug ipc "Re-registration requested by agent for PID $fish_pid"
    __gpy_register_with_agent >/dev/null 2>&1
    set -l register_status $status
    # Covers a refused registration; success already checked.
    __gpy_reload_if_theme_export_changed
    return $register_status
end

# Re-apply the theme export if the agent rewrote it before this shell was
# tracked for the .reload doorbell.
function __gpy_reload_if_theme_export_changed --description 'Re-apply the theme export if the agent rewrote it since this shell sourced it'
    set -l cache_path (__gpy_theme_export_cache_path)
    test -n "$cache_path" -a -f "$cache_path"; or return 0
    set -l mtime (__gpy_file_mtime "$cache_path"); or return 0
    if set -q __gpy_theme_export_mtime[1]; and test "$mtime" -le "$__gpy_theme_export_mtime"
        return 0
    end
    __gpy_log_debug ipc "Theme export rewritten since it was sourced; reloading"
    __gpy_apply_agent_reload
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end

function __gpy_register_with_agent --description 'Register this Fish process with the GPY agent'
    if set -q GPY_SUPERVISOR_CHILD
        return 1
    end

    if set -q __gpy_registered
        return 0
    end

    if test "$GPY_AGENT_ENABLED" = 0
        if set -q GPY_TEST_MODE
            __gpy_log_debug ipc "Registration skipped: GPY_AGENT_ENABLED=0"
        end
        return 1
    end

    if not __gpy_agent_available
        if set -q GPY_TEST_MODE
            __gpy_log_debug ipc "Registration skipped: agent not available"
        end
        return 1
    end

    set -l response (__gpy_request register)
    if test $status -ne 0
        if set -q GPY_TEST_MODE
            __gpy_log_debug ipc "Registration request failed with status $status"
        end
        return 1
    end

    if test -n "$response"; and string match -q '*"status":"ok"*' -- $response
        set -g __gpy_registered 1
        set -g __gpy_registered_pid %self
        set -g __gpy_last_workspace (pwd)
        __gpy_track_shell_for_agent_recovery
        __gpy_log_debug ipc "Registered Fish PID %self with agent"
        # Exports rewritten before tracking are caught here; later ones ring .reload.
        __gpy_reload_if_theme_export_changed
        return 0
    else if set -q GPY_TEST_MODE
        __gpy_log_debug ipc "Registration failed: invalid response '$response'"
    end

    return 1
end

function __gpy_unregister_from_agent --description 'Remove this Fish process from the GPY agent registry'
    if set -q GPY_SUPERVISOR_CHILD
        return 0
    end

    if not set -q __gpy_registered
        return 0
    end

    __gpy_request unregister >/dev/null 2>&1
    set -e __gpy_registered
    set -e __gpy_registered_pid
    set -e __gpy_last_workspace
    __gpy_untrack_shell_for_agent_recovery
    __gpy_log_debug ipc "Unregistered Fish PID %self from agent"
end

function __gpy_sync_workspace --description 'Report current workspace to the agent'
    if set -q GPY_SUPERVISOR_CHILD
        return 0
    end

    if not set -q __gpy_registered
        return 0
    end

    set -l cwd (pwd)

    # Always sync: detects an agent restart that lost our registration.
    set -l response (__gpy_request workspace $cwd)

    # Only a lost registration warrants re-registering.
    if string match -q '*not registered*' -- $response
        __gpy_log_debug ipc "Workspace sync failed (not registered), re-registering PID %self"
        set -e __gpy_registered
        set -e __gpy_registered_pid
        set -e __gpy_last_workspace
        __gpy_register_with_agent
    else if test -n "$response"; and not string match -q '*"error"*' -- $response
        set -g __gpy_last_workspace $cwd
    end
end

# The detached supervisor sees config changes only via the export cache;
# read just the enable flags and its own timing.
function __gpy_supervisor_refresh_settings --description 'Re-read the supervisor settings from the theme-export cache'
    set -l cache_path (__gpy_theme_export_cache_path)
    test -n "$cache_path" -a -r "$cache_path"; or return 0
    set -l pairs (string match -rg '^set -gx (GPY_AGENT_ENABLED|GPY_AGENT_SUPERVISOR_ENABLED|GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS|GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS) "(\d+)"$' <"$cache_path")
    while set -q pairs[2]
        set -gx $pairs[1] $pairs[2]
        set -e pairs[1..2]
    end
end

# Exits once $argv[1] (the claimed pidfile) no longer names this process.
function __gpy_agent_supervisor_loop --description 'Background loop for agent supervision'
    set -l pidfile $argv[1]

    set -l restart_count 0
    set -l last_restart_time 0
    # A stall shorter than one check interval clears by itself, so an agent
    # that is connected but silent is restarted only after this many checks in
    # a row; an unreachable one is restarted at once.
    set -l unresponsive_limit 2
    set -l unresponsive_checks 0

    while true
        # Re-read each pass so a config reload applies within one interval:
        # disabling stops the loop, new timing governs the next sleep.
        __gpy_supervisor_refresh_settings
        if test "$GPY_AGENT_SUPERVISOR_ENABLED" = 0; or test "$GPY_AGENT_ENABLED" = 0
            break
        end
        # Re-validate: an empty value would make sleep busy-loop.
        set -l check_interval (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS" 30)
        set -l max_restart_attempts (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS" 5)

        if test -n "$pidfile"; and test "$(cat "$pidfile" 2>/dev/null)" != "$fish_pid"
            break
        end

        if not __gpy_resolve_agent_binary >/dev/null
            sleep $check_interval; or break
            continue
        end

        __gpy_agent_is_healthy
        set -l health $status
        if test $health -eq 2
            set unresponsive_checks (math $unresponsive_checks + 1)
        else
            set unresponsive_checks 0
        end

        if test $health -eq 0
            set restart_count 0
        else if test $health -eq 1; or test $unresponsive_checks -ge $unresponsive_limit
            set -l current_time (date +%s)

            set -l rate_limit (__gpy_uint_or_default "$GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS" 60)

            if test (math "$current_time - $last_restart_time") -gt $rate_limit
                if test $restart_count -lt $max_restart_attempts
                    __gpy_log_warn ipc "Agent not responding, attempting restart ($restart_count/$max_restart_attempts)"
                    set unresponsive_checks 0
                    if __gpy_agent_restart
                        __gpy_log_info ipc "Agent restarted successfully"
                        set restart_count 0
                        set last_restart_time $current_time
                    else
                        __gpy_log_error ipc "Agent restart failed"
                        set restart_count (math "$restart_count + 1")
                        set last_restart_time $current_time
                    end
                else
                    set -l long_backoff (__gpy_uint_or_default "$GPY_SUPERVISOR_LONG_BACKOFF_SECONDS" 300)
                    __gpy_log_error ipc "Agent restart limit exceeded, backing off"
                    sleep $long_backoff; or break
                    set restart_count 0
                    continue
                end
            end
        end

        sleep $check_interval; or break
    end
end

# 0: healthy. 1: unreachable (no socket, nothing listening). 2: connected but
# no valid reply, which a briefly stalled agent also produces. Nothing waits on
# this background check, so it allows more than the prompt's reply budget.
function __gpy_agent_is_healthy --description 'Check the agent: 0 healthy, 1 unreachable, 2 unresponsive'
    set -l prompt_budget (__gpy_uint_or_default "$GPY_IPC_TIMEOUT_MS" 150)
    set -l response (__gpy_ipc_send '{"op":"ping"}' (math "max($prompt_budget, 1000)"))
    test $status -eq 1; and return 1

    if string match -q '*"status":"ok"*' -- "$response"
        return 0
    end
    return 2
end

# Socket-holder PIDs via lsof or fuser; returns 1 when neither exists, which the
# caller must not read as a stale socket.
function __gpy_socket_owner_pids --argument-names socket_path
    if command -q lsof
        lsof -t -- "$socket_path" 2>/dev/null
        return 0
    else if command -q fuser
        # psmisc prints PIDs on stdout (the path goes to stderr); BSD fuser
        # prints "path: pid ..." there, so keep only pure-digit words.
        string match -r '^[0-9]+$' -- (string split ' ' -- (string trim -- (fuser -- "$socket_path" 2>/dev/null)))
        return 0
    end
    return 1
end

function __gpy_agent_restart --description 'Restart the agent process'
    set -l socket_path (__gpy_ipc_endpoint)
    set -l agent_binary (__gpy_resolve_agent_binary)

    # Only with a live socket: on cold start the CLI prints a notice to stdout.
    if test -n "$agent_binary"; and test -S "$socket_path"
        "$agent_binary" stop >/dev/null 2>&1
        sleep 0.5
    end

    # Without lsof/fuser, leave the socket: deleting a live agent's socket
    # would orphan it, and `gpy-agent start` reclaims stale ones.
    if test -S "$socket_path"
        set -l socket_pid (__gpy_socket_owner_pids "$socket_path")
        if test $status -ne 0
            __gpy_log_warn ipc "neither lsof nor fuser found; cannot stop a lingering agent holding $socket_path (install lsof)"
        else
            if test -n "$socket_pid"
                kill -TERM $socket_pid 2>/dev/null
                sleep 0.3
                if kill -0 $socket_pid 2>/dev/null
                    kill -KILL $socket_pid 2>/dev/null
                    sleep 0.1
                end
            end
            rm -f "$socket_path" 2>/dev/null
        end
    end

    if test -n "$agent_binary"
        "$agent_binary" start 2>/dev/null

        if __gpy_wait_for_socket "$socket_path"
            return 0
        end
    end

    return 1
end

# Defer supervisor start to the first prompt to avoid blocking source.
if not set -q GPY_SUPERVISOR_CHILD; and not test "$GPY_AGENT_ENABLED" = 0 -a "$GPY_AGENT_SUPERVISOR_ENABLED" = 0
    function __gpy_start_supervisor_on_prompt --on-event fish_prompt
        if test "$GPY_AGENT_ENABLED" = 1
            # Only the restart loop depends on the supervisor flag.
            if not set -q __gpy_supervisor_initialized
                set -g __gpy_supervisor_initialized 1
                test "$GPY_AGENT_SUPERVISOR_ENABLED" = 0; or __gpy_agent_supervisor_start
            end

            # An agent speaking another protocol must be treated as not running.
            if not set -q __gpy_protocol_checked
                set -g __gpy_protocol_checked 1
                if __gpy_agent_available; and not __gpy_check_protocol_version
                    __gpy_log_warn ipc "Protocol version mismatch detected; restarting agent"
                    __gpy_agent_restart
                end
            end

            if not __gpy_agent_available
                set -l max_attempts (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_MAX_ATTEMPTS" 3)

                set -l attempts 0
                if set -q __gpy_autostart_attempts
                    set attempts $__gpy_autostart_attempts
                end

                if test $attempts -ge $max_attempts
                    # Crash loop: leave retries to the supervisor.
                    __gpy_log_error ipc "Agent autostart limit exceeded ($max_attempts attempts); disabling per-prompt autostart for this shell"
                    functions -e __gpy_start_supervisor_on_prompt
                    return
                end

                set -l rate_limit (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS" 10)

                set -l last_attempt_time 0
                if set -q __gpy_autostart_last_attempt_time
                    set last_attempt_time $__gpy_autostart_last_attempt_time
                end

                set -l current_time (date +%s)
                if test (math "$current_time - $last_attempt_time") -ge $rate_limit
                    set -g __gpy_autostart_last_attempt_time $current_time
                    set -g __gpy_autostart_attempts (math "$attempts + 1")

                    set -l agent_binary (__gpy_resolve_agent_binary)
                    if test -n "$agent_binary"
                        "$agent_binary" start >/dev/null 2>&1 &
                        disown $last_pid 2>/dev/null
                        sleep (math "$GPY_AGENT_START_DELAY_MS / 1000")
                    end
                end
            end

            if __gpy_register_with_agent
                functions -e __gpy_start_supervisor_on_prompt
                set -e __gpy_autostart_attempts
                set -e __gpy_autostart_last_attempt_time
            end
        else
            functions -e __gpy_start_supervisor_on_prompt
        end
    end

    function __gpy_unregister_on_exit --on-event fish_exit
        __gpy_unregister_from_agent
        __gpy_untrack_shell_for_agent_recovery
    end

    function __gpy_workspace_on_pwd --on-variable PWD
        __gpy_sync_workspace
    end
end

# Defined unconditionally. SIGURG's default disposition is ignore, so a shell
# that `exec`s itself survives an early ring; meaning travels in flag files.
function __gpy_doorbell_handler --on-signal SIGURG
    set -l registry_file (__gpy_shell_registry_file)
    # Each flag is removed BEFORE acting so a ring mid-action re-queues.
    if test -e $registry_file.reregister
        rm -f $registry_file.reregister
        __gpy_refresh_registration_after_restart
    end
    if test -e $registry_file.reload
        rm -f $registry_file.reload
        __gpy_apply_agent_reload
    end
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end

# Drop the agent-rendered memos. Their keys hold the theme name but no config,
# so a theme or config change (a reload, an edited re-export, or an agent
# restarted with another config) must not leave them serving the old render.
function __gpy_clear_render_memos --description 'Forget memoized agent renders'
    set -e __gpy_char_cache_key
    set -e __gpy_char_cache_val
    set -e __gpy_dir_cache_key
    set -e __gpy_dir_cache_val
end

function __gpy_apply_agent_reload --description 'Reload theme/config after the agent .reload doorbell'
    # The agent writes the export cache before ringing.
    __gpy_apply_theme_export

    # The export erases status icons the theme leaves unset; refill the
    # nerd/ascii fallbacks from the current __prompt_icons.
    __gpy_initialize_icons

    __gpy_ensure_segments_loaded
    __gpy_clear_render_memos
end

# Fish allows commandline -f repaint from variable handlers, not signal handlers.
function __gpy_repaint_on_variable --on-variable __gpy_repaint_trigger
    commandline -f force-repaint 2>/dev/null
end
