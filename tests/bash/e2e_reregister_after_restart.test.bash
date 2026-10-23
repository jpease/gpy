#!/usr/bin/env bash
# tests/bash/e2e_reregister_after_restart.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A Bash shell re-registers after the agent restarts, at its next prompt.
# (a) stop/start leaves `<pid>.reregister`; one Enter re-registers and shows the
# restarted agent's config (the directory memo is dropped). (b) edits then reach
# the shell. (c) the PID file under shells/ exists while running, gone after `exit`.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"directory", "git"'
shell_e2e_truncated_directory_config OLD/
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client bash "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
client_pid="$(shell_e2e_client_pid)"
shell_e2e_wait_for 'main' 10 >/dev/null || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }

registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || fail "client $client_pid never registered the first time"

shells_dir="$XDG_RUNTIME_DIR/gpy/shells"
if [ -f "$shells_dir/$client_pid" ]; then
    pass "the shell recorded its PID under $shells_dir for the restart nudge"
else
    fail "no PID file for $client_pid under $shells_dir"
fi

registrations_before="$(grep -c "Registering client: PID=$client_pid," "$GPY_DEBUG_LOG")"
scans_before="$(grep -c "Caching status for .*$(basename "$SHELL_E2E_REPO")" "$GPY_DEBUG_LOG" 2>/dev/null || true)"
: "${scans_before:=0}"
shell_e2e_stop_agent
[ -S "$GPY_AGENT_SOCKET_PATH" ] && fail "socket still present after stop"
shell_e2e_text_config '"directory", "git"'
shell_e2e_truncated_directory_config TRUNC/
# A restarted agent appends to the same debug log.
log_lines_before_restart="$(wc -l <"$GPY_DEBUG_LOG")"
shell_e2e_start_agent || { fail "agent did not restart"; shell_e2e_dump_transcript; exit 1; }

# Wait for this agent's own announcement, not just any flag: the first agent
# also announces at start, and on a slow host that can land after the shell
# began tracking itself, leaving a flag that Enter would consume before the
# restarted agent writes its own, which then outlives the re-registration.
announced() {
    tail -n "+$((log_lines_before_restart + 1))" "$GPY_DEBUG_LOG" |
        grep -q "Announcing agent start to tracked shell $client_pid "
}
# The log line precedes the flag write, so poll both.
flag_written() { announced && [ -e "$shells_dir/$client_pid.reregister" ]; }
if shell_e2e_poll 5 flag_written; then
    pass "the restarted agent left a .reregister flag for the shell"
else
    fail "no .reregister flag for $client_pid after the restart"
fi
off_restart="$(shell_e2e_size)"
shell_e2e_send '\r'
reregistered() {
    now="$(grep -c "Registering client: PID=$client_pid," "$GPY_DEBUG_LOG")"
    [ "${now:-0}" -gt "${registrations_before:-0}" ]
}
if shell_e2e_poll 5 reregistered; then
    pass "the shell re-registered at its next prompt after the restart"
else
    fail "the shell did not re-register within 5 s of the next prompt"
fi
flag_consumed() { [ ! -e "$shells_dir/$client_pid.reregister" ]; }
if shell_e2e_poll 5 flag_consumed; then
    pass "the shell consumed its .reregister flag"
else
    fail "the .reregister flag is still present after re-registration"
fi
counted() { "$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep -q 'Registered Clients: 1'; }
if shell_e2e_poll 5 counted; then
    pass "gpy-agent status lists the client again"
else
    fail "gpy-agent status does not list the client: $("$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep Registered)"
fi
if shell_e2e_wait_for 'TRUNC/' 5 "$off_restart" >/dev/null; then
    pass "the next prompt shows the restarted agent's directory render"
else
    fail "the prompt kept the memoized directory render after the restart"
fi

# --- (b) a change now reaches the shell as a live client ----------------------------
# Edit only after a post-restart scan has cached the clean state.
scanned() {
    now="$(grep -c "Caching status for .*$(basename "$SHELL_E2E_REPO")" "$GPY_DEBUG_LOG" 2>/dev/null || true)"
    [ "${now:-0}" -gt "$scans_before" ]
}
shell_e2e_poll 5 scanned || fail "the restarted agent never scanned the repo after re-registration"
# Digest only `.none` files: `{token}` files depend on queries and are pruned at restart.
git_digest() { find "$XDG_CACHE_HOME/gpy/instant-prompts" -name '*.git*.none.bash' -exec cat {} + 2>/dev/null | cksum; }
digest_before="$(git_digest)"
echo dirty >>"$SHELL_E2E_REPO/tracked.txt"
cache_changed() { [ "$(git_digest)" != "$digest_before" ]; }
if shell_e2e_poll 10 cache_changed; then
    pass "the restarted agent rewrote the instant cache for the edit"
else
    fail "the instant cache did not change after the edit"
fi
off="$(shell_e2e_size)"
shell_e2e_send '\r'
if shell_e2e_wait_for '✱1' 5 "$off" >/dev/null; then
    pass "the next prompt shows the unstaged marker through the restarted agent"
else
    fail "the prompt after Enter does not show the unstaged marker"
fi

shell_e2e_send 'exit\r'
pid_file_gone() { [ ! -f "$shells_dir/$client_pid" ]; }
if shell_e2e_poll 5 pid_file_gone; then
    pass "exit removed the shell's PID file"
else
    fail "PID file $shells_dir/$client_pid survived exit"
fi

if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "bash re-registers after an agent restart"
