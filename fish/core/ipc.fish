# Resolve the runtime root with the same precedence as the agent.
#
# An $XDG_* variable counts as set only when it is non-empty AND absolute: the
# XDG Base Directory spec says an empty value "should be treated as not set"
# and a relative one "is invalid and should be ignored". A bare `set -q` used
# to accept `XDG_RUNTIME_DIR=` and join onto "/", giving "/gpy" while the agent
# built a relative "gpy" -- so the prompt watched a socket the agent never
# bound (#626). Mirrors crate::paths::xdg_value.
function __gpy_runtime_root --description 'XDG runtime root for GPY'
    if set -q XDG_RUNTIME_DIR; and test -n "$XDG_RUNTIME_DIR"; and string match -q '/*' -- "$XDG_RUNTIME_DIR"
        echo "$XDG_RUNTIME_DIR/gpy"
        return
    else if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"; and string match -q '/*' -- "$XDG_CACHE_HOME"
        echo "$XDG_CACHE_HOME/gpy"
        return
    end
    # fallback
    if set -q HOME
        echo "$HOME/.cache/gpy"
    else
        echo /tmp/gpy
    end
end

# Directory holding the agent's pre-rendered instant-prompt cache files.
#
# Extracted from __gpy_read_instant_cache verbatim (same `set -q` guards, same
# precedence, same "no directory" signal: empty output) so the instant-cache
# read path and __gpy_debug_paths resolve it through one implementation rather
# than two. The parity harness diffs this against the agent's own resolver.
function __gpy_instant_cache_dir --description 'Instant-prompt cache directory'
    if set -q XDG_CACHE_HOME; and test -n "$XDG_CACHE_HOME"; and string match -q '/*' -- "$XDG_CACHE_HOME"
        echo "$XDG_CACHE_HOME/gpy/instant-prompts"
    else if set -q HOME
        echo "$HOME/.cache/gpy/instant-prompts"
    end
end

function __gpy_shell_registry_dir --description 'Directory of Fish shell PIDs for agent recovery nudges'
    echo (__gpy_runtime_root)/shells
end

function __gpy_shell_registry_file --description 'Fish shell PID file for agent recovery nudges'
    # Use $fish_pid, not %self: fish only performs %-job expansion at the start of
    # an argument, so "(dir)/%self" would yield a literal "%self" filename. The
    # agent parses this filename as the PID to ring (SIGURG) on restart, and the
    # .reload/.reregister doorbell flags derive from it, so it MUST be the
    # numeric PID or recovery silently breaks.
    echo (__gpy_shell_registry_dir)/$fish_pid
end

# Path to the Unix domain socket (Unix) or pipe hint on Windows.
function __gpy_ipc_endpoint --description 'Returns socket path or pipe hint'
    # Allow override for testing. An empty value is "unset", not "the empty
    # path": honouring it used to make this shell resolve nothing while Bash
    # and Zsh fell through to the runtime root, so the two halves of the same
    # session disagreed about where the agent listens (#626).
    if set -q GPY_AGENT_SOCKET_PATH; and test -n "$GPY_AGENT_SOCKET_PATH"
        echo "$GPY_AGENT_SOCKET_PATH"
        return
    end

    set -l root (__gpy_runtime_root)
    echo "$root/gpy.sock"
end

# Wait for socket to be available with exponential backoff
# Returns 0 if socket becomes available, 1 if timeout
function __gpy_socket_ready --argument-names socket_path
    if not test -S "$socket_path"
        return 1
    end

    if command -q timeout
        if command -q socat
            command timeout --foreground 1s socat -T 1 -u OPEN:/dev/null UNIX-CONNECT:$socket_path >/dev/null 2>/dev/null
            return $status
        else if command -q nc
            __gpy_nc_probe_capabilities
            if test "$__gpy_nc_supports_unix" -eq 1
                if test "$__gpy_nc_supports_zero" -eq 1
                    command timeout --foreground 1s nc -z -U "$socket_path" >/dev/null 2>/dev/null
                else
                    command timeout --foreground 1s nc -U "$socket_path" </dev/null >/dev/null 2>/dev/null
                end
                return $status
            end
        end
    end

    return 0
end

# Probe nc(1) flag support once per shell session and cache results in globals.
# Avoids re-running `nc -h` on every IPC send.
function __gpy_nc_probe_capabilities
    set -q __gpy_nc_probed; and return
    set -g __gpy_nc_probed 1
    set -g __gpy_nc_supports_unix 0
    set -g __gpy_nc_supports_zero 0
    if command -q nc
        set -l help (nc -h 2>&1)
        if string match -q '*-U*' -- $help
            set -g __gpy_nc_supports_unix 1
        end
        if string match -q '*-z*' -- $help
            set -g __gpy_nc_supports_zero 1
        end
    end
end

function __gpy_wait_for_socket --argument-names socket_path
    set -l max_attempts $GPY_SOCKET_WAIT_MAX_ATTEMPTS # Up to 2 seconds with 100ms intervals
    set -l attempt 1

    while test $attempt -le $max_attempts
        if __gpy_socket_ready "$socket_path"
            return 0
        end

        # Exponential backoff with jitter: start at 50ms, max 200ms
        set -l delay_ms (math "min(50 * (2 ^ min($attempt - 1, 3)), 200)")
        sleep (math "$delay_ms / 1000")

        set attempt (math "$attempt + 1")
    end

    return 1 # Timeout
end

# Reads a newline-terminated line from stdin and validates that the trailing
# `\n` delimiter was actually present before EOF. Must be the last stage of
# the IPC pipe so it reads directly from the peer's fd.
#
# `head -n1` (the previous implementation) happily returns whatever bytes it
# received once the upstream stage hits EOF/timeout, even without a trailing
# newline — so a peer that writes a partial response and then stalls or dies
# mid-write (e.g. `{"status":"o` then silence) produced a truncated line that
# was indistinguishable from a real, complete response. For `format:ansi`
# ops that string is `printf`'d straight into the prompt, corrupting terminal
# rendering (#300). Reading via `read` and requiring the delimiter closes
# that gap: prints the de-terminated line and returns 0 only when the frame
# was complete; prints nothing and returns 1 otherwise.
function __gpy_ipc_read_complete_line --description 'Validate newline framing on an IPC response'
    set -l raw
    read -lz raw
    if test -n "$raw"; and string match -qr '\n$' -- "$raw"
        printf '%s\n' (string trim --right --chars \n -- "$raw")
        return 0
    end
    return 1
