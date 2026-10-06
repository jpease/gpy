#!/usr/bin/env zsh
# tests/zsh/load_theme_cache.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #614: __gpy_load_theme sources the agent's cached theme-export.zsh file
# (written by gpy-agent/src/cache/theme_export.rs's write_theme_export_to_dir,
# beside theme-export.fish/.bash) when present, and only forks
# `gpy-agent theme export` when the cache is absent. Mirrors Fish's
# __gpy_apply_theme_export / __gpy_theme_export_cache_path (fish/core/init.fish,
# fish/core/util.fish).
#
# zsh's `(( $+commands[gpy-agent] ))` liveness check only ever sees
# PATH-hashed external commands, never shell functions, so the stub here is
# a real executable on PATH rather than a shadowing function -- it records
# every invocation into a file and always fails, so a cache-hit run that
# nonetheless invoked it is caught immediately.

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
INVOKED_FILE="$TMP_DIR/gpy-agent-invoked"

cat >"$STUB_BIN_DIR/gpy-agent" <<STUB
#!/usr/bin/env bash
printf 'x' >>"$INVOKED_FILE"
exit 1
STUB
chmod +x "$STUB_BIN_DIR/gpy-agent"

export PATH="$STUB_BIN_DIR:$PATH"
source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$TMP_DIR/dead-agent.sock"
export XDG_CACHE_HOME="$TMP_DIR/cache"
hash -r

source zsh/core/constants.zsh
source zsh/core/ipc.zsh
source zsh/core/init.zsh

# --- Cache present: source it, never invoke gpy-agent ---
CACHE_DIR="$TMP_DIR/cache/gpy"
mkdir -p "$CACHE_DIR"
cat >"$CACHE_DIR/theme-export.zsh" <<'EOF'
typeset -g __gpy_load_theme_cache_sentinel="from-cache-file"
EOF

unset __gpy_load_theme_cache_sentinel
: >"$INVOKED_FILE"
__gpy_load_theme
check "sources cache file: sentinel var set" "from-cache-file" "${__gpy_load_theme_cache_sentinel:-}"
invoked_count=$(wc -c <"$INVOKED_FILE" | tr -d ' ')
check "sources cache file: gpy-agent never invoked" 0 "$invoked_count"

# --- Cache absent: falls back to forking gpy-agent theme export ---
rm -f "$CACHE_DIR/theme-export.zsh"
unset __gpy_load_theme_cache_sentinel
: >"$INVOKED_FILE"
__gpy_load_theme
invoked_count=$(wc -c <"$INVOKED_FILE" | tr -d ' ')
check "cache absent: falls back to gpy-agent theme export" 1 "$invoked_count"

rm -rf "$TMP_DIR"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== load_theme cache test passed ==="
