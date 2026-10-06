#!/usr/bin/env zsh
# tests/zsh/e2e_git_live_content.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Live git content in an idle Zsh prompt (#647 row 1, pins #637).
#
# The Zsh repaint handler used to be `zle reset-prompt` alone; PROMPT is a
# string rendered once per precmd, so the redraw showed the same bytes and a
# git change could not appear until the next Enter, while the docs called
# Zsh live updates "Full". This drives `zsh -i` on a pty against a real
# agent, edits a tracked file from OUTSIDE the shell without `git add`, and
# asserts, with no keystroke sent:
#   (a) the agent's instant-cache entry for the repo changes (so a cache-only
#       update is distinguishable from a repaint),
#   (b) the prompt on screen shows the unstaged marker within 10 s.
# Then one Enter must show the same state (no divergence between the
# repaint and the next real prompt).

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

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

off="$(shell_e2e_wait_for 'main' 10)" || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || fail "client $client_pid never registered"

# The clean-state cache entry the agent wrote for this repo.
cache_file=""
find_cache() {
    cache_file="$(ls "$XDG_CACHE_HOME"/gpy/instant-prompts/*.git.*.zsh 2>/dev/null | head -n 1)"
    [ -n "$cache_file" ]
}
if shell_e2e_poll 5 find_cache; then
    pass "the agent wrote a git instant-cache entry for the repo"
else
    fail "no git instant-cache entry appeared"
fi
cp "$cache_file" "$SHELL_E2E_ROOT/before.ansi" 2>/dev/null || true

# The edit: from this test process, no `git add`, no keystroke.
echo dirty >>"$SHELL_E2E_REPO/tracked.txt"

cache_changed() { ! cmp -s "$cache_file" "$SHELL_E2E_ROOT/before.ansi"; }
if shell_e2e_poll 10 cache_changed; then
    pass "the instant-cache entry changed after the tracked-file edit"
else
    fail "the instant-cache entry did not change within 10 s"
fi

if shell_e2e_wait_for '✱1' 10 "$off" >/dev/null; then
    pass "the idle prompt repainted with the unstaged marker, no keystroke sent"
else
    fail "no keystroke-free repaint with the unstaged marker within 10 s"
fi

off2="$(shell_e2e_size)"
shell_e2e_send '\r'
if shell_e2e_wait_for '✱1' 5 "$off2" >/dev/null; then
    pass "the next real prompt shows the same dirty state"
else
    fail "the prompt after Enter does not show the unstaged marker"
fi

# Nothing the trap printed may leak around the prompt (zsh's `local`
# re-declaration output, for one).
leak="$(shell_e2e_transcript "$off" | grep -c 'seg_out=\|typeset\|local ' || true)"
if [ "${leak:-0}" -eq 0 ]; then
    pass "no shell-internal text leaked around the repainted prompt"
else
    fail "shell-internal text leaked around the prompt: $(shell_e2e_transcript "$off" | grep 'seg_out=\|typeset\|local ' | head -n 2)"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: zsh live git content repaints an idle prompt"
