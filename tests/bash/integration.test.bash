#!/usr/bin/env bash

# tests/bash/integration.test.bash
# Comprehensive integration test for all segments

# Get project root

# shellcheck disable=SC2329
# The functions below are mocks that shadow the real implementations for the
# code under test to call. Nothing invokes them by name, which is the point.

# shellcheck disable=SC2154
# __gpy_duration_method is assigned by bash/gpy.bash, sourced above.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# Drop inherited repo-scoping git env (GIT_DIR/GIT_WORK_TREE/...) so the worktree
# fixture below targets its own throwaway repo, not this one, when run from a git
# hook — `git -C <tmp>` does not override these (#275).
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
    GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
    GIT_PREFIX GIT_NAMESPACE 2>/dev/null || true
export GPY_AGENT_SUPERVISOR_ENABLED=0
# Hermetic XDG dirs (#632): __gpy_load_theme sources a theme-export cache from
# $XDG_CACHE_HOME/gpy when one exists (#614), which would replace the built-in
# defaults this file asserts on with the developer's own theme.
__gpy_integration_xdg_root=$(mktemp -d "${TMPDIR:-/tmp}/gpy-it-xdg.XXXXXX")
export XDG_CACHE_HOME="$__gpy_integration_xdg_root/cache"
export XDG_CONFIG_HOME="$__gpy_integration_xdg_root/config"
mkdir -p "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"
# Sourcing gpy.bash evals the theme export, which sets
# GPY_AGENT_SUPERVISOR_ENABLED from config and so overrides the export above:
# disable the supervisor in the sandbox config too, or a later prompt's
# supervisor check starts a real agent on the "missing" socket (#835). The
# socket also lives in the sandbox, never in the checkout.
mkdir -p "$XDG_CONFIG_HOME/gpy"
printf '[agent.supervisor]\nenabled = false\n' >"$XDG_CONFIG_HOME/gpy/config.toml"
export GPY_AGENT_SOCKET_PATH="$__gpy_integration_xdg_root/missing.sock"

source bash/gpy.bash

echo "=== Testing Clock Segment ==="
# Agent unreachable here (GPY_AGENT_SOCKET_PATH points at a missing socket),
# so this exercises the pure-bash fallback: still renders, uncapped.
output=$(__gpy_segment_clock)
if [[ -z "$output" ]]; then
    echo "FAIL: Clock segment empty"
    exit 1
fi
echo "PASS: Clock segment working"

# The fallback must honor the configured time format rather than hardcoding
# 24-hour, which is what it did before the clock became agent-rendered and is
# why bash showed 17:21:42 beside fish's 5:21 PM under the same theme.
spec="$(__time_format=12 __clock_show_seconds=0 __gpy_clock_time_spec)"
if [[ "$spec" != "%-I:%M %p" ]]; then
    echo "FAIL: 12-hour spec should be '%-I:%M %p'; got: $spec"
    exit 1
fi
spec="$(__time_format=24 __clock_show_seconds=1 __gpy_clock_time_spec)"
if [[ "$spec" != "%-H:%M:%S" ]]; then
    echo "FAIL: 24-hour+seconds spec should be '%-H:%M:%S'; got: $spec"
    exit 1
fi
echo "PASS: fallback clock honors the configured time format"

# When the agent does answer, the segment must delegate rather than render
# locally. Mock the request helper the way the duration test below does.
__gpy_request_clock() { printf 'AGENT-CLOCK'; }
delegated="$(__gpy_segment_clock "" "black" "true")"
if [[ "$delegated" != *"AGENT-CLOCK"* ]]; then
    echo "FAIL: clock should delegate to the agent when it answers; got: $delegated"
    exit 1
fi
unset -f __gpy_request_clock
echo "PASS: clock delegates to the agent when available"

echo "=== Testing Duration Segment ==="
# Test with duration above threshold - agent-rendered (#199): mock the IPC call
# so the test validates delegation without requiring a live agent.
__gpy_request_duration() {
    printf '\033[33m 3.5s \033[0m'
}
# Pin the threshold so the test is independent of theme configuration.
# The variable the agent exports is __duration_threshold_ms (#203).
__duration_threshold_ms=2000
__gpy_cmd_duration=3500
output=$(__gpy_segment_duration)
if [[ -z "$output" ]]; then
    echo "FAIL: Duration segment empty for 3.5s"
    exit 1
fi
echo "PASS: Duration shown for 3.5s"

# Test with duration below threshold (shell-side gate; no agent call needed)
__gpy_cmd_duration=500
output=$(__gpy_segment_duration)
if [[ -n "$output" ]]; then
    echo "FAIL: Duration shown for 0.5s (should be hidden with 2000ms threshold)"
    exit 1
fi
echo "PASS: Duration hidden for 0.5s"

# Test with no duration (Bash 3.x case)
if [[ $__gpy_duration_method == "none" ]]; then
    echo "WARN: Duration tracking disabled on Bash ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"
fi

# Language and git content are agent-rendered; this file runs with no agent
# and a socket that does not exist, so it cannot assert on either. The live
# renders (branch, dirty marker, registration, workspace sync) are asserted
# against a real agent and an interactive bash on a pty in
# tests/bash/e2e_agent_autostart.test.bash (#646).

echo "=== Testing Status Segment ==="
output=$(__gpy_segment_status 0)
if [[ -z "$output" ]]; then
    echo "FAIL: Status segment empty for success"
    exit 1
fi
# Check for green color code (success)
if [[ ! "$output" =~ 32m ]]; then
    echo "FAIL: Status segment incorrect color for success"
    exit 1
