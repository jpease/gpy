# zsh/core/init.zsh

autoload -Uz add-zsh-hook
zmodload zsh/datetime

typeset -g __gpy_registered=""
typeset -g __gpy_last_workspace=""
# Rejected by the agent; not re-sent until $PWD changes.
typeset -g __gpy_rejected_workspace=""

# Cached once at startup, never per prompt.
typeset -g __gpy_is_ssh=0
if [[ -n "${SSH_CONNECTION:-}" || -n "${SSH_CLIENT:-}" || -n "${SSH_TTY:-}" ]]; then
    __gpy_is_ssh=1
fi

typeset -g __gpy_is_root=0
if [[ "$(id -u)" -eq 0 ]]; then
    __gpy_is_root=1
fi

typeset -g __gpy_is_sudo=0
if [[ -n "${SUDO_USER:-}" ]]; then
    __gpy_is_sudo=1
fi

# Integers: EPOCHREALTIME is a float but the agent's duration_ms is a u64.
typeset -gi __gpy_cmd_start_time=0
typeset -gi __gpy_cmd_duration=0
typeset -g __duration_threshold_ms=100  # milliseconds; overwritten by `gpy theme export`

# Render memoization. __gpy_render_prompt runs in this shell, so the memos
# are plain variables, as in Bash.
typeset -g __gpy_char_cache_key=""
typeset -g __gpy_char_cache_val=""
typeset -g __gpy_dir_cache_key=""
typeset -g __gpy_dir_cache_val=""

# Prefer the agent's cached theme-export file over forking `theme export`.
# `fresh` (gpy-prompt-reload) never reads the cache file, which may be stale:
# it asks the agent, and only a successful export replaces the running state.
# Its stderr is kept, so an invalid config.toml shows the agent's warning.
function __gpy_load_theme() {
    local cache_path out first=""
    if [[ "${1:-}" == fresh ]]; then
        # `eval "$(cmd)"` succeeds even when cmd fails, so fetch first.
        command -v gpy-agent &>/dev/null || return 1
        out="$(gpy-agent theme export --shell zsh)" || return 1
        [[ -n "$out" ]] || return 1
        __gpy_g_clear_styles
        eval "$out"
        return 0
    fi
    __gpy_g_clear_styles
    cache_path=$(__gpy_theme_export_cache_path)
    # Only a file whose first line is the version sentinel is sourced: an older
    # agent's file exports GPY_* values this shell would read as user input.
    [[ -n "$cache_path" && -f "$cache_path" ]] && IFS= read -r first <"$cache_path"
    if [[ "$first" == 'typeset -g __gpy_export_version="1"' ]]; then
        source "$cache_path"
        return $?
    fi

    if command -v gpy-agent &>/dev/null; then
        eval "$(gpy-agent theme export --shell zsh 2>/dev/null)"
        return $?
    else
        return 1
    fi
}

function __gpy_preexec() {
    __gpy_cmd_start_time=$((EPOCHREALTIME * 1000))
    # preexec fires for a command, never for an empty Enter or Ctrl-C:
    # script segments are due at the next prompt.
    __gpy_g_due=1
    __gpy_g_cmd_seen=1
}

add-zsh-hook preexec __gpy_preexec

function __gpy_register_with_agent() {
    __gpy_agent_enabled || return 1

    # A no-op when registered; callers that must re-register clear the flag.
    [[ -z "$__gpy_registered" ]] || return 0

    local request response
    request=$(__gpy_build_register_payload)
    # An {"error":...} reply must leave us unregistered so the next prompt retries.
    response=$(__gpy_send_json "$request") || return 1
    [[ $response == *'"status":"ok"'* ]] || return 1

    __gpy_registered=1
    __gpy_last_workspace="$PWD"
    __gpy_track_shell_for_agent_recovery
    return 0
}

