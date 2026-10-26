#!/usr/bin/env fish
# A truncated (partial-write) IPC response must never reach the caller as if
# it were complete. An agent that wrote `{"status":"o` and then stalled or
# died mid-write must not have that truncated string handed to
# __gpy_request_character/_duration (raw ANSI, printf'd straight into the
# prompt) — it would corrupt terminal rendering.
#
# The "wedged-mid-write agent" is a stub listener in partial mode: it writes
# a partial (non-newline-terminated) frame, then goes silent. It exits after
# a few seconds so this test has a hard, deterministic upper bound even if the
# client hangs waiting for more bytes.
#
# A second case guards against over-strict validation: a normally-terminated
# complete response must still be returned intact.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "IPC partial-response validation"

if not command -q python3
    test_skip "python3 not installed, cannot simulate a partial-write agent"
end
if not command -q socat; and not command -q nc
    test_skip "neither socat nor nc installed, cannot exercise IPC send"
end

set -l test_tmp_dir (mktemp -d)
test_track_dir $test_tmp_dir

# --- Case 1: partial write then stall must not leak the truncated bytes ---

set -l sock1 $test_tmp_dir/gpy-partial-test.sock
test_stub_listener $sock1 partial --reply '{"status":"o' --lifetime 3

set -gx GPY_AGENT_SOCKET_PATH $sock1
# Well under the listener's stall duration, so the client's own timeout (not
# the listener exiting) is what ends this call.
set -l result (__gpy_ipc_send '{"op":"ping"}' 300)

test_stop_pids $test_stub_pid

if test -n "$result"
    fail "partial write is discarded, not returned" "got '$result'"
else
    pass "partial write is discarded, not returned"
end

# --- Case 2: a complete, newline-terminated response still round-trips ---

set -l sock2 $test_tmp_dir/gpy-complete-test.sock
test_stub_listener $sock2 reply --reply '{"status":"ok"}'

set -gx GPY_AGENT_SOCKET_PATH $sock2
set -l complete_result (__gpy_ipc_send '{"op":"ping"}' 1000)

test_stop_pids $test_stub_pid

if test "$complete_result" = '{"status":"ok"}'
    pass "complete response still round-trips"
else
    fail "complete response still round-trips" "got '$complete_result'"
end

test_finish "IPC partial-response validation"
