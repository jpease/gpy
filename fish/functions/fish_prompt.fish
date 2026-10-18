# SPDX-License-Identifier: GPL-3.0-or-later

function fish_prompt
    set -l last_status $status

    # Fish 4.2+ --final-rendering stays in scrollback; runs before the resets (shares the char memo).
    if contains -- --final-rendering $argv; and test "$GPY_UI_TRANSIENT_PROMPT" != 0
        __gpy_transient_prompt $last_status
        return
    end
    __gpy_sync_fish_transient_prompt

    set -g __gpy_last_status $last_status

    # "black" blends the first chevron into a dark terminal.
    set -g __gpy_last_segment_bg black

    # Git-root memo is valid for one render only, so a `cd` shows on the next prompt.
    set -e __gpy_git_root_this_render_dir
    set -e __gpy_git_root_this_render

    # One oneshot fork per render for a dead daemon; erasing is a builtin.
    set -e __gpy_oneshot_used

    # Never leak a stale clock pre-render.
    set -e __gpy_clock_prerendered

    # One `date` fork per render (Fish has no builtin epoch); the clock text rides along,
    # split on 0x1E, a byte `date` never emits.
    if contains clock $__enabled_segments; and functions -q __gpy_clock_date_format
        set -l clock_format (__gpy_clock_date_format)
        set -l sep \x1e
        set -l combined (date "+%s$sep$clock_format" 2>/dev/null)
        if test -n "$combined"
            set -l parts (string split -m1 -- $sep $combined)
            set -g __gpy_prompt_now $parts[1]
            if test (count $parts) -ge 2
                set -g __gpy_clock_prerendered (string trim -- $parts[2])
            end
        else
            set -g __gpy_prompt_now
        end
    else
        set -g __gpy_prompt_now (date +%s 2>/dev/null)
    end

    # Theme-controlled so bash/zsh match.
    if test "$__gpy_add_newline" != 0
        echo
    end

    set -g __gpy_segment_position first

    # Positions are baked into agent ANSI before rendering, so drop segments that will print nothing.
    set -l segments_to_render
    for segment in $__enabled_segments
        if functions -q segment_{$segment}_detect; and segment_{$segment}_detect
            if functions -q segment_{$segment}_omit; and segment_{$segment}_omit
                continue
            end
            set -a segments_to_render $segment
        end
    end

    set -l segment_count (count $segments_to_render)
    set -l current_idx 1
    for segment in $segments_to_render
        # Seed "": `set -l is_last` alone is an empty list, shifting $is_first into its slot.
        set -l is_last ""
        if test $current_idx -eq $segment_count
            set is_last true
        end

        # Position-based, not segment identity.
        set -l is_first ""
        if test $current_idx -eq 1
            set is_first true
        end

        # Keep the shell renderer's position in step for shell-rendered segments.
        if test $current_idx -eq 1
            set -g __gpy_segment_position first
        else
            set -g __gpy_segment_position middle
        end

        if functions -q segment_{$segment}_render
            segment_{$segment}_render $is_last $is_first
        end

        set current_idx (math $current_idx + 1)
    end

    set_color normal
    # Theme option `ui.two_line`, shared with Bash and Zsh.
    if test "$__gpy_two_line" = 1
        echo
    end

    __gpy_resolve_prompt_character $last_status

    # Skip when the character is agent-rendered: its exit colour already shows status.
    set -q GPY_SHOW_STATUS; or set -g GPY_SHOW_STATUS 1
    if test "$GPY_SHOW_STATUS" = 1; and test "$__gpy_char_agent_rendered" -ne 1
        if test $last_status -eq 0
            set_color green
            printf "%s " "$__icon_status_ok"
        else
            set_color red
            printf "%s " "$__icon_status_fail"
        end
        set_color normal
    end

    __gpy_print_prompt_symbol
    set_color normal
end

# Resolve __gpy_char_output/__gpy_char_agent_rendered, memoized on (theme, success, prev_bg).
function __gpy_resolve_prompt_character --argument-names last_status
    set -g __gpy_char_output ""
    set -g __gpy_char_agent_rendered 0
    if test "$__gpy_is_root" -ne 1
        set -l char_success 0
        test $last_status -eq 0; and set char_success 1
        # Theme name in the key invalidates on theme switch even if a reload doorbell was missed.
        set -l char_key "$__gpy_theme_name:$char_success:$__gpy_last_segment_bg"
        if test -n "$__gpy_char_cache_key"; and test "$char_key" = "$__gpy_char_cache_key"
            set -g __gpy_char_output $__gpy_char_cache_val
        else if functions -q __gpy_request_character
            # ipc.fish is not sourced in disabled mode.
            set -g __gpy_char_output (__gpy_request_character $char_success true $__gpy_last_segment_bg | string collect)
            # Never cache a failed render: the agent-down fallback must retry next prompt.
            if test -n "$__gpy_char_output"
                set -g __gpy_char_cache_key $char_key
                set -g __gpy_char_cache_val $__gpy_char_output
            end
        end
        test -n "$__gpy_char_output"; and set -g __gpy_char_agent_rendered 1
    end
end

# Needs __gpy_resolve_prompt_character first.
function __gpy_print_prompt_symbol
    if test "$__gpy_is_root" -eq 1
        set_color "$__root_prompt_color"
        printf "%s " "$__icon_root_prompt"
    else if test "$__gpy_char_agent_rendered" -eq 1
        printf '%s' "$__gpy_char_output"
    else
        set_color "$__prompt_color"
        printf "%s " "$__icon_prompt"
    end
    set_color normal
end

# Transient line: clock text without the pill, then the prompt symbol.
function __gpy_transient_prompt --argument-names last_status
    if test "$__gpy_add_newline" != 0
        echo
    end
    __gpy_resolve_prompt_character $last_status
    if contains clock $__enabled_segments; and functions -q __gpy_clock_date_format
        set -l now (string trim -- (date "+"(__gpy_clock_date_format) 2>/dev/null))
        set -q __prompt_time_pad; or set -g __prompt_time_pad " "
        if test -n "$__color_clock_fg"
            set_color $__color_clock_fg
        end
        printf '%s%s' "$__prompt_time_pad" "$now"
        set_color normal
        # Add a separator unless the character already opens with a space.
        set -l symbol_text ""
        if test "$__gpy_char_agent_rendered" -eq 1; and test "$__gpy_is_root" -ne 1
            set symbol_text (string replace -ra '\e\[[0-9;]*m' '' -- "$__gpy_char_output" | string collect)
        end
        string match -qr '^ ' -- "$symbol_text"; or printf ' '
    end
    __gpy_print_prompt_symbol
    set_color normal
end

# Enable Fish's fish_transient_prompt (4.2+) when needed; only erase a value GPY set.
function __gpy_sync_fish_transient_prompt
    test "$__gpy_fish_transient_capable" = 1; or return 0
    if test "$GPY_UI_TRANSIENT_PROMPT" != 0; or contains clock $__enabled_segments
        if not set -q __gpy_owns_fish_transient_prompt
            set -g fish_transient_prompt 1
            set -g __gpy_owns_fish_transient_prompt 1
        end
    else if set -q __gpy_owns_fish_transient_prompt
        set -e -g fish_transient_prompt
        set -e -g __gpy_owns_fish_transient_prompt
    end
end