end

# Low-level send/recv with timeout. Implements fail-fast mechanism to prevent hanging.
# Returns a single JSON line on success; prints nothing on timeout/error or on a
# truncated (non-newline-terminated) partial write.
function __gpy_ipc_send --argument-names payload timeout_ms
    set -q timeout_ms[1]; or set timeout_ms $GPY_IPC_TIMEOUT_MS
    set -l sock (__gpy_ipc_endpoint)

    # Pre-flight check: socket file must exist before attempting IPC.
    # A plain existence test is enough here; the real connect will fast-fail
    # if nothing is listening. The full __gpy_socket_ready probe is reserved
    # for __gpy_wait_for_socket during startup.
    if not test -S "$sock"
        return 1
    end

    if command -q socat
        # -T accepts fractional seconds; divide directly to preserve sub-second precision
        set -l secs (math "$timeout_ms / 1000")
        # Send payload, read one validated line; suppress stderr to avoid polluting prompt
        echo $payload | socat -T $secs - UNIX-CONNECT:$sock 2>/dev/null | __gpy_ipc_read_complete_line
        return
    else if command -q nc
        __gpy_nc_probe_capabilities
        if test "$__gpy_nc_supports_unix" -eq 1
            # Try coreutils timeout if available to enforce ms; otherwise rely on nc's defaults
            set -l result
            set -l nc_status
            set -l read_status
            if command -q timeout
                # Use timeout with aggressive cleanup to prevent zombie processes;
                # min 0.05s so short timeouts don't round to zero and hang
                set -l timeout_secs (math "max(0.05, $timeout_ms / 1000)")
                set result (echo $payload | timeout --foreground --kill-after=0.5s --signal=TERM $timeout_secs"s" nc -U $sock 2>/dev/null | __gpy_ipc_read_complete_line)
                set read_status $status
                set nc_status $pipestatus[-2] # Get the exit code of nc (second to last in pipeline)
            else
                # No coreutils timeout available: rely on nc's own -w flag so a
                # connected-but-silent socket can't block the prompt forever.
                # -w takes whole seconds only, so round up and floor at 1s.
                set -l nc_w_secs (math "max(1, ceil($timeout_ms / 1000))")
                set result (echo $payload | nc -U $sock -w $nc_w_secs 2>/dev/null | __gpy_ipc_read_complete_line)
                set read_status $status
                set nc_status $pipestatus[-2] # Get the exit code of nc (second to last)
            end
            # Require both a clean nc exit AND a newline-terminated read: a
            # truncated/partial write from the agent (stall or crash mid-response)
            # leaves read_status nonzero even when nc itself exits 0 (#300).
            if test -n "$nc_status" -a "$nc_status" -eq 0 -a "$read_status" -eq 0
                echo $result
                return 0
            else
                return 1
            end
        end
    end

    # Last-resort: no supported client available
    __gpy_log_warn ipc "no suitable IPC client found (socat or nc -U)"
    return 1
end

# Fire-and-forget send for background refreshes (#685). Mirrors
# __gpy_ipc_send's client selection but discards the reply and never waits for
# it. Fish does not fork for functions, so `some_function &` runs in the
# foreground with `$last_pid` unset; only a pipeline whose last element is an
# external command becomes a real background job. The client is therefore
# invoked through `command` (a user function cannot shadow it and turn the job
# back into a foreground one), and the payload is built by the caller in the
# foreground. With no socket there is nothing to send, so nothing is forked.
function __gpy_ipc_send_detached --argument-names payload timeout_ms --description 'Send an IPC payload in a detached background job, discarding the reply'
    set -q timeout_ms[1]; or set timeout_ms $GPY_IPC_TIMEOUT_MS
    set -l sock (__gpy_ipc_endpoint)
    test -S "$sock"; or return 1

    if command -q socat
        set -l secs (math "$timeout_ms / 1000")
        printf '%s\n' $payload | command socat -T $secs - UNIX-CONNECT:$sock >/dev/null 2>&1 &
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

# Helper function to properly escape strings for JSON
function __gpy_json_escape --argument-names str --description 'Escape a string for safe JSON inclusion'
    # Replace backslash first to avoid double-escaping; quote arg so lists collapse safely
    set -l escaped (string replace -a -- '\\' '\\\\' "$str" | string collect)
    set escaped (string replace -a -- '"' '\\"' "$escaped" | string collect)
    # Use actual control characters (unquoted \n/\r/\t expand in fish)
    set escaped (string replace -a -- \n '\\n' "$escaped" | string collect)
    set escaped (string replace -a -- \r '\\r' "$escaped" | string collect)
    set escaped (string replace -a -- \t '\\t' "$escaped" | string collect)
    printf '%s' "$escaped"
end

# Fork-free extraction of a `"field":"value"` string field from a flat JSON
# blob (#612). `string match -r` already returns the capture group as the
# second line of its output; command substitution captures that into a list
# we can index directly, so no `tail` subprocess is needed to pick the last
# line. Prints nothing and returns 1 when the field is absent.
function __gpy_json_extract_string --argument-names field json --description 'Fork-free "field":"value" extraction from a flat JSON blob'
    set -l matches (string match -r '"'"$field"'":"([^"]*)"' -- $json)
    set -q matches[2]; or return 1
    printf '%s' $matches[2]
end

# Fork-free extraction of a `"field":123` integer field from a flat JSON
# blob (#612). Same technique as __gpy_json_extract_string.
function __gpy_json_extract_int --argument-names field json --description 'Fork-free "field":123 extraction from a flat JSON blob'
    set -l matches (string match -r '"'"$field"'":([0-9]+)' -- $json)
    set -q matches[2]; or return 1
    printf '%s' $matches[2]
end

