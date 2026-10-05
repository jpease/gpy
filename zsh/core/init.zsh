# zsh/core/init.zsh

autoload -Uz add-zsh-hook
zmodload zsh/datetime

# Flag to track registration
typeset -g __gpy_registered=""
typeset -g __gpy_last_workspace=""

# SSH-session cache for the hostname segment's default visibility gate.
# Computed once at shell startup rather than per-prompt.
typeset -g __gpy_is_ssh=0
if [[ -n "${SSH_CONNECTION:-}" || -n "${SSH_CLIENT:-}" || -n "${SSH_TTY:-}" ]]; then
    __gpy_is_ssh=1
fi

# Root check cached once at shell startup — one `id -u` fork, never per-prompt.
# Mirrors fish's __gpy_is_root; feeds the username segment's visibility gate (#252).
typeset -g __gpy_is_root=0
if [[ "$(id -u)" -eq 0 ]]; then
    __gpy_is_root=1
fi

# Sudo-session cache — pure env-var check, no fork. sudo exports SUDO_USER into
# the elevated shell; a non-empty value means this shell was entered via sudo.
typeset -g __gpy_is_sudo=0
if [[ -n "${SUDO_USER:-}" ]]; then
    __gpy_is_sudo=1
fi

# Duration tracking
typeset -g __gpy_cmd_start_time=0
typeset -g __gpy_cmd_duration=0
typeset -g __duration_threshold_ms=2000  # milliseconds; overwritten by `gpy theme export`

# Character/directory render memoization (#343): keyed by the full input
# tuple + theme identity, cleared on reload (zsh/core/signals.zsh). A cache
# hit must cost zero IPC and zero forks.
#
# __gpy_render_prompt executes entirely inside the `$(...)` command
# substitution __gpy_precmd uses to capture $PROMPT, which forks a subshell.
# Reading these vars from in there is safe (a subshell inherits a copy of the
# parent's variables), but a *write* made in there would vanish the instant
# the subshell exits -- so a cache-miss write goes to one of the two relay
# files below instead of directly to the vars. __gpy_precmd reads each relay
# file back into these vars immediately after the subshell returns, then
# deletes it, so a stale relay is never reprocessed.
typeset -g __gpy_char_cache_key=""
typeset -g __gpy_char_cache_val=""
typeset -g __gpy_dir_cache_key=""
typeset -g __gpy_dir_cache_val=""

# The relay files live inside a PRIVATE per-session directory, not directly
# under a world-writable /tmp with a predictable $$-based name. `mktemp -d`
# gives an unpredictable name AND creates the dir mode 0700 (only this user
# can read/write/traverse), which closes two attacks at once: a local
# attacker can neither guess the path nor pre-create a file we'd read back
# into $PROMPT (a terminal-escape / prompt-injection vector). The relay
# content is only ever assigned into plain vars (never eval'd), but the 0700
# dir is what actually makes that content trusted.
#
# Fail SAFE if mktemp is unavailable or fails: leave __gpy_cache_dir empty and
# the relay paths empty. Every relay read/write below is guarded on a
# non-empty __gpy_cache_dir, so an empty value simply DISABLES the zsh
# subshell-relay memoization -- each prompt then does the normal IPC round
# trip. A slower-but-correct prompt is strictly better than writing prompt
# content through an insecure predictable temp path.
typeset -g __gpy_cache_dir=""
typeset -g __gpy_char_cache_relay_path=""
typeset -g __gpy_dir_cache_relay_path=""
__gpy_cache_dir=$(mktemp -d "${TMPDIR:-/tmp}/gpy_cache_XXXXXX" 2>/dev/null)
if [[ -n "$__gpy_cache_dir" ]]; then
    __gpy_char_cache_relay_path="$__gpy_cache_dir/char"
    __gpy_dir_cache_relay_path="$__gpy_cache_dir/dir"
fi

# Reads a cache-miss relay file (see the comment above the cache var
# declarations) back into the given shell-private cache vars, then removes
# the file so it's never reprocessed. A cheap stat check on the common case
# (no pending relay -- the last render was a cache hit or wrote nothing).
# Empty relay_path (mktemp failed -> memoization disabled) short-circuits.
function __gpy_load_cache_relay() {
    local relay_path="$1" key_var="$2" val_var="$3"
    [[ -n "$relay_path" && -s "$relay_path" ]] || return 0
    local content
    content=$(<"$relay_path")
    rm -f "$relay_path"
    typeset -g "$key_var=${content%%$'\n'*}"
    typeset -g "$val_var=${content#*$'\n'}"
}

