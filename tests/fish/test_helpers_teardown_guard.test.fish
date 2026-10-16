#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# Teardown guard in tests/lib/test_helpers.fish
#
# A test that ends early -- `exit N`, a failed assertion, SIGINT,
# SIGTERM -- must still stop the agent it started, kill the background
# processes it registered, and remove its scratch directories. Otherwise the
# daemon outlives the run and holds a socket under a directory nobody cleans.
#
# Each case runs a throwaway child test: it calls init_test_env, starts a real
# agent, registers a stub listener (test_track_pid) and a scratch directory
# holding a second stub socket (test_track_dir), reports the PIDs and paths,
# and then ends one of four ways. The parent asserts that every one of those
# is gone afterwards. Only PIDs the child itself started are ever checked or
# killed.

set -g repo_root (realpath (dirname (status -f))/../..)
source $repo_root/tests/lib/test_helpers.fish

command -q lsof; or test_skip "lsof not installed, cannot resolve socket holders"
command -q python3; or test_skip "python3 not installed, cannot run stub listeners"
command -q gpy-agent; or test_skip "gpy-agent not on PATH"

# Short path: unix socket paths are capped near 104 bytes on macOS.
set -g scratch (mktemp -d /tmp/gpy-fguard.XXXXXX)
set -g running_child
set -g report_pids

function __guard_test_cleanup --on-event fish_exit
    test -n "$running_child"; and kill -9 $running_child 2>/dev/null
    for pid in $report_pids
        kill -9 $pid 2>/dev/null
    end
    rm -rf $scratch
end

set -g failures 0
function check --argument-names label ok detail
    if test "$ok" = 1
        print_test_result "$label" PASS
    else
        set -g failures (math $failures + 1)
        print_test_result "$label" FAIL "$detail"
    end
end

# The child test; $argv = out_dir mode.
set -g child_script $scratch/child.fish
printf '%s\n' \
    'set -l out $argv[1]' \
    'set -l mode $argv[2]' \
    'source $argv[3]/tests/lib/test_helpers.fish' \
    'init_test_env; or exit 9' \
    'start_test_agent; or exit 9' \
    'set -l holder_dir (mktemp -d /tmp/gpy-fguard-held.XXXXXX)' \
    'test_track_dir $holder_dir' \
    'python3 -c "import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.listen(1)
time.sleep(120)" $holder_dir/held.sock &' \
    'set -l holder_pid $last_pid' \
    'poll_until 5 test -S $holder_dir/held.sock; or exit 9' \
    'sleep 120 &' \
    'set -l tracked_pid $last_pid' \
    'test_track_pid $tracked_pid' \
    'lsof -t $GPY_AGENT_SOCKET_PATH | head -n 1 >$out/agent_pid' \
    'echo $tracked_pid >$out/tracked_pid' \
    'echo $holder_pid >$out/holder_pid' \
    'echo $holder_dir >$out/holder_dir' \
    'echo $__gpy_test_tmp_dir >$out/sandbox' \
    'touch $out/ready' \
    'poll_until 60 test -e $out/go' \
    'switch $mode' \
    '    case exit3' \
    '        exit 3' \
    '    case normal' \
    '        exit 0' \
    end \
    'while true; sleep 0.1; end' >$child_script

function pid_alive --argument-names pid
    kill -0 $pid 2>/dev/null
end

function all_gone
    for pid in $argv
        pid_alive $pid; and return 1
    end
    return 0
end

# run_case MODE SIGNAL WANT_RC; SIGNAL "" means release the child instead.
# WANT_RC is the child's return code as Python reports it (-N: killed by
# signal N); "" skips the check.
function run_case --argument-names mode sig want_rc
    set -l label "$mode"
    test -n "$sig"; and set label "$mode/SIG$sig"
    set -l out $scratch/case-$mode-$sig
    mkdir -p $out

    # The launcher runs the child fish, records its PID and return code, and
    # starts it with SIGINT at its default (a background job of a
    # non-interactive shell can inherit it ignored, which no handler undoes;
    # a terminal's Ctrl-C would find it default).
    python3 -c 'import signal, subprocess, sys
out = sys.argv[1]
child = subprocess.Popen(sys.argv[2:], preexec_fn=lambda: signal.signal(signal.SIGINT, signal.SIG_DFL))
open(out + "/child_pid", "w").write(str(child.pid))
open(out + "/rc", "w").write(str(child.wait()))' $out fish $child_script $out $mode $repo_root >$out/log 2>&1 &
    poll_until 5 test -s $out/child_pid
    set running_child (cat $out/child_pid)

    if not poll_until 60 test -e $out/ready
        check "$label: the child reached ready" 0 (string collect <$out/log)
        kill -9 $running_child 2>/dev/null
        set running_child
        return
    end
    set -l pids (cat $out/agent_pid) (cat $out/tracked_pid) (cat $out/holder_pid)
    set -l sandbox (cat $out/sandbox)
    set -l holder_dir (cat $out/holder_dir)
    set -a report_pids $pids
    set -l alive_before 1
    for pid in $pids
        pid_alive $pid; or set alive_before 0
    end
    if test $alive_before = 0
        check "$label: the child's processes were alive before teardown" 0 "pids: $pids"
        return
    end

    # The child holds at `ready` until released; a signal case is never
    # released and is signalled instead.
    if test -n "$sig"
        kill -$sig $running_child
    else
        touch $out/go
    end
    poll_until 30 test -s $out/rc
    set running_child

    if test -n "$want_rc"
        set -l got (cat $out/rc 2>/dev/null)
        check "$label: exit status $want_rc preserved" (test "$got" = "$want_rc"; and echo 1; or echo 0) "child return code '$got'"
    end
    check "$label: the agent is stopped" (poll_until 10 all_gone $pids[1]; and echo 1; or echo 0) "agent pid $pids[1] still alive"
    check "$label: the registered process is killed" (poll_until 10 all_gone $pids[2]; and echo 1; or echo 0) "pid $pids[2] still alive"
    check "$label: the registered directory's socket holder is killed" (poll_until 10 all_gone $pids[3]; and echo 1; or echo 0) "pid $pids[3] still alive"
    check "$label: the sandbox is removed" (test -e $sandbox; and echo 0; or echo 1) "$sandbox exists"
    check "$label: the registered directory is removed" (test -e $holder_dir; and echo 0; or echo 1) "$holder_dir exists"
end

run_case normal "" 0
run_case exit3 "" 3
run_case hold TERM -15
run_case hold INT ""

if test $failures -gt 0
    echo "FAILED: $failures assertion(s)"
    exit 1
end
echo "PASS: an exiting, interrupted or terminated test leaves no agent, process or directory behind"
