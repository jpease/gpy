#!/usr/bin/env bash
# tests/bash/e2e_supervisor_settings_reload.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# `gpy config set agent.supervisor.enabled` reaches an open Bash shell without
# a restart: the `<pid>.reload` doorbell is consumed by the next prompt, and
# `gpy prompt` reads the setting on every prompt. With it off, a stopped
# agent stays down across prompts; turned back on (the agent down, so no
# doorbell), the next prompt restarts it.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"directory", "git"'
printf '\n[agent.supervisor]\nenabled = true\nmax_restart_attempts = 5\n' \
    >>"$XDG_CONFIG_HOME/gpy/config.toml"
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
# Every prompt is due a supervisor check (the config floor is 5 s).
export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client bash "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
client_pid="$(shell_e2e_client_pid)"
shell_e2e_wait_for 'main' 10 >/dev/null || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || fail "client $client_pid never registered"

reload_flag="$XDG_RUNTIME_DIR/gpy/shells/$client_pid.reload"
flag_written() { [ -e "$reload_flag" ]; }
flag_consumed() { [ ! -e "$reload_flag" ]; }
agent_down() { ! shell_e2e_agent_responding; }

if ! gpy config set agent.supervisor.enabled false >"$SHELL_E2E_ROOT/config-set.log" 2>&1; then
    fail "gpy config set agent.supervisor.enabled false failed: $(cat "$SHELL_E2E_ROOT/config-set.log")"
fi
if shell_e2e_poll 5 flag_written; then
    pass "the setting rang the reload doorbell"
else
    fail "no .reload flag after setting agent.supervisor.enabled"
fi
shell_e2e_send '\r'
if shell_e2e_poll 5 flag_consumed; then
    pass "the next prompt consumed the .reload flag"
else
    fail "the shell left the .reload flag"
fi

shell_e2e_stop_agent
shell_e2e_send '\r'
shell_e2e_send '\r'
# Absence cannot be polled for: give a wrong restart a fixed window.
sleep 1.5
if agent_down; then
    pass "with the supervisor off, prompts leave a stopped agent down"
else
    fail "the agent was restarted with agent.supervisor.enabled = false"
fi

if ! gpy config set agent.supervisor.enabled true >"$SHELL_E2E_ROOT/config-set.log" 2>&1; then
    fail "gpy config set agent.supervisor.enabled true failed: $(cat "$SHELL_E2E_ROOT/config-set.log")"
fi
shell_e2e_send '\r'
if shell_e2e_poll 8 shell_e2e_agent_responding; then
    pass "with the supervisor back on, the next prompt restarted the agent"
else
    fail "the agent stayed down after agent.supervisor.enabled = true"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "supervisor settings reach an open Bash shell"
