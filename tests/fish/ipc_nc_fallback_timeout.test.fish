#!/usr/bin/env fish
# Regression test for #299: the `nc -U` fallback used when coreutils `timeout`
# (and `socat`) are unavailable must not hang forever against a
# connected-but-silent socket — that freezes the prompt on stock macOS.
#
# The "wedged agent" is simulated with a tiny Python listener that accept()s
# the connection and then goes silent. Plain `nc -l` can't stand in for this:
# nc auto-closes as soon as it sees the client's half-close, which masks the
# bug this test guards against.
#
# The listener self-destructs after a few seconds so this test has a hard,
# deterministic upper bound even if the fallback regresses and hangs again —
# no fragile background-job PID tracking required.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "IPC nc fallback timeout (#299)"

if not command -q nc
    test_skip "nc not installed, cannot exercise this fallback"
end
if not command -q python3
    test_skip "python3 not installed, cannot simulate a wedged listener"
end

set -l real_date (command -v date)
set -l test_tmp_dir (mktemp -d)
set -l sock $test_tmp_dir/gpy-wedged-test.sock

# Listener that accepts the connection, then goes silent, then gives up and
# exits after a few seconds — the self-destruct is what bounds this test's
# worst case, not any polling/killing of the client side.
set -l listener_lifetime_secs 4
python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(1)
conn, _ = s.accept()
time.sleep(float(sys.argv[2]))
' $sock $listener_lifetime_secs &
set -l listener_pid $last_pid

for i in (seq 1 20)
    test -S $sock; and break
    sleep 0.1
end

function _ipc_nc_fallback_cleanup --argument-names listener_pid test_tmp_dir
    kill -9 $listener_pid 2>/dev/null
    rm -rf $test_tmp_dir
end

if not test -S $sock
    _ipc_nc_fallback_cleanup $listener_pid $test_tmp_dir
    test_skip "could not create test Unix socket listener"
end

# Restrict PATH to force the no-coreutils-timeout branch in __gpy_ipc_send
# regardless of what the host machine actually has installed.
set -l fake_bin_dir (mktemp -d)
ln -s (command -v nc) $fake_bin_dir/nc
ln -s (command -v head) $fake_bin_dir/head

set -gx GPY_AGENT_SOCKET_PATH $sock
set -l saved_path $PATH

set -l start_ns ($real_date +%s%N)
set -gx PATH $fake_bin_dir
set -l result (__gpy_ipc_send '{"op":"ping"}' 300)
set -gx PATH $saved_path
set -l end_ns ($real_date +%s%N)

_ipc_nc_fallback_cleanup $listener_pid $test_tmp_dir
functions -e _ipc_nc_fallback_cleanup

set -l elapsed_ms (math "($end_ns - $start_ns) / 1000000")
set -l test_result PASS

# Acceptance criterion: must return within ~1s. The listener self-destructs
# at {$listener_lifetime_secs}s, so a regression that removes `-w` would show
# up as ~{$listener_lifetime_secs}000ms here rather than a real hang.
set -l deadline_ms (math "$listener_lifetime_secs * 1000 - 500")
if test $elapsed_ms -lt $deadline_ms
    print_test_result "nc fallback returns instead of hanging" PASS "{$elapsed_ms}ms"
else
    print_test_result "nc fallback returns instead of hanging" FAIL "took {$elapsed_ms}ms, expected well under {$deadline_ms}ms"
    set test_result FAIL
end

# nc's own exit status on a `-w` idle timeout isn't reliable across
# platforms/implementations (BSD nc on macOS exits 0), so the contract
# callers actually depend on (see __gpy_request_* in ipc.fish) is an empty
# result triggering their oneshot-fallback path, not a specific nonzero status.
if test -n "$result"
    print_test_result "nc fallback produces empty output on silent socket" FAIL "got '$result'"
    set test_result FAIL
else
    print_test_result "nc fallback produces empty output on silent socket" PASS
end

print_test_footer "IPC nc fallback timeout" $test_result

if test "$test_result" = PASS
    exit 0
else
    exit 1
end
