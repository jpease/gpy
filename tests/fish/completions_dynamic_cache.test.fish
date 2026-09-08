#!/usr/bin/env fish
# tests/fish/completions_dynamic_cache.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #346 (Epic #326 follow-up): per-session caching of
# `gpy __complete <kind>` in fish/completions/gpy-dynamic.fish.
#
# `__gpy_complete_cached <kind>` is meant to fork `gpy __complete <kind>` at
# most once per kind per shell session -- every TAB press after the first
# should be served from a session-global variable instead of re-forking. A
# failed/empty result must NEVER be cached (so a transient failure, or `gpy`
# not yet on PATH, self-heals on the next TAB instead of being stuck).
#
# Rather than driving the real `complete -C` completion engine (already
# covered end-to-end by tests/fish/completions_dynamic.test.fish), this test
# calls `__gpy_complete_cached` directly against a stubbed `gpy` function
# with a call counter -- a fish function shadows any real `gpy` binary on
# PATH, mirroring the stub-with-counter pattern used in
# tests/fish/character_directory_memoization.test.fish (#343).

set -l test_root (status dirname)/../..
set -g repo_root (path resolve $test_root)

source $repo_root/fish/completions/gpy-dynamic.fish

set -g fail_count 0

function check --argument-names label expected actual
    if test "$expected" = "$actual"
        echo "✓ $label"
    else
        echo "FAIL: $label (expected [$expected], got [$actual])"
        set -g fail_count (math $fail_count + 1)
    end
end

function reset_cache_state
    set -e __gpy_complete_cache_theme
    set -e __gpy_complete_cache_segment
    set -g gpy_call_count 0
end

# ---------------------------------------------------------------------------
# 1. Warm cache: a second call for the same kind must not re-fork `gpy`.
# ---------------------------------------------------------------------------
reset_cache_state
function gpy
    set -g gpy_call_count (math $gpy_call_count + 1)
    if test "$argv[1]" = __complete
        printf 'default\nstarship\ntext\n'
    end
end

# NOTE: `check` receives its "actual" arg from a quoted list variable
# (`"$first"`), which fish joins with SPACES, not the original newlines
# (fish's quoting semantics for a multi-element list). Expected strings below
# are written space-joined to match, via `string join ' ' -- ...`.
set -l first (__gpy_complete_cached theme)
check "first call returns candidates" "default starship text" "$first"
check "first call forks gpy once" 1 $gpy_call_count

set -l second (__gpy_complete_cached theme)
check "second call returns the same candidates" "default starship text" "$second"
check "second call does not fork gpy again (cache hit)" 1 $gpy_call_count

# ---------------------------------------------------------------------------
# 2. Different kinds are cached independently.
# ---------------------------------------------------------------------------
function gpy
    set -g gpy_call_count (math $gpy_call_count + 1)
    if test "$argv[1]" = __complete
        switch $argv[2]
            case theme
                printf 'default\n'
            case segment
                printf 'clock\ngit\n'
        end
    end
end

# gpy_call_count carries over from section 1 (ended at 1): a segment miss
# forks once more (-> 2); the still-warm theme cache must not add a 3rd.
set -l seg_first (__gpy_complete_cached segment)
check "segment cache miss forks gpy" 2 $gpy_call_count
check "segment candidates correct" "clock git" "$seg_first"

set -l theme_again (__gpy_complete_cached theme)
check "theme cache is untouched by a different kind's miss" 2 $gpy_call_count
check "theme candidates still correct after segment lookup" "default starship text" "$theme_again"

# ---------------------------------------------------------------------------
# 3. Empty/failed result is never cached: every call retries.
# ---------------------------------------------------------------------------
reset_cache_state
function gpy
    set -g gpy_call_count (math $gpy_call_count + 1)
    return 1
end

set -l failed_first (__gpy_complete_cached theme)
check "failed first call returns nothing" "" "$failed_first"
check "failed call forks gpy" 1 $gpy_call_count

set -l failed_second (__gpy_complete_cached theme)
check "failed second call returns nothing" "" "$failed_second"
check "empty/failed result is never cached (retries)" 2 $gpy_call_count

# ---------------------------------------------------------------------------
# 4. Once a retry succeeds, the cache then holds for subsequent calls.
# ---------------------------------------------------------------------------
function gpy
    set -g gpy_call_count (math $gpy_call_count + 1)
    printf 'nord\n'
end

set -l recovered (__gpy_complete_cached theme)
check "recovered call returns candidates" nord "$recovered"
check "recovery forks gpy once more" 3 $gpy_call_count

set -l recovered_again (__gpy_complete_cached theme)
check "post-recovery call hits cache" nord "$recovered_again"
check "post-recovery call does not re-fork gpy" 3 $gpy_call_count

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed"
    exit 1
end
echo "PASS: all completion cache tests passed"
