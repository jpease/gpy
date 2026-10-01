#!/usr/bin/env fish
# Test: __gpy_ensure_segments_loaded lazily sources segment files newly added
# to __enabled_segments (e.g. by an agent config/theme reload) without a new shell.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"

set -g pass_count 0
set -g fail_count 0

function check --argument-names label result
    if test "$result" = pass
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label"
    end
end

# --- Setup: init GPY with a reduced segment list so 'duration' is never
# sourced at shell init, mirroring a theme that didn't include it. Erase any
# duration functions the ambient shell config may have already defined, so
# the "not loaded yet" assertion below is deterministic. ---
functions -e segment_duration_detect 2>/dev/null
functions -e segment_duration_render 2>/dev/null
functions -e segment_duration_init 2>/dev/null

set -e __enabled_segments
set -gx GPY_MINIMAL_SEGMENTS 1

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402). init.fish redefines
# __gpy_load_theme itself when sourced, so the no-op stub below only
# protects the call site if this isolation is ever removed.
set -gx XDG_CACHE_HOME (mktemp -d)

function __gpy_load_theme
    # No-op - avoid spawning/connecting to a real agent in this test.
end

source "$repo_root/fish/core/init.fish"
# fish_prompt lives in an autoloaded function file, not init.fish. Source it
# explicitly (as the other prompt-rendering tests do) instead of relying on the
# developer's installed copy — without this the assertion below silently
# measured Fish's *default* prompt on a clean machine, so it passed locally and
# failed in CI (#270).
source "$repo_root/fish/functions/fish_prompt.fish"
set -e GPY_MINIMAL_SEGMENTS

if functions -q segment_duration_detect
    check "duration segment is NOT loaded before reload" fail
else
    check "duration segment is NOT loaded before reload" pass
end

# --- Simulate a reload that enables 'duration' (segment new to this shell) ---
set -g __enabled_segments duration

__gpy_ensure_segments_loaded

if functions -q segment_duration_detect
    check "duration segment IS loaded after __gpy_ensure_segments_loaded" pass
else
    check "duration segment IS loaded after __gpy_ensure_segments_loaded" fail
end

# --- fish_prompt renders the newly-loaded segment ---
set -gx CMD_DURATION 5000

function __gpy_request_duration
    printf '%s' GPY_TEST_DURATION_MARKER
end

set -l prompt_output (fish_prompt | string collect)
if string match -q '*GPY_TEST_DURATION_MARKER*' -- "$prompt_output"
    check "fish_prompt output includes newly-loaded duration segment" pass
else
    check "fish_prompt output includes newly-loaded duration segment" fail
end

set -e CMD_DURATION

# --- Idempotency: already-loaded segments are left alone (no error, no re-init side effects) ---
set -g __gpy_test_duration_init_calls 0
function segment_duration_init
    set -g __gpy_test_duration_init_calls (math $__gpy_test_duration_init_calls + 1)
end
__gpy_ensure_segments_loaded
if test "$__gpy_test_duration_init_calls" -eq 0
    check "already-loaded segment is not re-initialized" pass
else
    check "already-loaded segment is not re-initialized" fail
end

# --- Negative case: an unknown segment must not error ---
set -g __enabled_segments duration does-not-exist-segment
if __gpy_ensure_segments_loaded
    check "unknown segment does not error out" pass
else
    check "unknown segment does not error out" fail
end

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count segment lazy-reload tests passed"
