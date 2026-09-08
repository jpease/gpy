#!/usr/bin/env zsh
# tests/zsh/missing_core_file_disables_cleanly.test.zsh
#
# Regression test for #309: a partial one-line install can leave gpy.zsh's
# core/ directory missing a file (e.g. a download that failed mid-transfer).
# Previously gpy.zsh sourced every core file unconditionally, so a missing
# file produced a raw "no such file or directory" error on every single new
# shell forever, and half-loaded the prompt (some functions/constants
# defined, others not).
#
# This copies zsh/ into a scratch directory, deletes one core file, sources
# gpy.zsh from the copy, and asserts:
#   (a) no raw "no such file or directory" leaks from `source`
#   (b) exactly one clean diagnostic line is printed
#   (c) the sourcing shell is not killed (proves `return`, not `exit`, is
#       used to disable GPY -- an `exit` in a sourced file would terminate
#       the user's entire interactive shell session)

ROOT=${0:a:h:h:h}

WORKDIR=$(mktemp -d)
cleanup() { rm -rf "$WORKDIR" }
trap cleanup EXIT

cp -R "$ROOT/zsh" "$WORKDIR/zsh"
rm -f "$WORKDIR/zsh/core/constants.zsh"

cd "$WORKDIR"

OUT_LOG="$WORKDIR/out.log"
source zsh/gpy.zsh >"$OUT_LOG" 2>&1

# If gpy.zsh used `exit` instead of `return` to disable itself, this test
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
