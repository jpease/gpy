#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: Git Segment Variant-Fallback Bounded Sync (#436)
# ============================================================================
#
# `__gpy_read_instant_cache` falls back to the context-free `.none` variant
# when no token-specific file exists yet for the render's real `prev_bg` (a
# NEW powerline context, e.g. the first render after the previous segment's
# background changed). The `.none` content is correct git status, but its
# opening chevron was rendered without prev_bg context, so its color is
# wrong -- and because the read still returns non-empty, pre-fix
# `segment_git_render` served it as an ordinary warm hit. The wrong-color
# chevron could then persist beyond a single render cycle.
#
# AC1: with the agent UP, the FIRST render at a new prev_bg context must show
# the CORRECT chevron color (a bounded synchronous IPC query with the real
# prev_bg, mirroring the #434 cold-miss branch) instead of the `.none`
# fallback's chevron.
#
# The stock default theme intentionally does NOT vary the git segment's caps
# by prev_bg (only `[segments.character]` does -- see the
# `git_cache_writes_one_file_per_prev_bg_context` regression test in
# gpy-agent/src/cache/instant_prompt.rs, which asserts the rendered bytes are
# IDENTICAL across prev_bg contexts for the default theme). To make the
# chevron-color difference assertable at the byte level from Fish, this test
# installs a custom theme whose git format reads `fg:prev_bg` for its opening
# separator (exactly like `[segments.character]` already does), which is
# exactly the class of theme #436 describes.
#
# Expected runtime: ~8 seconds

source (dirname (status -f))/../lib/test_helpers.fish

# The prev_bg context this test renders at. Numeric so it parses as a plain
# Ansi256 index (gpy-agent/src/template/style.rs `parse_color`), producing an
# unambiguous `38;5;222` SGR fragment in the rendered chevron -- distinct from
# any named theme color, so it cannot coincidentally collide with a color a
# real preceding segment (directory, etc.) already wrote a cache file for
# during registration's background render.
set -g __gpy_test_prev_bg 222

# Predicate for poll_until: a `.none` git instant-cache file exists for the repo.
function __gpy_git_none_cache_present
    set -l f (find $XDG_CACHE_HOME/gpy/instant-prompts -maxdepth 1 -name '*.git.none.ansi' 2>/dev/null | head -1)
    test -n "$f"
end

# Sanity predicate: no token-specific cache file exists yet for our chosen
# prev_bg context. Confirms the fallback scenario starts genuinely absent
# rather than accidentally warmed by an earlier step.
function __gpy_git_token_cache_absent
    set -l f (find $XDG_CACHE_HOME/gpy/instant-prompts -maxdepth 1 -name "*.git.$__gpy_test_prev_bg.ansi" 2>/dev/null | head -1)
    test -z "$f"
end

function test_variant_fallback_bounded_sync
    print_test_header "E2E Test: Git Segment Variant-Fallback Renders via Bounded Sync IPC (#436)"
    init_test_env

    # Install a custom theme whose git segment's opening separator reads
    # `fg:prev_bg` (like `[segments.character]` in the stock theme already
    # does) instead of `fg:$bg`, so the chevron's color actually varies with
    # prev_bg and the fix is observable at the byte level.
    set -l theme_dir $XDG_CONFIG_HOME/gpy/themes
    mkdir -p $theme_dir
    set -l theme_file $theme_dir/test-theme.toml
    cat $__gpy_root/config/themes/default.toml | string replace -- \
        '[$sep_open](fg:$bg bg:default))[ $symbol $branch]' \
        '[$sep_open](fg:prev_bg bg:default))[ $symbol $branch]' >$theme_file

    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '[ui]\ntheme = "test-theme"\nenabled_segments = ["git"]\n' >$XDG_CONFIG_HOME/gpy/config.toml

    # Source GPY (after the theme/config are in place) so segment_git_render,
    # its helpers, and the theme-derived color variables are available
    # in-process.
    source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
    source $__gpy_root/fish/segments/git.fish

    set -l repo (create_test_repo)
    if test -z "$repo"
        print_test_result "Create test repo" FAIL "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    if start_test_agent
        print_test_result "Agent Start" PASS
    else
        print_test_result "Agent Start" FAIL "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end

    # Register a client so the agent watches the repo and writes its
    # context-free `.none` instant cache (background writes carry no
    # prev_bg, so only `.none` files get produced by this step).
    set -l client (register_test_client 20 $repo)
    if test -z "$client"
        print_test_result "Register client" FAIL "Failed to spawn test client"
        cleanup_test_files
        return 1
    end

    if poll_until 8 __gpy_git_none_cache_present
        print_test_result "Instant cache created (.none variant)" PASS
    else
        print_test_result "Instant cache created (.none variant)" FAIL "no .none git cache written after registration"
        cleanup_test_files
        return 1
    end

    if __gpy_git_token_cache_absent
        print_test_result "No token-specific cache file for chosen prev_bg yet" PASS
    else
        print_test_result "No token-specific cache file for chosen prev_bg yet" FAIL "a git.$__gpy_test_prev_bg.ansi file already exists; test setup did not start with a genuine variant gap"
        cleanup_test_files
        return 1
    end

    # Render at the NEW prev_bg context -- never queried for this repo before.
    cd $repo
    set -e __gpy_oneshot_used
    set -g __gpy_last_segment_bg $__gpy_test_prev_bg
    set -l rendered (segment_git_render)

    if test -z "$rendered"
        print_test_result "Variant-fallback render is non-empty" FAIL "segment_git_render produced no output"
        cleanup_test_files
        return 1
    end
    print_test_result "Variant-fallback render is non-empty" PASS

    # The correct chevron for prev_bg=222 encodes as the SGR fragment
    # `38;5;222` (Color::Ansi256(222) fg). The `.none` fallback's chevron
    # would instead resolve Color::PrevBg -> Named("default") -> SGR `39` --
    # it never contains `38;5;222`. Pre-fix this assertion fails (the render
    # serves the raw `.none` bytes); post-fix the bounded sync IPC query uses
    # the real prev_bg, so the response bakes in the correct chevron color.
    if string match -q '*38;5;222*' -- $rendered
        print_test_result "First render at new prev_bg context shows correct chevron color" PASS
    else
        print_test_result "First render at new prev_bg context shows correct chevron color" FAIL "rendered output did not contain the prev_bg=222 chevron color (38;5;222); served the wrong-context .none fallback instead"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

if not test_variant_fallback_bounded_sync
    exit 1
end

exit 0
