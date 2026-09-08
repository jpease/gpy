#!/usr/bin/env fish
# Test: username segment (fish/segments/username.fish) — root/sudo visibility,
# pure-fish render (icon on/off, colour override), and the dual-path branch to
# agent-resolved rendering. Part of #252 (sudo/root prompt prefix segment).
#
# The segment is opt-in (never added to a theme's default __enabled_segments —
# see fish/segments/username.fish), so these tests source the segment file
# directly and drive its functions with stubbed globals rather than exercising
# it through fish_prompt.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/segments/username.fish"

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

# --- __gpy_username_should_show (pure): args are is_root is_sudo show_always ---

if __gpy_username_should_show 1 0 0
    check "should_show: root shows regardless of sudo/show_always" pass
else
    check "should_show: root shows regardless of sudo/show_always" fail
end

if __gpy_username_should_show 0 1 0
    check "should_show: sudo session shows" pass
else
    check "should_show: sudo session shows" fail
end

if not __gpy_username_should_show 0 0 0
    check "should_show: normal user with show_always=0 hides" pass
else
    check "should_show: normal user with show_always=0 hides" fail
end

if __gpy_username_should_show 0 0 1
    check "should_show: normal user with show_always=1 shows" pass
else
    check "should_show: normal user with show_always=1 shows" fail
end

# --- segment_username_detect (edge) ---

set -g __gpy_is_root 1
set -g __gpy_is_sudo 0
set -g __username_show_always 0
if segment_username_detect
    check "detect: root session is shown" pass
else
    check "detect: root session is shown" fail
end

set -g __gpy_is_root 0
set -g __gpy_is_sudo 1
set -g __username_show_always 0
if segment_username_detect
    check "detect: sudo session is shown" pass
else
    check "detect: sudo session is shown" fail
end

set -g __gpy_is_root 0
set -g __gpy_is_sudo 0
set -g __username_show_always 0
if not segment_username_detect
    check "detect: normal user with show_always=0 is hidden" pass
else
    check "detect: normal user with show_always=0 is hidden" fail
end

set -g __gpy_is_root 0
set -g __gpy_is_sudo 0
set -g __username_show_always 1
if segment_username_detect
    check "detect: normal user with show_always=1 is shown" pass
else
    check "detect: normal user with show_always=1 is shown" fail
end

# --- segment_username_render: pure-fish path (zero IPC, zero forks) ---
#
# $USER is a normal (writable) fish variable, so inject a fixture value.
function gpy_section_standalone --argument-names bg fg content is_last
    set -g __test_username_content "$content"
    set -g __test_username_bg "$bg"
    set -g __test_username_fg "$fg"
end

set -g USER root
set -g __username_format ""
set -g __icon_username ""
set -g __color_username_bg red
set -g __color_username_fg white
set -g __gpy_last_segment_bg black

segment_username_render true

if test "$__test_username_content" = root
    check "pure-fish render: renders \$USER and omits the icon when unset" pass
else
    check "pure-fish render: renders \$USER and omits the icon when unset (got: '$__test_username_content')" fail
end

if test "$__test_username_bg" = red; and test "$__test_username_fg" = white
    check "pure-fish render: applies the configured bg/fg colours" pass
else
    check "pure-fish render: applies the configured bg/fg colours (got: '$__test_username_bg'/'$__test_username_fg')" fail
end

if test "$__gpy_last_segment_bg" = red
    check "pure-fish render: tracks __gpy_last_segment_bg for the next segment's chevron" pass
else
    check "pure-fish render: tracks __gpy_last_segment_bg for the next segment's chevron (got: '$__gpy_last_segment_bg')" fail
end

# Colour override: a different theme colour flows straight through.
set -g __color_username_bg yellow
set -g __color_username_fg black
set -g __gpy_last_segment_bg black
segment_username_render true
if test "$__test_username_bg" = yellow; and test "$__test_username_fg" = black
    check "pure-fish render: honours a colour override" pass
else
    check "pure-fish render: honours a colour override (got: '$__test_username_bg'/'$__test_username_fg')" fail
end

set -g __color_username_bg red
set -g __color_username_fg white
set -g __icon_username ICON
set -g __gpy_last_segment_bg black
segment_username_render true

if test "$__test_username_content" = "ICON root"
    check "pure-fish render: includes the icon when __icon_username is set" pass
else
    check "pure-fish render: includes the icon when __icon_username is set (got: '$__test_username_content')" fail
end

set -g __icon_username ""
set -g __gpy_last_segment_bg black
segment_username_render true

if test "$__test_username_content" = root
    check "pure-fish render: omits the icon again once __icon_username is cleared" pass
else
    check "pure-fish render: omits the icon again once __icon_username is cleared (got: '$__test_username_content')" fail
end

# --- segment_username_render: dual-path branch to agent-resolved rendering ---
#
# __username_format's CONTENT is presence-only (the agent re-derives the real
# format server-side) — the branch must fire purely because the var is
# non-empty, regardless of what it contains.
function __gpy_request_username --argument-names user is_last prev_bg
    set -g __test_username_arg_user "$user"
    set -g __test_username_arg_is_last "$is_last"
    set -g __test_username_arg_prev_bg "$prev_bg"
    printf AGENT_SENTINEL
