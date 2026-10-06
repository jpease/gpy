#!/usr/bin/env zsh
# tests/zsh/e2e_agent_autostart.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A real interactive Zsh, a real agent, a real repository (#646).
#
# Nothing in tests/zsh/ started gpy-agent before this file: the integration
# suite runs with the supervisor off and a socket that does not exist, so
# autostart, registration, the real IPC transport (the zsh/net/socket
# builtin), first-prompt rendering and __gpy_sync_workspace could all regress
# without failing any gate. This drives `zsh -i` on a pseudo-terminal
# (tests/lib/pty_session.py) through tests/lib/shell_e2e.sh and asserts on
# what the terminal shows:
#
#   1. with no agent running, sourcing the integration starts one: the socket
#      appears within 3 s, the client registers as shell "zsh", and the first
#      prompt carries colour, the directory name, and no error text;
#   2. after `false`, the next prompt differs (exit-status colouring);
#   3. after `cd` into the repository the prompt names the branch, and an
#      edit to a tracked file made from outside the shell shows the dirty
#      token on the next prompt;
#   4. the client sent a `workspace` op for the `cd` (pins __gpy_sync_workspace);
#   5. `exit` unregisters the client (zsh/core/signals.zsh's zshexit hook)
#      and produces no error output.
#
# Every wait is a bounded poll; the transcript tail is dumped on failure.

ROOT=${0:a:h:h:h}
# The harness is POSIX sh; keep sh word-splitting for its functions.
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

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
echo "--- 1. autostart from an interactive zsh ---"
[ -S "$GPY_AGENT_SOCKET_PATH" ] && fail "an agent socket already exists before the shell starts"
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_ROOT/home" || exit 1
shell_e2e_spawn_client zsh "$ROOT" || {
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
if grep -q "Registering client: PID=$client_pid, shell=zsh" "$GPY_DEBUG_LOG"; then
    pass "registration names the shell as zsh"
else
    fail "registration did not carry shell=zsh: $(grep "PID=$client_pid" "$GPY_DEBUG_LOG" | head -n 1)"
fi

# The first prompt: wait for the prompt character, then inspect the raw
# bytes (an SGR sequence must be present) and the stripped text.
off1="$(shell_e2e_wait_for '❯' 10)" || fail "no prompt within 10 s"
if tail -c +1 "$SHELL_E2E_SESSION/transcript" | grep -q "$(printf '\033')\["; then
    pass "the first prompt is coloured (SGR escape present)"
else
    fail "no SGR escape in the first prompt"
fi
first="$(shell_e2e_transcript 0)"
case "$first" in
    *home*) pass "the first prompt shows the directory name" ;;
    *) fail "the first prompt does not name the directory: $first" ;;
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

# --- 3. git content: branch, then a dirty token with no keystroke --------------
echo "--- 3. git content ---"
shell_e2e_send "cd $SHELL_E2E_REPO\\r"
if off3="$(shell_e2e_wait_for 'main' 10 "$off2")"; then
    pass "after cd into the repository the prompt names the branch"
else
    fail "the branch name never appeared after cd"
    off3="$(shell_e2e_size)"
fi

echo dirty >>"$SHELL_E2E_REPO/tracked.txt"
shell_e2e_send '\r'
# Unstaged marker in the text theme with icons off: config default `✱` + count.
if shell_e2e_wait_for '✱1' 10 "$off3" >/dev/null; then
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
# The zshexit hook sends `unregister`; the agent's status report drops the
# client from its count.
unregistered() { "$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep -q 'Registered Clients: 0'; }
if shell_e2e_poll 5 unregistered; then
    pass "exit unregistered the client"
else
    fail "the agent still counts a registered client after exit: $("$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep 'Registered Clients')"
fi

if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh autostart to first prompt end to end"
