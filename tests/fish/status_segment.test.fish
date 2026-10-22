#!/usr/bin/env fish
# Test: status segment (fish/segments/status.fish) — powerline chevron
# tracking. `__gpy_last_segment_bg` must reflect the background the status
# pill actually rendered (ok vs fail branch, each with its own color and its
# own literal fallback) so the NEXT segment's opening chevron blends
# correctly.
#
# Sources the segment file directly and stubs gpy_section_standalone, mirroring
# hostname_segment.test.fish / username_segment.test.fish.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/segments/status.fish"

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

function gpy_section_standalone --argument-names bg fg content is_last
    set -g __test_status_bg "$bg"
    set -g __test_status_fg "$fg"
    set -g __test_status_content "$content"
end

# --- successful (status 0) path, theme colors configured ---

set -g __gpy_last_status 0
set -g __color_status_ok_bg cyan
set -g __color_status_ok_fg black
set -g __icon_status_ok OK
set -g __gpy_last_segment_bg magenta

segment_status_render last

if test "$__gpy_last_segment_bg" = cyan
    check "ok path: tracks __gpy_last_segment_bg to the configured ok background" pass
else
    check "ok path: tracks __gpy_last_segment_bg to the configured ok background (got: '$__gpy_last_segment_bg')" fail
end

if test "$__gpy_last_segment_bg" = "$__test_status_bg"
    check "ok path: tracker matches the bg actually passed to gpy_section_standalone" pass
else
    check "ok path: tracker matches the bg actually passed to gpy_section_standalone (got: '$__gpy_last_segment_bg' vs '$__test_status_bg')" fail
end

# --- failed (non-zero status) path, theme colors configured ---

set -g __gpy_last_status 1
set -g __color_status_fail_bg magenta
set -g __color_status_fail_fg white
set -g __icon_status_fail FAIL
set -g __gpy_last_segment_bg cyan

segment_status_render last

if test "$__gpy_last_segment_bg" = magenta
    check "fail path: tracks __gpy_last_segment_bg to the configured fail background" pass
else
    check "fail path: tracks __gpy_last_segment_bg to the configured fail background (got: '$__gpy_last_segment_bg')" fail
end

# --- successful path, theme color UNSET: must track the resolved literal
# fallback ("green"), not the raw (empty) __color_status_ok_bg. Setting the
# tracker to an empty string would silently break the next segment's chevron.

set -g __gpy_last_status 0
set -e __color_status_ok_bg
set -e __color_status_ok_fg
set -g __gpy_last_segment_bg sentinel

segment_status_render last

if test "$__gpy_last_segment_bg" = green
    check "ok path (theme color unset): tracks the resolved literal fallback 'green'" pass
else
    check "ok path (theme color unset): tracks the resolved literal fallback 'green' (got: '$__gpy_last_segment_bg')" fail
end

# --- failed path, theme color UNSET: must track the resolved literal
# fallback ("red"), not the raw (empty) __color_status_fail_bg.

set -g __gpy_last_status 1
set -e __color_status_fail_bg
set -e __color_status_fail_fg
set -g __gpy_last_segment_bg sentinel

segment_status_render last

if test "$__gpy_last_segment_bg" = red
    check "fail path (theme color unset): tracks the resolved literal fallback 'red'" pass
else
    check "fail path (theme color unset): tracks the resolved literal fallback 'red' (got: '$__gpy_last_segment_bg')" fail
end

# --- the local pill leads with one space, like every other pill ---

set -g __gpy_last_status 0
set -g __color_status_ok_bg cyan
set -g __color_status_ok_fg black
set -g __icon_status_ok OK
set -g __gpy_last_segment_bg magenta
set -e __status_format
segment_status_render last
if test "$__test_status_content" = " OK"
    check "local ok pill: one leading space before the icon" pass
else
    check "local ok pill: one leading space before the icon (got: '$__test_status_content')" fail
end

