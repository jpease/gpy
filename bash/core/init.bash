__gpy_bash_major="${BASH_VERSINFO[0]}"
__gpy_bash_minor="${BASH_VERSINFO[1]}"

# Probed once per shell so hot paths stay fork-free. `printf -v var '%(%s)T' -1`
# needs bash 4.2+; earlier bash errors, so callers MUST gate on this flag.
__gpy_have_printf_epoch=0
if [[ $__gpy_bash_major -gt 4 || ($__gpy_bash_major -eq 4 && $__gpy_bash_minor -ge 2) ]]; then
    __gpy_have_printf_epoch=1
fi

# Duration method into $2: EPOCHREALTIME on 5+; on 4, `date +%s%N` only if
# date is GNU (BSD date prints a literal `N`), else whole seconds; 3: none.
__gpy_select_duration_method() {
    local major="$1" out_var="$2" method
    if [[ $major -ge 5 ]]; then
        method="epochrealtime"
    elif [[ $major -eq 4 ]]; then
        if [[ "$(date +%s%N 2>/dev/null)" =~ ^[0-9]{12,}$ ]]; then
            method="date"
        else
            method="date-seconds"
        fi
    else
        method="none"
    fi
    printf -v "$out_var" '%s' "$method"
}
__gpy_duration_method=""
__gpy_select_duration_method "$__gpy_bash_major" __gpy_duration_method

__gpy_now_seconds() {
    if [[ $__gpy_have_printf_epoch -eq 1 ]]; then
        printf -v "$1" '%(%s)T' -1
    else
        printf -v "$1" '%s' "$(date +%s)"
    fi
}

__gpy_cmd_start_time=""
__gpy_cmd_duration=""
__gpy_registered=""
__gpy_last_workspace=""
# Directory whose workspace sync the agent rejected; not re-sent until $PWD changes
__gpy_rejected_workspace=""

# Character/directory render memo, cleared on reload. Only non-empty renders
# are cached, so a down agent is retried next prompt.
__gpy_char_cache_key=""
__gpy_char_cache_val=""
__gpy_dir_cache_key=""
__gpy_dir_cache_val=""

__gpy_is_ssh=0
if [[ -n "${SSH_CONNECTION:-}" || -n "${SSH_CLIENT:-}" || -n "${SSH_TTY:-}" ]]; then
    __gpy_is_ssh=1
fi

# One `id -u` fork at source time, never per prompt.
__gpy_is_root=0
if [[ "$(id -u)" -eq 0 ]]; then
    __gpy_is_root=1
fi

__gpy_is_sudo=0
if [[ -n "${SUDO_USER:-}" ]]; then
    __gpy_is_sudo=1
fi

# Prefer the agent's cached theme-export file; fork `gpy-agent theme export`
# only when it is absent.
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
        eval "$(gpy-agent theme export --format bash 2>/dev/null)"
        return $?
    else
        return 1
    fi
}

__gpy_preexec() {
    if [[ $__gpy_duration_method == "epochrealtime" ]]; then
        __gpy_cmd_start_time="$EPOCHREALTIME"
    elif [[ $__gpy_duration_method == "date" ]]; then
        __gpy_cmd_start_time=$(date +%s%N)
    elif [[ $__gpy_duration_method == "date-seconds" ]]; then
        __gpy_now_seconds __gpy_cmd_start_time
    fi
}

# Armed last in PROMPT_COMMAND (bash-preexec technique), so the first DEBUG
# firing after it is the user's first command. Without arming, other
# PROMPT_COMMAND entries reset the start time and a loop timed only its last
# command. Preserves `$?`.
__gpy_preexec_armed=""
__gpy_arm_preexec() {
    local s=$?
    __gpy_preexec_armed=1
    return $s
}

# PROMPT_COMMAND runs as an array of commands on bash >= 5.1.
__gpy_prompt_command_arrays=""
if (( BASH_VERSINFO[0] > 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] >= 1) )); then
    __gpy_prompt_command_arrays=1
fi

# Keep __gpy_arm_preexec the last PROMPT_COMMAND entry, else idle time before
# the prompt is counted. Idempotent and fork-free.
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
    # shellcheck disable=SC2178
    PROMPT_COMMAND="${pc:+$pc; }$hook"
}

