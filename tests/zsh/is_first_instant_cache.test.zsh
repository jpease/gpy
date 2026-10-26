#!/usr/bin/env zsh
# tests/zsh/is_first_instant_cache.test.zsh
#
# The instant-prompt cache writer
# (gpy-agent/src/cache/instant_prompt/mod.rs) writes all four is_last/is_first
# combinations unconditionally (git/git_last/git_first/git_first_last,
# lang/.../lang_first_last). This verifies the Zsh reader side
# (__gpy_cache_variant_suffix, __gpy_segment_git, __gpy_segment_language)
# picks the correct file for each position instead of always serving the
# not-first variant.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing-is-first.sock"
export GPY_LANGUAGE_ENABLED=1

source zsh/gpy.zsh


# --- Suffix helper: matches gpy-agent's cache variant names exactly ---
__gpy_cache_variant_suffix git false false suffix
expect_eq "not first, not last" "$suffix" "git"
__gpy_cache_variant_suffix git true false suffix
expect_eq "last only" "$suffix" "git_last"
__gpy_cache_variant_suffix git false true suffix
expect_eq "first only" "$suffix" "git_first"
__gpy_cache_variant_suffix git true true suffix
expect_eq "first and last" "$suffix" "git_first_last"
__gpy_cache_variant_suffix lang false true suffix
expect_eq "lang base suffix" "$suffix" "lang_first"

# --- Segment-level: is_first must select the *_first cache file ---
TMP_DIR="$(mktemp -d)"
export XDG_CACHE_HOME="$TMP_DIR/cache"
CACHE_DIR="$TMP_DIR/cache/gpy/instant-prompts"
mkdir -p "$CACHE_DIR"

FAKE_REPO="$TMP_DIR/myproject"
mkdir -p "$FAKE_REPO/.git"
cd "$FAKE_REPO" || exit 1

RESOLVED_ROOT="$(realpath "$FAKE_REPO" 2>/dev/null || echo "$FAKE_REPO")"
__gpy_path_to_cache_key "$RESOLVED_ROOT" CACHE_KEY

# Distinct sentinel bytes per variant so a mismatch is unambiguous.
printf 'GIT-NORMAL' >"$CACHE_DIR/$CACHE_KEY.git.none.zsh"
printf 'GIT-LAST' >"$CACHE_DIR/$CACHE_KEY.git_last.none.zsh"
printf 'GIT-FIRST' >"$CACHE_DIR/$CACHE_KEY.git_first.none.zsh"
printf 'GIT-FIRST-LAST' >"$CACHE_DIR/$CACHE_KEY.git_first_last.none.zsh"

printf 'LANG-NORMAL' >"$CACHE_DIR/$CACHE_KEY.lang.none.zsh"
printf 'LANG-LAST' >"$CACHE_DIR/$CACHE_KEY.lang_last.none.zsh"
printf 'LANG-FIRST' >"$CACHE_DIR/$CACHE_KEY.lang_first.none.zsh"
printf 'LANG-FIRST-LAST' >"$CACHE_DIR/$CACHE_KEY.lang_first_last.none.zsh"

# Segment functions receive is_last/is_first as "true"/"".
expect_eq "git segment: not first, not last" "$(__gpy_segment_git "" "")" "GIT-NORMAL"
expect_eq "git segment: last, not first" "$(__gpy_segment_git true "")" "GIT-LAST"
expect_eq "git segment: first, not last" "$(__gpy_segment_git "" "" true)" "GIT-FIRST"
expect_eq "git segment: first and last" "$(__gpy_segment_git true "" true)" "GIT-FIRST-LAST"

expect_eq "lang segment: not first, not last" "$(__gpy_segment_language "" "")" "LANG-NORMAL"
expect_eq "lang segment: last, not first" "$(__gpy_segment_language true "")" "LANG-LAST"
expect_eq "lang segment: first, not last" "$(__gpy_segment_language "" "" true)" "LANG-FIRST"
expect_eq "lang segment: first and last" "$(__gpy_segment_language true "" true)" "LANG-FIRST-LAST"

cd "$ROOT" || true
rm -rf "$TMP_DIR"

test_finish "is first instant cache"
