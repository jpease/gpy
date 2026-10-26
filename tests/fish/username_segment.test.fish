#!/usr/bin/env fish
# Test: username segment (fish/segments/username.fish) — root/sudo/SSH
# visibility, pure-fish render (icon on/off, root vs user colours), and the
# dual-path branch to agent-resolved rendering.
#
# The segment is opt-in (never added to a theme's default __enabled_segments —
# see fish/segments/username.fish), so these tests source the segment file
# directly and drive its functions with stubbed globals rather than exercising
# it through fish_prompt.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/segments/username.fish"

# --- detect without a theme export: show_always unset, nothing on stderr ---

set -e __username_show_always
set -g __gpy_is_root 0
set -g __gpy_is_sudo 0
set -g __gpy_is_ssh 0
set -l __stderr_file (mktemp)
segment_username_detect 2>$__stderr_file
check "detect: an ordinary user with show_always unset hides the segment" 1 $status
check "detect: unset show_always writes nothing to stderr" "" (cat $__stderr_file)
set -g __gpy_is_ssh 1
segment_username_detect 2>$__stderr_file
check "detect: SSH shows the segment with show_always unset" 0 $status
check "detect: SSH with show_always unset writes nothing to stderr" "" (cat $__stderr_file)
set -e __gpy_is_root __gpy_is_sudo __gpy_is_ssh
rm -f $__stderr_file

# --- __gpy_username_should_show (pure): args are is_root is_sudo is_ssh show_always ---

if __gpy_username_should_show 1 0 0 0
    check "should_show: root shows regardless of sudo/show_always" pass pass
else
    check "should_show: root shows regardless of sudo/show_always" pass fail
end

if __gpy_username_should_show 0 1 0 0
    check "should_show: sudo session shows" pass pass
else
    check "should_show: sudo session shows" pass fail
end

if __gpy_username_should_show 0 0 1 0
    check "should_show: SSH session shows" pass pass
else
    check "should_show: SSH session shows" pass fail
end

if not __gpy_username_should_show 0 0 0 0
    check "should_show: normal user with show_always=0 hides" pass pass
else
    check "should_show: normal user with show_always=0 hides" pass fail
end

if __gpy_username_should_show 0 0 0 1
    check "should_show: normal user with show_always=1 shows" pass pass
else
    check "should_show: normal user with show_always=1 shows" pass fail
end

# --- segment_username_detect (edge) ---

set -g __gpy_is_ssh 0
set -g __gpy_is_root 1
set -g __gpy_is_sudo 0
set -g __username_show_always 0
if segment_username_detect
    check "detect: root session is shown" pass pass
else
    check "detect: root session is shown" pass fail
end

set -g __gpy_is_root 0
set -g __gpy_is_sudo 1
set -g __username_show_always 0
if segment_username_detect
    check "detect: sudo session is shown" pass pass
else
    check "detect: sudo session is shown" pass fail
end

set -g __gpy_is_root 0
set -g __gpy_is_sudo 0
set -g __username_show_always 0
if not segment_username_detect
    check "detect: normal user with show_always=0 is hidden" pass pass
else
    check "detect: normal user with show_always=0 is hidden" pass fail
end

set -g __gpy_is_ssh 1
if segment_username_detect
    check "detect: SSH session is shown" pass pass
else
    check "detect: SSH session is shown" pass fail
end
set -g __gpy_is_ssh 0

set -g __gpy_is_root 0
set -g __gpy_is_sudo 0
set -g __username_show_always 1
if segment_username_detect
    check "detect: normal user with show_always=1 is shown" pass pass
else
    check "detect: normal user with show_always=1 is shown" pass fail
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
set -g __gpy_is_root 1
set -g __username_format ""
set -g __icon_username ""
set -g __color_username_bg red
set -g __color_username_fg white
set -g __color_username_user_bg yellow
set -g __color_username_user_fg black
set -g __gpy_last_segment_bg black

segment_username_render true

if test "$__test_username_content" = " root"
    check "pure-fish render: renders \$USER with one leading space and omits the icon when unset" pass pass
else
    check "pure-fish render: renders \$USER with one leading space and omits the icon when unset (got: '$__test_username_content')" pass fail
end

if test "$__test_username_bg" = red; and test "$__test_username_fg" = white
    check "pure-fish render: applies the configured bg/fg colours" pass pass
else
    check "pure-fish render: applies the configured bg/fg colours (got: '$__test_username_bg'/'$__test_username_fg')" pass fail
end

if test "$__gpy_last_segment_bg" = red
    check "pure-fish render: tracks __gpy_last_segment_bg for the next segment's chevron" pass pass
else
    check "pure-fish render: tracks __gpy_last_segment_bg for the next segment's chevron (got: '$__gpy_last_segment_bg')" pass fail
end

# Colour override: a different theme colour flows straight through.
set -g __color_username_bg magenta
set -g __color_username_fg cyan
set -g __gpy_last_segment_bg black
segment_username_render true
if test "$__test_username_bg" = magenta; and test "$__test_username_fg" = cyan
    check "pure-fish render: honours a colour override" pass pass
