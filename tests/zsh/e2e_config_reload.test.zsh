#!/usr/bin/env zsh
# tests/zsh/e2e_config_reload.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# config.toml edit -> agent `<pid>.reload` flag + SIGURG doorbell -> Zsh
# repaints through `gpy prompt zsh`, which reads the new config.
#
# This drives a real Zsh client through a real config change. With a real
# agent and a `zsh -i` on a pty, flipping `show_icons` in config.toml must
# repaint the idle prompt with no keystroke: the branch glyph the git segment
# shows with icons on disappears.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"

config="$XDG_CONFIG_HOME/gpy/config.toml"
shell_e2e_text_config '"directory", "git"' true
shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client zsh "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
client_pid="$(shell_e2e_client_pid)"

# The git branch glyph (U+E0A0) built from bytes so no editor or transport
# can drop the private-use character from this file.
glyph="$(printf '\xee\x82\xa0')"
# The raw transcript carries SGR codes between the glyph and the branch
# name, so wait for the branch and check for the glyph on the stripped text.
off="$(shell_e2e_wait_for 'main' 10)" || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
has_glyph() { shell_e2e_transcript 0 | grep -q "$glyph"; }
if shell_e2e_poll 5 has_glyph; then
    pass "with show_icons on the prompt carries the branch glyph"
else
    fail "no branch glyph in the first prompt with show_icons on"
fi
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || fail "client $client_pid never registered"

# --- the edit ----------------------------------------------------------------------
shell_e2e_text_config '"directory", "git"'

# Repaints redraw on the same line, so split the stripped transcript on the
# prompt character and look at the most recent render that names the branch.
latest_render() { shell_e2e_transcript "$off" | sed 's/❯/\n/g' | grep 'main' | tail -n 1; }
no_glyph_repaint() {
    r="$(latest_render)"
    [ -n "$r" ] && ! printf '%s' "$r" | grep -q "$glyph"
}
if shell_e2e_poll 10 no_glyph_repaint; then
    pass "the idle prompt repainted without the branch glyph, no keystroke sent"
else
    fail "no keystroke-free repaint without the glyph within 10 s"
fi
shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "zsh config edit reaches the prompt via the reload doorbell"
