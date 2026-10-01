#!/usr/bin/env bash
# tests/bash/e2e_config_reload.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# config.toml edit -> theme export rewritten -> `<pid>.reload` flag + SIGURG
# doorbell -> Bash re-renders (#647 row 3).
#
# `integration.test.bash` covers the reload doorbell with `__gpy_load_theme`
# stubbed; nothing drove a real Bash client through a real config change.
# With a real agent and a `bash -i` on a pty, flipping `show_icons` in
# config.toml must:
#   (a) rewrite $XDG_CACHE_HOME/gpy/theme-export.bash (mtime and content),
#   (b) show the change at the NEXT prompt: the doorbell handler re-sources the
#       export and re-renders PS1, but readline cannot repaint an idle prompt
#       (docs/user/bash-limitations.md), so one Enter is needed and recorded
#       here as the measured behaviour,
#   (c) leave an export that sources cleanly under `bash -u`.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

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
shell_e2e_spawn_client bash "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
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

export_file="$XDG_CACHE_HOME/gpy/theme-export.bash"
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

# Bash cannot repaint an idle prompt (readline); the re-rendered PS1 is shown
# at the next prompt. Measured: no keystroke-free change within 5 s, then
# one Enter shows the icon-less render.
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

# --- the export sources cleanly under nounset ----------------------------------------
if bash -u -c "source '$export_file'" 2>"$SHELL_E2E_ROOT/nounset.err"; then
    pass "the export sources cleanly under bash -u"
else
    fail "the export fails under nounset: $(cat "$SHELL_E2E_ROOT/nounset.err")"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: bash config edit reaches the next prompt via the reload doorbell"
