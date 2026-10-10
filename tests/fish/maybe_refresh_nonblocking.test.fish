#!/usr/bin/env fish
# Regression test for #685: fish background git/language refreshes ran in the
# foreground. Fish does not fork for functions, so `some_function … &` runs the
# function synchronously with `$last_pid` unset. `__gpy_maybe_refresh` therefore
# blocked the prompt for the whole IPC round-trip and, with the agent down, the
# language cold miss forked a foreground `gpy-agent oneshot` that spent the
# per-render oneshot budget the directory segment needs.
#
# Case 1: `__gpy_maybe_refresh git|lang` returns at once against a listener
#         that delays its reply by 3 s, and the listener still gets the payload.
# Case 2: agent down, cold-miss language render forks no `gpy-agent` and does
#         not claim the oneshot marker.
# Case 3: agent down, `fish_prompt` with `language directory` in a directory
#         holding only `Cargo.toml` still renders the directory segment.
#
# The listener is a python3 Unix-socket server (model:
# ipc_nc_fallback_timeout.test.fish) that self-destructs, so the worst case is
# bounded without tracking background PIDs.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Fish background refresh is non-blocking (#685)"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a slow agent"
end
if not command -q nc
    test_skip "nc not installed, cannot exercise the IPC client"
end

set -g test_result PASS
function check --argument-names name ok detail
    if test "$ok" = PASS
        print_test_result $name PASS $detail
    else
        print_test_result $name FAIL $detail
        set -g test_result FAIL
    end
end

set -l real_date (command -v date)
set -l tmp (mktemp -d)
set -l tmp (path resolve $tmp)
mkdir -p $tmp/proj $tmp/cache $tmp/tmp
touch $tmp/proj/Cargo.toml
set -gx XDG_CACHE_HOME $tmp/cache

# Agent stub: logs its argv; `oneshot directory` prints DIR, anything else
# prints nothing. Only ever reached through a oneshot fallback.
printf '%s\n' '#!/bin/sh' \
    'echo "$*" >> "$(dirname "$0")/calls.log"' \
    'case "$1 $2" in "oneshot directory") printf DIR;; esac' >$tmp/stub-agent
chmod +x $tmp/stub-agent
set -gx GPY_AGENT_BINARY_PATH $tmp/stub-agent

# --- Case 1: non-blocking against a slow listener ---------------------------
set -l sock $tmp/gpy.sock
python3 -c '
import socket, sys, threading, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
s.listen(8)
s.settimeout(0.2)
log = sys.argv[2]
deadline = time.time() + float(sys.argv[3])
def handle(conn):
    try:
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(4096)
            if not chunk:
                break
            data += chunk
        with open(log, "ab") as f:
            f.write(data)
        time.sleep(3)
        conn.sendall(b"{\"status\":\"ok\"}\n")
    except OSError:
        pass
    finally:
        conn.close()
while time.time() < deadline:
    try:
        conn, _ = s.accept()
    except socket.timeout:
        continue
    threading.Thread(target=handle, args=(conn,), daemon=True).start()
' $sock $tmp/server.log 10 &
set -g listener_pid $last_pid

poll_until 2 test -S $sock
if not test -S $sock
    kill -9 $listener_pid 2>/dev/null
    rm -rf $tmp
    test_skip "could not create test Unix socket listener"
end

# Restrict PATH so the IPC client is nc without coreutils `timeout` or socat
# (stock macOS), whatever the host has installed. That is the path where the
# old foreground send blocked for the full `nc -w 1` second.
set -l fake_bin $tmp/bin
mkdir -p $fake_bin
ln -s (command -v nc) $fake_bin/nc
set -gx GPY_AGENT_SOCKET_PATH $sock
set -g __gpy_registered 1
set -g __gpy_prompt_now ($real_date +%s)
set -l saved_path $PATH

for op in git lang
    rm -f $tmp/server.log
    set -gx PATH $fake_bin
    set -l start_ns (test_now_ns)
    __gpy_maybe_refresh $op $tmp/proj $op "" black ""
    set -l end_ns (test_now_ns)
    set -gx PATH $saved_path
    set -l elapsed_ms (math --scale=0 "($end_ns - $start_ns) / 1000000")

    if test $elapsed_ms -lt 300
        check "maybe_refresh $op returns immediately" PASS "$elapsed_ms"ms
    else
        check "maybe_refresh $op returns immediately" FAIL "took $elapsed_ms"ms", expected < 300ms against a 3s listener"
    end

    if poll_until 2 test -s $tmp/server.log; and string match -q "*\"op\":\"$op\"*" -- (cat $tmp/server.log)
        check "listener receives the $op refresh payload" PASS
    else
        check "listener receives the $op refresh payload" FAIL "server.log: ["(cat $tmp/server.log 2>/dev/null)"]"
    end
end

kill -9 $listener_pid 2>/dev/null
rm -f $sock
set -e GPY_AGENT_SOCKET_PATH
set -e __gpy_registered
set -e __gpy_prompt_now

# --- Case 2: agent down, a language cold miss forks no oneshot -------------
# Point the socket somewhere that never exists so nothing can answer.
set -gx GPY_AGENT_SOCKET_PATH $tmp/absent.sock
source "$repo_root/fish/segments/language.fish"
cd $tmp/proj
rm -f $tmp/calls.log
set -e __gpy_oneshot_used
set -e __gpy_last_segment_bg
set -l lang_out (segment_language_render "" true)

if test -e $tmp/calls.log
    check "cold-miss language render forks no gpy-agent" FAIL "calls: ["(cat $tmp/calls.log)"]"
else
    check "cold-miss language render forks no gpy-agent" PASS
end
if set -q __gpy_oneshot_used
    check "cold-miss language render leaves the oneshot budget unclaimed" FAIL
else
    check "cold-miss language render leaves the oneshot budget unclaimed" PASS
end

# --- Case 3: agent down, the directory segment still renders ---------------
source "$repo_root/fish/core/renderer.fish"
source "$repo_root/fish/segments/directory.fish"
source "$repo_root/fish/functions/fish_prompt.fish"
set -g __gpy_is_root 0
set -g __enabled_segments language directory
set -e __gpy_dir_cache_key
# Fresh throttle so the language refresh runs again this render.
for throttle_var in (set -n | string match '__gpy_lang_*refresh_ms_*')
    set -e $throttle_var
end
set -l prompt_out (fish_prompt 2>/dev/null | string collect)

if string match -q '*DIR*' -- "$prompt_out"
    check "directory segment renders with the agent down" PASS
else
    check "directory segment renders with the agent down" FAIL "prompt: ["(string escape -- "$prompt_out")"]"
end

cd $repo_root
rm -rf $tmp

print_test_footer "Fish background refresh is non-blocking" $test_result
test "$test_result" = PASS
