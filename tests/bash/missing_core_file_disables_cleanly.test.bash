#!/usr/bin/env bash
# tests/bash/missing_core_file_disables_cleanly.test.bash
# gpy.bash with a missing core file must disable cleanly: no raw "No such file"
# error, one diagnostic line, and `return` (not `exit`) so the shell survives.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

cp -R "$ROOT/bash" "$WORKDIR/bash"
rm -f "$WORKDIR/bash/core/constants.bash"

cd "$WORKDIR" || exit 1

OUT_LOG="$WORKDIR/out.log"
source bash/gpy.bash >"$OUT_LOG" 2>&1

echo "post-source: test process still alive (return was used, not exit)"

echo "--- captured output ---"
cat "$OUT_LOG"
echo "--- end captured output ---"

if grep -qi "no such file or directory" "$OUT_LOG"; then
    echo "FAIL: raw 'source' error leaked -- missing existence guard for core files"
    exit 1
fi

DIAG_COUNT=$(grep -c '^gpy\[init\]:' "$OUT_LOG")
if [[ "$DIAG_COUNT" -ne 1 ]]; then
    echo "FAIL: expected exactly 1 'gpy[init]:' diagnostic line, got $DIAG_COUNT"
    exit 1
fi

echo "PASS"
