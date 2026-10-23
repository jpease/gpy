#!/usr/bin/env bash
# shellcheck disable=SC2329,SC2016,SC2015,SC2030,SC2031
# tests/bash/e2e_git_variant_fallback_bounded_sync.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Git segment variant fallback: bounded synchronous IPC (the Bash twin of
# tests/fish/e2e_git_variant_fallback_bounded_sync.test.fish).
#
# A `.none` fallback hit (status bit 4) has the wrong chevron colour: agent up must
# sync-query the real prev_bg; agent down serves `.none`. Theme uses `fg:prev_bg` (222).

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

shell_e2e_init "$ROOT"

PREV_BG=222
cache_dir="$XDG_CACHE_HOME/gpy/instant-prompts"

mkdir -p "$XDG_CONFIG_HOME/gpy/themes"
sed 's/\[\$sep_open\](fg:\$bg bg:default))\[ \$symbol \$branch\]/[$sep_open](fg:prev_bg bg:default))[ $symbol $branch]/' \
    "$ROOT/config/themes/default.toml" >"$XDG_CONFIG_HOME/gpy/themes/test-theme.toml"
if cmp -s "$ROOT/config/themes/default.toml" "$XDG_CONFIG_HOME/gpy/themes/test-theme.toml"; then
    fail "the default theme no longer contains the git opening separator this test rewrites"
    exit 1
fi
printf '[ui]\ntheme = "test-theme"\nenabled_segments = ["git"]\n' >"$XDG_CONFIG_HOME/gpy/config.toml"

cd "$SHELL_E2E_REPO" || exit 1
# Source with the agent disabled and off PATH (no daemon); enable it afterwards.
shell_e2e_agent_free_begin
GPY_AGENT_ENABLED=0 source "$ROOT/bash/gpy.bash"
shell_e2e_agent_free_end
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_start_agent || exit 1

# Prime the `.none` variants (no prev_bg writes only those).
none_present() { [ -n "$(find "$cache_dir" -maxdepth 1 -name '*.git_last.none.bash' 2>/dev/null | head -n 1)" ]; }
token_files() { find "$cache_dir" -maxdepth 1 -name "*.git_last.$PREV_BG.bash" 2>/dev/null; }
__gpy_trigger_data_refresh git "$SHELL_E2E_REPO" true ""
if shell_e2e_poll 8 none_present; then
    pass "the agent wrote the .none git cache variant"
else
    fail "no .none git cache file appeared"
    exit 1
fi
[ -z "$(token_files)" ] && pass "no token-specific file exists for prev_bg $PREV_BG yet" \
    || fail "a token-specific file already exists; no genuine variant gap"

__gpy_read_instant_cache git_last "$SHELL_E2E_REPO" "$PREV_BG" ignored
cache_rc=$?
if [ "$cache_rc" -eq 4 ]; then
    pass "the cache read reports the variant fallback (status $cache_rc)"
else
    fail "expected cache status 4 (fresh variant fallback), got $cache_rc"
fi

out="$(__gpy_segment_git "true" "$PREV_BG" "")"
rc=$?
case "$out" in
    *"38;5;$PREV_BG"*) pass "first render at the new prev_bg shows the correct chevron (status $rc)" ;;
    *) fail "the render served the wrong-context .none chevron: $out" ;;
esac
[ "$rc" -eq 0 ] || fail "expected status 0, got $rc"
if [ -n "$(token_files)" ]; then
    pass "the agent's response wrote the token-specific cache file"
else
    fail "no token-specific cache file was written by the bounded query"
fi

# --- agent unreachable: serve the .none bytes unchanged ---
# Copy the real .none entry into a cache root the running agent never inherited.
# The agent remembers every prev_bg it served and a background refresh rewrites
# them all, so deleting the token file here races a late rewrite (as zsh's twin).
fallback_cache_home="$SHELL_E2E_ROOT/fallback-cache"
fallback_cache_dir="$fallback_cache_home/gpy/instant-prompts"
none_files=("$cache_dir"/*".git_last.none.bash")
if [ "${#none_files[@]}" -ne 1 ] || [ ! -f "${none_files[0]}" ]; then
    fail "expected one real .none git cache file to isolate, got ${#none_files[@]}"
    exit 1
fi
none_file="${none_files[0]}"
mkdir -p "$fallback_cache_dir" || exit 1
cp "$none_file" "$fallback_cache_dir/" || exit 1
fallback_bytes="$(<"$fallback_cache_dir/${none_file##*/}")"

# A fresh variant fallback, not stale-cache oneshot recovery: keep the copy fresh
# even when the gate heavily loads the host.
fallback_read="$(
    export XDG_CACHE_HOME="$fallback_cache_home" GPY_GIT_INSTANT_CACHE_TTL_SECONDS=3600
    fallback_text=""
    __gpy_read_instant_cache git_last "$SHELL_E2E_REPO" "$PREV_BG" fallback_text
    rc=$?
    printf '%s' "$fallback_text"
    exit $rc
)"
fallback_cache_rc=$?
if [ "$fallback_cache_rc" -eq 4 ]; then
    pass "the isolated cache reports a fresh .none variant fallback"
else
    fail "expected isolated cache status 4, got $fallback_cache_rc"
fi
[ "$fallback_read" = "$fallback_bytes" ] || fail "the isolated cache did not read the real .none bytes"

out2="$(
    export XDG_CACHE_HOME="$fallback_cache_home" GPY_AGENT_SOCKET_PATH="$SHELL_E2E_ROOT/absent.sock" \
        GPY_GIT_INSTANT_CACHE_TTL_SECONDS=3600
    __gpy_segment_git "true" "$PREV_BG" ""
)"
rc2=$?
case "$out2" in
    "") fail "with the agent unreachable the .none fallback was not served" ;;
    *"38;5;$PREV_BG"*) fail "with the agent unreachable the render unexpectedly carried the prev_bg chevron" ;;
    *)
        if [ "$out2" = "$fallback_bytes" ]; then
            pass "with the agent unreachable the .none fallback is served unchanged"
        else
            fail "with the agent unreachable the render changed the real .none fallback bytes"
        fi
        ;;
esac
[ "$rc2" -eq 0 ] || fail "expected unreachable fallback status 0, got $rc2"
[ -z "$(find "$fallback_cache_dir" -maxdepth 1 -name "*.git_last.$PREV_BG.bash" 2>/dev/null)" ] \
    && pass "the isolated cache still has no token-specific file" \
    || fail "the unreachable render wrote a token-specific file into the isolated cache"

test_finish "bash git segment corrects the variant fallback via a bounded sync query"
