#!/usr/bin/env fish
# Test: hostname segment (fish/segments/hostname.fish) — SSH-only visibility,
# pure-fish trim/render, and the dual-path branch to agent-resolved rendering.
# Part of #257 (hostname/SSH segment); this task is #262.
#
# The segment is opt-in (never added to a theme's default __enabled_segments —
# see fish/segments/hostname.fish, and #257 for the rationale), so
# these tests source the segment file directly and drive its functions with
# stubbed globals rather than exercising it through fish_prompt.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/segments/hostname.fish"

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

# --- __gpy_hostname_trim (pure) ---

set -l trimmed (__gpy_hostname_trim host.example.com .)
if test "$trimmed" = host
    check "trim cuts the value at the first delimiter occurrence" pass
else
    check "trim cuts the value at the first delimiter occurrence (got: '$trimmed')" fail
end

set -l untrimmed (__gpy_hostname_trim host.example.com "")
if test "$untrimmed" = host.example.com
    check "trim leaves the value unchanged when the delimiter is empty" pass
else
    check "trim leaves the value unchanged when the delimiter is empty (got: '$untrimmed')" fail
end

# --- __gpy_hostname_should_show (pure) ---

if __gpy_hostname_should_show 1 0
    check "should_show: SSH session shows regardless of show_always" pass
else
    check "should_show: SSH session shows regardless of show_always" fail
end

if not __gpy_hostname_should_show 0 0
    check "should_show: local session with show_always=0 hides" pass
else
    check "should_show: local session with show_always=0 hides" fail
end

if __gpy_hostname_should_show 0 1
    check "should_show: local session with show_always=1 shows" pass
else
    check "should_show: local session with show_always=1 shows" fail
end

# --- segment_hostname_detect (edge) ---

set -g __gpy_is_ssh 1
set -g __hostname_show_always 0
if segment_hostname_detect
    check "detect: SSH session is shown" pass
else
    check "detect: SSH session is shown" fail
end

set -g __gpy_is_ssh 0
set -g __hostname_show_always 0
if not segment_hostname_detect
    check "detect: local session with show_always=0 is hidden" pass
else
    check "detect: local session with show_always=0 is hidden" fail
end

set -g __gpy_is_ssh 0
set -g __hostname_show_always 1
if segment_hostname_detect
    check "detect: local session with show_always=1 is shown" pass
else
    check "detect: local session with show_always=1 is shown" fail
end

# --- segment_hostname_render: pure-fish path (zero IPC, zero forks) ---
#
# `$hostname` is one of fish's electric read-only variables (set once at shell
# start via gethostname(2)) — it cannot be overridden with `set`, so these
# tests read the real value instead of injecting a fixture one. `__hostname_trim_at`
# is left empty here so the wiring assertions below don't depend on whether the
# delimiter happens to appear in this machine's hostname; the trim behavior
# itself is already covered directly above.
function gpy_section_standalone --argument-names bg fg content is_last
    set -g __test_hostname_content "$content"
end

set -l real_hostname $hostname
set -g __hostname_trim_at ""
set -g __hostname_format ""
set -g __icon_hostname ""
set -g __color_hostname_bg blue
set -g __color_hostname_fg white
set -g __gpy_last_segment_bg black

segment_hostname_render true

if test "$__test_hostname_content" = "$real_hostname"
    check "pure-fish render: renders the hostname and omits the icon when unset" pass
else
    check "pure-fish render: renders the hostname and omits the icon when unset (got: '$__test_hostname_content')" fail
end

if test "$__gpy_last_segment_bg" = blue
    check "pure-fish render: tracks __gpy_last_segment_bg for the next segment's chevron" pass
else
    check "pure-fish render: tracks __gpy_last_segment_bg for the next segment's chevron (got: '$__gpy_last_segment_bg')" fail
end

# #826: the icon is Starship's ssh_symbol — drawn only in SSH sessions.
set -g __icon_hostname ICON
set -g __gpy_is_ssh 1
set -g __gpy_last_segment_bg black
segment_hostname_render true

if test "$__test_hostname_content" = "ICON $real_hostname"
    check "pure-fish render: includes the icon over SSH when __icon_hostname is set" pass
else
    check "pure-fish render: includes the icon over SSH when __icon_hostname is set (got: '$__test_hostname_content')" fail
end

set -g __gpy_is_ssh 0
segment_hostname_render true

if test "$__test_hostname_content" = "$real_hostname"
    check "pure-fish render: omits the icon in a local session (#826)" pass
