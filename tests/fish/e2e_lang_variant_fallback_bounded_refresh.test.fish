#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: Language Segment Variant-Fallback Bounded Refresh (#454)
# ============================================================================
#
# `__gpy_read_instant_cache` falls back to the context-free `.none` variant
# when no token-specific file exists yet for the render's real `prev_bg` (a
# NEW powerline context, e.g. the first render after the previous segment's
# background changed). For git this is already corrected (#436) via a bounded
# SYNCHRONOUS IPC query. Pre-#454 the language branch computed the same
# fallback but discarded it -- the language segment treated the fallback as an
# ordinary warm hit and could redisplay the wrong-context chevron indefinitely,
# never causing the token-specific entry to be written.
#
# Unlike git, language's cold-miss path is deliberately background-only (see
# the comment in fish/segments/language.fish) to avoid blocking the prompt on
# slow language detection. The language fix therefore corrects via a bounded
# BACKGROUND refresh, not a synchronous one: the `.none` content is already
# correct (only the chevron color is wrong), so the milder defect doesn't
# justify giving up language's "never blocks" latency guarantee.
#
# The bug report is specifically about SWITCHING backgrounds ("after the
# segment before language changes background color, the language segment's
# opening separator can retain the wrong foreground/background transition
# indefinitely") -- a single-token test cannot catch a fix that happens to
# special-case `none` while still serving one token's stale ANSI when the
# render moves to a DIFFERENT token. This test therefore exercises TWO
# distinct numeric prev_bg tokens (A and B) and asserts:
#
#   1. The FIRST render at token A still serves the `.none` fallback content
#      (correct language info, wrong-context chevron) -- proving the
#      correction is NOT synchronous.
#   2. A bounded background refresh (throttled, real prev_bg=A) writes the
#      token-A-specific cache file shortly after.
#   3. A SECOND render at token A then reads the token-A file directly and
#      shows token A's CORRECT chevron color.
#   4. A render at token B (a context switch, never queried before) does NOT
#      reuse token A's cached ANSI, and fires its OWN bounded background
#      correction (proving the throttle is keyed per-token, not just
#      per-path+suffix -- a shared key would let A's recent refresh starve
#      B's correction).
#   5. A SECOND render at token B then shows token B's CORRECT chevron color.
#
# The stock default theme does not vary the language segment's opening
# separator by prev_bg. To make the chevron-color difference assertable at
# the byte level from Fish, this test installs a custom theme whose language
# format reads `fg:prev_bg` for its opening separator (exactly like
# `[segments.character]` in the stock theme already does), mirroring
# tests/fish/e2e_git_variant_fallback_bounded_sync.test.fish.
#
# Expected runtime: ~10 seconds

source (dirname (status -f))/../lib/test_helpers.fish

# The two prev_bg contexts this test renders at. Numeric so each parses as a
# plain Ansi256 index (gpy-agent/src/template/style.rs `parse_color`),
# producing unambiguous `38;5;NNN` SGR fragments in the rendered chevron --
# distinct from each other, from the git e2e test's 222 token, and from any
# named theme color (which always renders as `38;2;r;g;b` true color, never
# `38;5;NNN`, so there is no cross-format collision risk either).
set -g __gpy_test_prev_bg_a 231
set -g __gpy_test_prev_bg_b 213

# Predicate for poll_until: a `.none` lang instant-cache file exists for the repo.
function __gpy_lang_none_cache_present
    set -l f (find $XDG_CACHE_HOME/gpy/instant-prompts -maxdepth 1 -name '*.lang.none.ansi' 2>/dev/null | head -1)
    test -n "$f"
end

# Sanity predicate: no token-specific cache file exists yet for the given
# prev_bg token. Confirms the fallback scenario starts genuinely absent rather
# than accidentally warmed by an earlier step.
function __gpy_lang_token_cache_absent --argument-names token
    set -l f (find $XDG_CACHE_HOME/gpy/instant-prompts -maxdepth 1 -name "*.lang.$token.ansi" 2>/dev/null | head -1)
    test -z "$f"
end

# Predicate for poll_until: the token-specific lang cache file for the given
# prev_bg token has been written by the bounded background correction.
function __gpy_lang_token_cache_present --argument-names token
    set -l f (find $XDG_CACHE_HOME/gpy/instant-prompts -maxdepth 1 -name "*.lang.$token.ansi" 2>/dev/null | head -1)
    test -n "$f"
end

