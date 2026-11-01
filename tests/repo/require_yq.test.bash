#!/usr/bin/env bash
# tests/repo/require_yq.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# `require_yq` (tests/lib/shell_e2e.sh) skips unless `yq` is mikefarah's and at
# least 4.52.4. This file does not call `require_yq` itself: it must run
# without yq.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

stub_dir="$(mktemp -d)"
trap 'rm -rf "$stub_dir"' EXIT

# run_require_yq VERSION_LINE -- prints require_yq's output, then `rc=N`.
# CI and GPY_GATE_STRICT are unset so a skip exits 0 rather than failing.
run_require_yq() {
    printf '#!/bin/sh\necho "%s"\n' "$1" >"$stub_dir/yq"
    chmod +x "$stub_dir/yq"
    (
        unset CI GPY_GATE_STRICT
        PATH="$stub_dir:$PATH"
        require_yq
        echo "returned"
    ) 2>&1
    echo "rc=$?"
}

out="$(run_require_yq 'yq 3.4.3')"
if [[ "$out" == *"SKIP:"* && "$out" == *"rc=0" && "$out" != *returned* ]]; then
    pass "a non-mikefarah yq skips"
else
    fail "a non-mikefarah yq did not skip: $out"
fi

out="$(run_require_yq 'yq (https://github.com/mikefarah/yq/) version v4.52.2')"
if [[ "$out" == *"SKIP:"* && "$out" == *"rc=0" && "$out" != *returned* ]]; then
    pass "mikefarah yq 4.52.2 skips"
else
    fail "mikefarah yq 4.52.2 did not skip: $out"
fi

out="$(run_require_yq 'yq (https://github.com/mikefarah/yq/) version v4.52.4')"
if [[ "$out" != *"SKIP:"* && "$out" == *returned* ]]; then
    pass "mikefarah yq 4.52.4 passes"
else
    fail "mikefarah yq 4.52.4 was rejected: $out"
fi

out="$(run_require_yq 'yq (https://github.com/mikefarah/yq/) version v4.54.1')"
if [[ "$out" != *"SKIP:"* && "$out" == *returned* && "$out" == *"rc=0" ]]; then
    pass "mikefarah yq 4.54.1 passes"
else
    fail "mikefarah yq 4.54.1 was rejected: $out"
fi

test_finish "require_yq gates on mikefarah yq >= 4.52.4"
