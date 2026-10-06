# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# HOSTNAME SEGMENT
# ============================================================================
# Opt-in, SSH-only by default (#257). This segment is intentionally NOT added
# to any theme's default __enabled_segments — a theme or user must opt in
# explicitly.
#
# Dual-path rendering: when the theme leaves __hostname_format empty,
# the segment renders entirely in pure Fish (zero forks, zero IPC) using
# $hostname, a builtin read-only variable fish populates once at shell start.
# When a theme sets __hostname_format, rendering is delegated to the agent so
# it can apply a Starship-compatible template.
#
# On both paths the theme icon is Starship's ssh_symbol: drawn only in SSH
# sessions (#826). The agent path forwards $__gpy_is_ssh so the agent can
# gate $symbol; the pure-fish path gates __icon_hostname itself.

function __gpy_hostname_trim --argument-names value delim
    test -z "$delim"; and echo -- $value; and return
    echo -- (string split -m1 -- $delim $value)[1]
end

function __gpy_hostname_should_show --argument-names is_ssh show_always
    test "$is_ssh" -eq 1; or test "$show_always" -eq 1
end

function segment_hostname_detect
    __gpy_hostname_should_show $__gpy_is_ssh $__hostname_show_always
end

function segment_hostname_render --argument-names is_last
    if test -n "$__hostname_format"
        # Agent-resolved path: a theme set a Starship-compatible format.
        # __hostname_format's CONTENT is presence-only — the agent re-derives
        # the real format server-side, so the branch below fires purely
        # because the variable is non-empty; its literal content is never
        # read or interpreted here.
        #
        # is_last arrives already as "true"/"" (#613: fish_prompt.fish's
        # dispatch-loop convention, matching __gpy_request_hostname's own
        # "true"/"" contract) -- no last/first-literal conversion needed, but
        # still normalize into an always-one-token local before forwarding: a
        # caller that passes zero arguments (or otherwise fails to supply
        # is_last) leaves the --argument-names binding an EMPTY LIST, not an
        # empty string, which would collapse on unquoted expansion, shifting
        # $__gpy_last_segment_bg into the is_last slot and dropping prev_bg.
        set -l is_last_value ""
        test "$is_last" = true; and set is_last_value true
        set -l result (__gpy_request_hostname $hostname $is_last_value "$__gpy_last_segment_bg" "$__gpy_is_ssh")
        if test -n "$result"
            printf '%s' $result
            # Track this segment's bg for the next segment's powerline
            # chevron, even on the agent-resolved path — but only when a
            # pill actually rendered. An empty/failed response emits
            # nothing, so the tracker must keep pointing at whatever
            # segment last actually rendered.
            set -g __gpy_last_segment_bg $__color_hostname_bg
        end
        return
    end

    # Pure-fish path: zero forks, zero IPC.
    set -l name (__gpy_hostname_trim $hostname $__hostname_trim_at)
    set -l label "$name"
    test "$__gpy_is_ssh" = 1; and test -n "$__icon_hostname"; and set label "$__icon_hostname $name"
    gpy_section_standalone $__color_hostname_bg $__color_hostname_fg "$label" $is_last
    set -g __gpy_last_segment_bg $__color_hostname_bg
end
