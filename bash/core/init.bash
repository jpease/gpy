# bash/core/init.bash
# Initialization and prompt rendering

# Detect Bash version capabilities
__gpy_bash_major="${BASH_VERSINFO[0]}"
__gpy_bash_minor="${BASH_VERSINFO[1]}"

# Fork-reduction capability flags (#341). The warm-prompt instant-cache read
# path (bash/core/ipc.bash) runs on every prompt; each capability it can't
# satisfy with a builtin costs a fork. Probe bash's version once per shell
# here so hot call sites pick the fork-free path behind a cheap variable
# check instead of re-deriving it every render.
#
# `printf -v var '%(%s)T' -1` expands to the current epoch seconds with no
# `date` fork, but needs bash 4.2+ (the strftime-style printf conversion) --
# earlier bash raises "not a valid time format specification", so callers
# MUST gate on this flag rather than assume the builtin is available.
__gpy_have_printf_epoch=0
if [[ $__gpy_bash_major -gt 4 || ($__gpy_bash_major -eq 4 && $__gpy_bash_minor -ge 2) ]]; then
    __gpy_have_printf_epoch=1
fi

# Duration tracking capability detection
if [[ $__gpy_bash_major -ge 5 ]]; then
    # Bash 5.0+: Use EPOCHREALTIME (microsecond precision)
    __gpy_duration_method="epochrealtime"
elif [[ $__gpy_bash_major -eq 4 ]]; then
    # Bash 4.x: Use date fallback (10-20ms overhead)
    __gpy_duration_method="date"
else
    # Bash 3.x: Disable duration tracking
    __gpy_duration_method="none"
fi

# Duration tracking state
__gpy_cmd_start_time=""
__gpy_cmd_duration=""
__gpy_registered=""
__gpy_last_workspace=""

# Character/directory render memoization (#343): keyed by the full input
# tuple + theme identity, cleared on reload (bash/core/signals.bash). A cache
# hit must cost zero IPC and zero forks; only a non-empty render is ever
# cached (see __gpy_render_prompt / __gpy_directory_segment_output below).
__gpy_char_cache_key=""
__gpy_char_cache_val=""
__gpy_dir_cache_key=""
__gpy_dir_cache_val=""

# Cache SSH-session detection once at source time — pure env-var checks, no fork.
__gpy_is_ssh=0
if [[ -n "${SSH_CONNECTION:-}" || -n "${SSH_CLIENT:-}" || -n "${SSH_TTY:-}" ]]; then
    __gpy_is_ssh=1
fi

# Cache root check once at source time — one `id -u` fork, never per-prompt.
# Mirrors fish's __gpy_is_root; feeds the username segment's visibility gate (#252).
__gpy_is_root=0
if [[ "$(id -u)" -eq 0 ]]; then
    __gpy_is_root=1
fi

# Cache sudo-session detection once — pure env-var check, no fork. sudo exports
# SUDO_USER into the elevated shell; a non-empty value means this shell was
# entered via sudo.
__gpy_is_sudo=0
if [[ -n "${SUDO_USER:-}" ]]; then
    __gpy_is_sudo=1
fi

# Load theme from agent (TOML-based configuration).
#
# Sources the agent's cached theme-export.bash file when present (written by
# write_theme_export_to_dir, gpy-agent/src/cache/theme_export.rs, on startup
# and every config/theme change -- no call-site change needed there) instead
# of forking `gpy-agent theme export` on every shell start and config
# reload (#614). Falls back to the fork only when the cache file is absent
# (e.g. a `gpy-agent` older than this cache, or a first run before it has
# ever written one). Mirrors Fish's __gpy_apply_theme_export
# (fish/core/init.fish) / __gpy_theme_export_cache_path (fish/core/util.fish).
# The reload path (__gpy_reload_config, signals.bash) calls this same function, so it
# picks up the cache too.
__gpy_load_theme() {
    local cache_path
    cache_path="$(__gpy_theme_export_cache_path)"
    if [[ -n "$cache_path" && -f "$cache_path" ]]; then
        # Generated at runtime by `gpy-agent theme export`; not in the tree.
        # shellcheck source=/dev/null
        source "$cache_path"
        return $?
    fi

    if command -v gpy-agent &>/dev/null; then
        # Source the theme export output
        eval "$(gpy-agent theme export --format bash 2>/dev/null)"
        return $?
    else
        return 1
    fi
}