# Called from TRAPURG after an agent restart. The agent may have restarted
# with another config, so its renders are dropped too.
function __gpy_reregister_with_agent() {
    __gpy_registered=""
    __gpy_last_workspace=""
    __gpy_clear_render_memos
    __gpy_register_with_agent
}

function __gpy_build_register_payload() {
    local json_cwd
    __gpy_escape_json_into json_cwd "$PWD"
    local shell_version="$ZSH_VERSION"
    print -r -- "{\"op\":\"register\",\"pid\":$$,\"cwd\":\"$json_cwd\",\"shell\":\"zsh\",\"shell_version\":\"$shell_version\"}"
}

function __gpy_build_workspace_payload() {
    local json_cwd
    __gpy_escape_json_into json_cwd "$PWD"
    print -r -- "{\"op\":\"workspace\",\"pid\":$$,\"cwd\":\"$json_cwd\"}"
}

function __gpy_sync_workspace() {
    __gpy_agent_enabled || return 0
    [[ -n "$__gpy_registered" ]] || return 0
    [[ "$__gpy_rejected_workspace" != "$PWD" ]] || return 0
    __gpy_rejected_workspace=""
    [[ "$__gpy_last_workspace" != "$PWD" ]] || return 0

    local request
    request=$(__gpy_build_workspace_payload)
    local response
    response=$(__gpy_send_json "$request" 2>/dev/null)
    local send_status=$?
    # Other error replies (e.g. rejected /etc) keep the registration.
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

# Positions are baked into agent output before rendering, so a segment sure
# to print nothing (an _omit returning 0) must be rejected here or its
# neighbour gets a dangling separator.
function __gpy_segment_would_render() {
    local seg=$1 tok
    # A script segment shows when its last run left text (decided here, before
    # positions are fixed, so no neighbour keeps a dangling separator).
    if __gpy_g_token "$seg" tok; then
        [[ -n ${__gpy_g_text[$tok]-} ]]
        return
    fi
    if (( ! $+functions[__gpy_segment_$seg] )); then
        return 1
    fi

    local detect_fn="__gpy_segment_${seg}_detect"
    if (( $+functions[$detect_fn] )); then
        "$detect_fn" || return 1
    fi
    local omit_fn="__gpy_segment_${seg}_omit"
    if (( $+functions[$omit_fn] )) && "$omit_fn"; then
        return 1
    fi

    return 0
}

function __gpy_precmd() {
    local ret=$?
    # Drop the dynamic-completion caches (zsh/completions/_gpy-dynamic).
    unset __gpy_complete_cache_theme __gpy_complete_cache_palette __gpy_complete_cache_segment
    # Kept for the signal handlers, which re-render outside precmd.
    __gpy_last_exit_code=$ret

    if [[ $__gpy_cmd_start_time -gt 0 ]]; then
        local end_time=$((EPOCHREALTIME * 1000))
        __gpy_cmd_duration=$(( end_time - __gpy_cmd_start_time ))
        __gpy_cmd_start_time=0
    else
        __gpy_cmd_duration=0
    fi

    # A doorbell ring zsh dropped leaves its flag behind; act on it now.
    __gpy_consume_shell_flags

    if [[ -z "$__gpy_registered" ]] && __gpy_agent_enabled; then
        __gpy_register_with_agent
    fi

    # Script segments: once per command, here in this shell.
    __gpy_g_maybe_run

    __gpy_update_prompts $ret

    # Retry until registered (agent started late or restarted), after the
    # render so a cold start never delays this prompt.
    if [[ -z "$__gpy_registered" ]] && __gpy_agent_enabled; then
        __gpy_register_with_agent &>/dev/null
    fi

    __gpy_sync_workspace

    __gpy_supervisor_check
}

function __gpy_render_prompt() {
    local last_ret=$1

    # Segments read this by fork inheritance.
    typeset -g __gpy_last_status=$last_ret
    local p=""
    [[ "${__gpy_add_newline:-1}" == "1" ]] && p=$'\n'
    local seg
    local -a segments_to_render

    # Per-render oneshot budget. Segment subshells can't write back, so they
    # signal via __gpy_seg_status_oneshot and later subshells inherit this by fork.
    local __gpy_oneshot_used=0

    for seg in $__enabled_segments; do
        if __gpy_segment_would_render "$seg"; then
            segments_to_render+=("$seg")
        fi
    done

    # Segment subshells can't write back, so the loop tracks prev_bg. "black"
    # blends the first chevron into a dark terminal.
    local prev_bg="black"
    local segment_count=${#segments_to_render}
    local segment_index=1
    for seg in $segments_to_render; do
        local g_tok=""
        if __gpy_g_token "$seg" g_tok || (( $+functions[__gpy_segment_$seg] )); then
            local is_last=""
            if (( segment_index == segment_count )); then
                is_last="true"
            fi
            # Position-based, so whichever segment is first can drop its opening cap.
            local is_first=""
            if (( segment_index == 1 )); then
                is_first="true"
            fi
            # A bare `local seg_out` on the second iteration re-declares and
            # prints `seg_out=...` into PROMPT (TYPESET_SILENT is off).
            local seg_out=""
            # Capture `$?` right after each `$(...)`, before any command overwrites it.
            local seg_status=0
            if [[ -n "$g_tok" ]]; then
                seg_out="$(__gpy_g_render "$g_tok" "$is_last" "$prev_bg" "$is_first")"
                seg_status=$?
            elif [[ "$seg" == "directory" ]]; then
                # Memoized in the shell; a miss fills the memo.
                local is_last_bool="${is_last:-false}"
                local is_first_bool="${is_first:-false}"
                local dir_key="${__gpy_theme_name:-}:${PWD}:${is_last_bool}:${is_first_bool}:${prev_bg}"
                if [[ -n "$__gpy_dir_cache_key" && "$dir_key" == "$__gpy_dir_cache_key" ]]; then
                    seg_out="$__gpy_dir_cache_val"
                else
                    seg_out="$(__gpy_segment_$seg "$is_last" "$prev_bg" "$is_first")"
                    seg_status=$?
                    # Never cache an empty render: the agent-down fallback must retry.
                    if [[ -n "$seg_out" ]]; then
                        typeset -g __gpy_dir_cache_key=$dir_key
                        typeset -g __gpy_dir_cache_val=$seg_out
                    fi
                fi
            else
                # Quote all args: an empty unquoted $is_last collapses to zero words.
                seg_out="$(__gpy_segment_$seg "$is_last" "$prev_bg" "$is_first")"
                seg_status=$?
            fi

            if [[ "$seg_status" -eq "$__gpy_seg_status_oneshot" ]]; then
                __gpy_oneshot_used=1
            fi

            p+="$seg_out"
            if [[ -n "$seg_out" ]]; then
                # Out-var call: no fork on the hot path.
                local seg_bg="" seg_fg=""
                if [[ -n "$g_tok" ]]; then
                    __gpy_g_colors "$g_tok" seg_bg seg_fg
                else
                    [[ "$seg" == "git" ]] && typeset -g __gpy_git_seg_status="$seg_status"
                    __gpy_segment_bg "$seg" seg_bg
                fi
                [[ -n "$seg_bg" ]] && prev_bg="$seg_bg"
            fi
        fi
        (( segment_index += 1 ))
    done

    # Two-line layout (theme opt-in).
    if [[ "${__gpy_two_line:-0}" == "1" ]]; then
        p+=$'\n'
    fi

    # Root never asks the agent; it and an empty agent reply fall back to
    # the shell-drawn symbol.
    local char_rendered=""
    if [[ "${__gpy_is_root:-0}" != "1" ]]; then
        local char_success=0
        [[ "$last_ret" -eq 0 ]] && char_success=1
        # The chevron uses fg:prev_bg. Memoized in the shell, as in Bash.
        local char_key="${__gpy_theme_name:-}:${char_success}:${prev_bg}"
        if [[ -n "$__gpy_char_cache_key" && "$char_key" == "$__gpy_char_cache_key" ]]; then
            char_rendered="$__gpy_char_cache_val"
        else
            char_rendered=$(__gpy_request_character "$char_success" "true" "$prev_bg")
            # Never cache an empty render: the agent-down fallback must retry.
            if [[ -n "$char_rendered" ]]; then
                typeset -g __gpy_char_cache_key=$char_key
                typeset -g __gpy_char_cache_val=$char_rendered
            fi
        fi
    fi
    local char_from_agent=""
    [[ -n "$char_rendered" ]] && char_from_agent=1
    if [[ -z "$char_from_agent" ]]; then
        __gpy_prompt_tail "$last_ret" char_rendered
    fi
    p+="$char_rendered"

    # The transient line needs the symbol alone, kept apart from the prompt.
    typeset -g __gpy_render_tail=""
    if [[ "${__gpy_emit_tail:-}" == "1" ]]; then
        if [[ -z "$char_from_agent" ]]; then
            local __gpy_hide_status_tail=1
            __gpy_prompt_tail "$last_ret" char_rendered
        fi
        __gpy_render_tail=$char_rendered
    fi

    typeset -g __gpy_render_out=$p
}

# __gpy_transient_prompt is what zle-line-finish swaps in on Enter so
# scrollback keeps the submit time: transient on → clock time + symbol;
# off with clock → full prompt (zle re-expands `%D{...}`); off without → empty.
function __gpy_update_prompts() {
    local last_ret=${1:-0}
    # Read here, not under `setopt local_options`: zsh expands PROMPT after
    # precmd returns, so a local option would already be reverted. Under
    # prompt_bang a data `!` must be written `!!`; memo values stay raw.
    local bang=0
    [[ -o prompt_bang ]] && bang=1
    local __gpy_emit_tail=1
    __gpy_render_prompt "$last_ret"
    local tail=$__gpy_render_tail
    if (( bang )); then
        PROMPT=${__gpy_render_out//\!/!!}
    else
        PROMPT=$__gpy_render_out
    fi

    local has_clock=0
    (( ${__enabled_segments[(Ie)clock]} )) && has_clock=1

    if [[ "${__gpy_cfg_transient_prompt-}" == "0" ]]; then
        if (( has_clock )); then
            # Already doubled above; copied unchanged.
            __gpy_transient_prompt=$PROMPT
        else
            __gpy_transient_prompt=""
        fi
        return 0
    fi

    local t=""
    [[ "${__gpy_add_newline:-1}" == "1" ]] && t=$'\n'
    if (( has_clock )) && (( $+functions[__gpy_clock_time_spec] )); then
        local spec pad sgr
        __gpy_clock_time_spec spec
        __gpy_prompt_escape "${__prompt_time_pad- }" pad
        __gpy_sgr_pair "${__color_clock_fg:-}" normal sgr
        t+="${sgr}${pad}%D{${spec}}%{${__GPY_SGR_ESC}[0m%}"
        # Themes whose symbol has no leading space (starship) still need one
        # between the time and the symbol.
        local plain=${(S)tail//\%\{*\%\}/}
        [[ "$plain" == " "* ]] || t+=" "
    fi
    t+=$tail
    if (( bang )); then
        __gpy_transient_prompt=${t//\!/!!}
    else
        __gpy_transient_prompt=$t
    fi
}

# CONTEXT=start skips vared and continuation lines.
function __gpy_line_finish() {
    [[ "$CONTEXT" == start && -n "$__gpy_transient_prompt" ]] || return 0
    PROMPT=$__gpy_transient_prompt
    zle .reset-prompt
    zle -R
}

autoload -Uz add-zle-hook-widget
zle -N __gpy_line_finish
add-zle-hook-widget zle-line-finish __gpy_line_finish

add-zsh-hook precmd __gpy_precmd
