# SPDX-License-Identifier: GPL-3.0-or-later

# Fish updates $CMD_DURATION only when a command runs, so a blank Enter or a
# Ctrl-C at the prompt would show the previous command's duration again.
# fish_postexec is emitted for a command, never for an empty Enter; the
# fish_prompt event is emitted once per new prompt (never for a repaint), so
# a prompt with no command since the last one hides the pill.
function __gpy_duration_on_postexec --on-event fish_postexec
    set -g __gpy_duration_command_ran 1
end

function __gpy_duration_on_prompt --on-event fish_prompt
    if set -q __gpy_duration_command_ran
        set -e __gpy_duration_command_ran
        set -e __gpy_duration_idle
    else
        set -g __gpy_duration_idle 1
    end
end

function segment_duration_detect
    set -q __gpy_duration_idle; and return 1
    set -l threshold (set -q __duration_threshold_ms && echo $__duration_threshold_ms || echo 100)
    test -n "$CMD_DURATION" && test "$CMD_DURATION" -ge $threshold
end

function segment_duration_render --argument-names is_last is_first
    # Seed missing args as "" (see fish_prompt.fish's dispatch loop).
    set -q is_last[1]; or set is_last ""
    set -q is_first[1]; or set is_first ""

    set -l ms $CMD_DURATION
    test -z "$ms"; and set ms 0
    set -l dur_output (__gpy_request_duration "$ms" $is_last $__gpy_last_segment_bg $is_first | string collect)
    printf '%s' "$dur_output"
    if test -n "$dur_output"
        set -g __gpy_last_segment_bg $__color_duration_bg
    end
end
