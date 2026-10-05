#!/usr/bin/env zsh
# tests/zsh/is_first_instant_cache.test.zsh
#
# Regression test for #401: the instant-prompt cache writer
# (gpy-agent/src/cache/instant_prompt.rs) writes all four is_last/is_first
# combinations unconditionally (git/git_last/git_first/git_first_last,
# lang/.../lang_first_last). This verifies the Zsh reader side
# (__gpy_cache_variant_suffix, __gpy_segment_git, __gpy_segment_language)
# picks the correct file for each position instead of always serving the
# not-first variant.

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1

export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing-is-first.sock"
export GPY_LANGUAGE_ENABLED=1

source zsh/gpy.zsh

FAILED=0

check() {
    local label="$1" actual="$2" expected="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label (expected '$expected', got '$actual')"
        FAILED=1
    fi
}

# --- Suffix helper: matches gpy-agent's variant_suffix exactly ---
check "not first, not last" "$(__gpy_cache_variant_suffix git false false)" "git"
check "last only" "$(__gpy_cache_variant_suffix git true false)" "git_last"
check "first only" "$(__gpy_cache_variant_suffix git false true)" "git_first"
check "first and last" "$(__gpy_cache_variant_suffix git true true)" "git_first_last"
check "lang base suffix" "$(__gpy_cache_variant_suffix lang false true)" "lang_first"

# --- Segment-level: is_first must select the *_first cache file ---
TMP_DIR="$(mktemp -d)"
export XDG_CACHE_HOME="$TMP_DIR/cache"
CACHE_DIR="$TMP_DIR/cache/gpy/instant-prompts"
mkdir -p "$CACHE_DIR"

FAKE_REPO="$TMP_DIR/myproject"
mkdir -p "$FAKE_REPO/.git"
cd "$FAKE_REPO" || exit 1

RESOLVED_ROOT="$(realpath "$FAKE_REPO" 2>/dev/null || echo "$FAKE_REPO")"
CACHE_KEY="$(__gpy_path_to_cache_key "$RESOLVED_ROOT")"

# Distinct sentinel bytes per variant so a mismatch is unambiguous.
printf 'GIT-NORMAL' >"$CACHE_DIR/$CACHE_KEY.git.none.zsh"
printf 'GIT-LAST' >"$CACHE_DIR/$CACHE_KEY.git_last.none.zsh"
printf 'GIT-FIRST' >"$CACHE_DIR/$CACHE_KEY.git_first.none.zsh"
printf 'GIT-FIRST-LAST' >"$CACHE_DIR/$CACHE_KEY.git_first_last.none.zsh"

printf 'LANG-NORMAL' >"$CACHE_DIR/$CACHE_KEY.lang.none.zsh"
printf 'LANG-LAST' >"$CACHE_DIR/$CACHE_KEY.lang_last.none.zsh"
printf 'LANG-FIRST' >"$CACHE_DIR/$CACHE_KEY.lang_first.none.zsh"
printf 'LANG-FIRST-LAST' >"$CACHE_DIR/$CACHE_KEY.lang_first_last.none.zsh"

# #613: segment functions now receive is_last/is_first as "true"/"" (not the
# old "last"/"first" literals) -- call arguments updated, assertions about
# which cache file gets selected are unchanged.
check "git segment: not first, not last" "$(__gpy_segment_git "" "")" "GIT-NORMAL"
check "git segment: last, not first" "$(__gpy_segment_git true "")" "GIT-LAST"
check "git segment: first, not last" "$(__gpy_segment_git "" "" true)" "GIT-FIRST"
check "git segment: first and last" "$(__gpy_segment_git true "" true)" "GIT-FIRST-LAST"

check "lang segment: not first, not last" "$(__gpy_segment_language "" "")" "LANG-NORMAL"
check "lang segment: last, not first" "$(__gpy_segment_language true "")" "LANG-LAST"
check "lang segment: first, not last" "$(__gpy_segment_language "" "" true)" "LANG-FIRST"
check "lang segment: first and last" "$(__gpy_segment_language true "" true)" "LANG-FIRST-LAST"

cd "$ROOT" || true
rm -rf "$TMP_DIR"

if [[ $FAILED -ne 0 ]]; then
    echo ""
    echo "=== FAILED ==="
    exit 1
fi

echo ""
echo "=== All Tests Passed ==="
