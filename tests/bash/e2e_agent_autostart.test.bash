#!/usr/bin/env bash
# tests/bash/e2e_agent_autostart.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A real interactive Bash, a real agent, a real repository.
# Real `bash -i` on a pty + real agent: autostart, registration, IPC, git, workspace sync, unregister.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

# The text theme renders plain tokens the assertions can name (no glyphs).
shell_e2e_text_config '"directory", "git"'

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

# First prompt: raw bytes must carry SGR; stripped text is inspected.
off1="$(shell_e2e_wait_for '❯' 10)" || fail "no prompt within 10 s"
if tail -c +1 "$SHELL_E2E_SESSION/transcript" | grep -q $'\x1b\\['; then
    pass "the first prompt is coloured (SGR escape present)"
else
    fail "no SGR escape in the first prompt"
fi
# Shell starts in $HOME, so the directory segment shows `~`.
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

echo "--- 3. git content ---"
shell_e2e_send "cd $SHELL_E2E_REPO\\r"
if off3="$(shell_e2e_wait_for 'main' 10 "$off2")"; then
    pass "after cd into the repository the prompt names the branch"
else
    fail "the branch name never appeared after cd"
    off3="$(shell_e2e_size)"
fi

echo dirty >>"$SHELL_E2E_REPO/tracked.txt"
# Bash has no SIGURG trap and readline can't repaint idle; poll with newlines (each renders fresh cache).
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

echo "--- 4. workspace op ---"
# A workspace op logs `attach_pid_to: ... cwd=<repo>`; registration logged $HOME.
if grep -q "attach_pid_to: pid=$client_pid, cwd=.*$(basename "$SHELL_E2E_REPO")" "$GPY_DEBUG_LOG" 2>/dev/null; then
    pass "a workspace op for PID $client_pid moved its watch to the repository"
else
    fail "no workspace update for PID $client_pid in the agent log"
fi

echo "--- 5. exit ---"
off5="$(shell_e2e_size)"
shell_e2e_send 'exit\r'
exited() { [ -f "$SHELL_E2E_SESSION/exit" ]; }
if shell_e2e_poll 5 exited; then
    pass "the shell exited"
else
    fail "the shell did not exit within 5 s"
fi
# The EXIT trap sends `unregister`; status drops the client.
unregistered() { "$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep -q 'Registered Clients: 0'; }
if shell_e2e_poll 5 unregistered; then
    pass "exit unregistered the client"
else
    fail "the agent still counts a registered client after exit: $("$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep 'Registered Clients')"
fi
tail_text="$(shell_e2e_transcript "$off5")"
case "$tail_text" in
    *rror*|*"gpy["*) fail "error output on exit: $tail_text" ;;
    *) pass "no error output on exit" ;;
esac

if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "bash autostart to first prompt end to end"