fi
echo "PASS: Status success"

output=$(__gpy_segment_status 127)
if [[ -z "$output" ]]; then
    echo "FAIL: Status segment empty for failure"
    exit 1
fi
# Check for red color code (failure)
if [[ ! "$output" =~ 31m ]]; then
    echo "FAIL: Status segment incorrect color for failure"
    exit 1
fi
echo "PASS: Status failure"

echo "=== Testing Git Worktree Detection ==="
worktree_base=$(mktemp -d)
worktree_repo="$worktree_base/repo"
worktree_checkout="$worktree_base/repo-worktree"
worktree_hooks="$worktree_base/hooks"
mkdir -p "$worktree_repo"
mkdir -p "$worktree_hooks"
git -C "$worktree_repo" init >/dev/null 2>&1
git -C "$worktree_repo" symbolic-ref HEAD refs/heads/main
git -C "$worktree_repo" config user.email "test@example.com"
git -C "$worktree_repo" config user.name "Test User"
git -C "$worktree_repo" config commit.gpgsign false
git -C "$worktree_repo" config core.hooksPath "$worktree_hooks"
printf 'test\n' > "$worktree_repo/file.txt"
git -C "$worktree_repo" add file.txt >/dev/null 2>&1
worktree_tree=$(git -C "$worktree_repo" write-tree 2>/dev/null)
worktree_commit=$(printf 'init\n' | git -C "$worktree_repo" commit-tree "$worktree_tree" 2>/dev/null)
git -C "$worktree_repo" update-ref refs/heads/main "$worktree_commit" 2>/dev/null
git -c core.hooksPath="$worktree_hooks" -C "$worktree_repo" worktree add -q "$worktree_checkout" 2>/dev/null
detected_root=$(__gpy_find_git_root "$worktree_checkout")
expected_root=$(realpath "$worktree_checkout")
if [[ "$detected_root" != "$expected_root" ]]; then
    echo "FAIL: Worktree root detection returned '$detected_root'"
    rm -rf "$worktree_base"
    exit 1
fi
rm -rf "$worktree_base"
echo "PASS: Git worktree detection"

echo "=== Testing Bash Agent Payloads ==="
register_payload=$(__gpy_build_register_payload)
if [[ "$register_payload" != *'"op":"register"'* || "$register_payload" != *'"cwd":'* || "$register_payload" != *'"shell":"bash"'* || "$register_payload" != *'"shell_version":'* ]]; then
    echo "FAIL: Register payload missing required fields: $register_payload"
    exit 1
fi
workspace_payload=$(__gpy_build_workspace_payload)
if [[ "$workspace_payload" != *'"op":"workspace"'* || "$workspace_payload" != *'"cwd":'* ]]; then
    echo "FAIL: Workspace payload missing required fields: $workspace_payload"
    exit 1
fi
echo "PASS: Bash agent payloads"

echo "=== Testing Bash Workspace Sync ==="
captured_workspace_payload=""
captured_workspace_file=$(mktemp)
__gpy_send_json() {
    printf '%s' "$1" > "$captured_workspace_file"
    echo '{"Ack":null}'
}
sync_dir=$(mktemp -d)
__gpy_registered=1
__gpy_last_workspace="/definitely/not/current"
pushd "$sync_dir" >/dev/null || exit 1
__gpy_sync_workspace
popd >/dev/null || exit 1
rm -rf "$sync_dir"
captured_workspace_payload=$(cat "$captured_workspace_file")
rm -f "$captured_workspace_file"
if [[ "$captured_workspace_payload" != *'"op":"workspace"'* ]]; then
    echo "FAIL: Workspace sync did not send workspace payload"
    exit 1
fi
echo "PASS: Bash workspace sync"

echo "=== Testing Supervisor Check Round-Trip Cost (#340) ==="
# Regression test for #340: __gpy_supervisor_check used to call
# __gpy_supervisor_is_running (a ping round-trip, plus a second
# __gpy_check_protocol_version status round-trip) as its very first action,
# before the existing rate-limit gate -- so a HEALTHY agent paid for both
# round-trips on every single prompt; only the restart itself was
# rate-limited. This verifies the restructured check: the rate-limit gate
# runs before any IPC call, and the protocol-version check is cached after
# its first success so a healthy agent only ever pays for the ping.
supervisor_test_tmp_dir=$(mktemp -d)
supervisor_test_sock="$supervisor_test_tmp_dir/gpy-supervisor-healthy.sock"

if ! command -v python3 &>/dev/null; then
    echo "SKIP: python3 not installed, cannot create a test Unix socket"
else
    # __gpy_send_json is fully stubbed below, so the socket only needs to
    # exist as an AF_UNIX special file for __gpy_supervisor_is_running's
    # `[[ -S "$socket_path" ]]` gate -- no listener is required.
    python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])
