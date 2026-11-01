#!/usr/bin/env bash
# tests/bash/e2e_exec_survives_doorbell.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A registered Bash shell that runs `exec bash` survives an agent
# notification during the new shell's startup.
# `exec` keeps the PID registered, so the agent may ring SIGURG (default: ignore) before gpy loads.
# The new shell's rc blocks before gpy until released; the PID must survive and run a command.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"directory", "git"' true
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client bash "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
client_pid="$(shell_e2e_client_pid)"
shell_e2e_wait_for 'main' 10 >/dev/null || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || { fail "client $client_pid never registered"; shell_e2e_dump_transcript; exit 1; }

release="$SHELL_E2E_ROOT/release-exec"
exec_rc="$SHELL_E2E_ROOT/exec-bashrc"
printf 'echo "EXEC_STARTED=$$"\nwhile [ ! -e "%s" ]; do sleep 0.05; done\nsource "%s/bash/gpy.bash"\n' \
    "$release" "$ROOT" >"$exec_rc"
off="$(shell_e2e_size)"
shell_e2e_send "exec bash --noprofile --rcfile '$exec_rc' -i\r"
shell_e2e_wait_for "EXEC_STARTED=$client_pid" 10 "$off" >/dev/null || { fail "the exec'd bash never started"; shell_e2e_dump_transcript; exit 1; }
counted() { "$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep -q 'registered_clients: 1'; }
shell_e2e_poll 5 counted || fail "the PID is not registered across exec; the notification cannot reach it"

signals_before="$(grep -c 'to reload via SIGURG' "$GPY_DEBUG_LOG" 2>/dev/null || true)"
shell_e2e_text_config '"directory", "git"'
shells_dir="$XDG_RUNTIME_DIR/gpy/shells"
notified() {
    now="$(grep -c 'to reload via SIGURG' "$GPY_DEBUG_LOG" 2>/dev/null || true)"
    [ -e "$shells_dir/$client_pid.reload" ] && [ "${now:-0}" -gt "${signals_before:-0}" ]
}
if shell_e2e_poll 10 notified; then
    pass "the agent left a reload flag and rang SIGURG during the exec'd shell's startup"
else
    fail "the agent did not notify the exec'd shell within 10 s"
fi

if kill -0 "$client_pid" 2>/dev/null; then
    pass "PID $client_pid is still alive after the notification"
else
    fail "PID $client_pid died from the agent's notification"
fi
: >"$release"
off="$(shell_e2e_size)"
# shellcheck disable=SC2016  # $((...)) must reach the pty unexpanded so the shell under test computes it
shell_e2e_send 'echo "ALIVE_$((40 + 2))"\r'
if shell_e2e_wait_for 'ALIVE_42' 10 "$off" >/dev/null; then
    pass "the exec'd shell finished loading gpy and runs commands"
else
    fail "the exec'd shell does not respond after the notification"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "bash survives an agent notification during exec startup"