else
    check "pure-fish render: honours a colour override (got: '$__test_username_bg'/'$__test_username_fg')" pass fail
end

# A non-root session draws the user style (Starship style_user).
set -g __gpy_is_root 0
set -g __gpy_last_segment_bg black
segment_username_render true
if test "$__test_username_bg" = yellow; and test "$__test_username_fg" = black; and test "$__gpy_last_segment_bg" = yellow
    check "pure-fish render: a non-root session uses the user colours" pass pass
else
    check "pure-fish render: a non-root session uses the user colours (got: '$__test_username_bg'/'$__test_username_fg'/'$__gpy_last_segment_bg')" pass fail
end
set -g __gpy_is_root 1

set -g __color_username_bg red
set -g __color_username_fg white
set -g __icon_username ICON
set -g __gpy_last_segment_bg black
segment_username_render true

if test "$__test_username_content" = " ICON root"
    check "pure-fish render: includes the icon when __icon_username is set" pass pass
else
    check "pure-fish render: includes the icon when __icon_username is set (got: '$__test_username_content')" pass fail
end

set -g __icon_username ""
set -g __gpy_last_segment_bg black
segment_username_render true

if test "$__test_username_content" = " root"
    check "pure-fish render: omits the icon again once __icon_username is cleared" pass pass
else
    check "pure-fish render: omits the icon again once __icon_username is cleared (got: '$__test_username_content')" pass fail
end

# --- segment_username_render: dual-path branch to agent-resolved rendering ---
#
# __username_format's CONTENT is presence-only (the agent re-derives the real
# format server-side) — the branch must fire purely because the var is
# non-empty, regardless of what it contains.
function __gpy_request_username --argument-names user is_last prev_bg is_root is_first
    set -g __test_username_arg_user "$user"
    set -g __test_username_arg_is_last "$is_last"
    set -g __test_username_arg_prev_bg "$prev_bg"
    set -g __test_username_arg_is_root "$is_root"
    set -g __test_username_arg_is_first "$is_first"
    printf AGENT_SENTINEL
end

set -g __username_format some-opaque-agent-side-value
set -g __test_username_content ""
set -g __gpy_last_segment_bg black
set -l agent_output (segment_username_render true | string collect)

if test "$agent_output" = AGENT_SENTINEL
    check "agent path: non-empty __username_format routes to __gpy_request_username" pass pass
else
    check "agent path: non-empty __username_format routes to __gpy_request_username (got: '$agent_output')" pass fail
end

if test -z "$__test_username_content"
    check "agent path: does not fall through to the pure-fish renderer" pass pass
else
    check "agent path: does not fall through to the pure-fish renderer" pass fail
end

if test "$__test_username_arg_user" = root
    check "agent path: forwards \$USER to __gpy_request_username" pass pass
else
    check "agent path: forwards \$USER to __gpy_request_username (got: '$__test_username_arg_user')" pass fail
end

if test "$__gpy_last_segment_bg" = red
    check "agent path: still tracks __gpy_last_segment_bg for the next segment" pass pass
else
    check "agent path: still tracks __gpy_last_segment_bg for the next segment (got: '$__gpy_last_segment_bg')" pass fail
end

if test "$__test_username_arg_is_root" = 1
    check "agent path: forwards __gpy_is_root=1 as the 4th arg" pass pass
else
    check "agent path: forwards __gpy_is_root=1 as the 4th arg (got: '$__test_username_arg_is_root')" pass fail
end

set -g __gpy_is_root 0
set -g __gpy_last_segment_bg black
segment_username_render true >/dev/null
if test "$__test_username_arg_is_root" = 0; and test "$__gpy_last_segment_bg" = yellow
    check "agent path: a non-root session forwards is_root=0 and tracks the user bg" pass pass
else
    check "agent path: a non-root session forwards is_root=0 and tracks the user bg (got: '$__test_username_arg_is_root'/'$__gpy_last_segment_bg')" pass fail
end
set -g __gpy_is_root 1

# When username is NOT the last rendered segment, the render is called with
# NO argument (is_last empty). The agent branch must still forward prev_bg
# into the 3rd slot and pass "" (not the bg) as is_last: an unquoted empty
# is_last would collapse on expansion and shift prev_bg out of position.
set -g __username_format some-opaque-agent-side-value
set -g __gpy_last_segment_bg green
set -e __test_username_arg_is_last
set -e __test_username_arg_prev_bg
segment_username_render >/dev/null

if test "$__test_username_arg_prev_bg" = green
    check "agent path (not last): prev_bg reaches __gpy_request_username as the 3rd arg" pass pass
else
    check "agent path (not last): prev_bg reaches __gpy_request_username as the 3rd arg (got: '$__test_username_arg_prev_bg')" pass fail
end

if test -z "$__test_username_arg_is_last"
    check "agent path (not last): is_last is forwarded as empty, not shifted" pass pass
else
    check "agent path (not last): is_last is forwarded as empty, not shifted (got: '$__test_username_arg_is_last')" pass fail
end

