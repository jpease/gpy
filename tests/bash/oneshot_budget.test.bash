#!/usr/bin/env bash
# tests/bash/oneshot_budget.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A dead daemon costs at most ONE `gpy-agent oneshot` fork per render. Segments
# run in $(...) subshells, so each signals use via GPY_SEG_STATUS_ONESHOT and the
# loop sets __gpy_oneshot_used=1 for later forks. The stub counts calls in a file.

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
# Supervisor off in a sandbox config: the theme export re-sets the flag.
source "$ROOT/tests/lib/supervisor_off.bash" "$TMP_DIR"
export GPY_AGENT_SOCKET_PATH="$TMP_DIR/dead-agent.sock"

source bash/gpy.bash

FAKE_REPO="$TMP_DIR/myrepo"
mkdir -p "$FAKE_REPO/.git"
cd "$FAKE_REPO" || exit 1

__enabled_segments="directory git duration"
__duration_threshold_ms=100
__gpy_cmd_duration=5000

__gpy_render_prompt 0 >/dev/null

oneshot_calls="$(wc -c <"$COUNTER_FILE" | tr -d ' ')"
check "at most one oneshot fork per render" 1 "$oneshot_calls"

# No predictable marker file under TMPDIR (dotglob: it would be a dotfile).
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