# Millisecond delta between EPOCHREALTIME strings into OUT_VAR. The decimal
# separator is locale-dependent, and leading-zero fractions parse as octal
# (erroring on 8/9), so every operand is forced with `10#`. Returns 1 if a
# fraction is not 6 digits so the caller can fall back to awk.
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

__gpy_precmd() {
    local exit_code=$?

    if [[ -n "$__gpy_cmd_start_time" ]]; then
        if [[ $__gpy_duration_method == "epochrealtime" ]]; then
            local end_time="$EPOCHREALTIME"
            if ! __gpy_epoch_diff_ms "$__gpy_cmd_start_time" "$end_time" __gpy_cmd_duration; then
                __gpy_cmd_duration=$(awk "BEGIN {printf \"%.0f\", ($end_time - $__gpy_cmd_start_time) * 1000}")
            fi
        elif [[ $__gpy_duration_method == "date" ]]; then
            local end_time
            end_time=$(date +%s%N)
            __gpy_cmd_duration=$(( (end_time - __gpy_cmd_start_time) / 1000000 ))
        elif [[ $__gpy_duration_method == "date-seconds" ]]; then
            local end_time
            __gpy_now_seconds end_time
            __gpy_cmd_duration=$(( (end_time - __gpy_cmd_start_time) * 1000 ))
        fi
    fi

    # Once per directory change, before any render reads the memo.
    __gpy_resolve_pwd

    # Before rendering, so a reload shows in this prompt.
    __gpy_consume_shell_flags

    __gpy_render_prompt "$exit_code"

    # Retry until registered (agent started late or restarted); one `test -S`
    # when nothing listens.
    if [[ -z "$__gpy_registered" ]]; then
        __gpy_register_with_agent &>/dev/null
    fi

    __gpy_sync_workspace

    # After the render, so the oneshot fallback still serves this prompt.
    __gpy_supervisor_check

    __gpy_cmd_start_time=""
    if [[ $__gpy_duration_method != "none" ]]; then
        __gpy_keep_arm_hook_last
    fi
    return "$exit_code"
}

# Positions are baked into agent output before rendering, so a segment certain
# to print nothing (its _omit returns 0) is rejected here; otherwise its
# neighbour gets a dangling separator.
__gpy_segment_would_render() {
    local segment="$1"
    local segment_fn="__gpy_segment_${segment}"
    local detect_fn="__gpy_segment_${segment}_detect"
    local omit_fn="__gpy_segment_${segment}_omit"

    declare -f "$segment_fn" &>/dev/null || return 1
    if declare -f "$detect_fn" &>/dev/null; then
        "$detect_fn" || return 1
    fi
    if declare -f "$omit_fn" &>/dev/null && "$omit_fn"; then
        return 1
    fi

    return 0
}

# The memo lives here, not in __gpy_segment_directory: that runs inside `$()`,
# where a cache write would be lost with the subshell.
__gpy_directory_segment_output() {
    local is_last="$1"
    local prev_bg="$2"
    local is_first="$3"
    local out_var="$4"

    local is_last_bool="${is_last:-false}"
    local is_first_bool="${is_first:-false}"
    local dir_key="${__gpy_theme_name:-}:${PWD}:${is_last_bool}:${is_first_bool}:${prev_bg}"

    if [[ -n "$__gpy_dir_cache_key" && "$dir_key" == "$__gpy_dir_cache_key" ]]; then
        printf -v "$out_var" '%s' "$__gpy_dir_cache_val"
        return 0
    fi

    local result
    result=$(__gpy_segment_directory "$is_last" "$prev_bg" "$is_first")
    # Propagate the segment's exit code (GPY_SEG_STATUS_ONESHOT) to the caller.
    local seg_status=$?
    if [[ -n "$result" ]]; then
        __gpy_dir_cache_key="$dir_key"
        __gpy_dir_cache_val="$result"
    fi
    printf -v "$out_var" '%s' "$result"
    return "$seg_status"
}

