#!/usr/bin/env fish
# Regression test for #300: a truncated (partial-write) IPC response must
# never reach the caller as if it were complete. All three shells previously
# treated any non-empty read as a valid response, so an agent that wrote
# `{"status":"o` and then stalled or died mid-write would have that exact
# truncated string handed to __gpy_request_character/_duration (raw ANSI,
# printf'd straight into the prompt) — corrupting terminal rendering.
#
# The "wedged-mid-write agent" is simulated with a tiny Python listener that
# accept()s the connection, writes a partial (non-newline-terminated) frame,
# then goes silent. The listener self-destructs after a few seconds so this
# test has a hard, deterministic upper bound even if the fix regresses and
# the client hangs waiting for more bytes.
#
# A second case guards against the fix being too strict: a normally-terminated
# complete response must still be returned intact.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "IPC partial-response validation (#300)"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a partial-write agent"
end
if not command -q socat; and not command -q nc
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
end

set -l test_tmp_dir (mktemp -d)
set -l test_result PASS

function _ipc_partial_cleanup --argument-names listener_pid
    kill -9 $listener_pid 2>/dev/null
end

# --- Case 1: partial write then stall must not leak the truncated bytes ---

set -l sock1 $test_tmp_dir/gpy-partial-test.sock
set -l listener_lifetime_secs 3
python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.sendall(b"{\"status\":\"o")
time.sleep(float(sys.argv[2]))
' $sock1 $listener_lifetime_secs &
set -l listener1_pid $last_pid

for i in (seq 1 20)
    test -S $sock1; and break
    sleep 0.1
end

if not test -S $sock1
    _ipc_partial_cleanup $listener1_pid
    rm -rf $test_tmp_dir
    test_skip "could not create test Unix socket listener"
end

set -gx GPY_AGENT_SOCKET_PATH $sock1
# Well under the listener's stall duration, so the client's own timeout (not
# the listener self-destructing) is what ends this call.
set -l result (__gpy_ipc_send '{"op":"ping"}' 300)

_ipc_partial_cleanup $listener1_pid

if test -n "$result"
    print_test_result "partial write is discarded, not returned" FAIL "got '$result'"
    set test_result FAIL
else
    print_test_result "partial write is discarded, not returned" PASS
end

# --- Case 2: a complete, newline-terminated response still round-trips ---

set -l sock2 $test_tmp_dir/gpy-complete-test.sock
python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
conn.sendall(b"{\"status\":\"ok\"}\n")
conn.close()
' $sock2 &
set -l listener2_pid $last_pid

for i in (seq 1 20)
    test -S $sock2; and break
    sleep 0.1
end

if not test -S $sock2
    _ipc_partial_cleanup $listener2_pid
    rm -rf $test_tmp_dir
    test_skip "could not create second test Unix socket listener"
end

set -gx GPY_AGENT_SOCKET_PATH $sock2
set -l complete_result (__gpy_ipc_send '{"op":"ping"}' 1000)

_ipc_partial_cleanup $listener2_pid
functions -e _ipc_partial_cleanup
rm -rf $test_tmp_dir

if test "$complete_result" = '{"status":"ok"}'
    print_test_result "complete response still round-trips" PASS
else
    print_test_result "complete response still round-trips" FAIL "got '$complete_result'"
    set test_result FAIL
end

print_test_footer "IPC partial-response validation" $test_result

if test "$test_result" = PASS
    exit 0
else
    exit 1
end
