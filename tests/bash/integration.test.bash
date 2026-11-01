#!/usr/bin/env bash

# tests/bash/integration.test.bash
# Integration test for all segments.


# shellcheck disable=SC2329
# The functions below are mocks that shadow the real implementations for the
# code under test to call. Nothing invokes them by name, which is the point.

# shellcheck disable=SC2154
# __gpy_duration_method is assigned by bash/gpy.bash, sourced above.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

# Drop inherited repo-scoping git env so fixtures target their own repo under
# git hooks; `git -C <tmp>` does not override GIT_DIR/GIT_WORK_TREE.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
    GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
    GIT_PREFIX GIT_NAMESPACE 2>/dev/null || true
# Hermetic XDG dirs: a developer theme-export cache would replace the defaults
# asserted here. The sandbox config also disables the supervisor so no real agent starts.
__gpy_integration_xdg_root=$(mktemp -d "${TMPDIR:-/tmp}/gpy-it-xdg.XXXXXX")
source tests/lib/supervisor_off.bash "$__gpy_integration_xdg_root"
export GPY_AGENT_SOCKET_PATH="$__gpy_integration_xdg_root/missing.sock"

source bash/gpy.bash

echo "=== Testing Clock Segment ==="
# Agent unreachable: exercises the pure-bash fallback.
output=$(__gpy_segment_clock)
if [[ -z "$output" ]]; then
    echo "FAIL: Clock segment empty"
    exit 1
fi
echo "PASS: Clock segment working"

# The fallback must honor the configured time format (12-hour like fish).
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

# When the agent answers, the segment must delegate rather than render locally.
__gpy_request_clock() { printf 'AGENT-CLOCK'; }
__clock_format=1
delegated="$(__gpy_segment_clock "" "black" "true")"
if [[ "$delegated" != *"AGENT-CLOCK"* ]]; then
    echo "FAIL: clock should delegate to the agent when it answers; got: $delegated"
    exit 1
fi
echo "PASS: clock delegates to the agent when available"

# ...only when the theme sets [segments.clock].format; otherwise no IPC round trip.
__clock_format=""
local_clock="$(__gpy_segment_clock "" "black" "true")"
if [[ "$local_clock" == *"AGENT-CLOCK"* || "$local_clock" != *'\D{'* ]]; then
    echo "FAIL: a theme without a clock format must render the clock locally; got: $local_clock"
    exit 1
fi
unset -f __gpy_request_clock
echo "PASS: clock renders locally when the theme sets no clock format"

echo "=== Testing Duration Segment ==="
__gpy_request_duration() {
    printf '\033[33m 3.5s \033[0m'
}
# Pin the threshold so the test is independent of theme configuration.
__duration_threshold_ms=2000
__gpy_cmd_duration=3500
output=$(__gpy_segment_duration)
if [[ -z "$output" ]]; then
    echo "FAIL: Duration segment empty for 3.5s"
    exit 1
fi
echo "PASS: Duration shown for 3.5s"

__gpy_cmd_duration=500
output=$(__gpy_segment_duration)
if [[ -n "$output" ]]; then
    echo "FAIL: Duration shown for 0.5s (should be hidden with 2000ms threshold)"
    exit 1
fi
echo "PASS: Duration hidden for 0.5s"

# No duration (Bash 3.x case)
if [[ $__gpy_duration_method == "none" ]]; then
    echo "WARN: Duration tracking disabled on Bash ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}"
fi

# Language and git content are agent-rendered; live renders are asserted in
# tests/bash/e2e_agent_autostart.test.bash.

echo "=== Testing Status Segment ==="
# Status pill: capped pill in ok/fail colors, status from __gpy_last_status.
__color_status_ok_bg=green __color_status_ok_fg=black __icon_status_ok="OK"
__color_status_fail_bg=red __color_status_fail_fg=white __icon_status_fail="NO"
__gpy_last_status=0
output=$(__gpy_segment_status "" "" "true")
if [[ -z "$output" ]]; then
    echo "FAIL: Status segment empty for success"
    exit 1
fi
# black text (30) on the green ok background (42), icon between the caps
if [[ "$output" != *"30;42m"*" OK"* ]]; then
    echo "FAIL: Status segment should draw the ok pill (black on green, one leading space); got: $output"
    exit 1
fi
echo "PASS: Status success"

__gpy_last_status=127
output=$(__gpy_segment_status "" "" "true")
if [[ -z "$output" ]]; then
    echo "FAIL: Status segment empty for failure"
    exit 1
fi
# white text (37) on the red fail background (41)
if [[ "$output" != *"37;41m"*" NO"* || "$output" == *"OK"* ]]; then
    echo "FAIL: Status segment should draw the fail pill (white on red, one leading space); got: $output"
    exit 1
