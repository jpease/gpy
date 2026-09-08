#!/usr/bin/env fish
# tests/fish/instant_cache_status.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Exit-code contract for __gpy_read_instant_cache (#612): status is now
# carried ENTIRELY by the exit code instead of four side-effect globals.
#   0 = fresh, exact-token hit
#   1 = miss
#   2 = stale, exact-token hit
#   4 = fresh, served via the `.none` variant fallback
#   6 = stale, served via the `.none` variant fallback
#
# Drives `now` via __gpy_prompt_now (the per-render memo the real function
# reads first) relative to the cache file's ACTUAL mtime (read back via
# __gpy_file_mtime, portable across GNU/BSD touch) rather than the wall
# clock, so freshness is deterministic. Covers both the git and lang suffix
# branches (they differ only in which TTL default applies).

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

set -g pass_count 0
set -g fail_count 0

function check --argument-names label expected actual
    if test "$actual" = "$expected"
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label (expected [$expected], got [$actual])"
    end
end

set -l tmp_dir (mktemp -d)
set -gx XDG_CACHE_HOME "$tmp_dir/cache"
set -l cache_dir "$tmp_dir/cache/gpy/instant-prompts"
mkdir -p "$cache_dir"

set -l repo "$tmp_dir/myrepo"
mkdir -p "$repo/.git"
set -l resolved_root (path resolve -- "$repo" 2>/dev/null)
test -z "$resolved_root"; and set resolved_root $repo
set -l cache_key (__gpy_path_to_cache_key "$resolved_root")

set -g GPY_GIT_INSTANT_CACHE_TTL_SECONDS 5
set -g GPY_LANGUAGE_CACHE_TTL_SECONDS 30

# --- Miss: no cache file at all, for either suffix ---

set -l out (__gpy_read_instant_cache git "$repo" mytoken)
check "git: miss status" 1 $status
check "git: miss prints nothing" "" "$out"

set -l out (__gpy_read_instant_cache lang "$repo" mytoken)
check "lang: miss status" 1 $status
check "lang: miss prints nothing" "" "$out"

# Table of (suffix, ttl_var) so the git/lang cases below share one driver.
for row in "git GPY_GIT_INSTANT_CACHE_TTL_SECONDS" "lang GPY_LANGUAGE_CACHE_TTL_SECONDS"
    set -l parts (string split ' ' -- $row)
    set -l suffix $parts[1]
    set -l ttl_var $parts[2]
    set -l ttl $$ttl_var

    # --- Fresh, exact-token hit (status 0) ---
    set -l token_file "$cache_dir/$cache_key.$suffix.mytoken.ansi"
    printf '%s content' "$suffix" >"$token_file"
    set -l mtime (__gpy_file_mtime "$token_file")
    set -g __gpy_prompt_now $mtime

    set -l out (__gpy_read_instant_cache $suffix "$repo" mytoken)
    check "$suffix: fresh exact-token hit status" 0 $status
    check "$suffix: fresh exact-token hit content" "$suffix content" "$out"

    # --- Stale, exact-token hit (status 2): age >= ttl ---
    set -g __gpy_prompt_now (math "$mtime + $ttl")
    set -l out (__gpy_read_instant_cache $suffix "$repo" mytoken)
    check "$suffix: stale exact-token hit status" 2 $status
    check "$suffix: stale exact-token hit content preserved" "$suffix content" "$out"
    set -l still_exists 0
    test -f "$token_file"; and set still_exists 1
    check "$suffix: stale exact-token hit file preserved" 1 "$still_exists"

    rm -f "$token_file"

    # --- Fresh via .none variant fallback (status 4): no token file, .none exists ---
    set -l none_file "$cache_dir/$cache_key.$suffix.none.ansi"
    printf '%s none-content' "$suffix" >"$none_file"
    set -l none_mtime (__gpy_file_mtime "$none_file")
    set -g __gpy_prompt_now $none_mtime

    set -l out (__gpy_read_instant_cache $suffix "$repo" mytoken)
    check "$suffix: fresh variant-fallback status" 4 $status
    check "$suffix: fresh variant-fallback content" "$suffix none-content" "$out"

    # --- Stale via .none variant fallback (status 6) ---
    set -g __gpy_prompt_now (math "$none_mtime + $ttl")
    set -l out (__gpy_read_instant_cache $suffix "$repo" mytoken)
    check "$suffix: stale variant-fallback status" 6 $status
    check "$suffix: stale variant-fallback content" "$suffix none-content" "$out"

    rm -f "$none_file"
end

# --- Predicate cross-check: every status this file actually produced agrees
#     with __gpy_cache_status_stale / __gpy_cache_status_variant. ---
for code in 0 1 2 4 6
    set -l expect_stale 0
    contains -- $code 2 6; and set expect_stale 1
    set -l expect_variant 0
    contains -- $code 4 6; and set expect_variant 1

    __gpy_cache_status_stale $code
    set -l got_stale 0
    test $status -eq 0; and set got_stale 1
    check "predicate cross-check: stale($code)" "$expect_stale" "$got_stale"

    __gpy_cache_status_variant $code
    set -l got_variant 0
    test $status -eq 0; and set got_variant 1
    check "predicate cross-check: variant($code)" "$expect_variant" "$got_variant"
end

rm -rf "$tmp_dir"

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count instant-cache status tests passed"