# Shared JSON tail for is_last/is_first/prev_bg (#613). All five IPC request
# builders below append this SAME sequence -- one implementation instead of
# five hand-built copies, so a bug fixed once is fixed everywhere. Convention
# (first-party, all three shells): a caller passes is_last/is_first as
# "true" or "" (never "false") -- see fish_prompt.fish's dispatch loop. Emits
# nothing for a flag that isn't literally "true" (an absent flag is OMITTED
# from the JSON, not sent as `false`; the agent defaults it to false via
# `#[serde(default)]`, gpy-agent/src/ipc/protocol.rs) and nothing for an
# empty prev_bg. No trailing newline -- callers concatenate this directly
# onto their payload string.
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

# The virtualenv to forward on `lang` requests: $VIRTUAL_ENV, else
# $CONDA_PREFIX when $CONDA_DEFAULT_ENV is set and not `base` (#729; pinned by
# tests/fixtures/venv_forwarding_vectors.tsv). Prints nothing when none applies.
function __gpy_forwarded_venv --description 'Virtualenv path to forward on lang requests'
    if test -n "$VIRTUAL_ENV"
        printf '%s' "$VIRTUAL_ENV"
    else if test -n "$CONDA_PREFIX"; and test -n "$CONDA_DEFAULT_ENV"; and test "$CONDA_DEFAULT_ENV" != base
        printf '%s' "$CONDA_PREFIX"
    end
end