# Record command start time. Called once per command line, from the first
# DEBUG-trap firing after __gpy_arm_preexec (#684).
__gpy_preexec() {
    if [[ $__gpy_duration_method == "epochrealtime" ]]; then
        __gpy_cmd_start_time="$EPOCHREALTIME"
    elif [[ $__gpy_duration_method == "date" ]]; then
        __gpy_cmd_start_time=$(date +%s%N)
    fi
}

# Duration start-time capture is armed once per command line, the
# bash-preexec technique (#684). __gpy_arm_preexec runs last in
# PROMPT_COMMAND, so the first DEBUG-trap firing after it is the first simple
# command of the line the user typed; __gpy_debug_trap records the start time
# there and disarms. Without arming, the start time was overwritten before
# every simple command: another PROMPT_COMMAND entry reset it right before
# __gpy_precmd measured (about 0 ms), and a list or loop counted only its
# last command. Preserves `$?` for anything that runs after it.
__gpy_preexec_armed=""
__gpy_arm_preexec() {
    local s=$?
    __gpy_preexec_armed=1
    return $s
}

# PROMPT_COMMAND runs as an array of commands on bash >= 5.1 (#684).
__gpy_prompt_command_arrays=""
if (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) )); then
    __gpy_prompt_command_arrays=1
fi

# Make __gpy_arm_preexec the last PROMPT_COMMAND entry, adding it if missing
# and moving it if a framework appended after it (else the start time would
# be taken before the prompt is drawn, counting idle time). String form:
# `...; __gpy_arm_preexec`; array form: its own last element. Idempotent and
# fork-free; __gpy_precmd calls it every prompt, so a move takes effect from
# the next prompt.
__gpy_keep_arm_hook_last() {
    local hook="__gpy_arm_preexec"
    if [[ -n "$__gpy_prompt_command_arrays" && "${PROMPT_COMMAND@a}" == *a* ]]; then
        [[ "${PROMPT_COMMAND[-1]}" == "$hook" ]] && return 0
        local -a kept=()
        local entry
        for entry in "${PROMPT_COMMAND[@]}"; do
            [[ "$entry" == "$hook" ]] || kept+=("$entry")
        done
        PROMPT_COMMAND=("${kept[@]}" "$hook")
        return 0
    fi
    local pc="${PROMPT_COMMAND:-}"
    while [[ "$pc" == *[\;[:space:]] ]]; do
        pc="${pc%?}"
    done
    [[ "$pc" == "$hook" || "$pc" == *[\;[:space:]]"$hook" ]] && return 0
    pc="${pc//; $hook/}"
    while [[ "$pc" == *[\;[:space:]] ]]; do
        pc="${pc%?}"
    done
    # String form only: the array form returned above (#684).
    # shellcheck disable=SC2178
    PROMPT_COMMAND="${pc:+$pc; }$hook"
}

