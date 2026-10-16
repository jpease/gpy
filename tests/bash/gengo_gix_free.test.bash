#!/usr/bin/env bash
# tests/bash/gengo_gix_free.test.bash
#
# GPY does not use gix; gpy-agent/Cargo.lock must resolve no gix or gix-* package.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

LOCK="gpy-agent/Cargo.lock"

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

if [[ ! -f "$LOCK" ]]; then
    fail "$LOCK not found"
    test_finish
fi

echo "--- the resolved dependency tree carries no gix crate ---"
if grep -qE '^name = "gix(-[a-zA-Z0-9_-]+)?"' "$LOCK"; then
    fail "$LOCK resolves one or more gix/gix-* packages; a dependency change has reintroduced the retired git backend"
fi

test_finish