__gpy_render_prompt() {
    local exit_code="${1:-0}"

    # Read by the status segment and __gpy_segment_bg via fork inheritance.
    __gpy_last_status="$exit_code"

    # One oneshot fork per render. Segment subshells cannot write back, so
    # they signal a fork via GPY_SEG_STATUS_ONESHOT and later subshells
    # inherit the folded value; no predictable /tmp marker file.
    local __gpy_oneshot_used=0

    local prompt_output=""
    if [[ "${__gpy_add_newline:-1}" == "1" ]]; then
        prompt_output=$'\n'
    fi
    local segments_to_render=()
    local segment

    # Space-separated on purpose.
    # shellcheck disable=SC2154
    for segment in $__enabled_segments; do
        if __gpy_segment_would_render "$segment"; then
            segments_to_render+=("$segment")
        fi
    done

    # Segments run in subshells, so the loop owns the previous-bg tracker.
    # "black" blends the first chevron into a dark terminal.
    local prev_bg="black"
    local segment_count="${#segments_to_render[@]}"
    local segment_index=0
    for segment in "${segments_to_render[@]}"; do
        local segment_fn="__gpy_segment_${segment}"

        if declare -f "$segment_fn" &>/dev/null; then
            local segment_output
            local is_last=""
            if (( segment_index == segment_count - 1 )); then
                is_last="true"
            fi
            local is_first=""
            if (( segment_index == 0 )); then
                is_first="true"
            fi
            if [[ "$segment" == "directory" ]]; then
                __gpy_directory_segment_output "$is_last" "$prev_bg" "$is_first" segment_output
            else
                segment_output=$($segment_fn "$is_last" "$prev_bg" "$is_first")
            fi

            local segment_status=$?
            if [[ "$segment_status" -eq "$GPY_SEG_STATUS_ONESHOT" ]]; then
                __gpy_oneshot_used=1
            fi

            if [[ -n "$segment_output" ]]; then
                prompt_output+="$segment_output"
                local segment_bg=""
                __gpy_segment_bg "$segment" segment_bg
                [[ -n "$segment_bg" ]] && prev_bg="$segment_bg"
            fi
        fi
        ((segment_index += 1))
    done

    if [[ "${__gpy_two_line:-0}" == "1" ]]; then
        prompt_output+=$'\n'
    fi

    # Root never asks the agent; it and an empty agent reply fall back to
    # __gpy_prompt_tail.
    local char_rendered=""
    if [[ "${__gpy_is_root:-0}" != "1" ]]; then
        local char_success=0
        [[ "$exit_code" -eq 0 ]] && char_success=1
        # The character's chevron uses fg:prev_bg. Memoized on theme, success
        # and prev_bg; this scope is never a subshell, so the write persists.
        local char_key="${__gpy_theme_name:-}:${char_success}:${prev_bg}"
        if [[ -n "$__gpy_char_cache_key" && "$char_key" == "$__gpy_char_cache_key" ]]; then
            char_rendered="$__gpy_char_cache_val"
        else
            char_rendered="$(__gpy_request_character "$char_success" "true" "$prev_bg")"
            if [[ -n "$char_rendered" ]]; then
                __gpy_char_cache_key="$char_key"
                __gpy_char_cache_val="$char_rendered"
            fi
        fi
    fi
    if [[ -n "$char_rendered" ]]; then
        prompt_output+="$char_rendered"
    else
        local prompt_tail
        __gpy_prompt_tail "$exit_code" prompt_tail
        prompt_output+="$prompt_tail"
    fi

    # Agent output is escaped assuming promptvars; re-assert it every render.
    shopt -s promptvars
    PS1="$prompt_output"
}

__gpy_register_with_agent() {
    __gpy_agent_enabled || return 1

    # No-op when registered, so cold-miss refresh paths cost no round trip.
    # Callers that must re-register clear __gpy_registered first.
    [[ -z "$__gpy_registered" ]] || return 0

    local socket_path
    socket_path=$(__gpy_ipc_endpoint)
    [[ -S "$socket_path" ]] || return 1

    local register_json
    register_json="$(__gpy_build_register_payload)"

    # Only an explicit "status":"ok" counts; an error reply leaves the shell
    # unregistered so the next prompt retries.
    local response
    response=$(__gpy_send_json "$register_json" 2>/dev/null) || return 1
    [[ $response == *'"status":"ok"'* ]] || return 1

    __gpy_registered=1
    __gpy_last_workspace="$PWD"
    __gpy_track_shell_for_agent_recovery
    return 0
}

