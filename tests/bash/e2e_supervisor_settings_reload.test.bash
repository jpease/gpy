#!/usr/bin/env bash
# tests/bash/e2e_supervisor_settings_reload.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# `gpy config set agent.supervisor.*` -> `<pid>.reload` -> an open Bash shell's next
# prompt re-reads the theme export, so its supervisor check sees the new enable flag,
# check interval and restart-attempt limit without a restart. Each setting renders
# nothing, so each is changed on its own.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

shell_e2e_text_config '"directory", "git"'
printf '\n[agent.supervisor]\nenabled = true\ncheck_interval_seconds = 30\nmax_restart_attempts = 5\n' \
    >>"$XDG_CONFIG_HOME/gpy/config.toml"
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client bash "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
client_pid="$(shell_e2e_client_pid)"
shell_e2e_wait_for 'main' 10 >/dev/null || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || fail "client $client_pid never registered"

reload_flag="$XDG_RUNTIME_DIR/gpy/shells/$client_pid.reload"
flag_written() { [ -e "$reload_flag" ]; }
flag_consumed() { [ ! -e "$reload_flag" ]; }

# Have the shell print the supervisor settings its own check reads, as
# SUP=<enabled>/<interval>/<attempts>, and wait for `$1`.
expect_settings() {
    _off="$(shell_e2e_size)"
    # shellcheck disable=SC2016 # expanded by the client shell, not here
    shell_e2e_send 'echo "SUP=$__gpy_cfg_supervisor_enabled/$__gpy_cfg_supervisor_check_interval/$__gpy_cfg_supervisor_max_restarts"\r'
    shell_e2e_wait_for "SUP=$1" 5 "$_off" >/dev/null
}

if expect_settings '1/30/5'; then
    pass "the shell starts with the configured supervisor settings"
else
    fail "the shell never reported SUP=1/30/5"
fi

# $1 key, $2 value, $3 the SUP= triple the open shell must then report.
apply_setting() {
    if ! gpy config set "$1" "$2" >"$SHELL_E2E_ROOT/config-set.log" 2>&1; then
        fail "gpy config set $1 $2 failed: $(cat "$SHELL_E2E_ROOT/config-set.log")"
        return
    fi
    if shell_e2e_poll 5 flag_written; then
        pass "$1 alone rang the reload doorbell"
    else
        fail "no .reload flag after setting $1 alone"
    fi
    # Bash polls the flag before each prompt; one Enter consumes it.
    shell_e2e_send '\r'
    shell_e2e_poll 5 flag_consumed || fail "the shell left the .reload flag after setting $1"
    if expect_settings "$3"; then
        pass "the open shell now reports SUP=$3 after setting $1"
    else
        fail "the open shell never reported SUP=$3 after setting $1"
    fi
}

apply_setting agent.supervisor.enabled false '0/30/5'
apply_setting agent.supervisor.check_interval_seconds 45 '0/45/5'
apply_setting agent.supervisor.max_restart_attempts 2 '0/45/2'

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "supervisor settings reach an open Bash shell via the reload doorbell"
