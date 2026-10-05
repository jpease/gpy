#!/usr/bin/env bash
# tests/bash/e2e_git_live_content.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Live git content and an idle Bash prompt (#647 row 1; measured for #637).
#
# The agent rings Bash's SIGURG doorbell, which Bash ignores (no trap, #678):
# readline has already expanded and drawn the previous prompt and has no
# `reset-prompt`, so fresh data is only shown at the next prompt. This test
# pins what a Bash user actually gets,
# which docs/user/bash-limitations.md states in the same words:
#   (a) the agent's instant-cache entry for the repo changes after an edit
#       made from outside the shell with no `git add` (the agent side works),
#   (b) the agent delivered the SIGURG doorbell to the shell,
#   (c) the prompt on screen does NOT change with no keystroke (measured;
#       if this ever starts passing, update the doc and flip the assertion),
#   (d) the next prompt after Enter shows the unstaged marker.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

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
shell_e2e_spawn_client bash "$ROOT" || { shell_e2e_dump_transcript; exit 1; }
client_pid="$(shell_e2e_client_pid)"

off="$(shell_e2e_wait_for 'main' 10)" || { fail "no branch prompt within 10 s"; shell_e2e_dump_transcript; exit 1; }
registered() { shell_e2e_assert_registered "$client_pid"; }
shell_e2e_poll 5 registered || fail "client $client_pid never registered"

cache_file=""
find_cache() {
    # shellcheck disable=SC2012  # the agent names these files itself; no odd characters
    cache_file="$(ls "$XDG_CACHE_HOME"/gpy/instant-prompts/*.git.*.bash 2>/dev/null | head -n 1)"
    [ -n "$cache_file" ]
}
if shell_e2e_poll 5 find_cache; then
    pass "the agent wrote a git instant-cache entry for the repo"
else
    fail "no git instant-cache entry appeared"
fi
cp "$cache_file" "$SHELL_E2E_ROOT/before.ansi" 2>/dev/null || true
signal_before="$(grep -c 'notifying clients via SIGURG' "$GPY_DEBUG_LOG" 2>/dev/null || true)"

echo dirty >>"$SHELL_E2E_REPO/tracked.txt"

cache_changed() { ! cmp -s "$cache_file" "$SHELL_E2E_ROOT/before.ansi"; }
if shell_e2e_poll 10 cache_changed; then
    pass "the instant-cache entry changed after the tracked-file edit"
else
    fail "the instant-cache entry did not change within 10 s"
fi
signalled() {
    now="$(grep -c 'notifying clients via SIGURG' "$GPY_DEBUG_LOG" 2>/dev/null || true)"
    [ "${now:-0}" -gt "${signal_before:-0}" ]
}
if shell_e2e_poll 10 signalled; then
    pass "the agent sent SIGURG for the change"
else
    fail "the agent never logged a SIGURG notification for the change"
fi

# (c) measured limitation: no keystroke-free repaint in Bash.
if shell_e2e_wait_for '✱1' 5 "$off" >/dev/null 2>&1; then
    fail "the idle Bash prompt repainted with no keystroke; docs/user/bash-limitations.md says it cannot -- update the doc and this test"
else
    pass "the idle Bash prompt did not repaint with no keystroke (documented readline limitation)"
fi

off2="$(shell_e2e_size)"
shell_e2e_send '\r'
if shell_e2e_wait_for '✱1' 5 "$off2" >/dev/null; then
    pass "the next prompt after Enter shows the unstaged marker"
else
    fail "the prompt after Enter does not show the unstaged marker"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: bash live git content reaches the next prompt (no idle repaint, by design)"
