#!/usr/bin/env fish
# Test: hostname/username agent-resolved render paths must NOT advance
# __gpy_last_segment_bg (the powerline chevron tracker) when the agent
# returns an empty response. An empty/failed optional segment emits nothing,
# so the next segment's opening chevron must still blend against whatever
# segment actually rendered before it — not silently recolor against a
# segment that produced no visible output. Regression for #453.
#
# Both segments are opt-in (never in a theme's default __enabled_segments),
# so this sources each segment file directly and drives it with stubbed
# globals, mirroring hostname_segment.test.fish / username_segment.test.fish.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)

set -g pass_count 0
set -g fail_count 0

function check --argument-names label result
    if test "$result" = pass
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label"
    end
end

# --- hostname: agent path, empty response ---

source "$repo_root/fish/segments/hostname.fish"

function __gpy_request_hostname --argument-names host is_last prev_bg
    # Simulate an agent-down / failed render: no output at all.
end

set -g __hostname_format some-opaque-agent-side-value
set -g __color_hostname_bg blue
set -g __gpy_last_segment_bg unchanged

set -l empty_output (segment_hostname_render last | string collect)

if test -z "$empty_output"
    check "hostname agent path: empty IPC response emits no output" pass
else
    check "hostname agent path: empty IPC response emits no output (got: '$empty_output')" fail
end

if test "$__gpy_last_segment_bg" = unchanged
    check "hostname agent path: empty IPC response preserves the previous __gpy_last_segment_bg" pass
else
    check "hostname agent path: empty IPC response preserves the previous __gpy_last_segment_bg (got: '$__gpy_last_segment_bg')" fail
end

# --- hostname: agent path, non-empty response still updates the tracker ---

function __gpy_request_hostname --argument-names host is_last prev_bg
    printf SENTINEL
end

set -g __gpy_last_segment_bg unchanged
set -l real_output (segment_hostname_render last | string collect)

if test "$real_output" = SENTINEL
    check "hostname agent path: non-empty IPC response is emitted" pass
else
    check "hostname agent path: non-empty IPC response is emitted (got: '$real_output')" fail
end

if test "$__gpy_last_segment_bg" = blue
    check "hostname agent path: non-empty IPC response still updates __gpy_last_segment_bg" pass
else
    check "hostname agent path: non-empty IPC response still updates __gpy_last_segment_bg (got: '$__gpy_last_segment_bg')" fail
end

functions -e __gpy_request_hostname
set -e __hostname_format

# --- username: agent path, empty response ---

source "$repo_root/fish/segments/username.fish"

function __gpy_request_username --argument-names user is_last prev_bg
    # Simulate an agent-down / failed render: no output at all.
end

set -g __username_format some-opaque-agent-side-value
set -g __color_username_bg red
set -g __gpy_last_segment_bg unchanged

set -l empty_output (segment_username_render last | string collect)

if test -z "$empty_output"
    check "username agent path: empty IPC response emits no output" pass
else
    check "username agent path: empty IPC response emits no output (got: '$empty_output')" fail
end

if test "$__gpy_last_segment_bg" = unchanged
    check "username agent path: empty IPC response preserves the previous __gpy_last_segment_bg" pass
else
    check "username agent path: empty IPC response preserves the previous __gpy_last_segment_bg (got: '$__gpy_last_segment_bg')" fail
end

# --- username: agent path, non-empty response still updates the tracker ---

function __gpy_request_username --argument-names user is_last prev_bg
    printf SENTINEL
end

set -g __gpy_last_segment_bg unchanged
set -l real_output (segment_username_render last | string collect)

if test "$real_output" = SENTINEL
    check "username agent path: non-empty IPC response is emitted" pass
else
    check "username agent path: non-empty IPC response is emitted (got: '$real_output')" fail
end

if test "$__gpy_last_segment_bg" = red
    check "username agent path: non-empty IPC response still updates __gpy_last_segment_bg" pass
else
    check "username agent path: non-empty IPC response still updates __gpy_last_segment_bg (got: '$__gpy_last_segment_bg')" fail
end

functions -e __gpy_request_username
set -e __username_format

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count segment bg tracking (empty response) tests passed"
