#!/usr/bin/env zsh
# tests/zsh/ipc_error_reply_refusal.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A reply starting with `{"error"` (a denied cwd such as /etc, a request the
# agent cannot parse) is a permanent refusal: no requester prints it into the
# prompt, a requester with a oneshot fallback takes it, and the git sync
# reports the refusal so the segment neither shows it nor schedules a
# refresh. An empty reply line still means "alive but late": no oneshot.
#
# A stub listener plays the agent; a recording `gpy-agent` stub stands in for
# oneshot.

ROOT=${0:a:h:h:h}
# Shared skip contract: exits 0 locally, fails under CI.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

if ! command -v python3 &>/dev/null; then
    test_skip "python3 not installed, cannot simulate an agent"
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-error-reply.XXXXXX") || exit 1
tmp=${tmp:A}
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$tmp"
}
trap cleanup EXIT
mkdir -p "$tmp/bin" "$tmp/cache"

oneshot_log="$tmp/oneshot.log"
print -r -- '#!/bin/sh
echo "$*" >> "$GPY_TEST_ONESHOT_LOG"
echo ONESHOT_OUTPUT' >"$tmp/bin/gpy-agent"
chmod +x "$tmp/bin/gpy-agent"

error_reply='{"error":"Path validation failed: denied"}'
sock="$tmp/error.sock"
shell_e2e_stub_listener "$sock" reply --reply "$error_reply" --lifetime 60

# run_in_shell BODY: BODY in a subshell with the integration's IPC layer
# loaded against the error-replying stub.
run_in_shell() {
    (
        export GPY_AGENT_SOCKET_PATH="$sock" GPY_AGENT_ENABLED=1 \
            GPY_TEST_ONESHOT_LOG="$oneshot_log" XDG_CACHE_HOME="$tmp/cache" \
            PATH="$tmp/bin:$PATH"
        source zsh/core/constants.zsh
        source zsh/core/renderer.zsh
        source zsh/core/ipc.zsh
        source zsh/segments/clock.zsh
        source zsh/segments/status.zsh
        rehash
        eval "$1"
    ) 2>/dev/null
}

: >"$oneshot_log"
out=$(run_in_shell '__gpy_request directory "$PWD" zsh-prompt "" "" true')
check "a directory error reply is never printed" false \
    "$([[ $out == *error* ]] && echo true || echo false)"
check "a directory error reply takes the oneshot fallback" "ONESHOT_OUTPUT" "$out"
check "the fallback ran oneshot directory" true \
    "$(grep -q '^oneshot directory ' "$oneshot_log" && echo true || echo false)"

: >"$oneshot_log"
out=$(run_in_shell '__gpy_request_character 1 true ""')
check "a character error reply takes the oneshot fallback" "ONESHOT_OUTPUT" "$out"

: >"$oneshot_log"
out=$(run_in_shell '__gpy_request_duration 1500 true "" ""')
check "a duration error reply takes the oneshot fallback" "ONESHOT_OUTPUT" "$out"

out=$(run_in_shell '__clock_format=1; __gpy_segment_clock "" "" true')
check "a clock error reply falls back to the local clock" true \
    "$([[ $out == *'%D{'* && $out != *error* ]] && echo true || echo false)"

out=$(run_in_shell '__status_format=1 __gpy_last_status=0; __gpy_segment_status "" "" true')
check "an exit_status error reply falls back to the local pill" true \
    "$([[ -n $out && $out != *error* ]] && echo true || echo false)"

out=$(run_in_shell '__gpy_sync_data_request git "$PWD" true "" true 500; print -r -- "rc=$?"')
check "a git sync error reply is reported as a refusal, never printed" \
    "rc=5" "$out"

shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

# An empty reply line keeps meaning "alive but late": no oneshot.
sock="$tmp/empty.sock"
shell_e2e_stub_listener "$sock" reply --reply "" --lifetime 60
: >"$oneshot_log"
out=$(run_in_shell '__gpy_request directory "$PWD" zsh-prompt "" "" true')
check "an empty reply line prints nothing" "" "$out"
check "an empty reply line never forks oneshot" "" "$(<"$oneshot_log")"
shell_e2e_stop_pids "$SHELL_E2E_STUB_PID"

test_finish "error replies are permanent refusals"
