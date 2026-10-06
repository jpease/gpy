#!/usr/bin/env zsh
# tests/zsh/e2e_exec_survives_doorbell.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A registered Zsh shell that runs `exec zsh` survives an agent
# notification during the new shell's startup (#674).
#
# `exec` keeps the PID, and zshexit hooks do not run on exec, so the PID
# stays registered with the agent. A notification that arrives before the new
# shell has sourced gpy used to be SIGUSR-style and killed it (default
# disposition: terminate), closing the terminal pane. The agent now only
# rings SIGURG, whose default disposition is ignore, and leaves the meaning
# in a `<pid>.reload` / `<pid>.reregister` flag file. With a real agent and a
# `zsh -i` on a pty:
#   (a) after `exec zsh`, the new shell's rc blocks before loading gpy until
#       the test releases it, so the notification lands deterministically
#       while no handler is installed;
#   (b) a config edit makes the agent write `<pid>.reload` and ring SIGURG to
#       that still-registered PID;
#   (c) the same PID is still alive afterwards, finishes loading gpy, and
#       runs a command typed into it.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"
failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

config="$XDG_CONFIG_HOME/gpy/config.toml"
printf '[ui]\nshow_icons = true\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' >"$config"
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client zsh "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
client_pid="$(shell_e2e_client_pid)"
shell_e2e_wait_for 'main' 10 >/dev/null || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || { fail "client $client_pid never registered"; shell_e2e_dump_transcript; exit 1; }

# --- (a) exec into a bash whose startup waits for the test --------------------
release="$SHELL_E2E_ROOT/release-exec"
exec_dir="$SHELL_E2E_ROOT/exec-zdotdir"
mkdir -p "$exec_dir"
exec_rc="$exec_dir/.zshrc"
printf 'echo "EXEC_STARTED=$$"\nwhile [ ! -e "%s" ]; do sleep 0.05; done\nsource "%s/zsh/gpy.zsh"\n' \
    "$release" "$ROOT" >"$exec_rc"
off="$(shell_e2e_size)"
shell_e2e_send "exec env ZDOTDIR='$exec_dir' zsh -i\r"
shell_e2e_wait_for "EXEC_STARTED=$client_pid" 10 "$off" >/dev/null || { fail "the exec'd zsh never started"; shell_e2e_dump_transcript; exit 1; }
counted() { "$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep -q 'Registered Clients: 1'; }
shell_e2e_poll 5 counted || fail "the PID is not registered across exec; the notification cannot reach it"

# --- (b) the agent notifies the PID while the new shell has no handler -------
signals_before="$(grep -c 'to reload via SIGURG' "$GPY_DEBUG_LOG" 2>/dev/null || true)"
printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' >"$config"
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

# --- (c) the shell survived ----------------------------------------------------
if kill -0 "$client_pid" 2>/dev/null; then
    pass "PID $client_pid is still alive after the notification"
else
    fail "PID $client_pid died from the agent's notification"
fi
: >"$release"
off="$(shell_e2e_size)"
shell_e2e_send 'echo "ALIVE_$((40 + 2))"\r'
if shell_e2e_wait_for 'ALIVE_42' 10 "$off" >/dev/null; then
    pass "the exec'd shell finished loading gpy and runs commands"
else
    fail "the exec'd shell does not respond after the notification"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh survives an agent notification during exec startup"