' "$supervisor_test_sock"

    if [[ ! -S "$supervisor_test_sock" ]]; then
        echo "SKIP: could not create test Unix socket for supervisor round-trip test"
    else
        __gpy_supervisor_test_saved_socket=$GPY_AGENT_SOCKET_PATH
        __gpy_supervisor_test_saved_enabled=$GPY_AGENT_SUPERVISOR_ENABLED

        export GPY_AGENT_SOCKET_PATH="$supervisor_test_sock"
        export GPY_AGENT_SUPERVISOR_ENABLED=1
        # Wide enough that three back-to-back calls in this test all land
        # within one window, independent of how long the test takes to run.
        export GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS=60

        # __gpy_send_json is always invoked via command substitution
        # (`response=$(__gpy_send_json ...)`), which forks a subshell in
        # bash -- a plain counter variable incremented inside the stub would
        # be discarded when that subshell exits. Count via a counter file
        # instead so the tally survives the subshell boundary.
        __gpy_supervisor_call_count_file="$supervisor_test_tmp_dir/send_json_calls"
        : > "$__gpy_supervisor_call_count_file"
        __gpy_send_json() {
            printf 'x' >> "$__gpy_supervisor_call_count_file"
            case "$1" in
                *'"op":"ping"'*)
                    # The daemon's real acknowledgement (its JSON formatter
                    # renders Ack as {"status":"ok"}); the stub used to
                    # answer an invented {"op":"pong"} the daemon never
                    # sends, which is exactly what supervisor.bash was
                    # wrongly waiting for (#638).
                    printf '{"status":"ok"}'
                    ;;
                *'"op":"status"'*)
                    printf '{"AgentStatus":{"version":"0.1.0-test","protocol_version":1,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'
                    ;;
            esac
            return 0
        }
        __gpy_supervisor_call_count() {
            wc -c < "$__gpy_supervisor_call_count_file" | tr -d ' '
        }

        # Simulate steady state after a successful __gpy_supervisor_start:
        # the protocol-version check has already succeeded once and is
        # cached, so the per-prompt checks below should only ever pay for
        # the ping. __gpy_supervisor_last_check_time=0 makes the very first
        # of the three calls land past the rate-limit window (a "fresh"
        # window boundary); the other two land inside it.
        __gpy_protocol_version_checked=1
        __gpy_supervisor_last_check_time=0
        __gpy_supervisor_check_attempts=0

        : > "$__gpy_supervisor_call_count_file"
        __gpy_supervisor_check
        first_check_calls=$(__gpy_supervisor_call_count)
        if [[ $first_check_calls -ne 1 ]]; then
            echo "FAIL: healthy supervisor check at the window boundary made $first_check_calls round-trip(s), want 1"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: healthy supervisor check at the window boundary makes exactly 1 round-trip (ping only; protocol cached)"

        : > "$__gpy_supervisor_call_count_file"
        __gpy_supervisor_check
        second_check_calls=$(__gpy_supervisor_call_count)
        if [[ $second_check_calls -ne 0 ]]; then
            echo "FAIL: healthy supervisor check within the rate-limit window made $second_check_calls round-trip(s), want 0"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: healthy supervisor check within the rate-limit window makes 0 round-trips"

        : > "$__gpy_supervisor_call_count_file"
        __gpy_supervisor_check
        third_check_calls=$(__gpy_supervisor_call_count)
        if [[ $third_check_calls -ne 0 ]]; then
            echo "FAIL: third consecutive healthy supervisor check within the rate-limit window made $third_check_calls round-trip(s), want 0"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: third consecutive healthy supervisor check within the rate-limit window makes 0 round-trips"

        # Down-path restart must invalidate the protocol cache in the PARENT
        # shell (#340 review gap). __gpy_supervisor_start runs backgrounded
        # (`&`) on the down-path, so any cache reset inside it happens in a
        # subshell and never reaches the parent. If the parent kept a stale
        # cached "1", the next health check would skip the #307 protocol
        # check on whatever agent takes over the socket post-restart -- so a
        # mismatched daemon appearing after the restart would go undetected.
        # Simulate a dead agent and confirm the parent's cache is cleared the
        # moment __gpy_supervisor_check initiates a restart.
        __gpy_supervisor_saved_is_running=$(declare -f __gpy_supervisor_is_running)
        __gpy_supervisor_saved_start=$(declare -f __gpy_supervisor_start)
        __gpy_supervisor_is_running() { return 1; }  # agent is down
        __gpy_supervisor_start() { return 1; }        # no-op: don't spawn a real agent

        __gpy_protocol_version_checked=1   # stale cache from the prior healthy session
        __gpy_supervisor_last_check_time=0 # past the rate-limit window
        __gpy_supervisor_check_attempts=0

        __gpy_supervisor_check

        if [[ "$__gpy_protocol_version_checked" != "0" ]]; then
            echo "FAIL: down-path restart left parent protocol cache stale (__gpy_protocol_version_checked=$__gpy_protocol_version_checked, want 0); a post-restart mismatched agent would be skipped (#307/#340)"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        if [[ "$__gpy_supervisor_check_attempts" != "1" ]]; then
            echo "FAIL: down-path restart did not increment restart attempts (got $__gpy_supervisor_check_attempts, want 1); the down-path was not actually taken"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: down-path restart invalidates the parent shell's protocol cache so the next health check re-checks the protocol (#307/#340)"

        # The interval and attempt cap come from the names the theme export
        # emits ([agent.supervisor] in config.toml), read on every check
        # (#762). Two prompts 15 s apart with a 60 s interval probe once; the
        # old 10 s default probed twice.
        __gpy_supervisor_probe_count=0
        __gpy_supervisor_is_running() { __gpy_supervisor_probe_count=$((__gpy_supervisor_probe_count + 1)); return 1; }
        unset GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS
        export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=60
        __gpy_supervisor_last_check_time=0
        __gpy_supervisor_check_attempts=0
        __gpy_supervisor_check
        __gpy_supervisor_last_check_time=$((__gpy_supervisor_last_check_time - 15))
        __gpy_supervisor_check
        if [[ "$__gpy_supervisor_probe_count" != "1" ]]; then
            echo "FAIL: two checks 15 s apart with GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=60 probed $__gpy_supervisor_probe_count time(s), want 1 (#762)"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS sets the health-check interval (#762)"

        # max_restart_attempts=1: a persistently dead agent gets one restart.
        export GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS=0
        export GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=1
        __gpy_supervisor_last_check_time=0
        __gpy_supervisor_check_attempts=0
        __gpy_supervisor_check
        __gpy_supervisor_check
        __gpy_supervisor_check
        if [[ "$__gpy_supervisor_check_attempts" != "1" ]]; then
            echo "FAIL: GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=1 allowed $__gpy_supervisor_check_attempts restart(s), want 1 (#762)"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS caps restarts (#762)"
        unset GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS __gpy_supervisor_probe_count

        # Restore the real supervisor functions the stubs shadowed above.
        eval "$__gpy_supervisor_saved_is_running"
        eval "$__gpy_supervisor_saved_start"
        unset __gpy_supervisor_saved_is_running __gpy_supervisor_saved_start

        # Restore __gpy_send_json to the same always-Ack stub the earlier
        # "Bash Workspace Sync" section leaves in place, rather than
        # `unset -f`-ing it entirely -- later sections (e.g. the character
        # segment inside __gpy_render_prompt) call it unconditionally, and
        # removing it outright would surface spurious "command not found"
        # output instead of leaving behavior unchanged for those sections.
        __gpy_send_json() {
            echo '{"Ack":null}'
        }
        unset -f __gpy_supervisor_call_count
        unset __gpy_supervisor_call_count_file
        unset GPY_SUPERVISOR_CHECK_RATE_LIMIT_SECONDS
        __gpy_protocol_version_checked=0
        __gpy_supervisor_last_check_time=0
        __gpy_supervisor_check_attempts=0

        if [[ -n "$__gpy_supervisor_test_saved_socket" ]]; then
            export GPY_AGENT_SOCKET_PATH=$__gpy_supervisor_test_saved_socket
        else
            unset GPY_AGENT_SOCKET_PATH
        fi
        if [[ -n "$__gpy_supervisor_test_saved_enabled" ]]; then
            export GPY_AGENT_SUPERVISOR_ENABLED=$__gpy_supervisor_test_saved_enabled
        else
            unset GPY_AGENT_SUPERVISOR_ENABLED
        fi
        unset __gpy_supervisor_test_saved_socket __gpy_supervisor_test_saved_enabled
    fi
fi
rm -rf "$supervisor_test_tmp_dir"

echo "=== Testing Last Segment Context ==="
# #613: dispatch passes is_last/is_first as "true"/"" (not the old
# "last"/"first" literals) directly to every segment -- no per-segment
# conversion.
__gpy_segment_alpha() { printf 'alpha:%s|' "$1"; }
__gpy_segment_beta() { printf 'beta:%s|' "$1"; }
__enabled_segments="alpha beta"
__gpy_render_prompt 0
if [[ "$PS1" != *"alpha:|"* || "$PS1" != *"beta:true|"* ]]; then
    echo "FAIL: Last segment context not passed correctly: $PS1"
    exit 1
fi
__gpy_request() {
    printf '%s:%s' "$1" "$4"
}
# __gpy_segment_git now reads the instant cache directly (#614) and only
# falls through to __gpy_request on a cold miss, so this stub is only
# reachable with a guaranteed miss -- point XDG_CACHE_HOME at an empty temp
# dir rather than relying on whatever real instant-prompt cache this
# machine's own gpy-agent may have already written for this repo.
__gpy_last_segment_test_saved_xdg_cache_home=${XDG_CACHE_HOME:-}
__gpy_last_segment_test_tmp_cache_home="$(mktemp -d)"
export XDG_CACHE_HOME="$__gpy_last_segment_test_tmp_cache_home"
git_last_output=$(__gpy_segment_git true)
git_last_status=$?
rm -rf "$XDG_CACHE_HOME"
if [[ -n "$__gpy_last_segment_test_saved_xdg_cache_home" ]]; then
    export XDG_CACHE_HOME="$__gpy_last_segment_test_saved_xdg_cache_home"
else
    unset XDG_CACHE_HOME
fi
unset __gpy_last_segment_test_saved_xdg_cache_home __gpy_last_segment_test_tmp_cache_home
if [[ "$git_last_output" != "git:true" ]]; then
    echo "FAIL: Git segment did not pass is_last=true: $git_last_output (status=$git_last_status)"
    exit 1
fi
echo "PASS: Last segment context"

echo "=== Testing Full Prompt ==="
__enabled_segments="status directory git clock duration language"
__gpy_cmd_duration=0
__gpy_render_prompt 0
if [[ -z "$PS1" ]]; then
    echo "FAIL: Full prompt empty"
    exit 1
fi
echo "PASS: Full prompt rendered"

echo "=== Testing Doorbell Reload (lazy segment gap regression) ==="
# gpy.bash sources every file under segments/*.bash unconditionally at init,
# regardless of $__enabled_segments (unlike Fish, which only sources files for
# segments already in the enabled list). So a theme switch that enables a
# segment new to this shell's __enabled_segments already has its detect/render
# functions in memory; the reload only needs to refresh __enabled_segments
# (via __gpy_load_theme) and re-render. This guards against a future
# regression to per-segment lazy sourcing that would reintroduce the gap Fish
# had (#296). Bash consumes the doorbell's flags at the next prompt, in
# __gpy_precmd, before it renders (#678).
doorbell_dir=$(mktemp -d)
__gpy_shell_flag_base="$doorbell_dir/$$"
# What the agent does for a config change: leave the reload flag (and ring
# SIGURG, which Bash ignores); then the user presses Enter.
__gpy_test_ring_reload() {
    : >"$__gpy_shell_flag_base.reload"
    __gpy_precmd
}
__gpy_load_theme() {
    __enabled_segments="duration"
}
__duration_threshold_ms=100
__gpy_cmd_duration=5000
__gpy_test_ring_reload
if [[ "$PS1" == *"3.5s"* ]]; then
    echo "PASS: newly-enabled duration segment rendered in the prompt after the reload flag"
else
    echo "FAIL: newly-enabled duration segment missing from the prompt after the reload flag (PS1=$PS1)"
    exit 1
fi
if [[ -e "$__gpy_shell_flag_base.reload" ]]; then
    echo "FAIL: the next prompt left the reload flag behind"
    exit 1
fi
echo "PASS: the next prompt consumes the reload flag"

# A prompt with no flag must not reload the theme.
__gpy_load_theme() {
    __enabled_segments="directory"
}
__gpy_precmd
if [[ "$__enabled_segments" == "duration" ]]; then
    echo "PASS: a prompt without a reload flag does not reload"
else
    echo "FAIL: a prompt without a reload flag reloaded (enabled=$__enabled_segments)"
    exit 1
fi

__gpy_load_theme() {
    __enabled_segments="duration does-not-exist-segment"
}
if __gpy_test_ring_reload; then
    echo "PASS: unknown segment in enabled list does not error the prompt"
else
    echo "FAIL: the prompt errored on an unknown segment after a reload"
    exit 1
fi
unset __duration_threshold_ms __gpy_cmd_duration

echo "=== Testing Directory Display Modes ==="
temp_dir=$(mktemp -d)
mkdir -p "$temp_dir/alpha/beta/project"
pushd "$temp_dir/alpha/beta/project" >/dev/null || exit 1
HOME="$temp_dir"

# Directory is agent-rendered (#199); the shell passes GPY_UI_DIRECTORY_DISPLAY and
# $PWD to the agent via __gpy_request. Mock __gpy_request to simulate the agent
# returning formatted path strings for each display mode so the assertions can
# verify delegation without requiring a live agent.
# Note: abbreviated uses the known test path structure (alpha/beta/project → a/b).
__gpy_request() {
    local path="$2"
    case "${GPY_UI_DIRECTORY_DISPLAY:-abbreviated}" in
        basename)
            # Pure shell expansion: no external command needed
            printf ' %s ' "${path##*/}"
            ;;
        abbreviated)
            # Simulate agent abbreviating HOME/alpha/beta/project → ~/a/b/project.
            # The test always creates alpha/beta/project under HOME so intermediate
            # component names are known; abbreviate each to its first letter.
            printf ' ~/a/b/%s ' "${path##*/}"
            ;;
        full)
            printf ' %s ' "$path"
            ;;
        *)
            printf ' %s ' "$path"
            ;;
    esac
}

