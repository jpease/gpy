#!/usr/bin/env zsh
# tests/zsh/e2e_reregister_after_restart.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# An idle Zsh shell re-registers after the agent restarts (#647 row 2, pins
# #638).
#
# When the agent (re)starts it sends SIGALRM to every PID recorded under
# <runtime root>/shells/, and keeps nudging tracked shells that stay
# unregistered. Zsh never recorded its PID and had no TRAPALRM, so an open
# shell that stayed in one directory stopped receiving SIGUSR1/SIGUSR2 for
# the rest of its life after any restart. With a client registered and
# idle (no keystrokes from here on):
#   (a) `gpy-agent stop` then `gpy-agent start` on the same socket leaves the
#       client registered again within 5 s;
#   (b) a tracked-file edit then repaints the idle prompt with no keystroke
#       (the shell is a live client of the new agent);
#   (c) the shell's PID file exists under the runtime root's shells/ directory
#       while it runs and is gone after `exit`.

ROOT=${0:a:h:h:h}
emulate sh -c ". $ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client zsh "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
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

# --- (a) restart the agent; the shell types nothing -----------------------------
registrations_before="$(grep -c "Registering client: PID=$client_pid," "$GPY_DEBUG_LOG")"
scans_before="$(grep -c "Caching status for .*$(basename "$SHELL_E2E_REPO")" "$GPY_DEBUG_LOG" 2>/dev/null || true)"
: "${scans_before:=0}"
shell_e2e_stop_agent
[ -S "$GPY_AGENT_SOCKET_PATH" ] && fail "socket still present after stop"
# A restarted agent appends to the same debug log.
shell_e2e_start_agent || { fail "agent did not restart"; shell_e2e_dump_transcript; exit 1; }

reregistered() {
    now="$(grep -c "Registering client: PID=$client_pid," "$GPY_DEBUG_LOG")"
    [ "${now:-0}" -gt "${registrations_before:-0}" ]
}
if shell_e2e_poll 5 reregistered; then
    pass "the idle shell re-registered within 5 s of the restart (SIGALRM nudge)"
else
    fail "the idle shell did not re-register within 5 s of the restart"
fi
counted() { "$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep -q 'Registered Clients: 1'; }
if shell_e2e_poll 5 counted; then
    pass "gpy-agent status lists the client again"
else
    fail "gpy-agent status does not list the client: $("$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep Registered)"
fi

# --- (b) a change now reaches the shell as a live client ----------------------------
# Re-registration triggers an initial scan of the repo; edit only after a
# post-restart scan has cached the clean state. Whether the change then
# reaches the shell through the watcher's SIGUSR1 or a request-triggered
# refresh is the agent's business; what must hold is that the re-registered
# shell sees it.
scanned() {
    now="$(grep -c "Caching status for .*$(basename "$SHELL_E2E_REPO")" "$GPY_DEBUG_LOG" 2>/dev/null || true)"
    [ "${now:-0}" -gt "$scans_before" ]
}
shell_e2e_poll 5 scanned || fail "the restarted agent never scanned the repo after re-registration"
cache_file="$(ls "$XDG_CACHE_HOME"/gpy/instant-prompts/*.git.*.ansi 2>/dev/null | head -n 1)"
cp "$cache_file" "$SHELL_E2E_ROOT/before.ansi" 2>/dev/null || true
echo dirty >>"$SHELL_E2E_REPO/tracked.txt"
cache_changed() { [ -n "$cache_file" ] && ! cmp -s "$cache_file" "$SHELL_E2E_ROOT/before.ansi"; }
if shell_e2e_poll 10 cache_changed; then
    pass "the restarted agent rewrote the instant cache for the edit"
else
    fail "the instant cache did not change after the edit"
fi
if shell_e2e_wait_for '✱1' 10 >/dev/null; then
    pass "the idle prompt repainted with the unstaged marker through the restarted agent"
else
    fail "no keystroke-free repaint after the restart"
fi

# --- (c) exit removes the PID file -----------------------------------------------------
shell_e2e_send 'exit\r'
pid_file_gone() { [ ! -f "$shells_dir/$client_pid" ]; }
if shell_e2e_poll 5 pid_file_gone; then
    pass "exit removed the shell's PID file"
else
    fail "PID file $shells_dir/$client_pid survived exit"
fi

if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh re-registers after an agent restart"
