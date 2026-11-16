#!/usr/bin/env fish
# tests/fish/ipc_late_agent_negative_cache.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# An agent that accepts connections but never answers (stopped, paused across
# sleep, wedged) costs one reply budget, not one per request: after a send the
# agent answers from memory (ping, character, clock, ...) waits out its
# budget, the next sends to that socket return 2 ("late") at once for a few
# seconds, as in Bash and Zsh. A path-bearing request (git, lang, directory)
# can be slow for its path alone and opens no window. A doorbell (the agent
# answering again) clears it.
#
# A silent stub listener plays the wedged agent and logs every request.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "A late agent is not asked again for a few seconds"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a wedged agent"
end
if not __gpy_ipc_client_available
    test_skip "neither socat nor an nc with -U is installed"
end

set -g tmp (mktemp -d)
test_track_dir $tmp
set -gx XDG_RUNTIME_DIR $tmp/run
set -gx GPY_AGENT_ENABLED 1
set -g requests $tmp/requests

function __sent_ns
    string match -rg '"n":([0-9]+)' <$requests | string join ' '
end

# --- direct sends ---

set -gx GPY_AGENT_SOCKET_PATH $tmp/wedged.sock
test_stub_listener $GPY_AGENT_SOCKET_PATH silent --requests $requests --lifetime 60

__gpy_ipc_send '{"op":"ping","n":1}' 100 >/dev/null
check "a timed-out send is status 2" 2 $status
set -l t1 (test_now_ns)
__gpy_ipc_send '{"op":"ping","n":2}' 100 >/dev/null
check "the next send in the window is status 2" 2 $status
set -l t2 (test_now_ns)
if test (math "$t2 - $t1") -lt 50000000
    pass "the gated send returns at once"
else
    fail "the gated send waited "(math "($t2 - $t1) / 1000000")" ms"
end
__gpy_ipc_send '{"op":"ping","n":3}' 100 $tmp/other.sock >/dev/null
check "the window applies to its socket only" 1 $status

# A doorbell clears the window.
set -g __gpy_agent_late_until 0
__gpy_ipc_send '{"op":"ping","n":4}' 100 >/dev/null
check "a send after the doorbell is status 2" 2 $status

test_stop_pids $test_stub_pid
check "the gated send never reached the agent" "1 4" (__sent_ns)

# --- the window closes after its seconds ---

set -g __gpy_agent_late_until (math (__gpy_now_seconds) - 1)
check "an expired window is closed" 1 (__gpy_agent_late_active $GPY_AGENT_SOCKET_PATH; echo $status)

# --- a path-bearing request opens no window ---

rm -f $requests
set -g __gpy_agent_late_until 0
set -gx GPY_AGENT_SOCKET_PATH $tmp/wedged-path.sock
test_stub_listener $GPY_AGENT_SOCKET_PATH silent --requests $requests --lifetime 60

__gpy_ipc_send '{"op":"git","cwd":"/","n":1}' 100 >/dev/null
__gpy_ipc_send '{"op":"ping","n":2}' 100 >/dev/null
test_stop_pids $test_stub_pid
check "a timed-out git request leaves the next request ungated" "1 2" (__sent_ns)

# --- an empty reply line is an agent that answered in time ---

set -g __gpy_agent_late_until 0
set -gx GPY_AGENT_SOCKET_PATH $tmp/empty.sock
test_stub_listener $GPY_AGENT_SOCKET_PATH reply --reply '' --lifetime 30
__gpy_ipc_send '{"op":"ping"}' 1000 >/dev/null
check "an empty reply is status 2" 2 $status
check "an empty reply opens no window" 0 "$__gpy_agent_late_until"
test_stop_pids $test_stub_pid

test_finish "A late agent is not asked again for a few seconds"
