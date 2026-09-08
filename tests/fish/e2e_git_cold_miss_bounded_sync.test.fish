#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: Git Segment Cold-Miss Bounded Synchronous IPC (#434)
# ============================================================================
#
# Before this fix, the FIRST prompt rendered after `cd`-ing into a repo whose
# instant-cache is cold (new repo, evicted entry, fresh shell) showed NO git
# segment at all -- segment_git_render's cold-miss branch only omitted the
# segment and kicked off a throttled background refresh, relying on the
# agent's SIGUSR1 repaint to fill it in on a LATER prompt (and only while the
# agent stays up).
#
# AC1: with the agent UP and the instant cache genuinely cold for a repo, the
# very first segment_git_render call must return non-empty, correct output --
# a bounded synchronous IPC round-trip (<= GPY_IPC_TIMEOUT_MS) instead of the
# old omit-and-wait-for-async-repaint behavior.
#
# AC2 (regression guard, already true pre-fix): with the agent DOWN, a
# cold-miss render must still omit gracefully. The `test -S` liveness gate
# must skip the IPC attempt entirely -- no connect, no timeout wait -- so the
# agent-down case is structurally instant rather than merely "usually fast".
#
# Both scenarios drive segment_git_render in-process (no subshell) against a
# freshly created, never-queried repo, so the instant-prompt cache is cold by
# construction rather than by racing an eviction.
#
# Expected runtime: ~6 seconds

source (dirname (status -f))/../lib/test_helpers.fish

# Predicate for poll_until: the agent socket file is gone (agent truly down).
function __gpy_socket_absent
    not test -S "$GPY_AGENT_SOCKET_PATH"
end

# Sanity predicate: no git instant-cache file exists yet in this test's
# isolated XDG_CACHE_HOME. Used to confirm a scenario really starts cold
# rather than accidentally inheriting a warm cache from a prior step. Uses
# `find`, not a `*.git.*.ansi` glob, so an empty (or not-yet-created)
# directory doesn't trip fish's "no matches for wildcard" error (mirrors
# cleanup_git_test_files in tests/lib/test_helpers.fish).
function __gpy_no_git_cache_files
    set -l cache_dir $XDG_CACHE_HOME/gpy/instant-prompts
    set -l f (find "$cache_dir" -maxdepth 1 -name '*.git.*.ansi' 2>/dev/null | head -1)
    test -z "$f"
end

function test_cold_miss_agent_up_renders_synchronously
    print_test_header "E2E Test: Cold-Miss Git Segment Renders via Bounded Sync IPC (agent up)"
    init_test_env

    # Source GPY so segment_git_render and its helpers are available in-process.
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

    # Confirm the instant cache is genuinely cold for this repo before the
    # render under test -- nothing has registered, queried, or watched it yet.
    if __gpy_no_git_cache_files
        print_test_result "Instant cache is cold before render" PASS
    else
        print_test_result "Instant cache is cold before render" FAIL "a git cache file already exists; test setup did not start cold"
        cleanup_test_files
        return 1
    end

    # cd into the never-queried repo and render immediately -- no warm-up
    # client, no prior registration, no prior IPC call for this repo. Pre-fix
    # this is exactly the cold-miss omit path (returns empty). Post-fix the
    # bounded synchronous IPC query inside the cold-miss branch should
    # populate correct output on THIS very render.
    cd $repo
    rm -f (__gpy_oneshot_marker) 2>/dev/null
    set -l rendered (segment_git_render)

    if test -n "$rendered"
        print_test_result "First render on cold cache is non-empty (bounded sync IPC)" PASS
    else
        print_test_result "First render on cold cache is non-empty (bounded sync IPC)" FAIL "segment_git_render returned empty on the very first render with the agent up"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

function test_cold_miss_agent_down_omits_within_budget
    print_test_header "E2E Test: Cold-Miss Git Segment Omits Gracefully (agent down)"
    init_test_env

    # Source GPY so segment_git_render and its helpers are available in-process.
    source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
    source $__gpy_root/fish/segments/git.fish

    set -l repo (create_test_repo)
    if test -z "$repo"
        print_test_result "Create test repo" FAIL "Unable to create temporary git repo"
        cleanup_test_files
        return 1
    end

    # Deliberately never start the agent for this scenario. Confirm the
    # socket is truly absent so the render's cheap `test -S` liveness gate --
    # the same gate __gpy_ipc_send uses -- skips the IPC attempt entirely
    # rather than racing a timeout.
    if poll_until 3 __gpy_socket_absent
        print_test_result "Agent socket absent" PASS
    else
        print_test_result "Agent socket absent" FAIL "a socket unexpectedly exists; cannot exercise the agent-down path"
        cleanup_test_files
        return 1
    end

    cd $repo
    rm -f (__gpy_oneshot_marker) 2>/dev/null
    set -l rendered (segment_git_render)

    if test -z "$rendered"
        print_test_result "Cold-miss render omits gracefully with agent down" PASS
    else
        print_test_result "Cold-miss render omits gracefully with agent down" FAIL "expected empty output with agent down, got: $rendered"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

if not test_cold_miss_agent_up_renders_synchronously
    exit 1
end

if not test_cold_miss_agent_down_omits_within_budget
    exit 1
end

exit 0