# Load theme from agent (TOML-based configuration).
#
# Sources the agent's cached theme-export.zsh file when present (written by
# write_theme_export_to_dir, gpy-agent/src/cache/theme_export.rs, on startup
# and every config/theme change -- no call-site change needed there) instead
# of forking `gpy-agent theme export` on every shell start and config
# reload (#614). Falls back to the fork only when the cache file is absent
# (e.g. a `gpy-agent` older than this cache, or a first run before it has
# ever written one). Mirrors Fish's __gpy_apply_theme_export
# (fish/core/init.fish) / __gpy_theme_export_cache_path (fish/core/util.fish).
# The reload path (__gpy_reload_config, signals.zsh) calls this same function, so it
# picks up the cache too.
function __gpy_load_theme() {
    local cache_path
    cache_path=$(__gpy_theme_export_cache_path)
    if [[ -n "$cache_path" && -f "$cache_path" ]]; then
        source "$cache_path"
        return $?
    fi

    if command -v gpy-agent &>/dev/null; then
        # Source the theme export output
        eval "$(gpy-agent theme export --format zsh 2>/dev/null)"
        return $?
    else
        return 1
    fi
}

function __gpy_preexec() {
    # Record start time in milliseconds
    __gpy_cmd_start_time=$((EPOCHREALTIME * 1000))
}

add-zsh-hook preexec __gpy_preexec

function __gpy_register_with_agent() {
    local request
    request=$(__gpy_build_register_payload)
    if __gpy_send_json "$request" >/dev/null; then
        __gpy_registered=1
        __gpy_last_workspace="$PWD"
        __gpy_track_shell_for_agent_recovery
        return 0
    fi
    return 1
}

# Forget the current registration and register again. Runs from TRAPURG
# (the doorbell) after an agent restart (#638); safe at any time -- a missing
# socket just fails and the next prompt retries.
function __gpy_reregister_with_agent() {
    __gpy_registered=""
    __gpy_last_workspace=""
    __gpy_register_with_agent
}

function __gpy_build_register_payload() {
    local json_cwd
    json_cwd=$(__gpy_escape_json "$PWD")
    local shell_version="$ZSH_VERSION"
    print -r -- "{\"op\":\"register\",\"pid\":$$,\"cwd\":\"$json_cwd\",\"shell\":\"zsh\",\"shell_version\":\"$shell_version\"}"
}

function __gpy_build_workspace_payload() {
    local json_cwd
    json_cwd=$(__gpy_escape_json "$PWD")
    print -r -- "{\"op\":\"workspace\",\"pid\":$$,\"cwd\":\"$json_cwd\"}"
}

function __gpy_sync_workspace() {
    [[ -n "$__gpy_registered" ]] || return 0
    [[ "$__gpy_last_workspace" != "$PWD" ]] || return 0

    local request
    request=$(__gpy_build_workspace_payload)
    local response
    response=$(__gpy_send_json "$request" 2>/dev/null)
    if [[ $? -ne 0 || "$response" == *"error"* || "$response" == *"not registered"* ]]; then
        __gpy_registered=""
        __gpy_last_workspace=""
        __gpy_register_with_agent
        return
    fi

    __gpy_last_workspace="$PWD"
}

function __gpy_segment_would_render() {
    local seg=$1
    if (( ! $+functions[__gpy_segment_$seg] )); then
        return 1
    fi

    local detect_fn="__gpy_segment_${seg}_detect"
    if (( $+functions[$detect_fn] )); then
        "$detect_fn"
        return
    fi

    return 0
}

function __gpy_precmd() {
    local ret=$?
    # Kept for the signal handlers, which re-render outside precmd (#637).
    __gpy_last_exit_code=$ret

    # Calculate command duration if we have a start time
    if [[ $__gpy_cmd_start_time -gt 0 ]]; then
        local end_time=$((EPOCHREALTIME * 1000))
        __gpy_cmd_duration=$(( end_time - __gpy_cmd_start_time ))
        __gpy_cmd_start_time=0
    else
        __gpy_cmd_duration=0
    fi

    # Register on first run
    if [[ -z "$__gpy_registered" ]]; then
        __gpy_register_with_agent
    fi

    # Construct prompt
    PROMPT=$(__gpy_render_prompt $ret)

    # Pull any cache-miss write made inside that subshell back into the real
    # cache vars (#343) -- see the comment above the cache var declarations.
    __gpy_load_cache_relay "$__gpy_char_cache_relay_path" __gpy_char_cache_key __gpy_char_cache_val
    __gpy_load_cache_relay "$__gpy_dir_cache_relay_path" __gpy_dir_cache_key __gpy_dir_cache_val

    __gpy_sync_workspace

    # Periodic health check (#638): a dead agent is restarted from the next
    # prompt, rate limited and off the render path, mirroring Bash's
    # __gpy_supervisor_check. Defined by supervisor.zsh only when the
    # supervisor is enabled.
    (( $+functions[__gpy_supervisor_check] )) && __gpy_supervisor_check
}