else
    check "pure-fish render: omits the icon in a local session (#826) (got: '$__test_hostname_content')" fail
end
set -g __gpy_is_ssh 1

set -g __icon_hostname ""
set -g __gpy_last_segment_bg black
segment_hostname_render true

if test "$__test_hostname_content" = "$real_hostname"
    check "pure-fish render: omits the icon again once __icon_hostname is cleared" pass
else
    check "pure-fish render: omits the icon again once __icon_hostname is cleared (got: '$__test_hostname_content')" fail
end

# --- segment_hostname_render: dual-path branch to agent-resolved rendering ---
#
# __hostname_format's CONTENT is presence-only (the agent re-derives the real
# format server-side) — the branch must fire purely because the var is
# non-empty, regardless of what garbage it contains.

# The stub records the exact positional args it received so we can verify the
# render function forwards is_last and prev_bg in the correct slots (regression
# guard for the "not last" bug where an empty is_last collapses on unquoted
# expansion and shifts prev_bg out of position).
function __gpy_request_hostname --argument-names host is_last prev_bg is_ssh
    set -g __test_hostname_arg_host "$host"
    set -g __test_hostname_arg_is_last "$is_last"
    set -g __test_hostname_arg_prev_bg "$prev_bg"
    set -g __test_hostname_arg_is_ssh "$is_ssh"
    printf AGENT_SENTINEL
end

set -g __hostname_format some-opaque-agent-side-value
set -g __test_hostname_content ""
set -g __gpy_last_segment_bg black
set -l agent_output (segment_hostname_render true | string collect)

if test "$agent_output" = AGENT_SENTINEL
    check "agent path: non-empty __hostname_format routes to __gpy_request_hostname" pass
else
    check "agent path: non-empty __hostname_format routes to __gpy_request_hostname (got: '$agent_output')" fail
end

if test -z "$__test_hostname_content"
    check "agent path: does not fall through to the pure-fish renderer" pass
else
    check "agent path: does not fall through to the pure-fish renderer" fail
end

if test "$__gpy_last_segment_bg" = blue
    check "agent path: still tracks __gpy_last_segment_bg for the next segment" pass
else
    check "agent path: still tracks __gpy_last_segment_bg for the next segment (got: '$__gpy_last_segment_bg')" fail
end

# Regression: when the hostname segment is NOT the last rendered segment,
# fish_prompt.fish calls segment_hostname_render with NO argument (is_last is
# empty/unset). The agent branch must still forward prev_bg into
# __gpy_request_hostname's 3rd slot and pass "" (not the bg) as is_last.
# Before the fix, an unquoted empty $is_last collapsed on expansion, shifting
# $__gpy_last_segment_bg into the is_last param and leaving prev_bg unset.
set -g __hostname_format some-opaque-agent-side-value
set -g __gpy_last_segment_bg green
set -e __test_hostname_arg_is_last
set -e __test_hostname_arg_prev_bg
segment_hostname_render >/dev/null

if test "$__test_hostname_arg_prev_bg" = green
    check "agent path (not last): prev_bg reaches __gpy_request_hostname as the 3rd arg" pass
else
    check "agent path (not last): prev_bg reaches __gpy_request_hostname as the 3rd arg (got: '$__test_hostname_arg_prev_bg')" fail
end

if test -z "$__test_hostname_arg_is_last"
    check "agent path (not last): is_last is forwarded as empty, not shifted" pass
else
    check "agent path (not last): is_last is forwarded as empty, not shifted (got: '$__test_hostname_arg_is_last')" fail
end

# #826: the 4th arg carries $__gpy_is_ssh, for both states, and stays in the
# 4th slot even when prev_bg is unset (an empty list would otherwise collapse
# on unquoted expansion and shift is_ssh into the prev_bg slot).
for ssh_state in 0 1
    set -g __gpy_is_ssh $ssh_state
    set -g __gpy_last_segment_bg green
    set -e __test_hostname_arg_is_ssh
    segment_hostname_render >/dev/null
    if test "$__test_hostname_arg_is_ssh" = $ssh_state
        check "agent path: __gpy_is_ssh=$ssh_state reaches __gpy_request_hostname as the 4th arg" pass
    else
        check "agent path: __gpy_is_ssh=$ssh_state reaches __gpy_request_hostname as the 4th arg (got: '$__test_hostname_arg_is_ssh')" fail
    end
end