GPY_UI_DIRECTORY_DISPLAY="basename"
output=$(__gpy_segment_directory)
if [[ "$output" != *" project "* ]]; then
    echo "FAIL: Basename mode did not render project name"
    popd >/dev/null || true
    rm -rf "$temp_dir"
    exit 1
fi

GPY_UI_DIRECTORY_DISPLAY="abbreviated"
output=$(__gpy_segment_directory)
if [[ "$output" != *" ~/a/b/project "* ]]; then
    echo "FAIL: Abbreviated mode did not render abbreviated path"
    popd >/dev/null || true
    rm -rf "$temp_dir"
    exit 1
fi

GPY_UI_DIRECTORY_DISPLAY="full"
output=$(__gpy_segment_directory)
if [[ "$output" != *" $temp_dir/alpha/beta/project "* ]]; then
    echo "FAIL: Full mode did not render full path"
    popd >/dev/null || true
    rm -rf "$temp_dir"
    exit 1
fi

unset GPY_UI_DIRECTORY_DISPLAY
unset GPY_UI_DIRECTORY_MAX_LENGTH
popd >/dev/null || exit 1
rm -rf "$temp_dir"
echo "PASS: Directory display modes"

echo "=== Testing Hostname Segment ==="

# Preserve ambient state so mutations here don't leak into later test sections
# (test hygiene: mirrors the save/restore pattern in the zsh equivalent of
# this test).
__gpy_hostname_test_had_hostname=${HOSTNAME+1}
__gpy_hostname_test_saved_hostname=${HOSTNAME:-}
__gpy_hostname_test_had_is_ssh=${__gpy_is_ssh+1}
__gpy_hostname_test_saved_is_ssh=${__gpy_is_ssh:-}
__gpy_hostname_test_had_show_always=${__hostname_show_always+1}
__gpy_hostname_test_saved_show_always=${__hostname_show_always:-}
__gpy_hostname_test_had_trim_at=${__hostname_trim_at+1}
__gpy_hostname_test_saved_trim_at=${__hostname_trim_at:-}
__gpy_hostname_test_had_icon=${__icon_hostname+1}
__gpy_hostname_test_saved_icon=${__icon_hostname:-}
__gpy_hostname_test_had_format=${__hostname_format+1}
__gpy_hostname_test_saved_format=${__hostname_format:-}

