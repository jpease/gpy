#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: Agent-Down One-Shot Fallback for a Stale Git Segment (#430)
# ============================================================================
#
# Serve-stale-first (#160) shows the last cached git segment instantly and
# refreshes in the background. That refresh is a fire-and-forget IPC send: with
# the agent DOWN there is nothing to recompute the status or SIGURG a repaint,
# so a stale entry is served indefinitely -- the prompt keeps showing the OLD
# working-tree state until the daemon returns.
#
# AC1: when the git segment would serve a STALE instant-cache entry AND the
# agent socket is absent (`test -S` fails), the render must run a bounded
# foreground `gpy-agent oneshot git` and display THAT correct-but-slower result
# instead of the stale value. The warm/fresh path is untouched.
#
# This test drives segment_git_render in-process: it populates the instant cache
# for a CLEAN repo (agent up), stops the agent, dirties the working tree, then
# renders again with the agent down. Pre-fix the render returns the stale clean
# value; post-fix it returns the fresh dirty value from the oneshot fallback.
#
# Expected runtime: ~8 seconds

source (dirname (status -f))/../lib/test_helpers.fish

# Predicate for poll_until: a git instant-cache file exists for the repo.
function __gpy_git_cache_file_present
    set -l f (ls $XDG_CACHE_HOME/gpy/instant-prompts/*.git.*.ansi 2>/dev/null | head -1)
    test -n "$f"
end

# Predicate for poll_until: the agent socket file is gone (agent truly down).
function __gpy_socket_absent
    not test -S "$GPY_AGENT_SOCKET_PATH"
end

function test_agent_down_oneshot_fallback
    print_test_header "E2E Test: Agent-Down One-Shot Fallback (stale git segment)"
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

    # Register a client so the agent watches the repo and writes its clean-state
    # instant cache (same warm-up path as e2e_git_live_content).
    set -l client (register_test_client 20 $repo)
    if test -z "$client"
        print_test_result "Register client" FAIL "Failed to spawn test client"
        cleanup_test_files
        return 1
    end

    if poll_until 8 __gpy_git_cache_file_present
        print_test_result "Instant cache created (clean state)" PASS
    else
        print_test_result "Instant cache created (clean state)" FAIL "no git cache written after registration"
        cleanup_test_files
        return 1
    end

    # Render from inside the repo. Force every cache read to be STALE so the
    # stale branch (the one the fix guards) is exercised deterministically.
    cd $repo
    set -gx GPY_GIT_INSTANT_CACHE_TTL_SECONDS 0

    # Snapshot the stale clean value the serve-stale path returns (agent up).
    set -l before (segment_git_render)
    if test -z "$before"
        print_test_result "Stale clean render is non-empty" FAIL "segment_git_render produced no output"
        cleanup_test_files
        return 1
    end
    print_test_result "Stale clean render is non-empty" PASS

    # Bring the agent down; confirm the socket is actually gone so the render's
    # cheap `test -S` agent-down probe fails (matches __gpy_ipc_send's gate).
    stop_test_agent >/dev/null
    if poll_until 4 __gpy_socket_absent
        print_test_result "Agent socket removed after stop" PASS
    else
        print_test_result "Agent socket removed after stop" FAIL "socket lingered; cannot exercise agent-down path"
        cleanup_test_files
        return 1
    end

    # Dirty the working tree so a correct render MUST differ from the stale
    # clean value (modify a tracked file -> unstaged marker + dirty color).
    echo dirty >>$repo/README.md

    # New render, agent down + stale cache present. Clear the per-render oneshot
    # marker first (fish_prompt normally does this each prompt).
    rm -f (__gpy_oneshot_marker) 2>/dev/null
    set -l after (segment_git_render)

    if test -z "$after"
        print_test_result "Agent-down render reflects new state" FAIL "segment_git_render produced no output"
        cleanup_test_files
        return 1
    end

    if test "$after" != "$before"
        print_test_result "Agent-down render reflects new state (oneshot fallback)" PASS
    else
        print_test_result "Agent-down render reflects new state (oneshot fallback)" FAIL "render served the STALE clean value instead of the fresh dirty one"
        cleanup_test_files
        return 1
    end

    # The claimed marker must not outlive the shell: a fresh fish that sources
    # the integration, claims its own marker and exits normally leaves nothing
    # named after its PID in TMPDIR (a later process reusing the PID would
    # otherwise start with a spent oneshot budget).
    set -l child_marker (fish -c "
        source $__gpy_root/fish/core/init.fish >/dev/null 2>&1
        __gpy_oneshot_claim
        __gpy_oneshot_marker
        test -e (__gpy_oneshot_marker); or echo NOT_CLAIMED" 2>/dev/null)
    if test -e "$child_marker[1]"
        print_test_result "oneshot marker is removed when the shell exits" FAIL "$child_marker[1] survived the shell that claimed it"
        rm -f "$child_marker[1]"
        cleanup_test_files
        return 1
    else if test "$child_marker[-1]" = NOT_CLAIMED
        print_test_result "oneshot marker is removed when the shell exits" FAIL "the child shell never claimed its marker"
        cleanup_test_files
        return 1
    else
        print_test_result "oneshot marker is removed when the shell exits" PASS
    end

    cleanup_test_files
    return 0
end

if not test_agent_down_oneshot_fallback
    exit 1
end

exit 0
