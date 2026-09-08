#!/usr/bin/env zsh
# tests/zsh/e2e_config_sigusr2_reload.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# config.toml edit -> theme export rewritten -> SIGUSR2 -> Zsh repaints
# (#647 row 3).
#
# `integration.test.zsh` covers `TRAPUSR2` with `__gpy_load_theme` stubbed;
# nothing drove a real Zsh client through a real config change. With a real
# agent and a `zsh -i` on a pty, flipping `show_icons` in config.toml must:
#   (a) rewrite $XDG_CACHE_HOME/gpy/theme-export.zsh (mtime and content),
#   (b) repaint the idle prompt with no keystroke -- the branch glyph the
#       git segment shows with icons on disappears,
#   (c) leave an export that sources cleanly under `zsh -o nounset`.

ROOT=${0:a:h:h:h}
emulate sh -c ". $ROOT/tests/lib/shell_e2e.sh"

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

export_file="$XDG_CACHE_HOME/gpy/theme-export.zsh"
[ -f "$export_file" ] || fail "no theme export at $export_file"
before_mtime="$(stat -f %m "$export_file" 2>/dev/null || stat -c %Y "$export_file")"
grep -q 'GPY_UI_SHOW_ICONS="1"' "$export_file" || fail "export does not carry show_icons=1 before the edit"

# --- the edit ----------------------------------------------------------------------
printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' >"$config"

export_rewritten() {
    now="$(stat -f %m "$export_file" 2>/dev/null || stat -c %Y "$export_file")"
    [ "$now" != "$before_mtime" ] && grep -q 'GPY_UI_SHOW_ICONS="0"' "$export_file"
}
if shell_e2e_poll 10 export_rewritten; then
    pass "the theme export was rewritten with show_icons=0"
else
    fail "the theme export was not rewritten within 10 s"
fi

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

# --- the export sources cleanly under nounset ----------------------------------------
if zsh -o nounset -c "source '$export_file'" 2>"$SHELL_E2E_ROOT/nounset.err"; then
    pass "the export sources cleanly under zsh -o nounset"
else
    fail "the export fails under nounset: $(cat "$SHELL_E2E_ROOT/nounset.err")"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh config edit reaches the prompt via SIGUSR2"
