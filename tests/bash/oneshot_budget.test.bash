#!/usr/bin/env bash
# tests/bash/oneshot_budget.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #614: a dead daemon must cost at most ONE `gpy-agent oneshot` fork per
# prompt render, and that budget must be enforced with a plain per-render
# shell variable (__gpy_oneshot_used) instead of a predictable
# ${TMPDIR:-/tmp}/.gpy_oneshot_used_$$ marker file. Segments run inside
# $(...) subshells and can't write back to __gpy_render_prompt's scope, so
# each oneshot-consuming request function signals its use via a distinct
# exit code (GPY_SEG_STATUS_ONESHOT); the render loop reads that off `$?`
# and sets __gpy_oneshot_used=1, which later segments' subshells inherit by
# fork.
#
# This test stubs `gpy-agent` on PATH (ahead of any real install) to count
# `oneshot` invocations into a file (a plain counter variable inside the
# stub would not survive the subshell it runs in), points
# GPY_AGENT_SOCKET_PATH at a socket that will never exist (dead daemon), and
# renders a prompt with three segments that would each independently try the
# oneshot fallback (directory, git, duration) plus the always-rendered
# prompt character. Only one of those four attempts may actually fork.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

FAILED=0
check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

TMP_DIR="$(mktemp -d)"
STUB_BIN_DIR="$TMP_DIR/bin"
mkdir -p "$STUB_BIN_DIR"
COUNTER_FILE="$TMP_DIR/oneshot-count"
: >"$COUNTER_FILE"

cat >"$STUB_BIN_DIR/gpy-agent" <<STUB
#!/usr/bin/env bash
if [[ "\$1" == "oneshot" ]]; then
    printf 'x' >>"$COUNTER_FILE"
    printf 'STUBBED-ONESHOT-%s' "\$2"
    exit 0
fi
# theme export (called on __gpy_load_theme) and anything else: emit nothing.
exit 0
STUB
chmod +x "$STUB_BIN_DIR/gpy-agent"

export PATH="$STUB_BIN_DIR:$PATH"
export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_AGENT_SOCKET_PATH="$TMP_DIR/dead-agent.sock"
export XDG_CACHE_HOME="$TMP_DIR/cache"

source bash/gpy.bash

# Deterministic segment list: directory (always renders), git (needs a repo),
# duration (needs a duration above threshold). Character rendering always
# happens too, after the segment loop.
FAKE_REPO="$TMP_DIR/myrepo"
mkdir -p "$FAKE_REPO/.git"
cd "$FAKE_REPO" || exit 1

__enabled_segments="directory git duration"
__duration_threshold_ms=100
__gpy_cmd_duration=5000

__gpy_render_prompt 0 >/dev/null

oneshot_calls="$(wc -c <"$COUNTER_FILE" | tr -d ' ')"
check "at most one oneshot fork per render" 1 "$oneshot_calls"

# No leftover predictable-name marker/flag file under TMPDIR for this PID
# (dotglob: the old marker, .gpy_oneshot_used_$$, is a dotfile).
leftover_count=0
shopt -s nullglob dotglob
for f in "${TMPDIR:-/tmp}"/*gpy*_"$$"*; do
    leftover_count=$((leftover_count + 1))
    echo "  leftover: $f"
done
shopt -u nullglob dotglob
check "no leftover predictable-name marker files" 0 "$leftover_count"

cd "$ROOT" || true
rm -rf "$TMP_DIR"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== oneshot budget test passed ==="
