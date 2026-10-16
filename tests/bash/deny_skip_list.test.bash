#!/usr/bin/env bash
# tests/bash/deny_skip_list.test.bash
#
# Every crate in gpy-agent/deny.toml's `[bans] skip` list is still duplicated
# in gpy-agent/Cargo.lock. A skip for a crate that has collapsed to a single
# version silently hides the next duplicate of that crate.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

DENY="gpy-agent/deny.toml"
LOCK="gpy-agent/Cargo.lock"

for f in "$DENY" "$LOCK"; do
    [[ -f "$f" ]] || fail "$f not found"
done
[[ $failures -eq 0 ]] || test_finish

deny_skip_crates() {
    awk '
        /^\[[a-z]+\]/ { section = $0 }
        section == "[bans]" && /^skip[[:space:]]*=[[:space:]]*\[/ { grab = 1; next }
        grab && /^\]/ { grab = 0 }
        grab { print }
    ' "$DENY" | sed -nE 's/.*name[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p'
}

lock_versions() {
    awk -v want="$1" '
        /^name = / { cur = $3; gsub(/"/, "", cur) }
        /^version = / { if (cur == want) { v = $3; gsub(/"/, "", v); print v } }
    ' "$LOCK"
}

while IFS= read -r crate; do
    [[ -n "$crate" ]] || continue
    count="$(lock_versions "$crate" | grep -c '[^[:space:]]' || true)"
    if [[ "$count" -eq 0 ]]; then
        fail "$DENY skips duplicate-checking for '$crate', which is not in $LOCK at all"
    elif [[ "$count" -lt 2 ]]; then
        fail "$DENY skips duplicate-checking for '$crate', which now has a single version; the skip hides a future duplicate"
    fi
done <<<"$(deny_skip_crates)"

test_finish "every deny.toml [bans] skip entry is still duplicated in Cargo.lock"