set -g __gpy_last_status 1
set -g __color_status_fail_bg magenta
set -g __icon_status_fail FAIL
segment_status_render last
if test "$__test_status_content" = " FAIL"
    check "local fail pill: one leading space before the icon" pass
else
    check "local fail pill: one leading space before the icon (got: '$__test_status_content')" fail
end

# --- agent path: a non-empty __status_format routes to __gpy_request_exit_status ---
#
# The format's CONTENT is presence-only (the agent re-derives it). The stub
# records the positional args so the success/is_last/is_first/prev_bg slots are
# verified: an empty is_last must not shift the rest.

function __gpy_request_exit_status --argument-names success is_last is_first prev_bg
    set -g __test_status_arg_success "$success"
    set -g __test_status_arg_is_last "$is_last"
    set -g __test_status_arg_is_first "$is_first"
    set -g __test_status_arg_prev_bg "$prev_bg"
    printf AGENT_SENTINEL
end

set -g __status_format some-opaque-agent-side-value
set -g __gpy_last_status 0
set -g __color_status_ok_bg cyan
set -g __gpy_last_segment_bg magenta
set -g __test_status_content ""
set -l agent_output (segment_status_render true true | string collect)
if test "$agent_output" = AGENT_SENTINEL
    check "agent path: non-empty __status_format routes to __gpy_request_exit_status" pass
else
    check "agent path: non-empty __status_format routes to __gpy_request_exit_status (got: '$agent_output')" fail
end

if test -z "$__test_status_content"
    check "agent path: does not fall through to the local pill" pass
else
    check "agent path: does not fall through to the local pill" fail
end

# The tracker is set in this shell, not the command-substitution subshell above.
set -g __gpy_last_segment_bg magenta
segment_status_render true true >/dev/null
if test "$__gpy_last_segment_bg" = cyan
    check "agent path: tracks __gpy_last_segment_bg to the ok background" pass
else
    check "agent path: tracks __gpy_last_segment_bg to the ok background (got: '$__gpy_last_segment_bg')" fail
end

if test "$__test_status_arg_success" = 1; and test "$__test_status_arg_is_last" = true; and test "$__test_status_arg_is_first" = true; and test "$__test_status_arg_prev_bg" = magenta
    check "agent path: success/is_last/is_first/prev_bg land in their own slots" pass
else
    check "agent path: success/is_last/is_first/prev_bg land in their own slots (got: '$__test_status_arg_success' '$__test_status_arg_is_last' '$__test_status_arg_is_first' '$__test_status_arg_prev_bg')" fail
end

set -g __gpy_last_status 3
set -g __color_status_fail_bg red
set -g __gpy_last_segment_bg magenta
segment_status_render >/dev/null
if test "$__test_status_arg_success" = 0; and test -z "$__test_status_arg_is_last"; and test -z "$__test_status_arg_is_first"; and test "$__test_status_arg_prev_bg" = magenta; and test "$__gpy_last_segment_bg" = red
    check "agent path (fail, not first/last): success=0, empty flags stay in place, fail bg tracked" pass
else
    check "agent path (fail, not first/last): success=0, empty flags stay in place, fail bg tracked (got: '$__test_status_arg_success' '$__test_status_arg_is_last' '$__test_status_arg_is_first' '$__test_status_arg_prev_bg' / '$__gpy_last_segment_bg')" fail
end

# An empty agent reply (no agent) falls back to the local pill.
function __gpy_request_exit_status --argument-names success is_last is_first prev_bg
end

set -g __gpy_last_status 0
set -g __gpy_last_segment_bg magenta
set -g __test_status_content ""
segment_status_render last >/dev/null
if test "$__test_status_content" = " OK"; and test "$__gpy_last_segment_bg" = cyan
    check "agent path: empty reply falls back to the local pill and tracks its bg" pass
else
    check "agent path: empty reply falls back to the local pill and tracks its bg (got: '$__test_status_content' / '$__gpy_last_segment_bg')" fail