function __gpy_render_prompt() {
    local last_ret=$1
    # Blank line before the prompt for visual separation between commands
    # (theme-controlled via `__gpy_add_newline`, default on). Fish has always
    # done this unconditionally; bash/zsh had no equivalent, so the same theme
    # rendered visibly tighter here.
    local p=""
    [[ "${__gpy_add_newline:-1}" == "1" ]] && p=$'\n'
    local seg
    local -a segments_to_render

    # Reset the per-render oneshot-fallback budget so a dead daemon gets one
    # fresh oneshot fork this render, not zero forever (#324). A plain local
    # variable, not a predictable `${TMPDIR:-/tmp}/.gpy_oneshot_used_$$`
    # marker file (#614): segments run inside `$(...)` subshells and can't
    # write back to this scope, so a request wrapper that actually forks
    # oneshot signals it via GPY_SEG_STATUS_ONESHOT, which the loop below
    # reads off `$?` and folds in here -- later segments' subshells inherit
    # this variable's value by fork, so __gpy_oneshot_claim (ipc.zsh) sees it
    # without any file I/O.
    local __gpy_oneshot_used=0

    # Default segments if theme not loaded
    if [[ ${#__enabled_segments} -eq 0 ]]; then
        __enabled_segments=(directory git)
    fi

    for seg in $__enabled_segments; do
        if __gpy_segment_would_render "$seg"; then
            segments_to_render+=("$seg")
        fi
    done

    # Track the previous segment's background across the render pass. Segments run
    # in command-substitution subshells and cannot write back to this scope, so the
    # loop owns the tracker (unlike Fish, where each segment sets it directly).
    # Start at "black" so the first chevron blends into a dark terminal background.
    local prev_bg="black"
    local segment_count=${#segments_to_render}
    local segment_index=1
    for seg in $segments_to_render; do
        if (( $+functions[__gpy_segment_$seg] )); then
            if [[ "$seg" == "status" ]]; then
                p+="$(__gpy_segment_$seg $last_ret)"
            else
                # Convention (#613): "true" or "" -- the same tokens IPC
                # payloads use (`,"is_last":true`) and every segment receives
                # directly, with no per-segment last/first-literal conversion
                # (mirrors fish_prompt.fish).
                local is_last=""
                if (( segment_index == segment_count )); then
                    is_last="true"
                fi
                # Position-based, not segment-identity-based: whichever segment
                # ends up first here (clock, duration, or anything else) gets
                # is_first, so it can suppress an opening-cap glyph that makes
                # no sense with nothing rendered before it (mirrors Fish).
                local is_first=""
                if (( segment_index == 1 )); then
                    is_first="true"
                fi
                # Declared WITH a value: a bare `local seg_out` on the second
                # loop iteration re-declares an existing local, and zsh prints
                # `seg_out=...` for that (TYPESET_SILENT is off by default),
                # which landed inside PROMPT as a stray line above every
                # prompt with two or more segments, and on the terminal when
                # rendered from a trap (#637).
                local seg_out=""
                # A segment whose request fell back to oneshot signals it via
                # GPY_SEG_STATUS_ONESHOT (#614): captured as soon as each
                # `seg_out=$(...)` assignment below runs (before any further
                # command -- e.g. the directory relay-file write -- would
                # otherwise overwrite `$?`) and folded into the per-render
                # budget so later segments' subshells (which inherit
                # __gpy_oneshot_used by fork) don't fork oneshot again. A
                # cache-hit assignment (plain `seg_out=$__gpy_dir_cache_val`,
                # no command substitution) always yields 0 here, never
                # GPY_SEG_STATUS_ONESHOT -- correct, since no request ran.
                local seg_status=0
                if [[ "$seg" == "directory" ]]; then
                    # Memoized (#343). Reading the cache vars here is safe even
                    # though this whole function runs inside precmd's `$(...)`
                    # subshell -- see the cache var declarations above for why
                    # a cache-miss *write* has to go through a relay file
                    # instead of a direct assignment.
                    #
                    # is_last/is_first arrive already as "true"/"" (above), so
                    # the cache key just normalizes the empty case to the
                    # literal "false" it has always used -- no last/first-literal
                    # conversion needed here anymore.
                    local is_last_bool="${is_last:-false}"
                    local is_first_bool="${is_first:-false}"
                    local dir_key="${__gpy_theme_name:-}:${PWD}:${is_last_bool}:${is_first_bool}:${prev_bg}"
                    if [[ -n "$__gpy_dir_cache_key" && "$dir_key" == "$__gpy_dir_cache_key" ]]; then
                        seg_out="$__gpy_dir_cache_val"
                    else
                        seg_out="$(__gpy_segment_$seg "$is_last" "$prev_bg" "$is_first")"
                        seg_status=$?
                        # Never cache an empty/failed render: the agent-down
                        # fallback must retry next prompt, not get stuck
                        # serving nothing forever. Empty relay path == mktemp
                        # failed, so memoization is disabled (no insecure
                        # fallback path) -- skip the relay write entirely.
                        if [[ -n "$seg_out" && -n "$__gpy_dir_cache_relay_path" ]]; then
                            printf '%s\n%s' "$dir_key" "$seg_out" > "$__gpy_dir_cache_relay_path" 2>/dev/null
                        fi
                    fi
                else
                    # Quote all args: an unquoted empty $is_last collapses to zero
                    # words in zsh, which would shift $prev_bg/$is_first out of place.
                    seg_out="$(__gpy_segment_$seg "$is_last" "$prev_bg" "$is_first")"
                    seg_status=$?
                fi

                if [[ "$seg_status" -eq "$GPY_SEG_STATUS_ONESHOT" ]]; then
                    __gpy_oneshot_used=1
                fi

                p+="$seg_out"
                if [[ -n "$seg_out" ]]; then
                    # Advance prev_bg to this segment's background for the next
                    # chevron. __gpy_segment_bg (constants.zsh default,
                    # overridden by the agent's `theme export` -- #614) is
                    # called directly (not via `$(...)`) with an out-var so
                    # this never forks a subshell on the hot render path
                    # (#342); mirrors bash so both shells stay identical.
                    local seg_bg=""
                    __gpy_segment_bg "$seg" seg_bg
                    [[ -n "$seg_bg" ]] && prev_bg="$seg_bg"
                fi
            fi
        fi
        (( segment_index += 1 ))
    done

    # Two-line layout (opt-in via theme): render the segments on one line and
    # the prompt character on the next. Default themes export 0 → single-line
    # (no regression). Fish is always two-line and ignores this flag.
    if [[ "${__gpy_two_line:-0}" == "1" ]]; then
        p+=$'\n'
    fi

    # Prompt char.
    # The character is always agent-rendered (#199): the agent renders the prompt
    # symbol colored by exit status (Starship-style). When the agent returns
    # nothing (unavailable, or no theme template), fall back to the legacy symbol.
    local char_success=0
    [[ "$last_ret" -eq 0 ]] && char_success=1
    local char_rendered
    # The character's opening chevron uses fg:prev_bg too (default theme), so pass
    # the background left behind by the last rendered segment.
    #
    # Memoized (#343): skip the IPC round-trip + fork entirely when the input
    # tuple (theme identity + success + prev_bg) matches the last render. See
    # the cache var declarations above for why a cache-miss write goes through
    # a relay file instead of a direct assignment (this function runs inside
    # precmd's `$(...)` subshell).
    local char_key="${__gpy_theme_name:-}:${char_success}:${prev_bg}"
    if [[ -n "$__gpy_char_cache_key" && "$char_key" == "$__gpy_char_cache_key" ]]; then
        char_rendered="$__gpy_char_cache_val"
    else
        char_rendered=$(__gpy_request_character "$char_success" "true" "$prev_bg")
        # Never cache an empty/failed render: the agent-down fallback must
        # retry on the very next prompt, not get stuck serving nothing forever.
        # Empty relay path == mktemp failed, so memoization is disabled (no
        # insecure fallback path) -- skip the relay write entirely.
        if [[ -n "$char_rendered" && -n "$__gpy_char_cache_relay_path" ]]; then
            printf '%s\n%s' "$char_key" "$char_rendered" > "$__gpy_char_cache_relay_path" 2>/dev/null
        fi
    fi
    if [[ -n "$char_rendered" ]]; then
        p+="$char_rendered"
    else
        p+="%F{$__prompt_color}${__icon_prompt}%f "
    fi

    print -r -- "$p"
}

add-zsh-hook precmd __gpy_precmd
