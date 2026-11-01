#!/usr/bin/env bash
# tests/repo/cleanup_test_agents.test.bash
#
# scripts/cleanup-test-agents.sh must only touch sockets and processes under
# the GPY test root; a decoy same-named socket outside the sandbox root must survive.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

assert_true() {
    if eval "$2"; then
        pass "$1"
    else
        fail "$1"
    fi
}

test_require_command python3 "python3 not available to bind AF_UNIX sockets for this test"

# Hermetic TMPDIR so the GPY test root never overlaps a real socket.
# Rooted under /tmp, not ${TMPDIR}: macOS caps AF_UNIX paths at ~104 bytes.
SANDBOX_TMPDIR="$(mktemp -d /tmp/gpy-cleanup.XXXXXX)"
TEST_USER="${USER:-$(id -un)}"
GPY_ROOT="${SANDBOX_TMPDIR%/}/gpy-test-$TEST_USER"
mkdir -p "$GPY_ROOT"

# Decoy socket OUTSIDE the sandboxed root (also /tmp for path length).
DECOY_DIR="$(mktemp -d /tmp/gpy-cleanup-decoy.XXXXXX)"
DECOY_SOCK="$DECOY_DIR/gpy-test-decoy.sock"
IN_ROOT_SOCK="$GPY_ROOT/gpy-test.sock"

cleanup_sandbox() {
    rm -rf "$SANDBOX_TMPDIR" "$DECOY_DIR" 2>/dev/null || true
}
trap cleanup_sandbox EXIT

shell_e2e_stub_listener "$DECOY_SOCK" bind-only
shell_e2e_stub_listener "$IN_ROOT_SOCK" bind-only

assert_true "decoy socket exists before cleanup" "[[ -S '$DECOY_SOCK' ]]"
assert_true "in-root socket exists before cleanup" "[[ -S '$IN_ROOT_SOCK' ]]"

SECONDS=0
CLEANUP_OUT="$(TMPDIR="$SANDBOX_TMPDIR" "$ROOT/scripts/cleanup-test-agents.sh" 2>&1)"
CLEANUP_STATUS=$?
ELAPSED_S=$SECONDS

echo "--- cleanup-test-agents.sh output ---"
echo "$CLEANUP_OUT"
echo "--------------------------------------"

assert_true "cleanup script exits 0" "[[ $CLEANUP_STATUS -eq 0 ]]"
assert_true "cleanup completes in under 5s (took ${ELAPSED_S}s)" "[[ $ELAPSED_S -lt 5 ]]"

assert_true "unrelated decoy socket survives cleanup" "[[ -S '$DECOY_SOCK' ]]"
assert_true "in-root socket was removed by cleanup" "[[ ! -e '$IN_ROOT_SOCK' ]]"

assert_true "cleanup script contains no whole-/tmp find traversal" \
    "! grep -qE 'find[[:space:]]+/tmp([[:space:]]|\$)' '$ROOT/scripts/cleanup-test-agents.sh'"
assert_true "cleanup script contains no whole-/var/folders find traversal" \
    "! grep -qE 'find[[:space:]]+/var/folders([[:space:]]|\$)' '$ROOT/scripts/cleanup-test-agents.sh'"

test_finish "cleanup-test-agents scoping"
