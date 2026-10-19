#!/usr/bin/env zsh
# tests/zsh/e2e_doorbell_ring_during_trap.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# An agent start rings an idle shell twice in a row: `.reload` (the theme
# export changed, as on an upgrade) and then `.reregister`. Zsh drops a SIGURG
# that arrives while TRAPURG is still running, so a re-register ring landing
# during the reload went unanswered: the idle shell stayed unregistered, with
# no live repaints, until the agent's next re-nudge a minute later.
#
# With a slow theme export (it sleeps while being sourced), each round writes
# `.reload` and rings, then writes `.reregister` and rings while the reload
# trap is still running. No keystroke is sent, so only TRAPURG can consume
# the flag: it must, every round, and the shell must register again.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"

shell_e2e_text_config '"directory", "git"'
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client zsh "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
client_pid="$(shell_e2e_client_pid)"
shell_e2e_wait_for 'main' 10 >/dev/null || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || fail "client $client_pid never registered"

# Sourcing the export is what a `.reload` does; make it take 0.6 s.
print -r -- 'sleep 0.6' >>"$XDG_CACHE_HOME/gpy/theme-export.zsh"

shells_dir="$XDG_RUNTIME_DIR/gpy/shells"
flag="$shells_dir/$client_pid"
flag_consumed() { [ ! -e "$flag.reregister" ]; }
registrations() { grep -c "Registering client: PID=$client_pid," "$GPY_DEBUG_LOG"; }
before="$(registrations)"
lost=0
for round in 1 2 3; do
    : >"$flag.reload"
    kill -URG "$client_pid"
    sleep 0.2
    : >"$flag.reregister"
    kill -URG "$client_pid"
    if ! shell_e2e_poll 5 flag_consumed; then
        lost=$((lost + 1))
        rm -f "$flag.reregister"
    fi
done
if [ "$lost" -eq 0 ]; then
    pass "a re-register ring that lands during the reload trap is answered (3 of 3)"
else
    fail "$lost of 3 re-register rings during the reload trap went unanswered"
fi
after="$(registrations)"
if [ "$after" -ge $((before + 3)) ]; then
    pass "the idle shell registered again each round"
else
    fail "the idle shell registered $((after - before)) times in 3 rounds"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "zsh answers a doorbell ring that lands during its own trap"