set -g __gpy_is_ssh 1
set -e __gpy_last_segment_bg
set -e __test_hostname_arg_is_ssh
set -e __test_hostname_arg_prev_bg
segment_hostname_render >/dev/null
if test "$__test_hostname_arg_is_ssh" = 1; and test -z "$__test_hostname_arg_prev_bg"
    check "agent path: unset prev_bg does not shift is_ssh out of the 4th slot" pass
else
    check "agent path: unset prev_bg does not shift is_ssh out of the 4th slot (prev_bg: '$__test_hostname_arg_prev_bg', is_ssh: '$__test_hostname_arg_is_ssh')" fail
end

functions -e __gpy_request_hostname
functions -e gpy_section_standalone
set -e __hostname_format

# --- __gpy_request_hostname (edge, IPC payload construction) ---
#
# Source the real IPC helpers and stub __gpy_ipc_send to capture the payload,
# so the JSON-escaping and is_last "true"/"" convention are verified directly
# (#613: hostname now shares the exact same "true"/"" contract as character,
# username, and every other segment -- there is no more distinct convention).
#
# Note: __gpy_request_hostname's first parameter is named `host`, not
# `hostname` — `--argument-names hostname` is a hard error in fish because
# `$hostname` is a read-only electric variable (see the pure-fish render
# comment above). The value passed in is still the caller's `$hostname`.
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

function __gpy_ipc_send --argument-names payload timeout_ms
    set -g __test_hostname_payload "$payload"
    echo RENDERED
end

set -l result_last (__gpy_request_hostname 'my"host' true prevbg)
if test "$result_last" = RENDERED
    check "__gpy_request_hostname: returns the IPC result on success" pass
else
    check "__gpy_request_hostname: returns the IPC result on success (got: '$result_last')" fail
end

if string match -q '*"is_last":true*' -- "$__test_hostname_payload"
    check "__gpy_request_hostname: is_last=true emits \"is_last\":true" pass
else
    check "__gpy_request_hostname: is_last=true emits \"is_last\":true (got: $__test_hostname_payload)" fail
end

if string match -q '*"hostname":"my\\"host"*' -- "$__test_hostname_payload"
    check "__gpy_request_hostname: JSON-escapes the hostname argument" pass
else
    check "__gpy_request_hostname: JSON-escapes the hostname argument (got: $__test_hostname_payload)" fail
end

if string match -q '*"prev_bg":"prevbg"*' -- "$__test_hostname_payload"
    check "__gpy_request_hostname: includes prev_bg when non-empty" pass
else
    check "__gpy_request_hostname: includes prev_bg when non-empty (got: $__test_hostname_payload)" fail
end

__gpy_request_hostname host2 "" "" >/dev/null
if not string match -q '*is_last*' -- "$__test_hostname_payload"
    check "__gpy_request_hostname: is_last='' omits is_last" pass
else
    check "__gpy_request_hostname: is_last='' omits is_last (got: $__test_hostname_payload)" fail
end

if not string match -q '*prev_bg*' -- "$__test_hostname_payload"
    check "__gpy_request_hostname: omits prev_bg when empty" pass
else
    check "__gpy_request_hostname: omits prev_bg when empty (got: $__test_hostname_payload)" fail
end

__gpy_request_hostname host3 "" "" 1 >/dev/null
if string match -q '*,"is_ssh":true}' -- "$__test_hostname_payload"
    check "__gpy_request_hostname: is_ssh=1 emits \"is_ssh\":true" pass
else
    check "__gpy_request_hostname: is_ssh=1 emits \"is_ssh\":true (got: $__test_hostname_payload)" fail
end

__gpy_request_hostname host4 "" "" 0 >/dev/null
if not string match -q '*is_ssh*' -- "$__test_hostname_payload"
    check "__gpy_request_hostname: is_ssh=0 omits is_ssh" pass
else
    check "__gpy_request_hostname: is_ssh=0 omits is_ssh (got: $__test_hostname_payload)" fail
end

functions -e __gpy_ipc_send

# No oneshot fallback exists for hostname (out of scope) — an empty IPC
# result must propagate as failure, not silently synthesize output.
function __gpy_ipc_send --argument-names payload timeout_ms
end

if not __gpy_request_hostname host3 true ""
    check "__gpy_request_hostname: returns nonzero when IPC is unavailable (no oneshot fallback)" pass
else
    check "__gpy_request_hostname: returns nonzero when IPC is unavailable (no oneshot fallback)" fail
end

functions -e __gpy_ipc_send

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count hostname segment tests passed"