end

functions -e __gpy_request_exit_status
set -g __test_status_content ""
segment_status_render last >/dev/null
if test "$__test_status_content" = " OK"
    check "agent path: without ipc.fish (disabled mode) the local pill is drawn" pass
else
    check "agent path: without ipc.fish (disabled mode) the local pill is drawn (got: '$__test_status_content')" fail
end
set -e __status_format

functions -e gpy_section_standalone

# --- __gpy_request_exit_status (IPC payload construction) ---

source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

function __gpy_ipc_send --argument-names payload timeout_ms
    set -g __test_status_payload "$payload"
    echo RENDERED
end

set -l rendered (__gpy_request_exit_status 1 true true prevbg)
if test "$rendered" = RENDERED
    check "__gpy_request_exit_status: returns the IPC result on success" pass
else
    check "__gpy_request_exit_status: returns the IPC result on success (got: '$rendered')" fail
end

if string match -q '{"op":"exit_status","success":true,"format":"ansi"*' -- "$__test_status_payload"
    check "__gpy_request_exit_status: success=1 emits the exit_status op with success true (not the health-check status op)" pass
else
    check "__gpy_request_exit_status: success=1 emits the exit_status op with success true (got: $__test_status_payload)" fail
end

if string match -q '*"is_last":true,"is_first":true,"prev_bg":"prevbg"}' -- "$__test_status_payload"
    check "__gpy_request_exit_status: is_last, is_first and prev_bg ride in the flags tail" pass
else
    check "__gpy_request_exit_status: is_last, is_first and prev_bg ride in the flags tail (got: $__test_status_payload)" fail
end

__gpy_request_exit_status 0 "" "" "" >/dev/null
if string match -q '*"success":false*' -- "$__test_status_payload"; and not string match -q '*is_last*' -- "$__test_status_payload"; and not string match -q '*is_first*' -- "$__test_status_payload"; and not string match -q '*prev_bg*' -- "$__test_status_payload"
    check "__gpy_request_exit_status: success=0 emits success false and omits empty flags" pass
else
    check "__gpy_request_exit_status: success=0 emits success false and omits empty flags (got: $__test_status_payload)" fail
end

functions -e __gpy_ipc_send
function __gpy_ipc_send --argument-names payload timeout_ms
end

if not __gpy_request_exit_status 1 true "" ""
    check "__gpy_request_exit_status: returns nonzero when IPC is unavailable" pass
else
    check "__gpy_request_exit_status: returns nonzero when IPC is unavailable" fail
end

# An older agent answers the unknown op with JSON: no reply, so the local pill.
function __gpy_ipc_send --argument-names payload timeout_ms
    echo '{"error":"Parse error: IPC communication failed: Unknown shell operation: exit_status"}'
end

set -l error_reply (__gpy_request_exit_status 1 true "" "" | string collect)
if not __gpy_request_exit_status 1 true "" "" >/dev/null; and test -z "$error_reply"
    check "__gpy_request_exit_status: an error reply prints nothing and returns nonzero" pass
else
    check "__gpy_request_exit_status: an error reply prints nothing and returns nonzero (got: '$error_reply')" fail
end

function gpy_section_standalone --argument-names bg fg content is_last
    set -g __test_status_content "$content"
    printf LOCAL_PILL
end
set -g __status_format some-opaque-agent-side-value
set -g __gpy_last_status 1
set -g __icon_status_fail FAIL
set -g __test_status_content ""
set -l error_output (segment_status_render last | string collect)
if test "$error_output" = LOCAL_PILL; and test "$__test_status_content" = " FAIL"
    check "agent path: an error reply draws the local pill, not the JSON" pass
else
    check "agent path: an error reply draws the local pill, not the JSON (got: '$error_output' / '$__test_status_content')" fail
end
set -e __status_format
functions -e gpy_section_standalone

functions -e __gpy_ipc_send

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count status segment tests passed"
