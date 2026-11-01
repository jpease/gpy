#!/usr/bin/env bash
# tests/bash/e2e_config_reload.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# config.toml edit -> `<pid>.reload` + SIGURG -> Bash's next prompt reloads.
# Flipping show_icons must (a) rewrite theme-export.bash, (b) show at the NEXT prompt
#   (Bash has no SIGURG trap), (c) source cleanly under `bash -u`.

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

export_file="$XDG_CACHE_HOME/gpy/theme-export.bash"
[ -f "$export_file" ] || fail "no theme export at $export_file"
# Content is the evidence of a rewrite; no mtime (`stat` flags differ GNU/BSD).
grep -q '__prompt_icons="nerd"' "$export_file" || fail "export does not carry show_icons=1 before the edit"

shell_e2e_text_config '"directory", "git"'

export_rewritten() {
    grep -q '__prompt_icons="ascii"' "$export_file"
}
if shell_e2e_poll 10 export_rewritten; then
    pass "the theme export was rewritten with show_icons=0"
else
    fail "the theme export was not rewritten within 10 s"
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

if bash -u -c 'source "$1"' bash "$export_file" 2>"$SHELL_E2E_ROOT/nounset.err"; then
    pass "the export sources cleanly under bash -u"
else
    fail "the export fails under nounset: $(cat "$SHELL_E2E_ROOT/nounset.err")"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "bash config edit reaches the next prompt via the reload doorbell"