# Millisecond delta between two EPOCHREALTIME-format strings ("SECONDS.microseconds",
# always 6 fractional digits) (#341). Assigns the result into the *named*
# variable OUT_VAR via indirect `printf -v` assignment (bash 3.1+, no
# nameref/`declare -n` needed) instead of `echo`+command-substitution, so the
# hot path (__gpy_precmd, every prompt) never forks a subshell.
#
# Two format gotchas fixed here, both confirmed against real bash 5 output
# under LC_NUMERIC=de_DE.UTF-8 (comma decimal separator):
#   - The decimal separator is locale-dependent ("." or ","), so split on a
#     character class instead of a literal ".".
#   - Either half can have leading zeros (e.g. fractional "058901"). Bash
#     treats a leading-zero numeric literal as octal and ERRORS on digits 8/9
#     -- every operand MUST be forced to base 10 with `10#`.
#
# Returns 1 (no assignment) if either fractional part isn't exactly 6 digits,
# so the caller can fall back to the awk-based computation instead of trusting
# arithmetic on a value that doesn't match EPOCHREALTIME's documented format.
__gpy_epoch_diff_ms() {
    local start="$1" end="$2" out_var="$3"
    local start_int="${start%%[.,]*}" start_frac="${start##*[.,]}"
    local end_int="${end%%[.,]*}" end_frac="${end##*[.,]}"

    if [[ ${#start_frac} -ne 6 || ${#end_frac} -ne 6 ]]; then
        return 1
    fi

    local start_us=$(( (10#$start_int) * 1000000 + (10#$start_frac) ))
    local end_us=$(( (10#$end_int) * 1000000 + (10#$end_frac) ))

    printf -v "$out_var" '%d' "$(( (end_us - start_us) / 1000 ))"
}

# Calculate command duration (called after each command)
__gpy_precmd() {
    local exit_code=$?

    # Calculate duration if tracking is enabled
    if [[ -n "$__gpy_cmd_start_time" ]]; then
        if [[ $__gpy_duration_method == "epochrealtime" ]]; then
            local end_time="$EPOCHREALTIME"
            # EPOCHREALTIME is in seconds with microsecond precision. Compute
            # in pure bash arithmetic (no fork); fall back to the original
            # awk formula only if the value doesn't match the expected
            # 6-fractional-digit format (defensive -- shouldn't happen).
            if ! __gpy_epoch_diff_ms "$__gpy_cmd_start_time" "$end_time" __gpy_cmd_duration; then
                __gpy_cmd_duration=$(awk "BEGIN {printf \"%.0f\", ($end_time - $__gpy_cmd_start_time) * 1000}")
            fi
        elif [[ $__gpy_duration_method == "date" ]]; then
            local end_time
            end_time=$(date +%s%N)
            # Nanoseconds to milliseconds
            __gpy_cmd_duration=$(( (end_time - __gpy_cmd_start_time) / 1000000 ))
        fi
    fi

    # Act on the agent's doorbell flags (signals.bash, #678) before rendering,
    # so a config reload shows in the prompt about to be drawn.
    __gpy_consume_shell_flags

    # Render prompt (pass exit code for status segment)
    __gpy_render_prompt "$exit_code"

    # Registration used to be attempted exactly once, at init, and never again
    # unless a `cd` happened; a shell that started while its autostarted
    # agent was still coming up (or whose agent restarted later) stayed
    # unregistered -- no agent notifications -- for its whole life (#638). Retry
    # on each prompt until it sticks; it is one `test -S` when nothing
    # listens.
    if [[ -z "$__gpy_registered" ]]; then
        __gpy_register_with_agent &>/dev/null
    fi

    __gpy_sync_workspace

    # Periodic health check: restarts a mid-session-dead agent (#324). Runs
    # after the render so a dead agent's oneshot fallback still serves this
    # prompt; the restart (rate limited, backgrounded) benefits the next one.
    __gpy_supervisor_check

    # Clear start time for next command, and keep the arming hook last in
    # PROMPT_COMMAND in case something was appended after it (#684).
    __gpy_cmd_start_time=""
    if [[ $__gpy_duration_method != "none" ]]; then
        __gpy_keep_arm_hook_last
    fi
}

__gpy_segment_would_render() {
    local segment="$1"
    local segment_fn="__gpy_segment_${segment}"
    local detect_fn="__gpy_segment_${segment}_detect"

    declare -f "$segment_fn" &>/dev/null || return 1
    if declare -f "$detect_fn" &>/dev/null; then
        "$detect_fn"
        return
    fi

    return 0
}

# Directory segment render is memoized (#343). __gpy_render_prompt (below) is
# always called directly, never via `$()` -- but __gpy_segment_directory
# itself IS invoked via command substitution to capture its stdout, which
# forks a subshell; a cache write made inside that subshell would be lost the
# instant it exits. So the cache check/write lives here, in the render loop's
# own process, and __gpy_segment_directory is only invoked (forked) on a miss.
__gpy_directory_segment_output() {
    local is_last="$1"
    local prev_bg="$2"
    local is_first="$3"
    local out_var="$4"

    # is_last/is_first arrive already as "true"/"" (#613: the dispatch-loop
    # convention below), so the cache key just normalizes the empty case to
    # the literal "false" it has always used -- no last/first-literal
    # conversion needed here anymore.
    local is_last_bool="${is_last:-false}"
    local is_first_bool="${is_first:-false}"
    local dir_key="${__gpy_theme_name:-}:${PWD}:${is_last_bool}:${is_first_bool}:${prev_bg}"

    if [[ -n "$__gpy_dir_cache_key" && "$dir_key" == "$__gpy_dir_cache_key" ]]; then
        printf -v "$out_var" '%s' "$__gpy_dir_cache_val"
        return 0
    fi

    local result
    result=$(__gpy_segment_directory "$is_last" "$prev_bg" "$is_first")
    # Propagate __gpy_segment_directory's own exit code (notably
    # GPY_SEG_STATUS_ONESHOT, #614) as this function's return status, so
    # __gpy_render_prompt's `$?` check after calling this directly (not via
    # `$(...)`, so its own `return` is visible) works the same for the
    # directory branch as for every other segment.
    local seg_status=$?
    # Never cache an empty/failed render: the agent-down fallback must retry
    # on the very next prompt, not get stuck serving nothing forever.
    if [[ -n "$result" ]]; then
        __gpy_dir_cache_key="$dir_key"
        __gpy_dir_cache_val="$result"
    fi
    printf -v "$out_var" '%s' "$result"
    return "$seg_status"
}

# Render the prompt
__gpy_render_prompt() {
    local exit_code="${1:-0}"

    # Reset the per-render oneshot-fallback budget so a dead daemon gets one
    # fresh oneshot fork this render, not zero forever (#324). A plain local
    # variable, not a predictable `${TMPDIR:-/tmp}/.gpy_oneshot_used_$$`
    # marker file (#614): segments run inside `$(...)` subshells and can't
    # write back to this scope, so a request wrapper that actually forks
    # oneshot signals it via GPY_SEG_STATUS_ONESHOT, which the loop below
    # reads off `$?` and folds in here -- later segments' subshells inherit
    # this variable's value by fork, so __gpy_oneshot_claim (ipc.bash) sees
    # it without any file I/O.
    local __gpy_oneshot_used=0

    # Build prompt from enabled segments
    # Blank line before the prompt for visual separation between commands
    # (theme-controlled via `__gpy_add_newline`, default on). Fish has always
    # done this unconditionally; bash/zsh had no equivalent, so the same theme
    # rendered visibly tighter here.
    local prompt_output=""
    if [[ "${__gpy_add_newline:-1}" == "1" ]]; then
        prompt_output=$'\n'
    fi
    local segments_to_render=()
    local segment

    # Space-separated on purpose; set by constants.bash, overridden by the
    # theme export.
    # shellcheck disable=SC2154
    for segment in $__enabled_segments; do
        if __gpy_segment_would_render "$segment"; then
            segments_to_render+=("$segment")
        fi
    done

    # Track the previous segment's background across the render pass. Segments run
    # in command-substitution subshells and cannot write back to this scope, so the
    # loop owns the tracker (unlike Fish, where each segment sets it directly).
    # Start at "black" so the first chevron blends into a dark terminal background.
    local prev_bg="black"
    local segment_count="${#segments_to_render[@]}"
    local segment_index=0
    for segment in "${segments_to_render[@]}"; do
        local segment_fn="__gpy_segment_${segment}"

        if declare -f "$segment_fn" &>/dev/null; then
            local segment_output
            if [[ "$segment" == "status" ]]; then
                segment_output=$($segment_fn "$exit_code")
            else
                # Convention (#613): "true" or "" -- the same tokens IPC
                # payloads use (`,"is_last":true`) and every segment receives
                # directly, with no per-segment last/first-literal conversion
                # (mirrors fish_prompt.fish).
                local is_last=""
                if (( segment_index == segment_count - 1 )); then
                    is_last="true"
                fi
                # Position-based, not segment-identity-based: whichever segment
                # ends up first here (clock, duration, or anything else) gets
                # is_first, so it can suppress an opening-cap glyph that makes
                # no sense with nothing rendered before it (mirrors Fish).
                local is_first=""
                if (( segment_index == 0 )); then
                    is_first="true"
                fi
                if [[ "$segment" == "directory" ]]; then
                    # Memoized (#343): checked/written here (not inside
                    # __gpy_segment_directory) so a cache hit never forks --
                    # see __gpy_directory_segment_output above.
                    __gpy_directory_segment_output "$is_last" "$prev_bg" "$is_first" segment_output
                else
                    segment_output=$($segment_fn "$is_last" "$prev_bg" "$is_first")
                fi
            fi

            # A segment whose request fell back to oneshot signals it via
            # GPY_SEG_STATUS_ONESHOT (#614): fold that into the per-render
            # budget so later segments' subshells (which inherit
            # __gpy_oneshot_used by fork) don't fork oneshot again. Works for
            # both call shapes above -- __gpy_directory_segment_output
            # propagates its own `$?` on a direct call, and `$(...)` command
            # substitution likewise sets `$?` from the substituted command.
            local segment_status=$?
            if [[ "$segment_status" -eq "$GPY_SEG_STATUS_ONESHOT" ]]; then
                __gpy_oneshot_used=1
            fi

            if [[ -n "$segment_output" ]]; then
                prompt_output+="$segment_output"
                # Advance prev_bg to this segment's background for the next
                # chevron. __gpy_segment_bg (constants.bash default, overridden
                # by the agent's `theme export` -- #614) is called directly
                # (not via `$(...)`) with an out-var so this never forks a
                # subshell on the hot render path (#342).
                local segment_bg=""
                __gpy_segment_bg "$segment" segment_bg
                [[ -n "$segment_bg" ]] && prev_bg="$segment_bg"
            fi
        fi
        ((segment_index += 1))
    done

    # Two-line layout (opt-in via theme): render the segments on one line and
    # the prompt character on the next. Default themes export 0 → single-line
    # (no regression). Fish is always two-line and ignores this flag.
    if [[ "${__gpy_two_line:-0}" == "1" ]]; then
        prompt_output+=$'\n'
    fi

    # Add final prompt character.
    # The character is always agent-rendered (#199): the agent renders the prompt
    # symbol colored by exit status (Starship-style). When the agent returns
    # nothing (unavailable, or no theme template), fall back to the legacy symbol.
    local char_success=0
    [[ "$exit_code" -eq 0 ]] && char_success=1
    local char_rendered
    # The character's opening chevron uses fg:prev_bg too (default theme), so pass
    # the background left behind by the last rendered segment.
    #
    # Memoized (#343): skip the IPC round-trip + fork entirely when the input
    # tuple (theme identity + success + prev_bg) matches the last render. This
    # write is made directly inside __gpy_render_prompt's own process (never a
    # subshell in bash), so it persists across prompts unlike a write inside a
    # `$()`-invoked segment function.
    local char_key="${__gpy_theme_name:-}:${char_success}:${prev_bg}"
    if [[ -n "$__gpy_char_cache_key" && "$char_key" == "$__gpy_char_cache_key" ]]; then
        char_rendered="$__gpy_char_cache_val"
    else
        char_rendered="$(__gpy_request_character "$char_success" "true" "$prev_bg")"
        # Never cache an empty/failed render: the agent-down fallback must
        # retry on the very next prompt, not get stuck serving nothing forever.
        if [[ -n "$char_rendered" ]]; then
            __gpy_char_cache_key="$char_key"
            __gpy_char_cache_val="$char_rendered"
        fi
    fi
    if [[ -n "$char_rendered" ]]; then
        prompt_output+="$char_rendered"
    elif declare -f __gpy_segment_prompt &>/dev/null; then
        prompt_output+=$(__gpy_segment_prompt "$exit_code")
    else
        # Fallback prompt character
        local color_code
        # Set by constants.bash, overridden by the theme export.
        # shellcheck disable=SC2154
        case "$__prompt_color" in
            red) color_code="31" ;;
            green) color_code="32" ;;
            yellow) color_code="33" ;;
            blue) color_code="34" ;;
            magenta) color_code="35" ;;
            cyan) color_code="36" ;;
            *) color_code="32" ;;
        esac
        # shellcheck disable=SC2154
        prompt_output+="\[\033[${color_code}m\]${__icon_prompt}\[\033[0m\] "
    fi

    # Set PS1. The agent's bash-prompt output escapes data text on the
    # assumption that promptvars is on (#677); with it off, every escaped
    # `\`, `$` and backtick would show its extra backslashes. Re-assert it on
    # every render so a later `shopt -u promptvars` cannot skew the display.
    shopt -s promptvars
    PS1="$prompt_output"
}

__gpy_register_with_agent() {
    local socket_path
    socket_path=$(__gpy_ipc_endpoint)
    [[ -S "$socket_path" ]] || return 1

    local register_json
    register_json="$(__gpy_build_register_payload)"

    if __gpy_send_json "$register_json" &>/dev/null; then
        __gpy_registered=1
        __gpy_last_workspace="$PWD"
        __gpy_track_shell_for_agent_recovery
        return 0
    fi

    return 1
}

# Forget the current registration and register again. Runs from the doorbell
# handler after an agent restart (#638) and is safe to call at any time: a
# missing socket just returns 1 and the next prompt retries.
__gpy_reregister_with_agent() {
    __gpy_registered=""
    __gpy_last_workspace=""
    __gpy_register_with_agent
}

__gpy_build_register_payload() {
    local json_cwd shell_version
    json_cwd="$(__gpy_escape_json "$PWD")"
    shell_version="$(__gpy_escape_json "$BASH_VERSION")"
    echo "{\"op\":\"register\",\"pid\":$$,\"cwd\":\"$json_cwd\",\"shell\":\"bash\",\"shell_version\":\"$shell_version\"}"
}

__gpy_build_workspace_payload() {
    local json_cwd
    json_cwd="$(__gpy_escape_json "$PWD")"
    echo "{\"op\":\"workspace\",\"pid\":$$,\"cwd\":\"$json_cwd\"}"
}

__gpy_sync_workspace() {
    [[ -n "$__gpy_registered" ]] || return 0
    [[ "$__gpy_last_workspace" != "$PWD" ]] || return 0

    local workspace_json response
    workspace_json="$(__gpy_build_workspace_payload)"

    response=$(__gpy_send_json "$workspace_json" 2>/dev/null)
    if [[ $? -ne 0 || "$response" == *"error"* || "$response" == *"not registered"* ]]; then
        __gpy_registered=""
        __gpy_last_workspace=""
        __gpy_register_with_agent
        return
    fi

    __gpy_last_workspace="$PWD"
}

# Any DEBUG trap installed before gpy's (e.g. bash-preexec, a user framework),
# captured by __gpy_setup_hooks and chained from __gpy_debug_trap so gpy never
# silently clobbers it (#320).
: "${__gpy_prev_debug_trap=}"

# One-shot PROMPT_COMMAND entry that captures a prior DEBUG trap. bash hides
# the DEBUG trap from functions AND from files run with `source` (no
# functrace), so neither __gpy_setup_hooks nor gpy.bash's own top level can
# read it. A PROMPT_COMMAND string is evaluated at the shell's top level, where
# `trap -p DEBUG` works, so the install is deferred to the first prompt. $?
# is passed through for the entries that follow.
# shellcheck disable=SC2016
__gpy_debug_oneshot='__gpy_dbg_s=$?; __gpy_dbg_seen="$(trap -p DEBUG)"; __gpy_install_debug_trap "$__gpy_dbg_s"'

# Run once from the first prompt: chain any prior DEBUG trap (never gpy's own,
# which would recurse), install gpy's, and drop the one-shot entry. Sourcing
# gpy.bash from inside a function still sees the trap at the first prompt, so
# plugin-manager loads are covered too.
__gpy_install_debug_trap() {
    if [[ -n "$__gpy_dbg_seen" && "$__gpy_dbg_seen" != *"__gpy_debug_trap"* ]]; then
        __gpy_prev_debug_trap="$(__gpy_trap_body "$__gpy_dbg_seen")"
    fi
    trap '__gpy_debug_trap "$_"' DEBUG
    local x
    if [[ -n "$__gpy_prompt_command_arrays" && "${PROMPT_COMMAND@a}" == *a* ]]; then
        local -a kept=()
        for x in "${PROMPT_COMMAND[@]}"; do
            x="${x//"$__gpy_debug_oneshot; "/}"
            x="${x//"$__gpy_debug_oneshot"/}"
            [[ -n "$x" ]] && kept+=("$x")
        done
        PROMPT_COMMAND=("${kept[@]}")
    else
        x="${PROMPT_COMMAND:-}"
        x="${x//"$__gpy_debug_oneshot; "/}"
        # String form only: the array form was handled above.
        # shellcheck disable=SC2178
        PROMPT_COMMAND="${x//"$__gpy_debug_oneshot"/}"
    fi
    __gpy_keep_arm_hook_last
    return "${1:-0}"
}

# Bash preexec emulation using DEBUG trap. Installed as
# `__gpy_debug_trap "$_"`: bash sets `$_` to the last argument of every
# simple command, the trap's own included, so the trap must end on the
# user's `$_` to leave it unchanged (#682). $1 is that value.
__gpy_debug_trap() {
    # Only execute for interactive commands. COMP_LINE is unset outside
    # completion, so guard with a default -- under `set -u` an unset
    # reference here would error on every command (#320).
    [[ -n "${COMP_LINE:-}" ]] && return  # Skip during completion

    # Once per command line: armed by __gpy_arm_preexec at the end of
    # PROMPT_COMMAND, so later simple commands, other PROMPT_COMMAND entries
    # and trap handlers never move the start time (#684).
    if [[ -n "$__gpy_preexec_armed" ]]; then
        __gpy_preexec_armed=""
        __gpy_preexec
    fi

    # `if/fi` (not `[[ ]] && eval`) so a missing prior trap leaves this
    # function's exit status 0 instead of leaking the failed test's 1 as the
    # DEBUG trap's own status.
    if [[ -n "$__gpy_prev_debug_trap" ]]; then
        # Hand the prior trap the user's `$_`, not this function's.
        : "${1-}"
        eval "$__gpy_prev_debug_trap"
    fi
}

# Setup hooks
__gpy_setup_hooks() {
    # Add precmd to PROMPT_COMMAND. Guarded with a default: PROMPT_COMMAND is
    # unset until a prompt framework assigns it, and an unguarded reference
    # errors under `set -u` (#320).
    if [[ ! "${PROMPT_COMMAND:-}" =~ __gpy_precmd ]]; then
        # On bash >= 5.1 an array PROMPT_COMMAND keeps its other elements;
        # gpy's hook is prepended to element 0, which is what this reads.
        # shellcheck disable=SC2128,SC2178
        PROMPT_COMMAND="__gpy_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
    fi

    # DEBUG trap for preexec emulation, chaining any trap already installed
    # instead of overwriting it (#320). Installed from the first prompt by
    # __gpy_install_debug_trap (see __gpy_debug_oneshot for why), which skips
    # capture when the trap is already gpy's own (re-init in the same shell).
    # The arming hook goes last in PROMPT_COMMAND (#684).
    if [[ $__gpy_duration_method != "none" ]]; then
        # Arm from the very first prompt, so the first command line is timed.
        __gpy_keep_arm_hook_last
        if [[ "${PROMPT_COMMAND:-}" != *"$__gpy_debug_oneshot"* ]]; then
            # shellcheck disable=SC2128,SC2178
            PROMPT_COMMAND="$__gpy_debug_oneshot${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
        fi
    fi
}

# Initialize GPY
__gpy_init() {
    # Check if GPY should be enabled
    if [[ -n "${GPY_AGENT_ENABLED:-}" && "${GPY_AGENT_ENABLED:-}" != "1" ]]; then
        return
    fi

    # Start supervisor if enabled
    if [[ -z "${GPY_AGENT_SUPERVISOR_ENABLED:-}" || "${GPY_AGENT_SUPERVISOR_ENABLED:-}" == "1" ]]; then
        __gpy_supervisor_start &>/dev/null
    fi

    # Register client with agent
    __gpy_register_with_agent &>/dev/null

    # Setup prompt hooks
    __gpy_setup_hooks
}

# Run initialization
__gpy_init
