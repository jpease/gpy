# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# USERNAME SEGMENT
# ============================================================================
# Starship `username`-module parity (#252): shows the effective username (e.g.
# `root`) as a styled prefix when running as root or under sudo, hidden
# otherwise. Opt-in — intentionally NOT added to any theme's default
# __enabled_segments; a theme or user must opt in explicitly.
#
# Dual-path rendering (mirrors hostname.fish): when the theme leaves
# __username_format empty, the segment renders entirely in pure Fish (zero
# forks, zero IPC) as a pill using $USER + the exported colors. When a theme
# sets __username_format, rendering is delegated to the agent so it can apply a
# Starship-compatible template (e.g. bold red).

function __gpy_username_should_show --argument-names is_root is_sudo show_always
    test "$show_always" -eq 1; or test "$is_root" -eq 1; or test "$is_sudo" -eq 1
end

function segment_username_detect
    __gpy_username_should_show $__gpy_is_root $__gpy_is_sudo $__username_show_always
end

function segment_username_render --argument-names is_last
    if test -n "$__username_format"
        # Agent-resolved path: a theme set a Starship-compatible format.
        # __username_format's CONTENT is presence-only — the agent re-derives
        # the real format server-side, so the branch fires purely because the
        # variable is non-empty; its literal content is never read here.
        #
        # is_last arrives already as "true"/"" (#613: fish_prompt.fish's
        # dispatch-loop convention, matching __gpy_request_username's own
        # "true"/"" contract) -- no last/first-literal conversion needed, but
        # still normalize into an always-one-token local before forwarding: a
        # caller that passes zero arguments (or otherwise fails to supply
        # is_last) leaves the --argument-names binding an EMPTY LIST, not an
        # empty string, which would collapse on unquoted expansion, shifting
        # $__gpy_last_segment_bg into the is_last slot and dropping prev_bg.
        set -l is_last_value ""
        test "$is_last" = true; and set is_last_value true
        set -l result (__gpy_request_username $USER $is_last_value $__gpy_last_segment_bg)
        if test -n "$result"
            printf '%s' $result
            # Track this segment's bg for the next segment's powerline
            # chevron — but only when a pill actually rendered. An
            # empty/failed response emits nothing, so the tracker must keep
            # pointing at whatever segment last actually rendered.
            set -g __gpy_last_segment_bg $__color_username_bg
        end
        return
    end

    # Pure-fish path: zero forks, zero IPC.
    set -l label "$USER"
    test -n "$__icon_username"; and set label "$__icon_username $USER"
    gpy_section_standalone $__color_username_bg $__color_username_fg "$label" $is_last
    set -g __gpy_last_segment_bg $__color_username_bg
end
