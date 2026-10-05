#!/usr/bin/env bash
# tests/bash/instant_cache_status.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Exit-code contract for __gpy_read_instant_cache (#614): mirrors the
# contract #612 gave Fish (tests/fish/instant_cache_status.test.fish). Status
# is now carried ENTIRELY by the exit code -- no more `__gpy_<suffix>_stale_$$`
# temp file for callers to `rm -f`/`-e` check.
#   0 = fresh, exact-token hit
#   1 = miss
#   2 = stale, exact-token hit
#   4 = fresh, served via the `.none` variant fallback
#   6 = stale, served via the `.none` variant fallback
#
# Drives staleness by back-dating the cache file's real mtime (portable
# GNU/BSD touch, same trick tests/bash/parity.test.bash already uses) rather
# than injecting a fake clock -- the real function has no clock-override hook.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing-instant-cache-status.sock"

source bash/gpy.bash

FAILED=0
check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        FAILED=1
    fi
}

backdate() {
    local file="$1" seconds_ago="$2"
    touch -t "$(date -v-"${seconds_ago}"S "+%Y%m%d%H%M.%S" 2>/dev/null || date -d "${seconds_ago} seconds ago" "+%Y%m%d%H%M.%S" 2>/dev/null)" "$file" 2>/dev/null \
        || python3 -c "import os,sys,time; os.utime(sys.argv[1], (time.time()-$seconds_ago, time.time()-$seconds_ago))" "$file" 2>/dev/null \
        || true
}

TMP_DIR="$(mktemp -d)"
export XDG_CACHE_HOME="$TMP_DIR/cache"
CACHE_DIR="$TMP_DIR/cache/gpy/instant-prompts"
mkdir -p "$CACHE_DIR"

REPO="$TMP_DIR/myrepo"
mkdir -p "$REPO/.git"
RESOLVED_ROOT="$(realpath "$REPO" 2>/dev/null || echo "$REPO")"
CACHE_KEY="$(__gpy_path_to_cache_key "$RESOLVED_ROOT")"

export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=5
export GPY_LANGUAGE_CACHE_TTL_SECONDS=30

# --- Miss: no cache file at all, for either suffix ---

out="$(__gpy_read_instant_cache git "$REPO" mytoken)"; status=$?
check "git: miss status" 1 "$status"
check "git: miss prints nothing" "" "$out"

out="$(__gpy_read_instant_cache lang "$REPO" mytoken)"; status=$?
check "lang: miss status" 1 "$status"
check "lang: miss prints nothing" "" "$out"

for row in "git 5" "lang 30"; do
    read -r suffix ttl <<<"$row"

    # --- Fresh, exact-token hit (status 0) ---
    token_file="$CACHE_DIR/$CACHE_KEY.$suffix.mytoken.bash"
    printf '%s content' "$suffix" >"$token_file"
    out="$(__gpy_read_instant_cache "$suffix" "$REPO" mytoken)"; status=$?
    check "$suffix: fresh exact-token hit status" 0 "$status"
    check "$suffix: fresh exact-token hit content" "$suffix content" "$out"

    # --- Stale, exact-token hit (status 2): age >= ttl ---
    backdate "$token_file" "$ttl"
    out="$(__gpy_read_instant_cache "$suffix" "$REPO" mytoken)"; status=$?
    check "$suffix: stale exact-token hit status" 2 "$status"
    check "$suffix: stale exact-token hit content preserved" "$suffix content" "$out"
    file_still_exists=1
    [[ -f "$token_file" ]] || file_still_exists=0
    check "$suffix: stale exact-token hit file preserved" 1 "$file_still_exists"

    rm -f "$token_file"

    # --- Fresh via .none variant fallback (status 4) ---
    none_file="$CACHE_DIR/$CACHE_KEY.$suffix.none.bash"
    printf '%s none-content' "$suffix" >"$none_file"
    out="$(__gpy_read_instant_cache "$suffix" "$REPO" mytoken)"; status=$?
    check "$suffix: fresh variant-fallback status" 4 "$status"
    check "$suffix: fresh variant-fallback content" "$suffix none-content" "$out"

    # --- Stale via .none variant fallback (status 6) ---
    backdate "$none_file" "$ttl"
    out="$(__gpy_read_instant_cache "$suffix" "$REPO" mytoken)"; status=$?
    check "$suffix: stale variant-fallback status" 6 "$status"
    check "$suffix: stale variant-fallback content" "$suffix none-content" "$out"

    rm -f "$none_file"
done

rm -rf "$TMP_DIR"

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All instant-cache status tests passed ==="