fi
echo "PASS: Status failure"

# With a __status_format the agent draws the pill: success, is_last, prev_bg and
# is_first reach __gpy_request_exit_status in their own slots (empty flags stay
# in place), and an empty reply falls back to the local pill.
__gpy_test_real_exit_status=$(declare -f __gpy_request_exit_status)
__gpy_request_exit_status() {
    printf 'AGENT[%s|%s|%s|%s|argc=%s]' "$1" "$2" "$3" "$4" "$#"
}
__status_format=1
__gpy_last_status=0
output=$(__gpy_segment_status true cyan true)
if [[ "$output" != "AGENT[1|true|cyan|true|argc=4]" ]]; then
    echo "FAIL: Status agent-path args incorrect (want success|is_last|prev_bg|is_first): $output"
    exit 1
fi
__gpy_last_status=3
output=$(__gpy_segment_status "" "" "")
if [[ "$output" != "AGENT[0||||argc=4]" ]]; then
    echo "FAIL: Status agent-path should keep empty flags in place on failure: $output"
    exit 1
fi
echo "PASS: Status dual-path routes to the agent renderer with correctly-ordered args"

__gpy_request_exit_status() {
    return 1
}
__gpy_last_status=0
output=$(__gpy_segment_status "" "" "true")
if [[ "$output" != *"30;42m"*" OK"* ]]; then
    echo "FAIL: Status with an empty agent reply should fall back to the local pill; got: $output"
    exit 1
fi
echo "PASS: Status falls back to the local pill when the agent replies nothing"
eval "$__gpy_test_real_exit_status"
unset __gpy_test_real_exit_status __status_format

# The next chevron follows the exit status, with fish's fallbacks when unset.
__gpy_last_status=0
if [[ "$(__gpy_segment_bg status)" != "green" ]]; then
    echo "FAIL: __gpy_segment_bg status should be the ok background after success"
    exit 1
fi
__gpy_last_status=1
if [[ "$(__gpy_segment_bg status)" != "red" ]]; then
    echo "FAIL: __gpy_segment_bg status should be the fail background after failure"
    exit 1
fi
unset __color_status_ok_bg __color_status_fail_bg
if [[ "$(__gpy_segment_bg status)" != "red" ]]; then
    echo "FAIL: __gpy_segment_bg status should fall back to red with no theme color"
    exit 1
fi
__gpy_last_status=0
if [[ "$(__gpy_segment_bg status)" != "green" ]]; then
    echo "FAIL: __gpy_segment_bg status should fall back to green with no theme color"
    exit 1
fi
echo "PASS: __gpy_segment_bg status follows the exit status"

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
detected_root=""
__gpy_find_git_root "$worktree_checkout" detected_root
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

# __gpy_send_json stubs honour the optional OUTVAR ($3) like the real one:
# assign the reply when it is given, print it otherwise.
__gpy_test_stub_reply() {
    if [[ -n "$2" ]]; then
        printf -v "$2" '%s' "$1"
    else
        printf '%s' "$1"
    fi
}

echo "=== Testing Bash Workspace Sync ==="
captured_workspace_payload=""
captured_workspace_file=$(mktemp)
__gpy_send_json() {
    printf '%s' "$1" > "$captured_workspace_file"
    __gpy_test_stub_reply '{"Ack":null}' "${3:-}"
}
sync_dir=$(mktemp -d)
__gpy_registered=1
__gpy_last_workspace="/definitely/not/current"
pushd "$sync_dir" >/dev/null || exit 1
# The sandbox config disables the agent (sourcing must not start one); workspace
# sync is agent-only, so enable it for this call.
GPY_AGENT_ENABLED=1 __gpy_sync_workspace
popd >/dev/null || exit 1
rm -rf "$sync_dir"
captured_workspace_payload=$(cat "$captured_workspace_file")
rm -f "$captured_workspace_file"
if [[ "$captured_workspace_payload" != *'"op":"workspace"'* ]]; then
    echo "FAIL: Workspace sync did not send workspace payload"
    exit 1
fi
echo "PASS: Bash workspace sync"

echo "=== Testing Supervisor Check Round-Trip Cost ==="
# __gpy_supervisor_check must rate-limit before any IPC call and cache the
# protocol-version check after first success, so a healthy agent only pays for the ping.
supervisor_test_tmp_dir=$(mktemp -d)
supervisor_test_sock="$supervisor_test_tmp_dir/gpy-supervisor-healthy.sock"

if ! command -v python3 &>/dev/null; then
    test_skip_part "python3 not installed, cannot create a test Unix socket"
