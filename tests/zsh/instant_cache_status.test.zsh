#!/usr/bin/env zsh
# tests/zsh/instant_cache_status.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Exit-code contract for __gpy_read_instant_cache, mirroring Fish's
# (tests/fish/instant_cache_status.test.fish). Status is carried ENTIRELY by
# the exit code; there is no side-channel temp file for callers to check.
#   0 = fresh, exact-token hit
#   1 = miss
#   2 = stale, exact-token hit
#   4 = fresh, served via the `.none` variant fallback
#   6 = stale, served via the `.none` variant fallback
#
# Drives staleness by back-dating the cache file's real mtime (portable
# GNU/BSD touch, as tests/zsh/parity.test.zsh does) rather
# than injecting a fake clock -- the real function has no clock-override hook.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT"

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing-instant-cache-status.sock"

source zsh/gpy.zsh

backdate() {
    local file=$1 seconds_ago=$2
    touch -t "$(date -v-"${seconds_ago}"S "+%Y%m%d%H%M.%S" 2>/dev/null || date -d "${seconds_ago} seconds ago" "+%Y%m%d%H%M.%S" 2>/dev/null)" "$file" 2>/dev/null \
        || python3 -c "import os,sys,time; os.utime(sys.argv[1], (time.time()-$seconds_ago, time.time()-$seconds_ago))" "$file" 2>/dev/null \
        || true
}

TMP_DIR=$(mktemp -d)
export XDG_CACHE_HOME="$TMP_DIR/cache"
CACHE_DIR="$TMP_DIR/cache/gpy/instant-prompts"
mkdir -p "$CACHE_DIR"

REPO="$TMP_DIR/myrepo"
mkdir -p "$REPO/.git"
RESOLVED_ROOT=$(realpath "$REPO" 2>/dev/null || echo "$REPO")
__gpy_path_to_cache_key "$RESOLVED_ROOT" CACHE_KEY

export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=5
export GPY_LANGUAGE_CACHE_TTL_SECONDS=30

# --- Miss: no cache file at all, for either suffix ---

out=""

__gpy_read_instant_cache git "$REPO" mytoken out; rc=$?
check "git: miss status" 1 "$rc"
check "git: miss prints nothing" "" "$out"

out=""

__gpy_read_instant_cache lang "$REPO" mytoken out; rc=$?
check "lang: miss status" 1 "$rc"
check "lang: miss prints nothing" "" "$out"

for row in "git 5" "lang 30"; do
    suffix=${row%% *}
    ttl=${row##* }

    # --- Fresh, exact-token hit (status 0) ---
    token_file="$CACHE_DIR/$CACHE_KEY.$suffix.mytoken.zsh"
    printf '%s content' "$suffix" >"$token_file"
    out=""
    __gpy_read_instant_cache "$suffix" "$REPO" mytoken out; rc=$?
    check "$suffix: fresh exact-token hit status" 0 "$rc"
    check "$suffix: fresh exact-token hit content" "$suffix content" "$out"

    # --- Stale, exact-token hit (status 2): age >= ttl ---
    backdate "$token_file" "$ttl"
    out=""
    __gpy_read_instant_cache "$suffix" "$REPO" mytoken out; rc=$?
    check "$suffix: stale exact-token hit status" 2 "$rc"
    check "$suffix: stale exact-token hit content preserved" "$suffix content" "$out"
    file_still_exists=1
    [[ -f "$token_file" ]] || file_still_exists=0
    check "$suffix: stale exact-token hit file preserved" 1 "$file_still_exists"

    rm -f "$token_file"

    # --- Fresh via .none variant fallback (status 4) ---
    none_file="$CACHE_DIR/$CACHE_KEY.$suffix.none.zsh"
    printf '%s none-content' "$suffix" >"$none_file"
    out=""
    __gpy_read_instant_cache "$suffix" "$REPO" mytoken out; rc=$?
    check "$suffix: fresh variant-fallback status" 4 "$rc"
    check "$suffix: fresh variant-fallback content" "$suffix none-content" "$out"

    # --- Stale via .none variant fallback (status 6) ---
    backdate "$none_file" "$ttl"
    out=""
    __gpy_read_instant_cache "$suffix" "$REPO" mytoken out; rc=$?
    check "$suffix: stale variant-fallback status" 6 "$rc"
    check "$suffix: stale variant-fallback content" "$suffix none-content" "$out"

    rm -f "$none_file"
done

rm -rf "$TMP_DIR"

test_finish "instant-cache status tests"