end

set -g __username_format some-opaque-agent-side-value
set -g __test_username_content ""
set -g __gpy_last_segment_bg black
set -l agent_output (segment_username_render true | string collect)

if test "$agent_output" = AGENT_SENTINEL
    check "agent path: non-empty __username_format routes to __gpy_request_username" pass
else
    check "agent path: non-empty __username_format routes to __gpy_request_username (got: '$agent_output')" fail
end

if test -z "$__test_username_content"
    check "agent path: does not fall through to the pure-fish renderer" pass
else
    check "agent path: does not fall through to the pure-fish renderer" fail
end

if test "$__test_username_arg_user" = root
    check "agent path: forwards \$USER to __gpy_request_username" pass
else
    check "agent path: forwards \$USER to __gpy_request_username (got: '$__test_username_arg_user')" fail
end

if test "$__gpy_last_segment_bg" = red
    check "agent path: still tracks __gpy_last_segment_bg for the next segment" pass
else
    check "agent path: still tracks __gpy_last_segment_bg for the next segment (got: '$__gpy_last_segment_bg')" fail
end

# Regression: when username is NOT the last rendered segment, the render is
# called with NO argument (is_last empty). The agent branch must still forward
# prev_bg into the 3rd slot and pass "" (not the bg) as is_last — mirroring the
# hostname segment's fix for the unquoted-empty-is_last collapse.
set -g __username_format some-opaque-agent-side-value
set -g __gpy_last_segment_bg green
set -e __test_username_arg_is_last
set -e __test_username_arg_prev_bg
segment_username_render >/dev/null

if test "$__test_username_arg_prev_bg" = green
    check "agent path (not last): prev_bg reaches __gpy_request_username as the 3rd arg" pass
else
    check "agent path (not last): prev_bg reaches __gpy_request_username as the 3rd arg (got: '$__test_username_arg_prev_bg')" fail
end

if test -z "$__test_username_arg_is_last"
    check "agent path (not last): is_last is forwarded as empty, not shifted" pass
else
    check "agent path (not last): is_last is forwarded as empty, not shifted (got: '$__test_username_arg_is_last')" fail
end

functions -e __gpy_request_username
functions -e gpy_section_standalone
set -e __username_format

# --- __gpy_request_username (edge, IPC payload construction) ---
#
# Source the real IPC helpers and stub __gpy_ipc_send to capture the payload,
# so JSON-escaping and the is_last "true"/"" convention are verified directly
# (#613: username now shares the exact same "true"/"" contract as character,
# hostname, and every other segment -- there is no more distinct convention).
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

function __gpy_ipc_send --argument-names payload timeout_ms
    set -g __test_username_payload "$payload"
    echo RENDERED
end

set -l result_last (__gpy_request_username 'ro"ot' true prevbg)
if test "$result_last" = RENDERED
    check "__gpy_request_username: returns the IPC result on success" pass
else
    check "__gpy_request_username: returns the IPC result on success (got: '$result_last')" fail
end

if string match -q '*"op":"username"*' -- "$__test_username_payload"
    check "__gpy_request_username: emits the username op" pass
else
    check "__gpy_request_username: emits the username op (got: $__test_username_payload)" fail
end

if string match -q '*"is_last":true*' -- "$__test_username_payload"
    check "__gpy_request_username: is_last=true emits \"is_last\":true" pass
else
    check "__gpy_request_username: is_last=true emits \"is_last\":true (got: $__test_username_payload)" fail
end

if string match -q '*"username":"ro\\"ot"*' -- "$__test_username_payload"
    check "__gpy_request_username: JSON-escapes the username argument" pass
else
    check "__gpy_request_username: JSON-escapes the username argument (got: $__test_username_payload)" fail
end

if string match -q '*"prev_bg":"prevbg"*' -- "$__test_username_payload"
    check "__gpy_request_username: includes prev_bg when non-empty" pass
else
    check "__gpy_request_username: includes prev_bg when non-empty (got: $__test_username_payload)" fail
end

__gpy_request_username user2 "" "" >/dev/null
if not string match -q '*is_last*' -- "$__test_username_payload"
    check "__gpy_request_username: is_last='' omits is_last" pass
else
    check "__gpy_request_username: is_last='' omits is_last (got: $__test_username_payload)" fail
end

if not string match -q '*prev_bg*' -- "$__test_username_payload"
    check "__gpy_request_username: omits prev_bg when empty" pass
else
    check "__gpy_request_username: omits prev_bg when empty (got: $__test_username_payload)" fail
end

functions -e __gpy_ipc_send

# No oneshot fallback exists for username — an empty IPC result must propagate
# as failure, not silently synthesize output.
function __gpy_ipc_send --argument-names payload timeout_ms
end

if not __gpy_request_username user3 true ""
    check "__gpy_request_username: returns nonzero when IPC is unavailable (no oneshot fallback)" pass
else
    check "__gpy_request_username: returns nonzero when IPC is unavailable (no oneshot fallback)" fail
end

functions -e __gpy_ipc_send

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count username segment tests passed"
