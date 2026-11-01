#!/usr/bin/env zsh
# tests/zsh/oneshot_budget.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A dead daemon must cost at most ONE `gpy-agent oneshot` fork per
# prompt render, and that budget must be enforced with a plain per-render
# shell variable (__gpy_oneshot_used) instead of a predictable
# ${TMPDIR:-/tmp}/.gpy_oneshot_used_$$ marker file. __gpy_render_prompt runs
# in the interactive shell, but each segment is invoked via a `$(...)`, so no
# segment can write back to the render's scope directly -- each
# oneshot-consuming request function instead signals its use via a distinct
# exit code (__gpy_seg_status_oneshot), which the render loop reads off `$?`
# and folds into __gpy_oneshot_used, which later segments' subshells inherit
# by fork.
#
# zsh's `(( $+commands[gpy-agent] ))` liveness check (unlike bash's
# `command -v`) only ever sees PATH-hashed external commands, never shell
# functions, so the stub here is a real executable on PATH rather than a
# shadowing function.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

TMP_DIR=$(mktemp -d)
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
source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$TMP_DIR/dead-agent.sock"
export XDG_CACHE_HOME="$TMP_DIR/cache"

source zsh/gpy.zsh

FAKE_REPO="$TMP_DIR/myrepo"
mkdir -p "$FAKE_REPO/.git"
cd "$FAKE_REPO"

__enabled_segments=(directory git duration)
typeset -g __duration_threshold_ms=100
typeset -g __gpy_cmd_duration=5000

__gpy_render_prompt 0; PROMPT=$__gpy_render_out

oneshot_calls=$(wc -c <"$COUNTER_FILE" | tr -d ' ')
check "at most one oneshot fork per render" 1 "$oneshot_calls"

# No leftover predictable-name marker/flag file under TMPDIR for this PID.
leftover_count=0
setopt local_options nullglob dotglob
for f in "${TMPDIR:-/tmp}"/*gpy*_$$*; do
    leftover_count=$((leftover_count + 1))
    echo "  leftover: $f"
done

cd "$ROOT"
rm -rf "$TMP_DIR"

check "no leftover predictable-name marker files" 0 "$leftover_count"

test_finish "oneshot budget test"