# detect: SSH session shows regardless of show_always
__gpy_is_ssh=1
__hostname_show_always=0
if ! __gpy_segment_hostname_detect; then
    echo "FAIL: Hostname detect should show on SSH session"
    exit 1
fi
echo "PASS: Hostname detect shows on SSH"

# detect: local session hides by default
__gpy_is_ssh=0
__hostname_show_always=0
if __gpy_segment_hostname_detect; then
    echo "FAIL: Hostname detect should hide on local session"
    exit 1
fi
echo "PASS: Hostname detect hides on local"

# detect: show_always forces display even locally
__gpy_is_ssh=0
__hostname_show_always=1
if ! __gpy_segment_hostname_detect; then
    echo "FAIL: Hostname detect should show when show_always=1"
    exit 1
fi
echo "PASS: Hostname detect shows with show_always"

# Pure-bash render path: trim at first delimiter, zero forks
unset __hostname_format
HOSTNAME="host.example.com"
__hostname_trim_at="."
__icon_hostname=""
output=$(__gpy_segment_hostname)
if [[ "$output" != *" host "* || "$output" == *"host.example.com"* ]]; then
    echo "FAIL: Hostname trim did not shorten to 'host': $output"
    exit 1
fi
echo "PASS: Hostname trimmed at delimiter"

