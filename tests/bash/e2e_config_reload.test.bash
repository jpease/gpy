#!/usr/bin/env bash
# tests/bash/e2e_config_reload.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# config.toml edit -> `<pid>.reload` + SIGURG -> Bash's next prompt reloads.
# Flipping show_icons must (a) write the shell's `.reload` flag, (b) show at the
#   NEXT prompt (Bash has no SIGURG trap).

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

# Branch glyph (U+E0A0) built from bytes so editors can't drop it.
glyph="$(printf '\xee\x82\xa0')"
# SGR codes split glyph and name; check the glyph on stripped text.
off="$(shell_e2e_wait_for 'main' 10)" || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
has_glyph() { shell_e2e_transcript 0 | grep -q "$glyph"; }
if shell_e2e_poll 5 has_glyph; then
    pass "with show_icons on the prompt carries the branch glyph"
else
    fail "no branch glyph in the first prompt with show_icons on"
fi
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || fail "client $client_pid never registered"

reload_flag="$XDG_RUNTIME_DIR/gpy/shells/$client_pid.reload"
# The agent's start announcement (`.reload` + `.reregister`) can catch this
# shell's PID file on a fast host; it has run by the time the client
# registered, so drop its flag and let only the edit raise one.
rm -f "$reload_flag"

shell_e2e_text_config '"directory", "git"'

flag_written() { [ -e "$reload_flag" ]; }
if shell_e2e_poll 10 flag_written; then
    pass "the edit wrote the shell's .reload flag"
else
    fail "no .reload flag within 10 s of the edit"
fi

# Readline can't repaint idle; the new PS1 appears after one Enter.
latest_render() { shell_e2e_transcript "$off" | sed 's/❯/\n/g' | grep 'main' | tail -n 1; }
no_glyph_repaint() {
    r="$(latest_render)"
    [ -n "$r" ] && ! printf '%s' "$r" | grep -q "$glyph"
}
if shell_e2e_poll 5 no_glyph_repaint; then
    fail "the idle Bash prompt repainted with no keystroke; docs/user/bash-limitations.md says it cannot -- update the doc and this test"
else
    pass "no keystroke-free repaint in Bash (documented readline limitation)"
fi
shell_e2e_send '\r'
if shell_e2e_poll 5 no_glyph_repaint; then
    pass "the next prompt after Enter renders without the branch glyph"
else
    fail "the prompt after Enter still carries the branch glyph"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "bash config edit reaches the next prompt via the reload doorbell"
