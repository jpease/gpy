#!/usr/bin/env bash
# tests/bash/cleanup_test_agents.test.bash
#
# scripts/cleanup-test-agents.sh must only touch sockets and processes under
# the GPY test root; a decoy same-named socket outside the sandbox root must survive.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

FAILED=0

check() {
    local label="$1"
    if eval "$2"; then
        echo "PASS: $label"
    else
        echo "FAIL: $label"
        FAILED=1
    fi
}

if ! command -v python3 >/dev/null 2>&1; then
    echo "SKIP: python3 not available to bind AF_UNIX sockets for this test"
    exit 0
fi

# Hermetic TMPDIR so the GPY test root never overlaps a real socket.
# Rooted under /tmp, not ${TMPDIR}: macOS caps AF_UNIX paths at ~104 bytes.
# so the sandbox path stays short: AF_UNIX socket paths are capped at ~104
# bytes on macOS, and the ambient TMPDIR on macOS is already a long
SANDBOX_TMPDIR="$(mktemp -d /tmp/gpy619.XXXXXX)"
TEST_USER="${USER:-$(id -un)}"
GPY_ROOT="${SANDBOX_TMPDIR%/}/gpy-test-$TEST_USER"
mkdir -p "$GPY_ROOT"

# Decoy socket OUTSIDE the sandboxed root (also /tmp for path length).
DECOY_DIR="$(mktemp -d /tmp/gpy619decoy.XXXXXX)"
DECOY_SOCK="$DECOY_DIR/gpy-test-decoy.sock"
IN_ROOT_SOCK="$GPY_ROOT/gpy-test.sock"

cleanup_sandbox() {
    rm -rf "$SANDBOX_TMPDIR" "$DECOY_DIR" 2>/dev/null || true
}
trap cleanup_sandbox EXIT

bind_af_unix_socket() {
    python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
' "$1"
}

bind_af_unix_socket "$DECOY_SOCK"
bind_af_unix_socket "$IN_ROOT_SOCK"

check "decoy socket exists before cleanup" "[[ -S '$DECOY_SOCK' ]]"
check "in-root socket exists before cleanup" "[[ -S '$IN_ROOT_SOCK' ]]"

SECONDS=0
CLEANUP_OUT="$(TMPDIR="$SANDBOX_TMPDIR" "$ROOT/scripts/cleanup-test-agents.sh" 2>&1)"
CLEANUP_STATUS=$?
ELAPSED_S=$SECONDS

echo "--- cleanup-test-agents.sh output ---"
echo "$CLEANUP_OUT"
echo "--------------------------------------"

check "cleanup script exits 0" "[[ $CLEANUP_STATUS -eq 0 ]]"
check "cleanup completes in under 5s (took ${ELAPSED_S}s)" "[[ $ELAPSED_S -lt 5 ]]"

check "unrelated decoy socket survives cleanup" "[[ -S '$DECOY_SOCK' ]]"
check "in-root socket was removed by cleanup" "[[ ! -e '$IN_ROOT_SOCK' ]]"

check "cleanup script contains no whole-/tmp find traversal" \
    "! grep -qE 'find[[:space:]]+/tmp([[:space:]]|\$)' '$ROOT/scripts/cleanup-test-agents.sh'"
check "cleanup script contains no whole-/var/folders find traversal" \
    "! grep -qE 'find[[:space:]]+/var/folders([[:space:]]|\$)' '$ROOT/scripts/cleanup-test-agents.sh'"

if [[ $FAILED -ne 0 ]]; then
    echo ""
    echo "=== FAILED ==="
    exit 1
fi

echo ""
echo "=== All Tests Passed ==="