# Empty trim delimiter leaves the hostname unchanged
__hostname_trim_at=""
output=$(__gpy_segment_hostname)
if [[ "$output" != *"host.example.com"* ]]; then
    echo "FAIL: Empty trim delimiter should leave hostname unchanged: $output"
    exit 1
fi
echo "PASS: Hostname unchanged with empty trim delimiter"

# Icon prefixes the label when set, and is omitted when empty
__hostname_trim_at="."
__icon_hostname="@"
output=$(__gpy_segment_hostname)
if [[ "$output" != *" @ host "* ]]; then
    echo "FAIL: Hostname icon missing from render: $output"
    exit 1
fi
echo "PASS: Hostname icon shown when set"

__icon_hostname=""
output=$(__gpy_segment_hostname)
if [[ "$output" == *"@ host"* ]]; then
    echo "FAIL: Hostname icon should be omitted when unset: $output"
    exit 1
fi
echo "PASS: Hostname icon omitted when unset"

# Dual-path: a non-empty __hostname_format routes to the agent renderer,
# quoting every positional arg (including a possibly-empty prev_bg) so it
# lands in the correct slot.
# __gpy_segment_hostname runs __gpy_request_hostname inside its own
# command-substitution subshell, so a stub cannot report back via a side-channel
# variable (it would be dropped when the subshell exits) — encode the args the
# stub received directly into its stdout instead.
#
# #613: is_last is forwarded as-is ("true"/"") -- no more per-segment
# last/first-literal conversion to a "true"/"false" string.
__gpy_request_hostname() {
    printf 'AGENT[%s|%s|%s|argc=%s]' "$1" "$2" "$3" "$#"
}
__hostname_format="{hostname}"
output=$(__gpy_segment_hostname true "cyan")
if [[ "$output" != "AGENT[host.example.com|true|cyan|argc=3]" ]]; then
    echo "FAIL: Hostname agent-path args incorrect (want hostname|is_last|prev_bg): $output"
    exit 1
fi
echo "PASS: Hostname dual-path routes to agent renderer with correctly-ordered args"

# is_last must stay the possibly-empty string (not "false") when this is not
# the last segment, and an empty prev_bg must still land in the 3rd slot (not
# shift left) — the bug class called out for this task. argc=3 proves the
# empty prev_bg was passed as a real (empty) positional arg, not omitted.
output=$(__gpy_segment_hostname "" "")
if [[ "$output" != "AGENT[host.example.com|||argc=3]" ]]; then
    echo "FAIL: Hostname agent-path args incorrect for not-last/empty-prev_bg: $output"
    exit 1
fi
echo "PASS: Hostname dual-path threads empty is_last and empty prev_bg correctly"

unset __hostname_format
unset -f __gpy_request_hostname

# Restore ambient state mutated above (test hygiene: avoid leaking
# HOSTNAME/__hostname_*/__gpy_is_ssh into later test sections or the parent shell).
if [[ -n "$__gpy_hostname_test_had_hostname" ]]; then
    HOSTNAME=$__gpy_hostname_test_saved_hostname
else
    unset HOSTNAME
fi
if [[ -n "$__gpy_hostname_test_had_is_ssh" ]]; then
    __gpy_is_ssh=$__gpy_hostname_test_saved_is_ssh
else
    unset __gpy_is_ssh
fi
if [[ -n "$__gpy_hostname_test_had_show_always" ]]; then
    __hostname_show_always=$__gpy_hostname_test_saved_show_always
else
    unset __hostname_show_always
fi
if [[ -n "$__gpy_hostname_test_had_trim_at" ]]; then
    __hostname_trim_at=$__gpy_hostname_test_saved_trim_at
else
    unset __hostname_trim_at
fi
if [[ -n "$__gpy_hostname_test_had_icon" ]]; then
    __icon_hostname=$__gpy_hostname_test_saved_icon
else
    unset __icon_hostname
fi
if [[ -n "$__gpy_hostname_test_had_format" ]]; then
    __hostname_format=$__gpy_hostname_test_saved_format
else
    unset __hostname_format
fi
unset __gpy_hostname_test_had_hostname __gpy_hostname_test_saved_hostname \
    __gpy_hostname_test_had_is_ssh __gpy_hostname_test_saved_is_ssh \
    __gpy_hostname_test_had_show_always __gpy_hostname_test_saved_show_always \
    __gpy_hostname_test_had_trim_at __gpy_hostname_test_saved_trim_at \
    __gpy_hostname_test_had_icon __gpy_hostname_test_saved_icon \
    __gpy_hostname_test_had_format __gpy_hostname_test_saved_format

echo "=== Testing Username Segment ==="

