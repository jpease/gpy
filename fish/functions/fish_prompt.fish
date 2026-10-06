# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# MAIN PROMPT FUNCTION
# Part of GPY (Guppy Prompt, Yay!) - https://github.com/jpease/gpy
# ============================================================================

function fish_prompt
    # Capture the exit status immediately; expose globally so segments can read it
    set -l last_status $status
    set -g __gpy_last_status $last_status

    # Reset the powerline prev_bg tracker for this render pass.
    # Start at "black" so the first segment's opening chevron blends into a dark
    # terminal background (fg:black on segment-bg = invisible left edge).
    set -g __gpy_last_segment_bg black

    # Clear the per-render memoized git-root lookup (#342, core/ipc.fish) BEFORE
    # any segment runs. This MUST happen at the very top of every render: the
    # memo is only valid within a single render pass, and clearing it here
    # (rather than caching across renders) is what guarantees a `cd` into or
    # out of a repo between prompts is reflected by the git and language
    # segments on the very next prompt.
    set -e __gpy_git_root_this_render_dir
    set -e __gpy_git_root_this_render

    # Reset the per-render oneshot-fallback marker so a dead daemon gets one
    # fresh oneshot fork this render, not zero forever (#324). The path is a
    # session constant resolved once at source time into
    # __gpy_oneshot_marker_path (core/ipc.fish); reuse it directly and guard
    # the rm with a builtin existence check so the healthy path (marker never
    # created) costs zero forks instead of a cmd-sub + rm every prompt (#342).
    set -q __gpy_oneshot_marker_path[1]; and test -e "$__gpy_oneshot_marker_path"; and rm -f -- "$__gpy_oneshot_marker_path"

    # Clear any clock pre-render from a previous pass so a stale value never leaks
    # into a render where the clock segment doesn't run (#342).
    set -e __gpy_clock_prerendered

    # Snapshot current time once per render. Segments and cache readers reuse this
    # global via $__gpy_prompt_now to avoid repeated `date` forks. This single fork
    # is the known remaining subprocess on the warm prompt path: Fish has no built-in
    # epoch-seconds source, so `date +%s` is unavoidable here (#156, #167).
    #
    # When the clock segment is enabled, fold its `date` call into this same fork
    # (#342): request "<epoch><RS><clock-format>" in one invocation and split on
    # an ASCII Record Separator (0x1E, `\x1e`) -- a byte `date` never emits for
    # any of the clock's tokens (digits, ':', ' ', AM/PM text) -- capped to the
    # first split (`-m1`) as belt-and-suspenders: the epoch is always the first
    # field and is pure digits, so even a hypothetical stray separator later
    # could only affect where the *clock* text is cut, never the epoch. Falls
    # back to a bare epoch fetch when the clock isn't enabled (or its format
    # helper isn't loaded), matching today's behavior exactly.
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

    # Blank line for visual separation between commands. Fish did this
    # unconditionally; it is now theme-controlled (`__gpy_add_newline`,
    # default on) so bash and zsh can match instead of rendering tighter
    # prompts from the same theme.
    if test "$__gpy_add_newline" != 0
        echo
    end

    # Reset segment position tracker
    set -g __gpy_segment_position first

    # Build list of segments to render (for position tracking)
    set -l segments_to_render
    for segment in $__enabled_segments
        if functions -q segment_{$segment}_detect; and segment_{$segment}_detect
            set -a segments_to_render $segment
        end
    end

    # Render all enabled segments that should be shown
    set -l segment_count (count $segments_to_render)
    set -l current_idx 1
    for segment in $segments_to_render
        # Check if this is the last segment
        #
        # `set -l is_last` (with no value) creates an EMPTY LIST, not an
        # empty string -- Fish expands that to zero arguments below, which
        # silently shifts $is_first into the $is_last slot for any
        # first-but-not-last segment (#629). Seed an explicit "" so the
        # variable always expands to exactly one (possibly empty) argument.
        #
        # Convention (#613): "true" or "" -- the same tokens IPC payloads use
        # (`,"is_last":true`) and every segment's render function receives
        # directly, with no per-segment last/first-literal conversion.
        set -l is_last ""
        if test $current_idx -eq $segment_count
            set is_last true
        end

        # Check if this is the first segment (for opening-cap suppression).
        # Position-based, not segment-identity-based: whichever segment ends up
        # first here — clock, duration, or anything else — gets is_first, not
        # just "clock" specifically.
        set -l is_first ""
        if test $current_idx -eq 1
            set is_first true
        end

        # Keep the shell-side renderer's position global (read by
        # gpy_section_start / gpy_section_standalone) in step with the same
        # index, so a shell-rendered segment after an agent-rendered one opens
        # with the start delimiter instead of the first-position cap (#765).
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

    # Final prompt symbol with optional status indicator
    set_color normal
    echo

    # Resolve the prompt character first so the status indicator can react to it.
    # The character is always agent-rendered for non-root (#199): the agent applies
    # the theme template and returns the exit-colored `❯`. Root and agent-unavailable
    # cases fall back to the legacy set_color path.
    # __gpy_is_root is cached at init time to avoid a per-prompt `id -u` fork.
    set -l char_output ""
    set -l char_agent_rendered 0
    if test "$__gpy_is_root" -ne 1
        set -l char_success 0
        test $last_status -eq 0; and set char_success 1
        # The character's opening chevron uses fg:prev_bg (default theme), so pass
        # the background left behind by the last rendered segment.
        #
        # Memoized (#343): skip the IPC round-trip + fork entirely when the input
        # tuple (theme identity + success + prev_bg) matches the last render.
        # __gpy_theme_name is agent-controlled (read-only here); folding it into
        # the key means a theme switch invalidates a stale entry via the key
        # alone, even if a reload doorbell got missed mid-reconnect (belt-and-suspenders
        # -- the agent reload path in core/ipc.fish is the primary invalidation path).
        set -l char_key "$__gpy_theme_name:$char_success:$__gpy_last_segment_bg"
        if test -n "$__gpy_char_cache_key"; and test "$char_key" = "$__gpy_char_cache_key"
            set char_output $__gpy_char_cache_val
        else if functions -q __gpy_request_character
            # __gpy_request_character is defined by core/ipc.fish, which is
            # never sourced in GPY's disabled-footprint mode (#452). Guard the
            # call so char_output just stays empty there and falls through to
            # the legacy fallback below, the same as any other agent-down case.
            set char_output (__gpy_request_character $char_success true $__gpy_last_segment_bg)
            # Never cache an empty/failed render: the agent-down fallback must
            # retry on the very next prompt, not get stuck forever.
            if test -n "$char_output"
                set -g __gpy_char_cache_key $char_key
                set -g __gpy_char_cache_val $char_output
            end
        end
        test -n "$char_output"; and set char_agent_rendered 1
    end

    # Show status indicator if GPY_SHOW_STATUS is enabled (default: 1).
    # Skip it when the character is agent-rendered: the exit-colored `❯` already
    # conveys success/failure, so a separate indicator would be redundant
    # (Starship-style presets).
    set -q GPY_SHOW_STATUS; or set -g GPY_SHOW_STATUS 1
    if test "$GPY_SHOW_STATUS" = 1; and test "$char_agent_rendered" -ne 1
        if test $last_status -eq 0
            set_color green
            printf "%s " "$__icon_status_ok"
        else
            set_color red
            printf "%s " "$__icon_status_fail"
        end
        set_color normal
    end

    # Emit the prompt symbol.
    if test "$__gpy_is_root" -eq 1
        # Root prompt: always use the legacy set_color path unchanged.
        set_color "$__root_prompt_color"
        printf "%s " "$__icon_root_prompt"
    else if test "$char_agent_rendered" -eq 1
        printf '%s' $char_output
    else
        # Fallback when the agent is unavailable or the theme produces no output.
        set_color "$__prompt_color"
        printf "%s " "$__icon_prompt"
    end
    set_color normal
end