else
# __gpy_send_json is stubbed; the socket only needs to pass the `-S` gate.
    python3 tests/lib/stub_listener.py "$supervisor_test_sock" bind-only

    if [[ ! -S "$supervisor_test_sock" ]]; then
        test_skip_part "could not create test Unix socket for supervisor round-trip test"
    else
        __gpy_supervisor_test_saved_socket=$GPY_AGENT_SOCKET_PATH
        __gpy_supervisor_test_saved_enabled=$GPY_AGENT_SUPERVISOR_ENABLED
        __gpy_supervisor_test_saved_agent=$GPY_AGENT_ENABLED

        export GPY_AGENT_SOCKET_PATH="$supervisor_test_sock"
        # The sandbox config disables the agent (sourcing must not start one);
        # restart supervision needs it enabled.
        export GPY_AGENT_ENABLED=1
        export GPY_AGENT_SUPERVISOR_ENABLED=1
# Wide enough that three back-to-back calls land within one window.
        export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=60

# The stub runs in a command-substitution subshell; count via a file.
        __gpy_supervisor_call_count_file="$supervisor_test_tmp_dir/send_json_calls"
        : > "$__gpy_supervisor_call_count_file"
        __gpy_send_json() {
            printf 'x' >> "$__gpy_supervisor_call_count_file"
            local stub_reply=""
            case "$1" in
                *'"op":"ping"'*)
# The daemon's real Ack is {"status":"ok"}; supervisor.bash must accept exactly that.
                    stub_reply='{"status":"ok"}'
                    ;;
                *'"op":"status"'*)
                    stub_reply='{"AgentStatus":{"version":"0.1.0-test","protocol_version":1,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'
                    ;;
            esac
            __gpy_test_stub_reply "$stub_reply" "${3:-}"
            return 0
        }
        __gpy_supervisor_call_count() {
            wc -c < "$__gpy_supervisor_call_count_file" | tr -d ' '
        }

# Steady state: protocol check already cached. last_check_time=0 puts the
# first call past the rate-limit window; the other two land inside it.
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

# Down-path restart must clear the protocol cache in the PARENT shell:
# __gpy_supervisor_start is backgrounded, so a reset inside it never reaches the
# parent, and a mismatched daemon after restart would go undetected.
        __gpy_supervisor_saved_is_running=$(declare -f __gpy_supervisor_is_running)
        __gpy_supervisor_saved_start=$(declare -f __gpy_supervisor_start)
        __gpy_supervisor_is_running() { return 1; }  # agent is down
        __gpy_supervisor_start() { return 1; }        # no-op: don't spawn a real agent

        __gpy_protocol_version_checked=1   # stale cache from the prior healthy session
        __gpy_supervisor_last_check_time=0 # past the rate-limit window
        __gpy_supervisor_check_attempts=0

        __gpy_supervisor_check

        if [[ "$__gpy_protocol_version_checked" != "0" ]]; then
            echo "FAIL: down-path restart left parent protocol cache stale (__gpy_protocol_version_checked=$__gpy_protocol_version_checked, want 0); a post-restart mismatched agent would be skipped"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        if [[ "$__gpy_supervisor_check_attempts" != "1" ]]; then
            echo "FAIL: down-path restart did not increment restart attempts (got $__gpy_supervisor_check_attempts, want 1); the down-path was not actually taken"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: down-path restart invalidates the parent shell's protocol cache so the next health check re-checks the protocol"

# Interval/attempt cap are read from the theme export on every check:
# two prompts 15 s apart with a 60 s interval probe once.
        __gpy_supervisor_probe_count=0
        __gpy_supervisor_is_running() { __gpy_supervisor_probe_count=$((__gpy_supervisor_probe_count + 1)); return 1; }
        unset GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS
        export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=60
        __gpy_supervisor_last_check_time=0
        __gpy_supervisor_check_attempts=0
        __gpy_supervisor_check
        __gpy_supervisor_last_check_time=$((__gpy_supervisor_last_check_time - 15))
        __gpy_supervisor_check
        if [[ "$__gpy_supervisor_probe_count" != "1" ]]; then
            echo "FAIL: two checks 15 s apart with GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=60 probed $__gpy_supervisor_probe_count time(s), want 1"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS sets the health-check interval"

        # max_restart_attempts=1: a persistently dead agent gets one restart.
        export GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0
        export GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=1
        __gpy_supervisor_last_check_time=0
        __gpy_supervisor_check_attempts=0
        __gpy_supervisor_check
        __gpy_supervisor_check
        __gpy_supervisor_check
        if [[ "$__gpy_supervisor_check_attempts" != "1" ]]; then
            echo "FAIL: GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS=1 allowed $__gpy_supervisor_check_attempts restart(s), want 1"
            rm -rf "$supervisor_test_tmp_dir"
            exit 1
        fi
        echo "PASS: GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS caps restarts"
        unset GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS __gpy_supervisor_probe_count

        eval "$__gpy_supervisor_saved_is_running"
        eval "$__gpy_supervisor_saved_start"
        unset __gpy_supervisor_saved_is_running __gpy_supervisor_saved_start