# Preserve ambient state so mutations here don't leak into later sections.
__gpy_username_test_had_user=${USER+1}
__gpy_username_test_saved_user=${USER:-}
__gpy_username_test_had_is_root=${__gpy_is_root+1}
__gpy_username_test_saved_is_root=${__gpy_is_root:-}
__gpy_username_test_had_is_sudo=${__gpy_is_sudo+1}
__gpy_username_test_saved_is_sudo=${__gpy_is_sudo:-}
__gpy_username_test_had_show_always=${__username_show_always+1}
__gpy_username_test_saved_show_always=${__username_show_always:-}
__gpy_username_test_had_icon=${__icon_username+1}
__gpy_username_test_saved_icon=${__icon_username:-}
__gpy_username_test_had_format=${__username_format+1}
__gpy_username_test_saved_format=${__username_format:-}

# detect: root session shows regardless of sudo/show_always
__gpy_is_root=1
__gpy_is_sudo=0
__username_show_always=0
if ! __gpy_segment_username_detect; then
    echo "FAIL: Username detect should show for root"
    exit 1
fi
echo "PASS: Username detect shows for root"

# detect: sudo session shows
__gpy_is_root=0
__gpy_is_sudo=1
if ! __gpy_segment_username_detect; then
    echo "FAIL: Username detect should show for sudo"
    exit 1
fi
echo "PASS: Username detect shows for sudo"

# detect: normal user hides by default
__gpy_is_root=0
__gpy_is_sudo=0
__username_show_always=0
if __gpy_segment_username_detect; then
    echo "FAIL: Username detect should hide for normal user"
    exit 1
fi
echo "PASS: Username detect hides for normal user"

# detect: show_always forces display even for a normal user
__username_show_always=1
if ! __gpy_segment_username_detect; then
    echo "FAIL: Username detect should show when show_always=1"
    exit 1
fi
echo "PASS: Username detect shows with show_always"

# Pure-bash render path: renders $USER, applies colours, no fork
unset __username_format
USER="root"
__icon_username=""
output=$(__gpy_segment_username)
if [[ "$output" != *" root "* ]]; then
    echo "FAIL: Username pure-bash render missing \$USER: $output"
    exit 1
fi
echo "PASS: Username pure-bash renders \$USER"

# Icon prefixes the label when set, omitted when empty
__icon_username="#"
output=$(__gpy_segment_username)
if [[ "$output" != *" # root "* ]]; then
    echo "FAIL: Username icon missing from render: $output"
    exit 1
fi
echo "PASS: Username icon shown when set"

__icon_username=""
output=$(__gpy_segment_username)
if [[ "$output" == *"# root"* ]]; then
    echo "FAIL: Username icon should be omitted when unset: $output"
    exit 1
fi
echo "PASS: Username icon omitted when unset"

# Dual-path: a non-empty __username_format routes to the agent renderer,
# quoting every positional arg so an empty prev_bg lands in the correct slot.
#
# #613: is_last is forwarded as-is ("true"/"") -- no more per-segment
# last/first-literal conversion to a "true"/"false" string.
__gpy_request_username() {
    printf 'AGENT[%s|%s|%s|argc=%s]' "$1" "$2" "$3" "$#"
}
__username_format="{username}"
output=$(__gpy_segment_username true "cyan")
if [[ "$output" != "AGENT[root|true|cyan|argc=3]" ]]; then
    echo "FAIL: Username agent-path args incorrect (want username|is_last|prev_bg): $output"
    exit 1
fi
echo "PASS: Username dual-path routes to agent renderer with correctly-ordered args"

# is_last must stay the possibly-empty string (not "false") when this is not
# the last segment, and an empty prev_bg must still land in the 3rd slot.
output=$(__gpy_segment_username "" "")
if [[ "$output" != "AGENT[root|||argc=3]" ]]; then
    echo "FAIL: Username agent-path args incorrect for not-last/empty-prev_bg: $output"
    exit 1
fi
echo "PASS: Username dual-path threads empty is_last and empty prev_bg correctly"

unset __username_format
unset -f __gpy_request_username

# Restore ambient state mutated above.
if [[ -n "$__gpy_username_test_had_user" ]]; then
    USER=$__gpy_username_test_saved_user
else
    unset USER
fi
if [[ -n "$__gpy_username_test_had_is_root" ]]; then
    __gpy_is_root=$__gpy_username_test_saved_is_root
else
    unset __gpy_is_root
fi
if [[ -n "$__gpy_username_test_had_is_sudo" ]]; then
    __gpy_is_sudo=$__gpy_username_test_saved_is_sudo
else
    unset __gpy_is_sudo
fi
if [[ -n "$__gpy_username_test_had_show_always" ]]; then
    __username_show_always=$__gpy_username_test_saved_show_always
else
    unset __username_show_always
fi
if [[ -n "$__gpy_username_test_had_icon" ]]; then
    __icon_username=$__gpy_username_test_saved_icon
else
    unset __icon_username
fi
if [[ -n "$__gpy_username_test_had_format" ]]; then
    __username_format=$__gpy_username_test_saved_format
else
    unset __username_format
fi
unset __gpy_username_test_had_user __gpy_username_test_saved_user \
    __gpy_username_test_had_is_root __gpy_username_test_saved_is_root \
    __gpy_username_test_had_is_sudo __gpy_username_test_saved_is_sudo \
    __gpy_username_test_had_show_always __gpy_username_test_saved_show_always \
    __gpy_username_test_had_icon __gpy_username_test_saved_icon \
    __gpy_username_test_had_format __gpy_username_test_saved_format