# Helper function to build JSON payloads for git/lang operations
function __gpy_build_data_payload --argument-names op cwd format is_last prev_bg is_first --description 'Build JSON payload for data requests (git/lang)'
    set -q cwd[1]; or set cwd $PWD
    set -l escaped_cwd (__gpy_json_escape "$cwd")

    set -l payload (string join '' '{"op":"' $op '","cwd":"' $escaped_cwd '","format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg"))
    # Forward the activated virtualenv for language detection so the agent
    # daemon (which never inherits the shell's VIRTUAL_ENV) reports the venv's
    # Python version rather than its own global interpreter.
    if test "$op" = lang
        set -l venv (__gpy_forwarded_venv)
        if test -n "$venv"
            set -l escaped_venv (__gpy_json_escape "$venv")
            set payload (string join '' $payload ',"virtual_env":"' $escaped_venv '"')
        end
    end
    echo (string join '' $payload '}')
end

# PID-scoped marker limiting oneshot-fallback forks to one per prompt render.
# Without this, a dead/hung daemon costs one `gpy-agent oneshot` fork per
# segment (git, lang, directory, duration, character -- up to 5 per prompt),
# each with its own process-start latency (#324). fish_prompt clears the
# marker at the start of each render; the first segment whose IPC call fails
# claims it and forks oneshot, later segments in the same render see it
# already claimed and render nothing rather than fork again.
# The marker path is a session constant (TMPDIR + $fish_pid never change over
# the shell's lifetime), so it's resolved once here at source time (ipc.fish is
# sourced once per shell) into a global instead of re-derived every render.
# __gpy_oneshot_marker() reads that same global, so the per-render reset
# (fish_prompt) and the claim path below can never diverge (#342).
set -l __gpy_oneshot_tmp_dir /tmp
set -q TMPDIR; and test -n "$TMPDIR"; and set __gpy_oneshot_tmp_dir "$TMPDIR"
set -g __gpy_oneshot_marker_path "$__gpy_oneshot_tmp_dir/.gpy_oneshot_used_$fish_pid"

# The marker is only ever cleared at the start of the next render, so a shell
# whose last prompt claimed it (agent down) left `.gpy_oneshot_used_<pid>`
# behind in TMPDIR for good, and a later process reusing that PID inherited a
# spent budget. Unconditional (unlike the supervisor's exit hook below): every
# shell that can claim the marker must also remove it.
function __gpy_oneshot_marker_cleanup_on_exit --on-event fish_exit
    rm -f "$__gpy_oneshot_marker_path" 2>/dev/null
end

function __gpy_oneshot_marker --description 'Path to the current render oneshot-fallback marker'
    printf '%s' "$__gpy_oneshot_marker_path"
end

function __gpy_oneshot_claim --description 'Claim the one oneshot-fallback fork allowed for this prompt render'
    set -l marker (__gpy_oneshot_marker)
    test -e "$marker"; and return 1
    touch "$marker" 2>/dev/null
    return 0
end

# Helper function to execute oneshot fallback for git/lang operations
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

# Send a duration render request to the agent via IPC, with oneshot fallback.
#
# Unlike git/lang/directory operations there is no cwd — only a scalar
# duration_ms value is required.  The payload is therefore built inline here
# rather than reusing __gpy_build_data_payload.
function __gpy_request_duration --argument-names duration_ms is_last prev_bg is_first --description 'Send duration render request to agent'
    set -l format ansi

    set -l payload (string join '' '{"op":"duration","duration_ms":' $duration_ms ',"format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" "$is_first" "$prev_bg"))
    set payload (string join '' $payload '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    # Oneshot fallback when no daemon is running.
    # NOTE: oneshot fallback does not pass prev_bg — the CLI subcommand has no
    # --prev-bg flag, so the powerline opening chevron color is skipped on
    # first-shell startup until the daemon connects. is_first IS passed
    # (--first, #401), so the opening cap's presence/absence is still correct.
    if test -z "$result"; and __gpy_resolve_agent_binary >/dev/null; and __gpy_oneshot_claim
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

    # Oneshot fallback when no daemon is running.
    # NOTE: oneshot fallback does not pass prev_bg — the CLI subcommand has no --prev-bg flag.
    # Powerline opening chevrons are skipped on first-shell startup until the daemon connects.
    if test -z "$result"; and __gpy_resolve_agent_binary >/dev/null; and __gpy_oneshot_claim
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
# starship theme with a dead daemon, the hostname segment should render
# nothing until the daemon comes back up, rather than silently paying a
# per-prompt subprocess cost as a fallback. If IPC is unavailable the caller
# renders nothing for this segment on this prompt.
#
# Arguments:
#   host     — the hostname string ($hostname, fish's builtin variable). Named
#              `host` rather than `hostname` because `$hostname` is a
#              read-only electric variable in fish — using it as an
#              --argument-names binding is a hard error.
#   is_last  — "true" if this is the last prompt segment, "" otherwise (#613:
#              same true/"" convention as every other segment now).
#   prev_bg  — the previous segment's background, for the powerline chevron.
function __gpy_request_hostname --argument-names host is_last prev_bg --description 'Send hostname render request to agent'
    set -l format ansi
    set -l escaped_host (__gpy_json_escape "$host")

    set -l payload (string join '' '{"op":"hostname","hostname":"' $escaped_host '","format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" '' "$prev_bg"))
    set payload (string join '' $payload '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    if test -z "$result"
        return 1
    end
    printf '%s\n' $result
end

# Send a username render request to the agent via IPC.
#
# Like __gpy_request_hostname (and unlike character/duration), there is NO
# oneshot fallback here: under the starship theme with a dead daemon, the
# username segment renders nothing until the daemon returns rather than paying a
# per-prompt subprocess cost. A `oneshot username` CLI subcommand exists for the
# starship-parity harness, but this segment intentionally does not call it.
#
# Arguments:
#   user     — the effective username string ($USER). Named `user` rather than
#              `username` for symmetry with __gpy_request_hostname's `host`.
#   is_last  — "true" if this is the last prompt segment, "" otherwise (#613:
#              same true/"" convention as every other segment now).
#   prev_bg  — the previous segment's background, for the powerline chevron.
function __gpy_request_username --argument-names user is_last prev_bg --description 'Send username render request to agent'
    set -l format ansi
    set -l escaped_user (__gpy_json_escape "$user")

    set -l payload (string join '' '{"op":"username","username":"' $escaped_user '","format":"' $format '"')
    set payload (string join '' $payload (__gpy_json_flags_tail "$is_last" '' "$prev_bg"))
    set payload (string join '' $payload '}')

    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    if test -z "$result"
        return 1
    end
    printf '%s\n' $result
end

# Helper function to find the git repository root for a given directory
function __gpy_find_git_root --argument-names start_dir --description 'Find git repository root from directory'
    set -q start_dir[1]; or set start_dir $PWD

    # Convert to absolute path using Fish builtin (no subprocess).
    # Fall back to the raw path if resolve yields nothing. Then require the
    # directory to exist — mirrors realpath's failure mode and prevents
    # relative-path collisions with the CWD when a malformed path is passed.
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

# Per-render memoized wrapper around __gpy_find_git_root (#342). Without this,
# a single render walks $PWD upward to the filesystem root up to four times:
# segment_git_detect, segment_language_detect, and once each from the git and
# language segments' instant-cache reads (both go through
# __gpy_read_instant_cache below). All are builtin (no forks), but the
# repeated traversals are still wasted work every prompt. fish_prompt clears
# both globals below at the very top of each render (before any segment
# runs), so a `cd` between prompts is always reflected on the very next one --
# this MUST NOT be cached across renders.
#
# Keyed by the queried directory (not just a single unconditional slot): every
# real render only ever queries one directory ($PWD, constant for the whole
# render), so this is still a single computation per render on the hot path --
# but keying by directory also makes the memo safe for callers outside a
# fish_prompt render (e.g. tests exercising __gpy_read_instant_cache directly
# against several different throwaway repos in one process), where a bare
# unconditional cache would wrongly serve the first repo's root for every
# later repo queried in the same process.
#
# __gpy_find_git_root may return nothing (empty) when there's no repo; that
# empty result is itself stored (fish's `set -g var (cmd-that-outputs-nothing)`
# still marks `var` as set, just with zero elements -- `set -q` sees it as set)
# so a repo-less directory doesn't repeat the walk on a later call for the same
# directory within the render.
function __gpy_memoized_git_root --argument-names start_dir --description 'Per-render memoized git root lookup'
    set -q start_dir[1]; or set start_dir $PWD
    if not set -q __gpy_git_root_this_render_dir; or test "$__gpy_git_root_this_render_dir" != "$start_dir"
        set -g __gpy_git_root_this_render_dir $start_dir
        set -g __gpy_git_root_this_render (__gpy_find_git_root $start_dir)
    end
    printf '%s' "$__gpy_git_root_this_render"
end

# Helper function to convert a path to a cache key (matches Rust implementation)
# Uses a reversible escape scheme so distinct paths never collide. The escape
# character `_` is doubled first, then each separator class maps to a unique
# `_`-prefixed token. Order MUST match path_to_cache_key in
# gpy-agent/src/cache/instant_prompt.rs exactly.
# Example: /Users/foo/project -> _sUsers_sfoo_sproject
function __gpy_path_to_cache_key --argument-names path --description 'Convert path to cache key'
    string replace -a -- _ __ "$path" | string replace -a -- / _s | string replace -a -- '\\' _b | string replace -a -- ':' _c | string replace -a -- ' ' _w
end

# On-disk stem of a cache key (#771): unchanged up to 200 characters (existing
# names stay byte-for-byte), else 50-character chunks joined by `/` so no
# filename component exceeds NAME_MAX. MUST match `cache_key_path_for` in
# gpy-agent/src/cache/instant_prompt.rs (pinned by
# tests/fixtures/cache_key_vectors.tsv). Builtins only; fish counts
# characters (code points), as the agent does.
function __gpy_chunk_cache_key --argument-names key --description 'On-disk stem of a cache key'
    if test (string length -- "$key") -le 200
        printf '%s\n' "$key"
        return
    end
    string match -ra -- '(?s).{1,50}' "$key" | string join /
end

# Filesystem-safe token identifying the previous-segment background a cache entry
# was rendered with. Empty -> "none" (context-free). Every non-alphanumeric byte
# becomes "_". MUST match `prev_bg_token` in
# gpy-agent/src/cache/instant_prompt.rs so both sides resolve the same cache file.
function __gpy_prev_bg_token --argument-names prev_bg --description 'Cache-key token for prev_bg'
    if test -z "$prev_bg"
        echo none
        return
    end
    string replace -ra -- '[^a-zA-Z0-9]' _ "$prev_bg"
end

# Cache-filename suffix for a given (is_last, is_first) combination, e.g.
# `__gpy_cache_variant_suffix git true false` -> "git_last". MUST match
# `variant_suffix` in gpy-agent/src/cache/instant_prompt.rs so both sides
# resolve the same cache file (#401).
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

# Resolve the path an instant-cache entry for `suffix` (git* or lang*) under
# `cwd` is keyed by, or print nothing and return 1 when there is none. Git
# caches are keyed by the repository root. Language caches additionally cover
# non-Git project directories (detected by package.json, Cargo.toml, etc.),
# which the agent keys by the canonical request path. Mirror that fallback so
# warm language caches are consumed outside Git repositories instead of
# cold-missing forever (#173). Builtin-only (memoized git root, `path
# resolve`): it runs on every prompt (#766).
function __gpy_instant_cache_key_path --argument-names suffix cwd --description 'Path an instant-cache entry is keyed by'
    set -l key_path (__gpy_memoized_git_root $cwd)
    if test -z "$key_path"; and string match -q 'lang*' -- $suffix
        # No Git root: language cache is keyed by the canonical request path.
        # `path resolve` mirrors the agent's `canonicalize` (resolves symlinks).
        set key_path (path resolve -- "$cwd" 2>/dev/null)
    end
    test -n "$key_path"; or return 1
    printf '%s' "$key_path"
end

# Return 0 iff any instant-cache entry exists for `base` (git|lang) under
# `cwd`, in any position variant or prev_bg context (#766). The agent writes
# every is_last/is_first variant at once (write_git / write_language_variants),
# but the context-free `.none` token only when some request carried no
# prev_bg, so a probe of `.none` alone would miss a cache populated only by
# contextual requests. No resolvable cache directory counts as no entry,
# matching __gpy_read_instant_cache's miss. Fork-free: a glob that matches
# nothing in `set` is an empty list, not an error.
function __gpy_instant_cache_present --argument-names base cwd --description 'Whether any instant-cache entry exists for git|lang under cwd'
    set -l key_path (__gpy_instant_cache_key_path $base $cwd); or return 1
    set -l cache_dir (__gpy_instant_cache_dir)
    test -n "$cache_dir"; or return 1
    set -l cache_key (__gpy_path_to_cache_key $key_path)
    # `<key>.<base>.<token>.ansi` and `<key>.<base>_<position>.<token>.ansi`.
    set -l hits $cache_dir/$cache_key.$base.*.ansi $cache_dir/$cache_key.$base"_"*.ansi
    set -q hits[1]
end

# Read the instant-prompt cache for a git repository or language project.
#
# Content (when found) is printed to stdout exactly as before. Status is now
# carried ENTIRELY by the exit code (#612) -- no more per-suffix
# staleness/variant-fallback globals to set-and-clear around every call.
# Codes (bit 2 = stale, bit 4 = variant-fallback; see
# __gpy_cache_status_stale / __gpy_cache_status_variant in core/util.fish):
#   0 = fresh, exact-token hit
#   1 = miss (no cache file at all; nothing printed)
#   2 = stale, exact-token hit
#   4 = fresh, served via the `.none` variant fallback
#   6 = stale, served via the `.none` variant fallback
#
# Git and language caches used to be two hand-duplicated branches differing
# only in which TTL var they read and which global they set; now it's one
# path parameterised by TTL (via __gpy_cache_freshness), selected by whether
# `suffix` starts with `lang`.
function __gpy_read_instant_cache --argument-names suffix cwd prev_bg --description 'Read instant-prompt cache; status via exit code (see comment above)'
    set -q cwd[1]; or set cwd $PWD
    if test -z "$suffix"
        set suffix git
    end

    # Resolve the cache-key path (git root, or the canonical project path for
    # language caches outside Git).
    set -l key_path (__gpy_instant_cache_key_path $suffix $cwd); or return 1

    # Compute cache directory
    set -l cache_dir (__gpy_instant_cache_dir)
    if test -z "$cache_dir"
        return 1
    end

    # Convert resolved path to cache key and build cache file path. The rendered
    # ANSI bakes in the fg:prev_bg opening chevron, so the cache is keyed by the
    # previous-segment background too. Fall back to the context-free ("none")
    # render when no context-specific entry exists yet — the caller then triggers
    # a refresh that populates the correct token file and repaints via SIGURG.
    #
    # `suffix` already encodes is_first (see __gpy_cache_variant_suffix and its
    # callers in __gpy_request): the writer side (write_git/write_language in
    # cache/instant_prompt.rs) always populates all four is_last/is_first
    # variants, so this lookup resolves the correct opening-cap state directly
    # (#401) rather than rendering as if not-first and self-correcting later.
    set -l cache_key (__gpy_chunk_cache_key (__gpy_path_to_cache_key $key_path))
    set -l token (__gpy_prev_bg_token "$prev_bg")
    set -l cache_file "$cache_dir/$cache_key.$suffix.$token.ansi"
    # Track whether this read is about to fall back to the `.none` variant
    # instead of serving the requested token's own file (#436/#454): the
    # content is still correct, but its opening chevron was rendered without
    # this render's real prev_bg context.
    set -l used_variant_fallback 0
    if not test -f "$cache_file"; and test "$token" != none
        set cache_file "$cache_dir/$cache_key.$suffix.none.ansi"
        set used_variant_fallback 1
    end

    if not test -f "$cache_file"
        return 1
    end

    # Language cache is served unconditionally regardless of age (avoids
    # flicker on clock repaints); git cache is served-stale too (issue #160).
    # Both just differ in which TTL default applies once staleness is flagged.
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

# Public helper: sends an op with optional cwd and is_last flag, returns JSON (or empty on timeout/error).
# Falls back to oneshot mode if IPC is unavailable.
# For git operations, tries instant-prompt cache first for 0ms latency.
function __gpy_request --argument-names op cwd is_last prev_bg is_first --description 'Send a JSON op to the agent with oneshot fallback'
    # For git/lang operations, try instant cache first (0ms latency). Any
    # non-miss status (fresh, stale, or variant-fallback) is a hit here --
    # __gpy_request only needs SOME content to serve, not the freshness
    # distinction the segments use to decide whether to also fire a refresh.
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
            echo $cached_result
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

    # Try IPC first
    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    # If IPC failed and this is a data request (git/lang), fall back to oneshot.
    # NOTE: oneshot fallback does not pass prev_bg — the CLI subcommand has no
    # --prev-bg flag, so the powerline opening chevron color is skipped on
    # first-shell startup until the daemon connects. is_first IS passed
    # (--first, #401), so the opening cap's presence/absence is still correct.
    if test -z "$result"; and __gpy_resolve_agent_binary >/dev/null
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

# Shared throttled background refresh dispatcher (#612). Replaces
# __gpy_maybe_refresh_git (segments/git.fish) and three near-identical inline
# copies in segments/language.fish (cold-miss, variant-fallback, stale) that
# differed only in throttle-var bookkeeping.
#
# The refresh goes straight to the agent, bypassing the instant cache: a
# stale-but-present entry must not be re-served instead of refreshed (#458).
# The agent recomputes, rewrites the instant cache and, when the rendered
# output changed, repaints live shells via the SIGURG doorbell. The reply is
# discarded, so there is never a oneshot fallback: with no daemon there is
# nothing to refresh, and forking `gpy-agent oneshot` here would block the
# prompt and spend the per-render oneshot budget a visible segment needs
# (#324, #685).
#
# Arguments:
#   op                 — git|lang: the throttle-var prefix and the data op.
#   root               — cwd/repo root the refresh is for.
#   cache_suffix       — e.g. "git_last" / "lang_first" (see
#                        __gpy_cache_variant_suffix).
#   is_last, prev_bg,
#   is_first           — pass-through render-context flags, so the refresh
#                        asks for the same cache variant the render read.
#   throttle_key_extra — optional extra token appended to the throttle
#                        variable name. The language variant-fallback path
#                        needs a throttle keyed by prev_bg TOKEN as well as
#                        path+suffix (a recent miss/stale refresh for a
#                        DIFFERENT prev_bg context must not starve this
#                        context's correction, and vice versa); passing the
#                        token here keeps that throttle bucket distinct from
#                        the plain refresh throttle without a second copy of
#                        the bookkeeping below.
#
# Everything except the IPC client runs in the foreground: fish does not fork
# for functions, so the only thing put in the background is the external
# client pipeline in __gpy_ipc_send_detached (#685).
# __gpy_register_with_agent runs here for BOTH ops: it's a cheap no-op once
# already registered (#419) and records success via a global, so it must stay
# in-process. Previously only git's throttle helper called it; language's
# three inline copies never did -- that was a latent asymmetry (language
# refreshes never opportunistically recovered a stale registration after an
# agent restart), not a deliberate difference, so consolidating picks up the
# git behavior for language too.
function __gpy_maybe_refresh --argument-names op root cache_suffix is_last prev_bg is_first throttle_key_extra --description 'Shared throttled background refresh dispatcher for git/lang segments'
    __gpy_resolve_agent_binary >/dev/null; or return

    set -q is_last[1]; or set is_last ""
    set -q prev_bg[1]; or set prev_bg ""
    set -q is_first[1]; or set is_first ""
    set -q throttle_key_extra[1]; or set throttle_key_extra ""

    set -l safe_path (string replace -ra '[^a-zA-Z0-9_]' _ "$root")
    set -l throttle_var "__gpy_"$op"_refresh_ms_$safe_path"_"$cache_suffix"
    if test -n "$throttle_key_extra"
        set throttle_var "__gpy_"$op"_variant_refresh_ms_$safe_path"_"$cache_suffix"_"$throttle_key_extra"
    end

    set -l now_ms (__gpy_get_time_ms)
    set -l last_ms 0
    set -q $throttle_var; and set last_ms $$throttle_var

    __gpy_throttle_elapsed "$last_ms" "$now_ms" 500; or return
    set -g $throttle_var $now_ms

    __gpy_register_with_agent >/dev/null 2>&1

    set -l payload (__gpy_build_data_payload $op "$root" ansi "$is_last" "$prev_bg" "$is_first")
    __gpy_ipc_send_detached $payload $GPY_IPC_TIMEOUT_MS
end

# Helper function to get the agent version and protocol version
# Returns the version string if agent is available, empty string otherwise
# Sets global variables __gpy_agent_version and __gpy_agent_protocol_version
function __gpy_get_agent_version --description 'Get the version and protocol version from the agent'
    # Do nothing if agent is disabled
    set -q GPY_AGENT_ENABLED; or set -g GPY_AGENT_ENABLED 1
    if test "$GPY_AGENT_ENABLED" -ne 1
        return 1
    end

    # Send status request to agent (JSON format)
    set -l payload '{"op":"status"}'
    set -l result (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

    # If no result, agent might not be running
    if test -z "$result"
        return 1
    end

    # Parse the version and protocol_version from the JSON status response
    # Response format: {"AgentStatus":{"version":"0.1.0","protocol_version":2,"watched_repos":5,...}}

    # Extract version field (fork-free, #612: no trailing `tail` subprocess)
    # NOTE: local var is NOT named `version` -- that's fish's own read-only
    # special variable, and `set -l version ...` errors ("wrong scope").
    set -l agent_version_str (__gpy_json_extract_string version $result)
    if test $status -ne 0
        return 1
    end
    set -g __gpy_agent_version $agent_version_str

    # Extract protocol_version field (fork-free, #612: no trailing `tail` subprocess)
    set -l protocol_version (__gpy_json_extract_int protocol_version $result)
    if test $status -eq 0
        set -g __gpy_agent_protocol_version $protocol_version
    else
        set -g __gpy_agent_protocol_version 0
    end

    echo $__gpy_agent_version
    return 0
end

# Helper function to check protocol version compatibility
# Displays a warning if there's a version mismatch
function __gpy_check_protocol_version --description 'Check and warn on protocol version mismatch'
    # Expected protocol version (must match gpy-agent/src/ipc/protocol.rs::PROTOCOL_VERSION)
    set -l EXPECTED_PROTOCOL_VERSION 2

    # Get agent version and protocol version
    if not __gpy_get_agent_version >/dev/null
        # Agent not available, no check needed
        return 0
    end

    # Check if protocol version matches
    if test "$__gpy_agent_protocol_version" -ne "$EXPECTED_PROTOCOL_VERSION"
        echo "" >&2
        echo "⚠️  WARNING: Protocol Version Mismatch" >&2
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

# Agent process supervision and automatic restart
function __gpy_agent_supervisor_start --description 'Start agent supervisor for automatic restart'
    # The flags exported from config.toml are the source of truth (#657, #699):
    # with the agent disabled there is nothing to supervise.
    if test "$GPY_AGENT_SUPERVISOR_ENABLED" = 0; or test "$GPY_AGENT_ENABLED" = 0
        return 0
    end

    if not __gpy_resolve_agent_binary >/dev/null
        __gpy_log_warn ipc "Agent binary not found in PATH; skipping supervisor"
        return 1
    end

    # Use PID file to track supervisor across all fish instances
    set -l runtime_root (__gpy_runtime_root)
    mkdir -p "$runtime_root"
    set -l supervisor_pidfile "$runtime_root/supervisor.pid"

    # Check if supervisor is already running via PID file
    if test -f "$supervisor_pidfile"
        set -l existing_pid (cat "$supervisor_pidfile" 2>/dev/null)
        if test -n "$existing_pid"; and kill -0 $existing_pid 2>/dev/null
            # Supervisor already running
            return 0
        else
            # Stale PID file, remove it
            rm -f "$supervisor_pidfile" 2>/dev/null
        end
    end

    # Spawn supervisor in completely detached process to avoid blocking.
    # Paths are passed as $argv[1] and $argv[2] to avoid quoting problems with
    # paths that contain spaces. All three core files are sourced so the loop
    # function is available without assuming ipc.fish re-sources its deps.
    # fds are redirected at spawn: in fish, a redirect-only `exec` does not
    # reopen the shell's own fds, so the child would keep this terminal (#767).
    # `cd /` keeps the long-lived child from pinning the spawning shell's cwd.
    set -l config_root (__gpy_config_root)
    set -l supervisor_script '
        cd /
        set -gx GPY_SUPERVISOR_CHILD 1
        set -l pidfile $argv[1]
        set -l cfgroot $argv[2]
        echo %self > "$pidfile"
        source "$cfgroot/gpy/core/constants.fish"; or exit 1
        source "$cfgroot/gpy/core/util.fish"; or exit 1
        source "$cfgroot/gpy/core/ipc.fish"; or exit 1
        __gpy_agent_supervisor_loop
        rm -f "$pidfile"
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

    # Existing shells keep a stale __gpy_registered flag across an agent
    # restart and would receive no live updates until they happened to render
    # another prompt. The agent's .reregister flag tells the shell to forget it
    # and register with the new instance. On failure (e.g. the circuit breaker
    # is still backing off from pings made while the agent was down) nothing
    # is recorded: the agent's re-nudge writes the flag again and retries.
    set -e __gpy_registered
    set -e __gpy_registered_pid
    set -e __gpy_last_workspace
    __gpy_log_debug ipc "Re-registration requested by agent for PID $fish_pid"
    __gpy_register_with_agent >/dev/null 2>&1
    set -l register_status $status
    # A successful registration already checked; this covers a refused one
    # (the new agent's export is on disk either way).
    __gpy_reload_if_theme_export_changed
    return $register_status
end

# The agent may have rewritten the theme-export cache since this shell sourced
# it: a config edited while no agent ran is only exported when the agent next
# starts, and that write can land before this shell registers, so before the
# shell is tracked for the agent's .reload doorbell (#701). Re-apply the export
# when the file is newer than the one sourced, or when none was recorded (the
# spawn fallback ran) and the file now exists. Fork-free (`path mtime` is a
# builtin) and only run on registration, never on the per-prompt path.
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
        # After tracking: an export rewrite from here on is rung as .reload by
        # the agent; one that already landed is caught here (#701).
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

    # Always send workspace sync (don't skip based on last workspace)
    # This ensures we detect if the agent restarted and lost our registration
    set -l response (__gpy_request workspace $cwd)

    # Only a lost registration (agent restarted) warrants re-registering. Any
    # other error (path rejected, transient failure) keeps the registration so
    # the next valid cd syncs normally (#764).
    if string match -q '*not registered*' -- $response
        __gpy_log_debug ipc "Workspace sync failed (not registered), re-registering PID %self"
        set -e __gpy_registered
        set -e __gpy_registered_pid
        set -e __gpy_last_workspace
        __gpy_register_with_agent
    else if test -n "$response"; and not string match -q '*"error"*' -- $response
        # Success - update last workspace
        set -g __gpy_last_workspace $cwd
    end
end

function __gpy_agent_supervisor_loop --description 'Background loop for agent supervision'
    # Guard against unset/empty/non-numeric variables that would cause sleep to
    # receive no argument, producing a 100% CPU busy-loop. constants.fish
    # already validates these at load time; re-validate here too via the same
    # pure helper (not a re-inlined regex) since a test or config reload can
    # `set -gx` a fresh value after constants.fish has already sourced (#612).
    set -l check_interval (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS" 30)
    set -l max_restart_attempts (__gpy_uint_or_default "$GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS" 5)

    set -l restart_count 0
    set -l last_restart_time 0

    while true
        # Re-read both flags every iteration so a value changed after spawn
        # stops the loop (#699).
        if test "$GPY_AGENT_SUPERVISOR_ENABLED" = 0; or test "$GPY_AGENT_ENABLED" = 0
            break
        end

        if not __gpy_resolve_agent_binary >/dev/null
            sleep $check_interval; or break
            continue
        end

        # Check if agent is responding
        if not __gpy_agent_is_healthy
            set -l current_time (date +%s)

            set -l rate_limit (__gpy_uint_or_default "$GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS" 60)

            # Rate limit restarts
            if test (math "$current_time - $last_restart_time") -gt $rate_limit
                if test $restart_count -lt $max_restart_attempts
                    __gpy_log_warn ipc "Agent not responding, attempting restart ($restart_count/$max_restart_attempts)"
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
        else
            set restart_count 0
        end

        sleep $check_interval; or break
    end
end

function __gpy_agent_is_healthy --description 'Check if agent is healthy and responding'
    set -l socket_path (__gpy_ipc_endpoint)

    # Check if socket exists
    if not test -S "$socket_path"
        return 1
    end

    # Try to ping the agent
    set -l response (__gpy_request ping)
    if test -z "$response"
        return 1
    end

    # Parse response to check for success
    if string match -q '*"status":"ok"*' "$response"
        return 0
    end

    return 1
end

function __gpy_agent_restart --description 'Restart the agent process'
    set -l socket_path (__gpy_ipc_endpoint)
    set -l agent_binary (__gpy_resolve_agent_binary)

    # Try graceful shutdown only when a live socket exists. On cold start there
    # is nothing to stop, and the CLI prints a user-facing notice to stdout.
    if test -n "$agent_binary"; and test -S "$socket_path"
        "$agent_binary" stop >/dev/null 2>&1
        # Wait briefly for graceful shutdown
        sleep 0.5
    end

    # If socket still exists, try to find and stop the process using lsof (more targeted than pgrep)
    if test -S "$socket_path"
        # Use lsof to find the specific process using this socket
        set -l socket_pid (lsof -t "$socket_path" 2>/dev/null)
        if test -n "$socket_pid"
            # Kill only the process using our specific socket
            kill -TERM $socket_pid 2>/dev/null
            sleep 0.3
            # Force kill if still running
            if kill -0 $socket_pid 2>/dev/null
                kill -KILL $socket_pid 2>/dev/null
                sleep 0.1
            end
        end
        # Clean up socket file
        rm -f "$socket_path" 2>/dev/null
    end

    # Start new agent
    if test -n "$agent_binary"
        "$agent_binary" start 2>/dev/null

        # Wait for agent to start
        if __gpy_wait_for_socket "$socket_path"
            return 0
        end
    end

    return 1
end

# Initialize agent supervisor on first prompt to avoid hanging during source
# Only set up event handlers if GPY is not completely disabled
if not set -q GPY_SUPERVISOR_CHILD; and not test "$GPY_AGENT_ENABLED" = 0 -a "$GPY_AGENT_SUPERVISOR_ENABLED" = 0
    function __gpy_start_supervisor_on_prompt --on-event fish_prompt
        if test "$GPY_AGENT_ENABLED" = 1
            # Only the restart loop depends on the supervisor flag; protocol
            # check, bounded autostart and registration always run (#700).
            if not set -q __gpy_supervisor_initialized
                set -g __gpy_supervisor_initialized 1
                test "$GPY_AGENT_SUPERVISOR_ENABLED" = 0; or __gpy_agent_supervisor_start
            end

            # One-time protocol version check per shell session: an agent
            # that responds to ping but speaks a different wire protocol
            # (e.g. left running across an upgrade) must be treated as not
            # running, or this shell silently talks past it indefinitely
            # (#307). Guarded so it only fires once, not on every retry.
            if not set -q __gpy_protocol_checked
                set -g __gpy_protocol_checked 1
                if __gpy_agent_available; and not __gpy_check_protocol_version
                    __gpy_log_warn ipc "Protocol version mismatch detected; restarting agent"
                    __gpy_agent_restart
                end
            end

            # Auto-start agent if not running
            if not __gpy_agent_available
                set -l max_attempts (__gpy_uint_or_default "$GPY_AGENT_AUTOSTART_MAX_ATTEMPTS" 3)

                set -l attempts 0
                if set -q __gpy_autostart_attempts
                    set attempts $__gpy_autostart_attempts
                end

                if test $attempts -ge $max_attempts
                    # Crash-looping agent binary: stop re-forking every prompt and
                    # let the background supervisor loop keep retrying instead.
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
                        # Give agent time to start
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

# The doorbell handler must be defined outside the conditional block above so
# it's always available in interactive shells.
#
# WHY SIGURG:
# The agent rings every shell with SIGURG only; the meaning travels in
# per-shell flag files next to the registry file (<pid>.reregister,
# <pid>.reload) that the agent creates before signalling. SIGURG's default
# disposition is ignore, so a shell that `exec`s itself (same PID, still
# registered) survives a notification that lands before this handler is
# installed (#674). Every ring repaints all open terminals via the
# variable-change pattern: incrementing __gpy_repaint_trigger fires
# __gpy_repaint_on_variable which calls commandline -f force-repaint.
function __gpy_doorbell_handler --on-signal SIGURG
    set -l registry_file (__gpy_shell_registry_file)
    # Builtin existence checks keep the common no-flag path fork-free. Each
    # flag is removed BEFORE acting so a ring arriving mid-action re-queues.
    if test -e $registry_file.reregister
        rm -f $registry_file.reregister
        __gpy_refresh_registration_after_restart
    end
    if test -e $registry_file.reload
        rm -f $registry_file.reload
        __gpy_apply_agent_reload
    end
    # Trigger repaint via variable change (this is the Hydro/Tide approach)
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end

function __gpy_apply_agent_reload --description 'Reload theme/config after the agent .reload doorbell'
    # Agent writes the theme export cache before ringing, so source the
    # file directly (no fork). Fall back to spawning the binary if the cache is
    # absent (e.g., very first reload before the first agent write completes).
    # Deliberately does not branch on success (#576): proceeds unconditionally
    # to the cache clears below even if neither path produced fresh theme
    # variables. The caller (__gpy_doorbell_handler) repaints afterwards.
    __gpy_apply_theme_export

    # The export erases status icons the theme leaves unset; refill the
    # nerd/ascii fallbacks from the current __prompt_icons (#791).
    __gpy_initialize_icons

    # Load implementation files for any segment newly added to __enabled_segments
    __gpy_ensure_segments_loaded

    # Clear language detection caches
    for var in (set -n | string match '__gpy_lang_cache_*')
        set -e $var
    end

    # Clear character/directory render caches (#343): a theme change can alter
    # the rendered templates, so a stale cache entry must not survive a reload
    # even if the input tuple happens to repeat.
    set -e __gpy_char_cache_key
    set -e __gpy_char_cache_val
    set -e __gpy_dir_cache_key
    set -e __gpy_dir_cache_val
end

# Variable change handler that actually triggers the repaint
# This works because Fish allows commandline -f repaint from variable event handlers
function __gpy_repaint_on_variable --on-variable __gpy_repaint_trigger
    commandline -f force-repaint 2>/dev/null
end
