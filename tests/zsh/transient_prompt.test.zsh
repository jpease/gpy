#!/usr/bin/env zsh
# tests/zsh/transient_prompt.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# On Enter, zle-line-finish swaps PROMPT for the transient line `gpy prompt
# zsh` printed after 0x1F and redraws, so that is what stays in scrollback:
#   - ui.transient_prompt on, with the clock: the time and the symbol, no
#     other segment;
#   - on, no clock: the symbol alone;
#   - off, with the clock: the full prompt again (its time is the one Enter
#     was pressed);
#   - off, no clock: nothing is redrawn.
# The binary's exact transient bytes are pinned by the prompt goldens; this
# drives a real `zsh -i` on a pty and asserts on what the terminal received
# between Enter and the command's output. No agent.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
test_require_command gpy

shell_e2e_init "$ROOT"
export GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0
workdir="$SHELL_E2E_ROOT/tdirname"
mkdir -p "$workdir"
cd "$workdir" || exit 1

# run_case LABEL SEGMENTS TRANSIENT: spawn a client with that config, submit
# one command and set SUBMITTED to what was drawn from Enter to the output.
SUBMITTED=""
case_n=0
run_case() {
    local label=$1 off
    case_n=$((case_n + 1))
    shell_e2e_text_config "$2"
    printf 'transient_prompt = %s\n\n[agent]\nenabled = false\n' "$3" >>"$XDG_CONFIG_HOME/gpy/config.toml"
    shell_e2e_use_session "$case_n"
    shell_e2e_spawn_client zsh "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
    off=$(shell_e2e_wait_for '❯' 10) || { fail "$label: no prompt"; return; }
    shell_e2e_send "echo GPY_MARK_$case_n"
    off=$(shell_e2e_wait_for "GPY_MARK_$case_n" 5 "$off") || { fail "$label: no echo"; return; }
    shell_e2e_send '\r'
    shell_e2e_wait_for "\nGPY_MARK_$case_n" 5 "$off" >/dev/null || { fail "$label: no output"; return; }
    SUBMITTED=$(shell_e2e_transcript "$off")
    SUBMITTED=${SUBMITTED%%$'\n'GPY_MARK_$case_n*}
    shell_e2e_send 'exit\r'
    shell_e2e_stop_client
}

# A clock time as the text theme draws it.
time_re='[0-9]{1,2}:[0-9]{2}'

run_case "on, clock" '"clock", "directory"' true
if [[ $SUBMITTED =~ $time_re && $SUBMITTED == *"❯"* && $SUBMITTED != *tdirname* ]]; then
    pass "on, clock: the time and the symbol replace the prompt"
else
    fail "on, clock: unexpected redraw [$SUBMITTED]"
fi

run_case "on, no clock" '"directory"' true
if [[ $SUBMITTED == *"❯"* && $SUBMITTED != *tdirname* ]] && ! [[ $SUBMITTED =~ $time_re ]]; then
    pass "on, no clock: the symbol alone replaces the prompt"
else
    fail "on, no clock: unexpected redraw [$SUBMITTED]"
fi

run_case "off, clock" '"clock", "directory"' false
if [[ $SUBMITTED =~ $time_re && $SUBMITTED == *tdirname* && $SUBMITTED == *"❯"* ]]; then
    pass "off, clock: the full prompt is redrawn"
else
    fail "off, clock: unexpected redraw [$SUBMITTED]"
fi

run_case "off, no clock" '"directory"' false
if [[ $SUBMITTED != *"❯"* && $SUBMITTED != *tdirname* ]]; then
    pass "off, no clock: nothing is redrawn"
else
    fail "off, no clock: unexpected redraw [$SUBMITTED]"
fi

test_finish "transient prompt"