echo "=== Testing Character/Directory Render Memoization (#343) ==="
# __gpy_request_character and __gpy_segment_directory are both invoked via
# command substitution (from __gpy_render_prompt / __gpy_directory_segment_output),
# which forks a subshell -- an in-memory counter incremented inside a stub
# would not survive that subshell, so record calls to a file instead (mirrors
# the __gpy_supervisor_call_count_file pattern used above).
memo_tmp_dir=$(mktemp -d)
memo_char_calls_file="$memo_tmp_dir/char_calls"
memo_dir_calls_file="$memo_tmp_dir/dir_calls"
: > "$memo_char_calls_file"
: > "$memo_dir_calls_file"

__gpy_request_character() {
    printf 'x' >> "$memo_char_calls_file"
    printf 'CHAR'
}
__gpy_segment_directory() {
    printf 'x' >> "$memo_dir_calls_file"
    printf 'DIR'
}
# Earlier sections in this file permanently redefine __gpy_load_theme (e.g.
# "Testing Doorbell Reload" leaves it setting
# __enabled_segments="duration does-not-exist-segment"), and a real gpy-agent
# binary may also be on PATH. Pin it here so the doorbell's reload
# is deterministic for this section regardless of what ran before it.
__gpy_load_theme() {
    __enabled_segments="directory"
}

__gpy_memo_call_count() {
    wc -c < "$1" 2>/dev/null | tr -d ' '
}

__gpy_memo_reset() {
    : > "$memo_char_calls_file"
    : > "$memo_dir_calls_file"
    __gpy_char_cache_key=""
    __gpy_char_cache_val=""
    __gpy_dir_cache_key=""
    __gpy_dir_cache_val=""
}

__enabled_segments="directory"
__color_directory_bg="blue"
__gpy_theme_name="testtheme"

memo_dir_a=$(mktemp -d)
memo_dir_b=$(mktemp -d)
cd "$memo_dir_a" || exit 1

# 1. Cache hit: identical status/PWD/theme -> zero additional IPC calls
__gpy_memo_reset
__gpy_render_prompt 0 # baseline render (miss for both)
__gpy_render_prompt 0 # identical render (must hit for both)
if [[ "$(__gpy_memo_call_count "$memo_char_calls_file")" == "1" && "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "1" ]]; then
    echo "PASS: identical render is a cache hit for character+directory (zero extra IPC)"
else
    echo "FAIL: identical render made extra IPC calls (char=$(__gpy_memo_call_count "$memo_char_calls_file"), dir=$(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

# 2. Exit status flip re-renders the character only
__gpy_memo_reset
__gpy_render_prompt 0 # baseline (miss for both)
__gpy_render_prompt 1 # status flipped
if [[ "$(__gpy_memo_call_count "$memo_char_calls_file")" == "2" && "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "1" ]]; then
    echo "PASS: exit status flip re-renders character only"
else
    echo "FAIL: exit status flip counts wrong (char=$(__gpy_memo_call_count "$memo_char_calls_file"), dir=$(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

# 3. cd re-renders the directory only
__gpy_memo_reset
cd "$memo_dir_a" || exit 1
__gpy_render_prompt 0 # baseline (miss for both)
cd "$memo_dir_b" || exit 1
__gpy_render_prompt 0 # PWD changed
if [[ "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "2" && "$(__gpy_memo_call_count "$memo_char_calls_file")" == "1" ]]; then
    echo "PASS: cd re-renders directory only"
else
    echo "FAIL: cd counts wrong (char=$(__gpy_memo_call_count "$memo_char_calls_file"), dir=$(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

# 4. Theme change (reload doorbell) re-renders both segments. The handler
# itself re-renders immediately when PS1 is set (it always is here, from
# earlier sections), so the invalidation is already exercised by that
# internal render; the trailing explicit render is a hit against its result.
__gpy_memo_reset
__gpy_theme_name="themeA"
__gpy_render_prompt 0 # baseline (miss for both)
__gpy_theme_name="themeB"
__gpy_test_ring_reload >/dev/null 2>&1
__gpy_render_prompt 0
if [[ "$(__gpy_memo_call_count "$memo_char_calls_file")" == "2" && "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "2" ]]; then
    echo "PASS: theme change re-renders character and directory"
else
    echo "FAIL: theme change counts wrong (char=$(__gpy_memo_call_count "$memo_char_calls_file"), dir=$(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

# 5. An empty/failed render is never cached (agent-down fallback must retry
# on the very next prompt, not get stuck serving nothing forever).
__gpy_request_character() {
    printf 'x' >> "$memo_char_calls_file"
}
__gpy_memo_reset
__gpy_render_prompt 0
__gpy_render_prompt 0
if [[ "$(__gpy_memo_call_count "$memo_char_calls_file")" == "2" ]]; then
    echo "PASS: empty character render is never cached"
else
    echo "FAIL: empty character render count wrong ($(__gpy_memo_call_count "$memo_char_calls_file"))"
    exit 1
fi

__gpy_segment_directory() {
    printf 'x' >> "$memo_dir_calls_file"
}
__gpy_memo_reset
__gpy_render_prompt 0
__gpy_render_prompt 0
if [[ "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "2" ]]; then
    echo "PASS: empty directory render is never cached"
else
    echo "FAIL: empty directory render count wrong ($(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

cd "$ROOT" || exit 1
rm -rf "$memo_tmp_dir" "$memo_dir_a" "$memo_dir_b"
unset -f __gpy_memo_call_count __gpy_memo_reset
echo "PASS: Character/directory render memoization"

rm -rf "${__gpy_integration_xdg_root:-}"
echo "=== All Tests Passed ==="
