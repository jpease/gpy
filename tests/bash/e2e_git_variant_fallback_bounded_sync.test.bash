#!/usr/bin/env bash
# shellcheck disable=SC2329,SC2016,SC2015
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

__gpy_read_instant_cache git_last "$SHELL_E2E_REPO" "$PREV_BG" >/dev/null
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

rm -f "$cache_dir"/*".git_last.$PREV_BG.bash"
saved_socket="$GPY_AGENT_SOCKET_PATH"
export GPY_AGENT_SOCKET_PATH="$SHELL_E2E_ROOT/absent.sock"
out2="$(__gpy_segment_git "true" "$PREV_BG" "")"
export GPY_AGENT_SOCKET_PATH="$saved_socket"
case "$out2" in
    "") fail "with the agent unreachable the .none fallback was not served" ;;
    *"38;5;$PREV_BG"*) fail "with the agent unreachable the render unexpectedly carried the prev_bg chevron" ;;
    *) pass "with the agent unreachable the .none fallback is served unchanged" ;;
esac

test_finish "bash git segment corrects the variant fallback via a bounded sync query"
