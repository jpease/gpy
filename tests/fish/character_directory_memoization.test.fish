#!/usr/bin/env fish
# tests/fish/character_directory_memoization.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Verifies the character and directory segments memoize their agent-rendered
# output (#343): a repeat render with the same input tuple must make zero
# additional IPC calls (cache hit), an exit-status flip must re-render the
# character, a `cd` must re-render the directory, a theme change (SIGUSR2)
# must re-render both, a manual reload (__gpy_reload_theme / prompt-reload)
# must re-render both even without a theme name change (#576), and an
# empty/failed render must never be cached.

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402). Pre-seed it with an empty,
# no-op cache file so every source-or-spawn call in this test
# (__gpy_load_theme, __gpy_reload_theme, __gpy_sigusr2_handler) takes the
# fast "source the cache" path instead of possibly falling through to spawn
# a REAL gpy-agent binary, if one happens to be resolvable on this machine
# -- a real spawn would clobber __gpy_theme_name with a real theme's name,
# defeating the "reload without a theme name change" isolation step 5 below
# needs (#576).
set -gx XDG_CACHE_HOME (mktemp -d)
mkdir -p "$XDG_CACHE_HOME/gpy"
touch "$XDG_CACHE_HOME/gpy/theme-export.fish"

source fish/core/init.fish
source fish/functions/fish_prompt.fish
source fish/segments/directory.fish

set -g __gpy_char_call_count 0
set -g __gpy_dir_call_count 0

function __gpy_request_character
    set -g __gpy_char_call_count (math $__gpy_char_call_count + 1)
    printf CHAR
end

function __gpy_request
    set -g __gpy_dir_call_count (math $__gpy_dir_call_count + 1)
    printf DIR
end

# Minimal, deterministic prompt environment.
set -g __enabled_segments directory
set -g __gpy_is_root 0
set -g __prompt_color white
set -g __icon_prompt "❯"
set -g __icon_status_ok STATUSOK
set -g __icon_status_fail STATUSFAIL
set -g __gpy_theme_name testtheme
set -g __color_directory_bg blue

set -g fail_count 0

function check --argument-names label expected actual
    if test "$expected" -eq "$actual"
        echo "✓ $label"
    else
        echo "FAIL: $label (expected $expected, got $actual)"
        set -g fail_count (math $fail_count + 1)
    end
end

# Every step below clears both caches first so it starts from a known state:
# a baseline render always misses (call count -> 1 for both segments), and
# the step then asserts what a second, deliberately-varied render does.
function reset_caches
    set -e __gpy_char_cache_key
    set -e __gpy_char_cache_val
    set -e __gpy_dir_cache_key
    set -e __gpy_dir_cache_val
end

set -l tmp_a (mktemp -d)
set -l tmp_b (mktemp -d)
cd $tmp_a

# --- 1. Cache hit: identical status/PWD/theme -> zero additional IPC calls ---
reset_caches
set -g __gpy_char_call_count 0
set -g __gpy_dir_call_count 0
true
fish_prompt >/dev/null 2>&1 # baseline render (miss for both)
true
fish_prompt >/dev/null 2>&1 # identical render (must hit for both)
check "second identical render makes no extra character IPC call" 1 $__gpy_char_call_count
check "second identical render makes no extra directory IPC call" 1 $__gpy_dir_call_count

# --- 2. Exit status flip re-renders the character only ---
reset_caches
set -g __gpy_char_call_count 0
set -g __gpy_dir_call_count 0
true
fish_prompt >/dev/null 2>&1 # baseline (miss for both, count=1 each)
false
fish_prompt >/dev/null 2>&1 # status flipped
check "exit status flip re-renders character" 2 $__gpy_char_call_count
check "exit status flip does not re-render directory" 1 $__gpy_dir_call_count

# --- 3. cd re-renders the directory only ---
reset_caches
set -g __gpy_char_call_count 0
set -g __gpy_dir_call_count 0
cd $tmp_a
true
fish_prompt >/dev/null 2>&1 # baseline (miss for both, count=1 each)
cd $tmp_b
true
fish_prompt >/dev/null 2>&1 # PWD changed
check "cd re-renders directory" 2 $__gpy_dir_call_count
check "cd does not re-render character" 1 $__gpy_char_call_count

# --- 4. Theme change (SIGUSR2 handler) re-renders both segments ---
reset_caches
set -g __gpy_char_call_count 0
set -g __gpy_dir_call_count 0
set -g __gpy_theme_name themeA
true
fish_prompt >/dev/null 2>&1 # baseline (miss for both, count=1 each)
set -g __gpy_theme_name themeB
__gpy_sigusr2_handler >/dev/null 2>&1
true
fish_prompt >/dev/null 2>&1 # theme changed
check "theme change re-renders character" 2 $__gpy_char_call_count
check "theme change re-renders directory" 2 $__gpy_dir_call_count

# --- 5. Manual reload (prompt-reload / __gpy_reload_theme) re-renders both
#        segments even when the theme NAME does not change (#576) ---
#
# The character/directory cache key includes the theme name but not its
# content, so a reload of the SAME-NAMED theme (e.g. after editing its
# colors in place) must rely on __gpy_reload_theme's own explicit cache
# clear, not a natural key miss from a name change -- unlike step 4 above.
reset_caches
set -g __gpy_char_call_count 0
set -g __gpy_dir_call_count 0
set -g __gpy_theme_name samename
true
fish_prompt >/dev/null 2>&1 # baseline (miss for both, count=1 each)
__gpy_reload_theme 0 >/dev/null 2>&1
true
fish_prompt >/dev/null 2>&1 # reloaded, same theme name
check "manual reload re-renders character without a name change" 2 $__gpy_char_call_count
check "manual reload re-renders directory without a name change" 2 $__gpy_dir_call_count

# --- 6. An empty/failed render is never cached (agent-down retries) ---
function __gpy_request_character
    set -g __gpy_char_call_count (math $__gpy_char_call_count + 1)
    printf ''
end
reset_caches
set -g __gpy_char_call_count 0
true
fish_prompt >/dev/null 2>&1
true
fish_prompt >/dev/null 2>&1
check "empty character render is never cached" 2 $__gpy_char_call_count

function __gpy_request
    set -g __gpy_dir_call_count (math $__gpy_dir_call_count + 1)
    printf ''
end
reset_caches
set -g __gpy_dir_call_count 0
true
fish_prompt >/dev/null 2>&1
true
fish_prompt >/dev/null 2>&1
check "empty directory render is never cached" 2 $__gpy_dir_call_count

cd -
rm -rf $tmp_a $tmp_b

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed"
    exit 1
end
echo "PASS: all character/directory memoization tests passed"
