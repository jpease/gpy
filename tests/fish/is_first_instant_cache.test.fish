#!/usr/bin/env fish
# Test: is_first-aware instant-prompt cache suffix selection
#
# The instant-prompt cache writer (gpy-agent/src/cache/instant_prompt/mod.rs)
# writes all four is_last/is_first combinations unconditionally
# (git/git_last/git_first/git_first_last, lang/.../lang_first_last). This test
# verifies the Fish reader side (__gpy_cache_variant_suffix,
# segment_git_render, segment_language_render) picks the correct file for
# each position instead of always serving the not-first variant.

source (dirname (status --filename))/../lib/test_helpers.fish

source fish/core/init.fish
source fish/segments/git.fish
source fish/segments/language.fish

# Isolate environment from user configuration (mirrors segments.test.fish).
set -l temp_home (mktemp -d)
set -gx HOME $temp_home
set -gx XDG_CONFIG_HOME $temp_home/.config
mkdir -p $XDG_CONFIG_HOME/gpy

# --- Suffix helper: matches gpy-agent's cache variant names exactly ---
check "not first, not last" pass (test (__gpy_cache_variant_suffix git "" "") = git; and echo pass; or echo fail)
check "last only" pass (test (__gpy_cache_variant_suffix git true "") = git_last; and echo pass; or echo fail)
check "first only" pass (test (__gpy_cache_variant_suffix git "" true) = git_first; and echo pass; or echo fail)
check "first and last" pass (test (__gpy_cache_variant_suffix git true true) = git_first_last; and echo pass; or echo fail)
check "lang base suffixes" pass (test (__gpy_cache_variant_suffix lang "" true) = lang_first; and echo pass; or echo fail)

# --- Segment-level: is_first must select the *_first cache file ---
set -l tmp_dir (mktemp -d)
set -gx XDG_CACHE_HOME "$tmp_dir/cache"
set -l cache_dir "$tmp_dir/cache/gpy/instant-prompts"
mkdir -p "$cache_dir"

set -l fake_repo "$tmp_dir/myproject"
mkdir -p "$fake_repo/.git"
cd "$fake_repo"

set -l resolved_root (path resolve -- "$fake_repo" 2>/dev/null)
test -z "$resolved_root"; and set resolved_root $fake_repo
set -l cache_key (__gpy_path_to_cache_key "$resolved_root")

# Distinct sentinel bytes per variant so a mismatch is unambiguous.
printf GIT-NORMAL >"$cache_dir/$cache_key.git.none.ansi"
printf GIT-LAST >"$cache_dir/$cache_key.git_last.none.ansi"
printf GIT-FIRST >"$cache_dir/$cache_key.git_first.none.ansi"
printf GIT-FIRST-LAST >"$cache_dir/$cache_key.git_first_last.none.ansi"

printf LANG-NORMAL >"$cache_dir/$cache_key.lang.none.ansi"
printf LANG-LAST >"$cache_dir/$cache_key.lang_last.none.ansi"
printf LANG-FIRST >"$cache_dir/$cache_key.lang_first.none.ansi"
printf LANG-FIRST-LAST >"$cache_dir/$cache_key.lang_first_last.none.ansi"

set -g __gpy_prompt_now (date +%s)
set -gx GPY_LANGUAGE_CACHE_TTL_SECONDS 30

# Segment render functions receive is_last/is_first as "true"/"".
check "git: not first, not last" pass (test (segment_git_render) = GIT-NORMAL; and echo pass; or echo fail)
check "git: last, not first" pass (test (segment_git_render true) = GIT-LAST; and echo pass; or echo fail)
check "git: first, not last" pass (test (segment_git_render "" true) = GIT-FIRST; and echo pass; or echo fail)
check "git: first and last" pass (test (segment_git_render true true) = GIT-FIRST-LAST; and echo pass; or echo fail)

check "lang: not first, not last" pass (test (segment_language_render) = LANG-NORMAL; and echo pass; or echo fail)
check "lang: last, not first" pass (test (segment_language_render true) = LANG-LAST; and echo pass; or echo fail)
check "lang: first, not last" pass (test (segment_language_render "" true) = LANG-FIRST; and echo pass; or echo fail)
check "lang: first and last" pass (test (segment_language_render true true) = LANG-FIRST-LAST; and echo pass; or echo fail)

# Cleanup
cd -
rm -rf "$tmp_dir"

test_finish "is_first instant-cache tests"
