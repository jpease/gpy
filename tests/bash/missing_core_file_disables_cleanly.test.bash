#!/usr/bin/env bash
# tests/bash/missing_core_file_disables_cleanly.test.bash
#
# Regression test for #309: a partial one-line install can leave gpy.bash's
# core/ directory missing a file (e.g. a download that failed mid-transfer).
# Previously gpy.bash sourced every core file unconditionally, so a missing
# file produced a raw "No such file or directory" error on every single new
# shell forever, and half-loaded the prompt (some functions/constants
# defined, others not).
#
# This copies bash/ into a scratch directory, deletes one core file, sources
# gpy.bash from the copy, and asserts:
#   (a) no raw "No such file or directory" leaks from `source`
#   (b) exactly one clean diagnostic line is printed
#   (c) the sourcing shell is not killed (proves `return`, not `exit`, is
#       used to disable GPY -- an `exit` in a sourced file would terminate
#       the user's entire interactive shell session)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

cp -R "$ROOT/bash" "$WORKDIR/bash"
rm -f "$WORKDIR/bash/core/constants.bash"

cd "$WORKDIR" || exit 1

OUT_LOG="$WORKDIR/out.log"
source bash/gpy.bash >"$OUT_LOG" 2>&1

# If gpy.bash used `exit` instead of `return` to disable itself, this test
# process would have been killed by the `source` above and we would never
# reach this line -- so getting here already proves criterion (c).
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