function test_variant_fallback_bounded_refresh
    print_test_header "E2E Test: Language Segment Variant-Fallback Renders via Bounded Background Refresh (#454)"
    init_test_env

    # Install a custom theme whose language segment's opening separator reads
    # `fg:prev_bg` instead of `fg:$bg`, so the chevron's color actually varies
    # with prev_bg and the fix is observable at the byte level.
    set -l theme_dir $XDG_CONFIG_HOME/gpy/themes
    mkdir -p $theme_dir
    set -l theme_file $theme_dir/test-theme.toml
    cat $__gpy_root/config/themes/default.toml | string replace -- \
        '[$sep_open](fg:$bg bg:default))[ $symbol]($style)([ $version]' \
        '[$sep_open](fg:prev_bg bg:default))[ $symbol]($style)([ $version]' >$theme_file

    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '[ui]\ntheme = "test-theme"\nenabled_segments = ["language"]\n' >$XDG_CONFIG_HOME/gpy/config.toml

    # Source GPY (after the theme/config are in place) so segment_language_render,
    # its helpers, and the theme-derived color variables are available
    # in-process.
    source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
    source $__gpy_root/fish/segments/language.fish

    set -l repo (create_test_repo)
    if test -z "$repo"
        print_test_result "Create test repo" FAIL "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    # A bare repo (just .git + README) has no detectable language, so the
    # agent would render empty content and the fallback-vs-corrected chevron
    # would never be byte-observable. A bare Cargo.toml alone only detects as
    # a generic "TOML" file with no version, which also renders empty -- add a
    # real .rs source file too so the agent detects Rust and runs its version
    # detector (`rustc --version`).
    mkdir -p $repo/src
    printf '[package]\nname = "gpy-e2e-test"\nversion = "0.1.0"\n' >$repo/Cargo.toml
    printf 'fn main() {}\n' >$repo/src/main.rs

    # Stub out `rustc` with a deterministic fake so the rendered version text
    # doesn't depend on whatever real Rust toolchain (rustup/mise/etc.) happens
    # to be installed on the machine running this test -- version-manager
    # shims can themselves depend on $XDG_CONFIG_HOME/$HOME state that this
    # test's isolation (init_test_env) deliberately overrides, which would
    # otherwise make `rustc --version` resolve inconsistently. Prepending to
    # PATH before starting the agent means the daemon subprocess inherits it.
    set -l fake_bin_dir $__gpy_test_tmp_dir/fakebin
    mkdir -p $fake_bin_dir
    printf '#!/bin/sh\necho "rustc 1.99.0 (fake e2e binary)"\n' >$fake_bin_dir/rustc
    chmod +x $fake_bin_dir/rustc
    set -gx PATH $fake_bin_dir $PATH

    if start_test_agent
        print_test_result "Agent Start" PASS
    else
        print_test_result "Agent Start" FAIL "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    cd $repo

    # Prime the context-free `.none` language cache: the first render in this
    # repo (default prev_bg, unset -> "none" token) cold-misses and fires the
    # existing throttled background refresh (language.fish step 2), which
    # populates the `.none` variant files via the agent.
    rm -f (__gpy_oneshot_marker) 2>/dev/null
    set -e __gpy_last_segment_bg
    segment_language_render >/dev/null

    if poll_until 8 __gpy_lang_none_cache_present
        print_test_result "Instant cache created (.none variant)" PASS
    else
        print_test_result "Instant cache created (.none variant)" FAIL "no .none lang cache written after initial render"
        cleanup_test_files
        return 1
    end

    if __gpy_lang_token_cache_absent $__gpy_test_prev_bg_a
        print_test_result "No token-specific cache file for token A yet" PASS
    else
        print_test_result "No token-specific cache file for token A yet" FAIL "a lang.$__gpy_test_prev_bg_a.ansi file already exists; test setup did not start with a genuine variant gap"
        cleanup_test_files
        return 1
    end

    # --- Render at token A: a NEW prev_bg context, never queried before. ---
    rm -f (__gpy_oneshot_marker) 2>/dev/null
    set -g __gpy_last_segment_bg $__gpy_test_prev_bg_a
    set -l rendered_a1 (segment_language_render)

    if test -z "$rendered_a1"
        print_test_result "First render at token A is non-empty" FAIL "segment_language_render produced no output"
        cleanup_test_files
        return 1
    end
    print_test_result "First render at token A is non-empty" PASS

    # AC2: the correction must be BOUNDED, not synchronous -- the first render
    # at the new context still serves the `.none` fallback's chevron (does NOT
    # contain token A's SGR fragment). This documents the language-vs-git
    # judgment call: language's cold-miss path is deliberately background-only,
    # so the correction here is a throttled background refresh, not a bounded
    # synchronous IPC query.
    if string match -q "*38;5;$__gpy_test_prev_bg_a*" -- $rendered_a1
        print_test_result "First render at token A does not block for a synchronous correction" FAIL "unexpectedly contained the corrected chevron already; language's cold-miss path is supposed to stay background-only"
        cleanup_test_files
        return 1
    else
        print_test_result "First render at token A does not block for a synchronous correction" PASS
    end

    # AC3: the corrected token-A-specific cache entry is written (bounded
    # background refresh) and later reused.
    if poll_until 8 __gpy_lang_token_cache_present $__gpy_test_prev_bg_a
        print_test_result "Token-A cache file written by bounded background refresh" PASS
    else
        print_test_result "Token-A cache file written by bounded background refresh" FAIL "no lang.$__gpy_test_prev_bg_a.ansi file appeared after the variant-fallback render"
        cleanup_test_files
        return 1
    end

    # Second render at token A now hits the token-specific file directly and
    # shows the correct chevron color. segment_language_render overwrites
    # __gpy_last_segment_bg with its OWN background on completion (so a
    # following segment would chain off it) -- reset it back to token A to
    # simulate a fresh render where the PRECEDING segment again produced this
    # same background.
    rm -f (__gpy_oneshot_marker) 2>/dev/null
    set -g __gpy_last_segment_bg $__gpy_test_prev_bg_a
    set -l rendered_a2 (segment_language_render)

    if string match -q "*38;5;$__gpy_test_prev_bg_a*" -- $rendered_a2
        print_test_result "Second render at token A shows token A's correct chevron color" PASS
    else
        print_test_result "Second render at token A shows token A's correct chevron color" FAIL "rendered output did not contain token A's chevron color (38;5;$__gpy_test_prev_bg_a): $rendered_a2"
        cleanup_test_files
        return 1
    end

    # --- Now switch to token B: this is the actual reported bug. A fix that
    # only special-cases the `none` token could still wrongly reuse token A's
    # now-warm cached ANSI (or fail to fire its own correction) when the
    # render moves to a DIFFERENT prev_bg context. ---
    if __gpy_lang_token_cache_absent $__gpy_test_prev_bg_b
        print_test_result "No token-specific cache file for token B yet" PASS
    else
        print_test_result "No token-specific cache file for token B yet" FAIL "a lang.$__gpy_test_prev_bg_b.ansi file already exists; test setup did not start with a genuine variant gap"
        cleanup_test_files
        return 1
    end

    rm -f (__gpy_oneshot_marker) 2>/dev/null
    set -g __gpy_last_segment_bg $__gpy_test_prev_bg_b
    set -l rendered_b1 (segment_language_render)

    if test -z "$rendered_b1"
        print_test_result "First render at token B is non-empty" FAIL "segment_language_render produced no output"
        cleanup_test_files
        return 1
    end
    print_test_result "First render at token B is non-empty" PASS

    # Must NOT reuse token A's cached ANSI -- this is the core regression
    # guard for the reported bug (switching backgrounds retaining the wrong
    # transition indefinitely).
    if string match -q "*38;5;$__gpy_test_prev_bg_a*" -- $rendered_b1
        print_test_result "First render at token B does not reuse token A's chevron" FAIL "rendered output still contained token A's chevron color (38;5;$__gpy_test_prev_bg_a): $rendered_b1"
        cleanup_test_files
        return 1
    else
        print_test_result "First render at token B does not reuse token A's chevron" PASS
    end

    # AC3 for token B, and a live test of the per-token throttle key (judgment
    # call #2): if the throttle were keyed only by path+suffix (not also by
    # token), token A's very recent correction could starve this one.
    if poll_until 8 __gpy_lang_token_cache_present $__gpy_test_prev_bg_b
        print_test_result "Token-B cache file written by bounded background refresh" PASS
    else
        print_test_result "Token-B cache file written by bounded background refresh" FAIL "no lang.$__gpy_test_prev_bg_b.ansi file appeared after the variant-fallback render"
        cleanup_test_files
        return 1
    end

    rm -f (__gpy_oneshot_marker) 2>/dev/null
    set -g __gpy_last_segment_bg $__gpy_test_prev_bg_b
    set -l rendered_b2 (segment_language_render)

    if string match -q "*38;5;$__gpy_test_prev_bg_b*" -- $rendered_b2
        print_test_result "Second render at token B shows token B's correct chevron color" PASS
    else
        print_test_result "Second render at token B shows token B's correct chevron color" FAIL "rendered output did not contain token B's chevron color (38;5;$__gpy_test_prev_bg_b): $rendered_b2"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

if not test_variant_fallback_bounded_refresh
    exit 1
end

exit 0
