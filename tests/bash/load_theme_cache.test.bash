#!/usr/bin/env bash
# tests/bash/load_theme_cache.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_load_theme sources the cached theme-export.bash when present and only
# forks `gpy-agent theme export` when absent. The stub gpy-agent always fails,
# so a cache-hit run that invoked it is caught.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

TMP_DIR="$(mktemp -d)"
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
# Supervisor off in a sandbox config: the theme export re-sets the flag.
source "$ROOT/tests/lib/supervisor_off.bash" "$TMP_DIR"
export GPY_AGENT_SOCKET_PATH="$TMP_DIR/dead-agent.sock"

source bash/core/constants.bash
source bash/core/ipc.bash
source bash/core/init.bash

CACHE_DIR="$TMP_DIR/cache/gpy"
mkdir -p "$CACHE_DIR"
cat >"$CACHE_DIR/theme-export.bash" <<'EOF'
export __gpy_load_theme_cache_sentinel="from-cache-file"
EOF

unset __gpy_load_theme_cache_sentinel
: >"$INVOKED_FILE"
__gpy_load_theme
check "sources cache file: sentinel var set" "from-cache-file" "${__gpy_load_theme_cache_sentinel:-}"
invoked_count="$(wc -c <"$INVOKED_FILE" | tr -d ' ')"
check "sources cache file: gpy-agent never invoked" 0 "$invoked_count"

rm -f "$CACHE_DIR/theme-export.bash"
unset __gpy_load_theme_cache_sentinel
: >"$INVOKED_FILE"
__gpy_load_theme
invoked_count="$(wc -c <"$INVOKED_FILE" | tr -d ' ')"
check "cache absent: falls back to gpy-agent theme export" 1 "$invoked_count"

rm -rf "$TMP_DIR"

test_finish "load_theme cache test"