# Safe at any time: a missing socket returns 1 and the next prompt retries.
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
    __gpy_agent_enabled || return 0
    [[ -n "$__gpy_registered" ]] || return 0
    [[ "$__gpy_rejected_workspace" != "$PWD" ]] || return 0
    __gpy_rejected_workspace=""
    [[ "$__gpy_last_workspace" != "$PWD" ]] || return 0

    local workspace_json response
    workspace_json="$(__gpy_build_workspace_payload)"

    response=$(__gpy_send_json "$workspace_json" 2>/dev/null)
    local send_status=$?
    # Only a failed send or lost registration re-registers; other errors
    # (a rejected path like /etc) keep it.
    if [[ $send_status -ne 0 || "$response" == *"not registered"* ]]; then
        __gpy_registered=""
        __gpy_last_workspace=""
        __gpy_register_with_agent
        return
    fi
    if [[ "$response" == *'"error"'* ]]; then
        __gpy_rejected_workspace="$PWD"
        return 0
    fi

    __gpy_last_workspace="$PWD"
}

# A DEBUG trap installed before gpy's, chained rather than clobbered.
: "${__gpy_prev_debug_trap=}"

# bash hides the DEBUG trap from functions and sourced files (no functrace),
# so capture is deferred to a PROMPT_COMMAND string, evaluated at top level
# where `trap -p DEBUG` works. Passes `$?` through.
# shellcheck disable=SC2016
__gpy_debug_oneshot='__gpy_dbg_s=$?; __gpy_dbg_seen="$(trap -p DEBUG)"; __gpy_install_debug_trap "$__gpy_dbg_s"'

# Chain any prior DEBUG trap (never gpy's own, which would recurse).
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
        # shellcheck disable=SC2178
        PROMPT_COMMAND="${x//"$__gpy_debug_oneshot"/}"
    fi
    __gpy_keep_arm_hook_last
    return "${1:-0}"
}

# Installed as `__gpy_debug_trap "$_"`: the trap's own command resets `$_`,
# so it must end on the user's `$_` (passed as $1) to leave it unchanged.
__gpy_debug_trap() {
    # COMP_LINE default: unset outside completion, and `set -u` would error.
    [[ -n "${COMP_LINE:-}" ]] && return  # Skip during completion

    if [[ -n "$__gpy_preexec_armed" ]]; then
        __gpy_preexec_armed=""
        __gpy_preexec
    fi

    # `if/fi`, not `[[ ]] && eval`, so a missing prior trap does not leak
    # status 1 as the DEBUG trap's status.
    if [[ -n "$__gpy_prev_debug_trap" ]]; then
        # Hand the prior trap the user's `$_`, not this function's.
        : "${1-}"
        eval "$__gpy_prev_debug_trap"
    fi
}

__gpy_setup_hooks() {
    # Default guard: PROMPT_COMMAND may be unset under `set -u`.
    if [[ ! "${PROMPT_COMMAND:-}" =~ __gpy_precmd ]]; then
        # bash >= 5.1 array form: the hook is prepended to element 0.
        # shellcheck disable=SC2128,SC2178
        PROMPT_COMMAND="__gpy_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
    fi

    # DEBUG trap is installed from the first prompt (see __gpy_debug_oneshot).
    if [[ $__gpy_duration_method != "none" ]]; then
        __gpy_keep_arm_hook_last
        if [[ "${PROMPT_COMMAND:-}" != *"$__gpy_debug_oneshot"* ]]; then
            # shellcheck disable=SC2128,SC2178
            PROMPT_COMMAND="$__gpy_debug_oneshot${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
        fi
    fi
}

__gpy_init() {
    # Agent-free mode still installs the prompt hook; every render uses oneshot.
    if __gpy_agent_enabled; then
        __gpy_supervisor_start &>/dev/null
        __gpy_register_with_agent &>/dev/null
    fi

    __gpy_setup_hooks
}
