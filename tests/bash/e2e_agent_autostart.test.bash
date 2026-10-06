#!/usr/bin/env bash
# tests/bash/e2e_agent_autostart.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A real interactive Bash, a real agent, a real repository (#646).
#
# Nothing in tests/bash/ started gpy-agent before this file: the integration
# suite runs with the supervisor off and a socket that does not exist, so
# autostart, registration, the real IPC transport, first-prompt rendering and
# __gpy_sync_workspace could all regress without failing any gate. This
# drives `bash -i` on a pseudo-terminal (tests/lib/pty_session.py) through
# tests/lib/shell_e2e.sh and asserts on what the terminal shows:
#
#   1. with no agent running, sourcing the integration starts one: the socket
#      appears within 3 s, the client registers as shell "bash", and the first
#      prompt carries colour, `~` for $HOME, and no error text;
#   2. after `false`, the next prompt differs (exit-status colouring);
#   3. after `cd` into the repository the prompt names the branch, and an
#      edit to a tracked file made from outside the shell shows the dirty
#      token on the next prompt;
#   4. the client sent a `workspace` op for the `cd` (pins __gpy_sync_workspace);
#   5. `exit` produces no error output.
#
# Every wait is a bounded poll; the transcript tail is dumped on failure.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}
pass() {
    echo "PASS: $*"
}

# The text theme renders plain tokens the assertions can name (no glyphs).
printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"

# --- 1. autostart, registration, first prompt --------------------------------
echo "--- 1. autostart from an interactive bash ---"
[ -S "$GPY_AGENT_SOCKET_PATH" ] && fail "an agent socket already exists before the shell starts"
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_ROOT/home" || exit 1
shell_e2e_spawn_client bash "$ROOT" || {
    shell_e2e_dump_transcript
    exit 1
}
client_pid="$(shell_e2e_client_pid)"

socket_exists() { [ -S "$GPY_AGENT_SOCKET_PATH" ]; }
if shell_e2e_poll 3 socket_exists; then
    pass "the agent socket appeared within 3 s of the shell starting"
else
    fail "no agent socket within 3 s (autostart)"
fi

registered() { shell_e2e_assert_registered "$client_pid"; }
if shell_e2e_poll 5 registered; then
    pass "client $client_pid registered with the agent"
else
    fail "client $client_pid never registered"
fi
if grep -q "Registering client: PID=$client_pid, shell=bash" "$GPY_DEBUG_LOG"; then
    pass "registration names the shell as bash"
else
    fail "registration did not carry shell=bash: $(grep "PID=$client_pid" "$GPY_DEBUG_LOG" | head -n 1)"
fi

# The first prompt: wait for the prompt character, then inspect the raw
# bytes (an SGR sequence must be present) and the stripped text.
off1="$(shell_e2e_wait_for '❯' 10)" || fail "no prompt within 10 s"
if tail -c +1 "$SHELL_E2E_SESSION/transcript" | grep -q $'\x1b\\['; then
    pass "the first prompt is coloured (SGR escape present)"
else
    fail "no SGR escape in the first prompt"
fi
# The shell starts in $HOME, so the directory segment contracts it to `~`
# (the logical path the shell reports, #697).
first="$(shell_e2e_transcript 0)"
case "$first" in
    *'~'*) pass "the first prompt shows the home directory as ~" ;;
    *) fail "the first prompt does not show ~ for \$HOME: $first" ;;
esac
for bad in 'nohup:' 'gpy[' 'Error' 'command not found'; do
    case "$first" in
        *"$bad"*) fail "the first prompt carries '$bad': $first" ;;
    esac
done

# --- 2. exit status changes the prompt ------------------------------------------
echo "--- 2. exit status is reflected ---"
raw_before="$(tail -c 200 "$SHELL_E2E_SESSION/transcript" | tr -d '\n')"
shell_e2e_send 'false\r'
off2="$(shell_e2e_wait_for '❯' 5 "$off1")" || fail "no prompt after false"
raw_after="$(tail -c 200 "$SHELL_E2E_SESSION/transcript" | tr -d '\n')"
if [ "$raw_before" != "$raw_after" ]; then
    pass "the prompt after a failing command differs from the one before"
else
    fail "the prompt after false is byte-identical to the previous one"
fi

# --- 3. git content: branch, then a dirty token on the next render -------------
echo "--- 3. git content ---"
shell_e2e_send "cd $SHELL_E2E_REPO\\r"
if off3="$(shell_e2e_wait_for 'main' 10 "$off2")"; then
    pass "after cd into the repository the prompt names the branch"
else
    fail "the branch name never appeared after cd"
    off3="$(shell_e2e_size)"
fi

echo dirty >>"$SHELL_E2E_REPO/tracked.txt"
# gpy-agent writes the instant-prompt cache before it signals at all (the
# watcher event lands, the cache is refreshed, *then* SIGURG goes out), so
# the data is ready within milliseconds of the edit. What is NOT available is
# Bash showing it while it sits idle in readline: Bash ignores the SIGURG (no
# trap, #678), and readline cannot repaint an idle prompt anyway.
# __gpy_precmd renders synchronously off already-fresh data on every
# keystroke-driven prompt, so poll by sending real newlines instead of
# waiting on the async push -- each one drives an ordinary render that reads
# whatever the cache currently holds, converging as soon as the watcher has
# caught up (usually the first or second newline) and bounded by
# shell_e2e_timeout_scale like every other wait here.
_dirty_scale="$(shell_e2e_timeout_scale)"
_dirty_attempts=$(( 20 * _dirty_scale ))  # one newline every 0.5 s -> 10 s * scale
_dirty_seen=""
while [ "$_dirty_attempts" -gt 0 ]; do
    shell_e2e_send '\r'
    sleep 0.5
    case "$(shell_e2e_transcript "$off3")" in
        *'✱1'*) _dirty_seen=1; break ;;
    esac
    _dirty_attempts=$(( _dirty_attempts - 1 ))
done
if [ -n "$_dirty_seen" ]; then
    pass "the prompt after the edit shows the unstaged marker"
else
    fail "no unstaged marker after editing a tracked file"
fi

# --- 4. workspace sync ---------------------------------------------------------------
echo "--- 4. workspace op ---"
# A workspace op re-registers the PID's watch at its new cwd, which the
# watcher logs as `attach_pid_to: pid=<pid>, cwd=<repo>` (#718); the first
# registration logged the home directory, so a line naming the repo can only
# come from the workspace update.
if grep -q "attach_pid_to: pid=$client_pid, cwd=.*$(basename "$SHELL_E2E_REPO")" "$GPY_DEBUG_LOG" 2>/dev/null; then
    pass "a workspace op for PID $client_pid moved its watch to the repository"
else
    fail "no workspace update for PID $client_pid in the agent log"
fi

# --- 5. exit --------------------------------------------------------------------------
echo "--- 5. exit ---"
off5="$(shell_e2e_size)"
shell_e2e_send 'exit\r'
exited() { [ -f "$SHELL_E2E_SESSION/exit" ]; }
if shell_e2e_poll 5 exited; then
    pass "the shell exited"
else
    fail "the shell did not exit within 5 s"
fi
tail_text="$(shell_e2e_transcript "$off5")"
case "$tail_text" in
    *rror*|*"gpy["*) fail "error output on exit: $tail_text" ;;
    *) pass "no error output on exit" ;;
esac

if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: bash autostart to first prompt end to end"