# is_first rides in the 5th slot: "true" when first, "" (not shifted) otherwise, so
# the agent can drop the opening cap of the first pill.
set -g __gpy_last_segment_bg green
set -e __test_username_arg_is_first
segment_username_render "" true >/dev/null
if test "$__test_username_arg_is_first" = true
    check "agent path: is_first=true reaches __gpy_request_username as the 5th arg" pass pass
else
    check "agent path: is_first=true reaches __gpy_request_username as the 5th arg (got: '$__test_username_arg_is_first')" pass fail
end

set -g __test_username_arg_is_first sentinel
segment_username_render true >/dev/null
if test -z "$__test_username_arg_is_first"
    check "agent path: a non-first username forwards is_first as empty" pass pass
else
    check "agent path: a non-first username forwards is_first as empty (got: '$__test_username_arg_is_first')" pass fail
end

functions -e __gpy_request_username
functions -e gpy_section_standalone
set -e __username_format

# --- __gpy_request_username (edge, IPC payload construction) ---
#
# Source the real IPC helpers and stub __gpy_ipc_send to capture the payload,
# so JSON-escaping and the is_last "true"/"" convention (shared by every
# segment) are verified directly.
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

function __gpy_ipc_send --argument-names payload timeout_ms
    set -g __test_username_payload "$payload"
    echo RENDERED
end

set -l result_last (__gpy_request_username 'ro"ot' true prevbg)
if test "$result_last" = RENDERED
    check "__gpy_request_username: returns the IPC result on success" pass pass
else
    check "__gpy_request_username: returns the IPC result on success (got: '$result_last')" pass fail
end

if string match -q '*"op":"username"*' -- "$__test_username_payload"
    check "__gpy_request_username: emits the username op" pass pass
else
    check "__gpy_request_username: emits the username op (got: $__test_username_payload)" pass fail
end

if string match -q '*"is_last":true*' -- "$__test_username_payload"
    check "__gpy_request_username: is_last=true emits \"is_last\":true" pass pass
else
    check "__gpy_request_username: is_last=true emits \"is_last\":true (got: $__test_username_payload)" pass fail
end

if string match -q '*"username":"ro\\"ot"*' -- "$__test_username_payload"
    check "__gpy_request_username: JSON-escapes the username argument" pass pass
else
    check "__gpy_request_username: JSON-escapes the username argument (got: $__test_username_payload)" pass fail
end

if string match -q '*"prev_bg":"prevbg"*' -- "$__test_username_payload"
    check "__gpy_request_username: includes prev_bg when non-empty" pass pass
else
    check "__gpy_request_username: includes prev_bg when non-empty (got: $__test_username_payload)" pass fail
end

__gpy_request_username user2 "" "" >/dev/null
if not string match -q '*is_last*' -- "$__test_username_payload"
    check "__gpy_request_username: is_last='' omits is_last" pass pass
else
    check "__gpy_request_username: is_last='' omits is_last (got: $__test_username_payload)" pass fail
end

if not string match -q '*prev_bg*' -- "$__test_username_payload"
    check "__gpy_request_username: omits prev_bg when empty" pass pass
else
    check "__gpy_request_username: omits prev_bg when empty (got: $__test_username_payload)" pass fail
end

__gpy_request_username user4 "" "" "" true >/dev/null
if string match -q '*"is_first":true*' -- "$__test_username_payload"
    check "__gpy_request_username: is_first=true emits \"is_first\":true" pass pass
else
    check "__gpy_request_username: is_first=true emits \"is_first\":true (got: $__test_username_payload)" pass fail
end

__gpy_request_username user5 "" "" "" "" >/dev/null
if not string match -q '*is_first*' -- "$__test_username_payload"
    check "__gpy_request_username: is_first='' omits is_first" pass pass
else
    check "__gpy_request_username: is_first='' omits is_first (got: $__test_username_payload)" pass fail
end

if not string match -q '*is_root*' -- "$__test_username_payload"
    check "__gpy_request_username: a non-root request omits is_root" pass pass
else
    check "__gpy_request_username: a non-root request omits is_root (got: $__test_username_payload)" pass fail
end

__gpy_request_username user6 true "" 1 true >/dev/null
if test "$__test_username_payload" = '{"op":"username","username":"user6","format":"ansi","is_last":true,"is_first":true,"is_root":true}'
    check "__gpy_request_username: is_root=1 appends \"is_root\":true" pass pass
else
    check "__gpy_request_username: is_root=1 appends \"is_root\":true (got: $__test_username_payload)" pass fail
end

functions -e __gpy_ipc_send

# No oneshot fallback exists for username — an empty IPC result must propagate
# as failure, not silently synthesize output.
function __gpy_ipc_send --argument-names payload timeout_ms
end

if not __gpy_request_username user3 true ""
    check "__gpy_request_username: returns nonzero when IPC is unavailable (no oneshot fallback)" pass pass
else
    check "__gpy_request_username: returns nonzero when IPC is unavailable (no oneshot fallback)" pass fail
end

functions -e __gpy_ipc_send

test_finish "username segment tests"
