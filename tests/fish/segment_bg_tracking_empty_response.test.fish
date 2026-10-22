#!/usr/bin/env fish
# Test: hostname/username agent-resolved render paths with no usable agent
# reply. An empty reply (agent down, disabled or late) or an `{"error":...}`
# reply (an older agent) must draw the local pill — with its one leading
# space — and advance __gpy_last_segment_bg to that pill's background, so the
# segment never vanishes and the next chevron blends against what rendered.
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

function gpy_section_standalone --argument-names bg fg content is_last
    set -g __test_pill_bg "$bg"
    set -g __test_pill_content "$content"
    printf LOCAL_PILL
end

source "$repo_root/fish/segments/hostname.fish"
source "$repo_root/fish/segments/username.fish"

set -g __gpy_is_ssh 0
# Empty delimiter: no trim, so the local label is exactly " $hostname".
set -g __hostname_trim_at ""
set -g __icon_hostname ""
set -g __color_hostname_bg blue
set -g __color_hostname_fg white
set -g __icon_username ""
set -g __color_username_bg red
set -g __color_username_fg white
set -g USER root
set -g __hostname_format some-opaque-agent-side-value
set -g __username_format some-opaque-agent-side-value

# Renders hostname then username; records each output and the tracker after it.
function __test_render_pair
    set -g __test_pill_content ""
    set -g __gpy_last_segment_bg unchanged
    set -g __test_host_out (segment_hostname_render "" | string collect)
    segment_hostname_render "" >/dev/null
    set -g __test_host_content "$__test_pill_content"
    set -g __test_host_bg "$__gpy_last_segment_bg"

    set -g __test_pill_content ""
    set -g __gpy_last_segment_bg unchanged
    set -g __test_user_out (segment_username_render last | string collect)
    segment_username_render last >/dev/null
    set -g __test_user_content "$__test_pill_content"
    set -g __test_user_bg "$__gpy_last_segment_bg"
end

function __test_assert_local --argument-names label
    if test "$__test_host_out" = LOCAL_PILL; and test "$__test_host_content" = " $hostname"
        check "$label: hostname draws the local pill with one leading space" pass
    else
        check "$label: hostname draws the local pill with one leading space (got: '$__test_host_out' / '$__test_host_content')" fail
    end
    if test "$__test_host_bg" = blue
        check "$label: hostname advances __gpy_last_segment_bg to its pill" pass
    else
        check "$label: hostname advances __gpy_last_segment_bg to its pill (got: '$__test_host_bg')" fail
    end
    if test "$__test_user_out" = LOCAL_PILL; and test "$__test_user_content" = " root"
        check "$label: username draws the local pill with one leading space" pass
    else
        check "$label: username draws the local pill with one leading space (got: '$__test_user_out' / '$__test_user_content')" fail
    end
    if test "$__test_user_bg" = red
        check "$label: username advances __gpy_last_segment_bg to its pill" pass
    else
        check "$label: username advances __gpy_last_segment_bg to its pill (got: '$__test_user_bg')" fail
    end
end

# --- no ipc.fish (agent disabled): the request helpers do not exist ---

__test_render_pair
__test_assert_local "agent disabled"

# --- real request helpers, agent unreachable (empty IPC reply) ---

source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

function __gpy_ipc_send --argument-names payload timeout_ms
end

__test_render_pair
__test_assert_local "agent unreachable"

# --- real request helpers, older agent answers with an error ---

function __gpy_ipc_send --argument-names payload timeout_ms
    echo '{"error":"Parse error: IPC communication failed: Unknown shell operation: exit_status"}'
end

__test_render_pair
__test_assert_local "error reply"

if not __gpy_request_hostname box "" "" 0 "" >/dev/null
    check "__gpy_request_hostname: an error reply returns nonzero" pass
else
    check "__gpy_request_hostname: an error reply returns nonzero" fail
end
if not __gpy_request_username root "" "" "" >/dev/null
    check "__gpy_request_username: an error reply returns nonzero" pass
else
    check "__gpy_request_username: an error reply returns nonzero" fail
end

# --- a rendered reply is still emitted verbatim and tracked ---

function __gpy_ipc_send --argument-names payload timeout_ms
    echo SENTINEL
end

__test_render_pair
if test "$__test_host_out" = SENTINEL; and test -z "$__test_host_content"; and test "$__test_host_bg" = blue
    check "rendered reply: hostname emits the agent pill and tracks its bg" pass
else
    check "rendered reply: hostname emits the agent pill and tracks its bg (got: '$__test_host_out' / '$__test_host_content' / '$__test_host_bg')" fail
end
if test "$__test_user_out" = SENTINEL; and test -z "$__test_user_content"; and test "$__test_user_bg" = red
    check "rendered reply: username emits the agent pill and tracks its bg" pass
else
    check "rendered reply: username emits the agent pill and tracks its bg (got: '$__test_user_out' / '$__test_user_content' / '$__test_user_bg')" fail
end

functions -e __gpy_ipc_send gpy_section_standalone
set -e __hostname_format
set -e __username_format

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count segment bg tracking (empty response) tests passed"
