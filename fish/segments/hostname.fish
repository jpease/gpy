# SPDX-License-Identifier: GPL-3.0-or-later
# Opt-in, SSH-only by default. The icon is Starship's ssh_symbol: SSH sessions only.

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
        # Format content is presence-only; the agent re-derives it.
        # One-token local: an empty-list binding would shift prev_bg.
        set -l is_last_value ""
        test "$is_last" = true; and set is_last_value true
        set -l result (__gpy_request_hostname $hostname $is_last_value "$__gpy_last_segment_bg" "$__gpy_is_ssh")
        if test -n "$result"
            printf '%s' $result
            # Only track bg when a pill rendered.
            set -g __gpy_last_segment_bg $__color_hostname_bg
        end
        return
    end

    # Unset trim delimiter means `.`; explicit empty means "no trim".
    set -l trim_at .
    set -q __hostname_trim_at; and set trim_at "$__hostname_trim_at"
    set -l name (__gpy_hostname_trim $hostname "$trim_at")
    set -l label "$name"
    test "$__gpy_is_ssh" = 1; and test -n "$__icon_hostname"; and set label "$__icon_hostname $name"
    gpy_section_standalone $__color_hostname_bg $__color_hostname_fg "$label" $is_last
    set -g __gpy_last_segment_bg $__color_hostname_bg
end
