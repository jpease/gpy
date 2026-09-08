#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: Git Segment Live Content Update (clock disabled)
# ============================================================================
#
# Regression guard for the live-update pipeline. Unlike the signal-flow test,
# this asserts the agent-produced git segment CONTENT actually changes in
# response to a working-tree edit — the user-visible behaviour, not just that a
# signal was delivered.
#
# It runs with the clock segment DISABLED. Historically the per-minute clock
# broadcast masked git-update gaps; with it gone, the git segment must update on
# its own via working-tree watching. It also edits a tracked file WITHOUT
# `git add`, which only the worktree watcher (not a .git-only watcher) catches.
#
# Expected runtime: ~10 seconds

source (dirname (status -f))/../lib/test_helpers.fish

function test_git_live_content
    print_test_header "E2E Test: Git Segment Live Content Update"
    init_test_env

    # Isolate the instant-prompt cache as well as config/socket.
    set -gx XDG_CACHE_HOME $__gpy_test_tmp_dir/cache
    mkdir -p $XDG_CACHE_HOME

    # Config with the clock segment disabled (removes the periodic heartbeat).
    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '[ui]\nenabled_segments = ["directory", "git"]\n' >$XDG_CONFIG_HOME/gpy/config.toml

    # Build a committed repo with a tracked file.
    set -l repo $__gpy_test_tmp_dir/repo
    mkdir -p $repo
    git -C $repo init -q
    git -C $repo config user.email test@example.com
    git -C $repo config user.name Test
    # gpy-agent#386/#388: git's own fsmonitor spawns a background daemon per
    # repo that independently subscribes to FSEvents for the same path gpy's
    # own watcher is watching, on a machine where `core.fsmonitor` is enabled
    # globally (a common perf setting). That competing daemon measurably
    # starves gpy's own FSEventStream of its first event under load -- exactly
    # the scenario this test exercises (a worktree edit right after the watch
    # is armed). Mirrors the same fix already applied to the Rust test
    # fixtures (`gpy-agent/tests/fixtures/git_repo.rs`,
    # `gpy-agent/tests/multi_repo_integration_tests.rs`).
    git -C $repo config core.fsmonitor false
    echo hello >$repo/tracked.txt
    git -C $repo add tracked.txt
    git -C $repo commit -qm init >/dev/null

    if not start_test_agent
        print_test_result "Agent Start" "FAIL" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end
    print_test_result "Agent Start" "PASS"

    set -l client (register_test_client 15 $repo)
    if test -z "$client"
        print_test_result "Register client" "FAIL" "Failed to spawn test client"
        cleanup_test_files
        return 1
    end
    print_test_result "Register client" "PASS"

    # Locate the git instant-cache file the agent wrote for this repo.
    set -l cache_file ""
    for i in (seq 1 12)
        # Cache files now carry a prev_bg token: `{key}.git.{token}.ansi`.
        set cache_file (ls $XDG_CACHE_HOME/gpy/instant-prompts/*.git.*.ansi 2>/dev/null | head -1)
        test -n "$cache_file"; and break
        sleep 0.25
    end
    if test -z "$cache_file"
        print_test_result "Instant cache created" "FAIL" "no git cache written after registration"
        cleanup_test_files
        return 1
    end
    print_test_result "Instant cache created" "PASS"

    # Give the worktree watch a moment to warm up (some fs backends drop events
    # that occur immediately after a stream starts).
    sleep 2

    # Snapshot the clean-state content, then edit the tracked file WITHOUT git add.
    cp $cache_file $__gpy_test_tmp_dir/before.ansi
    echo "modified" >>$repo/tracked.txt

    # The agent should rewrite git.ansi (clean -> dirty) within the debounce window.
    set -l changed 0
    for i in (seq 1 24)
        sleep 0.25
        if not cmp -s $cache_file $__gpy_test_tmp_dir/before.ansi
            set changed 1
            break
        end
    end

    if test $changed -eq 1
        print_test_result "Worktree edit updates git segment (clock disabled)" "PASS"
    else
        print_test_result "Worktree edit updates git segment (clock disabled)" "FAIL" "git.ansi content did not change after a tracked-file edit"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

if not test_git_live_content
    exit 1
end

exit 0
