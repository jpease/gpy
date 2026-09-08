#!/usr/bin/env bash
# tests/bash/cleanup_test_agents.test.bash
#
# Regression test for #619: scripts/cleanup-test-agents.sh used to delete
# any socket file named gpy-test.sock/gpy-test-*.sock anywhere under /tmp
# or /var/folders, and to kill any gpy-agent process by --socket name alone
# -- both name-only, location-unbounded checks that could remove or kill
# something the GPY test harness did not create, and that traversed the
# entire system temp tree (observed to take over a minute on the
# 2026-09-03 audit run).
#
# This test drives the real cleanup script against a hermetic sandbox: a
# GPY test root created under a throwaway TMPDIR (so it can never collide
# with a real leftover socket on the machine running the suite), plus a
# same-named decoy socket deliberately created OUTSIDE that root. It
# asserts the decoy survives, the in-root socket is removed, the script
# finishes in well under five seconds, and the script's source no longer
# contains a whole-/tmp or whole-/var/folders traversal.
#
# Without the fix this test fails on the "decoy survives" assertion (the
# old script matched by socket basename alone, regardless of location) and
# on the "no whole-tmp traversal" source check.

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

# A dedicated TMPDIR so this test's GPY test root ($TMPDIR/gpy-test-$USER)
# is hermetic -- it can never overlap a real leftover socket, or a socket a
# concurrently running test suite created, on the machine running this test.
#
# Rooted explicitly under /tmp (an absolute template, not "${TMPDIR:-/tmp}")
# so the sandbox path stays short: AF_UNIX socket paths are capped at ~104
# bytes on macOS, and the ambient TMPDIR on macOS is already a long
# /var/folders/<hash>/T path -- stacking this test's own sandbox prefix and
# the GPY root's "gpy-test-<user>" segment on top of that would overflow the
# limit before a single socket is even created.
SANDBOX_TMPDIR="$(mktemp -d /tmp/gpy619.XXXXXX)"
TEST_USER="${USER:-$(id -un)}"
GPY_ROOT="${SANDBOX_TMPDIR%/}/gpy-test-$TEST_USER"
mkdir -p "$GPY_ROOT"

# A decoy socket living OUTSIDE the sandboxed GPY root -- also rooted under
# /tmp directly for the same path-length reason -- standing in for an
# unrelated process's same-named socket that the cleanup script must never
# touch.
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

# --- Run the real cleanup script, scoped to the sandbox TMPDIR, and time it ---
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
