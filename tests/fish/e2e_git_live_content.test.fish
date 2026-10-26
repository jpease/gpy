#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# E2E Test: Git Segment Live Content Update (clock disabled)
#
# Guards the live-update pipeline. Unlike the signal-flow test,
# this asserts the agent-produced git segment CONTENT actually changes in
# response to a working-tree edit — the user-visible behaviour, not just that a
# signal was delivered.
#
# It runs with the clock segment DISABLED, because the per-minute clock
# broadcast would mask git-update gaps; the git segment must update on its own
# via working-tree watching. It also edits a tracked file WITHOUT `git add`,
# which only the worktree watcher (not a .git-only watcher) catches.
#
# Expected runtime: ~10 seconds

source (dirname (status -f))/../lib/test_helpers.fish

# Predicate for poll_until: a git instant-cache file exists; records its path
# in __live_cache_file. Cache files carry a prev_bg token:
# `{key}.git.{token}.ansi`.
function __live_cache_file_found
    set -g __live_cache_file (ls $XDG_CACHE_HOME/gpy/instant-prompts/*.git.*.ansi 2>/dev/null | head -1)
    test -n "$__live_cache_file"
end

# Predicate for poll_until: $file differs from the clean-state snapshot.
function __live_cache_changed --argument-names file
    not cmp -s $file $__gpy_test_tmp_dir/before.ansi
end

# Predicate for poll_until: edit the tracked file, then test whether $file changed.
function __live_edit_changed_cache --argument-names file
    echo modified >>$__live_repo/tracked.txt
    __live_cache_changed $file
end

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
    # Disable git's own fsmonitor: it spawns a background daemon per repo that
    # independently subscribes to FSEvents for the same path gpy's own watcher
    # is watching, on a machine where `core.fsmonitor` is enabled globally (a
    # common perf setting). That competing daemon measurably starves gpy's own
    # FSEventStream of its first event under load -- exactly the scenario this
    # test exercises (a worktree edit right after the watch is armed). The Rust
    # test fixtures (`gpy-agent/tests/common/git.rs`,
    # `gpy-agent/tests/multi_repo_integration_tests.rs`) do the same.
    git -C $repo config core.fsmonitor false
    echo hello >$repo/tracked.txt
    git -C $repo add tracked.txt
    git -C $repo commit -qm init >/dev/null

    if not start_test_agent
        fail "Agent Start" "gpy-agent failed to start"
        cleanup_test_files
        return 1
    end
    pass "Agent Start"

    set -l client (register_test_client 15 $repo)
    if test -z "$client"
        fail "Register client" "Failed to spawn test client"
        cleanup_test_files
        return 1
    end
    pass "Register client"

    # Locate the git instant-cache file the agent wrote for this repo.
    poll_until 3 __live_cache_file_found
    set -l cache_file $__live_cache_file
    if test -z "$cache_file"
        fail "Instant cache created" "no git cache written after registration"
        cleanup_test_files
        return 1
    end
    pass "Instant cache created"

    # Snapshot the clean-state content, then edit the tracked file WITHOUT git add.
    cp $cache_file $__gpy_test_tmp_dir/before.ansi

    # The agent exposes no "watch ready" signal and some fs backends drop events
    # right after a stream starts, so each poll attempt re-edits the file until
    # git.ansi is rewritten (clean -> dirty).
    set -g __live_repo $repo
    set -l changed 0
    poll_until 8 __live_edit_changed_cache $cache_file; and set changed 1

    if test $changed -eq 1
        pass "Worktree edit updates git segment (clock disabled)"
    else
        fail "Worktree edit updates git segment (clock disabled)" "git.ansi content did not change after a tracked-file edit"
        cleanup_test_files
        return 1
    end

    cleanup_test_files
    return 0
end

test_git_live_content; or exit 1
test_finish "git live content"