# Restore the always-Ack stub rather than unsetting: later sections call
# __gpy_send_json unconditionally.
        __gpy_send_json() {
            __gpy_test_stub_reply '{"Ack":null}' "${3:-}"
        }
        unset -f __gpy_supervisor_call_count
        unset __gpy_supervisor_call_count_file
        unset GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS
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
        export GPY_AGENT_ENABLED=$__gpy_supervisor_test_saved_agent
        unset __gpy_supervisor_test_saved_socket __gpy_supervisor_test_saved_enabled __gpy_supervisor_test_saved_agent
    fi
fi
rm -rf "$supervisor_test_tmp_dir"

echo "=== Testing Last Segment Context ==="
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
# On a cold cache miss __gpy_segment_git makes a synchronous query via
# __gpy_sync_data_request; force a miss with an empty XDG_CACHE_HOME.
__gpy_sync_data_request() {
    printf '%s:%s' "$1" "$3"
}
__gpy_register_with_agent() { :; }
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
unset -f __gpy_sync_data_request
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
# gpy.bash sources every segments/*.bash at init, so a theme switch enabling a
# new segment only needs __enabled_segments refreshed. Guards against lazy sourcing.
# Bash consumes the doorbell flags in __gpy_precmd before rendering.
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

# Directory is agent-rendered; mock __gpy_request per display mode.
__gpy_request() {
    local path="$2"
    case "${GPY_UI_DIRECTORY_DISPLAY:-abbreviated}" in
        basename)
            printf ' %s ' "${path##*/}"
            ;;
        abbreviated)
# Abbreviate HOME/alpha/beta/project → ~/a/b/project.
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

echo "=== Testing Exit-Status Request Payload ==="

# Segment label with non-printing wrappers and SGR sequences removed.
visible_text() {
    printf '%s' "$1" | sed -e 's/\\\[//g' -e 's/\\\]//g' -e $'s/\033\\[[0-9;]*m//g'
}

# is_first/is_last ride in the shared flags tail; the exit_status op is
# distinct from the agent health-check "status" op. The __gpy_send_json stub
# lives in the command-substitution subshell only.
output=$(__gpy_send_json() { __gpy_test_stub_reply "$1" "${3:-}"; }; __gpy_request_exit_status 1 true "" true)
if [[ "$output" != '{"op":"exit_status","success":true,"format":"bash-prompt","is_last":true,"is_first":true}' ]]; then
    echo "FAIL: Exit-status request payload incorrect: $output"
    exit 1
fi
output=$(__gpy_send_json() { __gpy_test_stub_reply "$1" "${3:-}"; }; __gpy_request_exit_status 0 "" "blue" "")
if [[ "$output" != '{"op":"exit_status","success":false,"format":"bash-prompt","prev_bg":"blue"}' ]]; then
    echo "FAIL: Exit-status failure request payload incorrect: $output"
    exit 1
fi
echo "PASS: is_first and exit_status request payloads"

# With a format set but no usable agent reply — agent unreachable (the send
# fails) or an older agent answering the unknown op with {"error":...} —
# status draws the local pill (one leading space) rather than vanishing or
# printing the JSON. Earlier sections left an always-Ack __gpy_send_json, so
# each case stubs it in the subshell; locals keep the segment inputs out of
# the ambient state.
__gpy_test_render_fallback_pill() {
    local __status_format=1 __gpy_last_status=0
    visible_text "$(__gpy_segment_status true "")"
}
for gpy_test_reply in unreachable error; do
    if [[ "$gpy_test_reply" == error ]]; then
        output=$(__gpy_send_json() { __gpy_test_stub_reply '{"error":"Parse error: IPC communication failed: Unknown shell operation: exit_status"}' "${3:-}"; }; __gpy_test_render_fallback_pill)
    else
        output=$(__gpy_send_json() { return 1; }; __gpy_test_render_fallback_pill)
    fi
    if [[ "$output" != *" OK"* || "$output" == *error* ]]; then
        echo "FAIL: Agent $gpy_test_reply with a status format should draw the local status pill with one leading space; got: $output"
        exit 1
    fi
done
unset -f __gpy_test_render_fallback_pill
unset gpy_test_reply
echo "PASS: Status falls back to the local pill when the agent is unreachable or replies with an error"

echo "=== Testing Character/Directory Render Memoization ==="
# Both run in command-substitution subshells; record calls to a file.
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
# Earlier sections redefine __gpy_load_theme; pin it so the reload is deterministic.
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

# 4. Theme change re-renders both segments (the handler renders immediately;
# the trailing render is a hit).
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

# 5. An empty/failed render is never cached, so agent-down retries next prompt.
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
