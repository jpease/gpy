#!/usr/bin/env zsh
# tests/zsh/oneshot_budget.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #614: a dead daemon must cost at most ONE `gpy-agent oneshot` fork per
# prompt render, and that budget must be enforced with a plain per-render
# shell variable (__gpy_oneshot_used) instead of a predictable
# ${TMPDIR:-/tmp}/.gpy_oneshot_used_$$ marker file. __gpy_render_prompt runs
# entirely inside the $(...) subshell __gpy_precmd uses to capture $PROMPT,
# and each segment is itself invoked via a further $(...), so no segment can
# write back to a parent scope directly -- each oneshot-consuming request
# function instead signals its use via a distinct exit code
# (GPY_SEG_STATUS_ONESHOT), which the render loop reads off `$?` and folds
# into __gpy_oneshot_used, which later segments' subshells inherit by fork.
#
# zsh's `(( $+commands[gpy-agent] ))` liveness check (unlike bash's
# `command -v`) only ever sees PATH-hashed external commands, never shell
# functions, so the stub here is a real executable on PATH rather than a
# shadowing function.

ROOT=${0:a:h:h:h}
cd "$ROOT"

FAILED=0
check() {
    local label=$1 expected=$2 actual=$3
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

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
export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_AGENT_SOCKET_PATH="$TMP_DIR/dead-agent.sock"
export XDG_CACHE_HOME="$TMP_DIR/cache"

source zsh/gpy.zsh

FAKE_REPO="$TMP_DIR/myrepo"
mkdir -p "$FAKE_REPO/.git"
cd "$FAKE_REPO"

__enabled_segments=(directory git duration)
typeset -g __duration_threshold_ms=100
typeset -g __gpy_cmd_duration=5000

PROMPT=$(__gpy_render_prompt 0)

oneshot_calls=$(wc -c <"$COUNTER_FILE" | tr -d ' ')
check "at most one oneshot fork per render" 1 "$oneshot_calls"

# No leftover predictable-name marker/flag file under TMPDIR for this PID.
# The shell's own private relay directory (gpy_cache_<pid>_<random>, 0700,
# #763) carries the PID by design and is not a marker file.
leftover_count=0
setopt local_options nullglob dotglob
for f in "${TMPDIR:-/tmp}"/*gpy*_$$*; do
    [[ -n "${__gpy_cache_dir:-}" && "${f:A}" == "${__gpy_cache_dir:A}" ]] && continue
    leftover_count=$((leftover_count + 1))
    echo "  leftover: $f"
done

cd "$ROOT"
rm -rf "$TMP_DIR"

check "no leftover predictable-name marker files" 0 "$leftover_count"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== oneshot budget test passed ==="
