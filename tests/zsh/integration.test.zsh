#!/usr/bin/env zsh

# tests/zsh/integration.test.zsh
# Comprehensive integration test for all segments

# Get project root
ROOT=${0:a:h:h:h}
cd "$ROOT"

# Drop inherited repo-scoping git env (GIT_DIR/GIT_WORK_TREE/...) so the worktree
# fixture below targets its own throwaway repo, not this one, when run from a git
# hook — `git -C <tmp>` does not override these.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
    GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
    GIT_PREFIX GIT_NAMESPACE 2>/dev/null || true

stderr_file=$(mktemp)
exec 3>&2
exec 2>"$stderr_file"

function __gpy_finish_test() {
    local test_status=$?
    exec 2>&3
    if [[ $test_status -eq 0 && -s "$stderr_file" ]]; then
        echo "FAIL: Unexpected stderr output"
        cat "$stderr_file"
        rm -f "$stderr_file"
        exit 1
    fi
    rm -f "$stderr_file"
    rm -rf "${__gpy_integration_xdg_root:-}"
    exit $test_status
}
trap __gpy_finish_test EXIT

# Hermetic XDG dirs: __gpy_load_theme sources a theme-export cache from
# $XDG_CACHE_HOME/gpy when one exists, which would replace the built-in
# defaults this file asserts on with the developer's own theme.
__gpy_integration_xdg_root=$(mktemp -d "${TMPDIR:-/tmp}/gpy-it-xdg.XXXXXX")
export XDG_CACHE_HOME="$__gpy_integration_xdg_root/cache"
export XDG_CONFIG_HOME="$__gpy_integration_xdg_root/config"
mkdir -p "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"
export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_AGENT_SOCKET_PATH="$__gpy_integration_xdg_root/missing.sock"
# Keep the test independent from an ambient development binary while retaining
# standard tools such as sed, git, mktemp, and stat. With no gpy-agent on PATH
# there is no agent to start and no theme export, so no supervisor flag either.
path=(/bin /usr/bin /usr/sbin /sbin)
rehash

source zsh/gpy.zsh

echo "=== Testing Clock Segment ==="
# Agent unreachable here (GPY_AGENT_SOCKET_PATH points at a missing socket),
# so this exercises the pure-zsh fallback: still renders, uncapped.
output=$(__gpy_segment_clock)
if [[ -z "$output" ]]; then
    echo "FAIL: Clock segment empty"
    exit 1
fi
echo "PASS: $output"

# The fallback must honor the configured time format rather than hardcoding
# 24-hour, so zsh shows the same 5:21 PM as fish under the same theme.
__time_format=12 __clock_show_seconds=0 spec=$(__gpy_clock_time_spec)
if [[ "$spec" != "%-I:%M %p" ]]; then
    echo "FAIL: 12-hour spec should be '%-I:%M %p'; got: $spec"
    exit 1
fi
__time_format=24 __clock_show_seconds=1 spec=$(__gpy_clock_time_spec)
if [[ "$spec" != "%-H:%M:%S" ]]; then
    echo "FAIL: 24-hour+seconds spec should be '%-H:%M:%S'; got: $spec"
    exit 1
fi
echo "PASS: fallback clock honors the configured time format"

# When the agent does answer, the segment must delegate rather than render
# locally. Mock the request helper the way the duration test below does.
function __gpy_request_clock() {
    printf 'AGENT-CLOCK'
}
__clock_format=1
delegated=$(__gpy_segment_clock "" "black" "true")
if [[ "$delegated" != *"AGENT-CLOCK"* ]]; then
    echo "FAIL: clock should delegate to the agent when it answers; got: $delegated"
    exit 1
fi
echo "PASS: clock delegates to the agent when available"

# ...but only for a theme that sets [segments.clock].format (the exported
# presence flag): without one the shell draws the clock itself, and must
# not spend an IPC round trip on it.
__clock_format=""
local_clock=$(__gpy_segment_clock "" "black" "true")
if [[ "$local_clock" == *"AGENT-CLOCK"* || "$local_clock" != *'%D{'* ]]; then
    echo "FAIL: a theme without a clock format must render the clock locally; got: $local_clock"
    exit 1
fi
unfunction __gpy_request_clock
echo "PASS: clock renders locally when the theme sets no clock format"

echo "=== Testing Duration Segment ==="
# Pin the threshold so the test is independent of theme configuration.
__duration_threshold_ms=2000
# Above threshold - agent-rendered: mock the IPC call so the test validates
# delegation without requiring a live agent.
function __gpy_request_duration() {
    printf '\033[33m 3.5s \033[0m'
}
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

# Language and git content are agent-rendered; this file runs with no agent
# and a socket that does not exist, so it cannot assert on either. The live
# renders (branch, dirty marker, registration, workspace sync, unregister on
# exit) are asserted against a real agent and an interactive zsh on a pty in
# tests/zsh/e2e_agent_autostart.test.zsh.

echo "=== Testing Status Segment ==="
# The status pill is drawn like fish's: a capped pill in the theme's
# ok/fail colors, the exit status read from __gpy_last_status (as the render
# loop publishes it).
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
__gpy_test_real_exit_status=$(functions __gpy_request_exit_status)
function __gpy_request_exit_status() {
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

function __gpy_request_exit_status() {
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

# The next segment's chevron needs the pill's background: it follows the exit
# status, with fish's literal fallbacks when the theme sets no color.
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

echo "=== Testing Full Prompt ==="
__enabled_segments=(status directory git clock duration language)
__gpy_cmd_duration=0
PROMPT=$(__gpy_render_prompt 0)
if [[ -z "$PROMPT" ]]; then
    echo "FAIL: Full prompt empty"
    exit 1
fi
echo "PASS: Full prompt rendered"

echo "=== Testing Doorbell Reload (lazy segment gap regression) ==="
# gpy.zsh sources every file under segments/*.zsh unconditionally at init,
# regardless of $__enabled_segments (unlike Fish, which only sources files for
# segments already in the enabled list). So a theme switch that enables a
# segment new to this shell's __enabled_segments already has its detect/render
# functions in memory; the doorbell's reload only needs to refresh
# __enabled_segments (via __gpy_load_theme) before the next render.
# Per-segment lazy sourcing would leave a newly enabled segment without its
# functions until the shell sourced it.
doorbell_dir=$(mktemp -d)
doorbell_stderr="$doorbell_dir/stderr"
__gpy_shell_flag_base="$doorbell_dir/$$"
# What the agent does for a config change: leave the reload flag, ring SIGURG.
function __gpy_test_ring_reload() {
    : >"$__gpy_shell_flag_base.reload"
    TRAPURG
}
function __gpy_load_theme() {
    __enabled_segments=(duration)
}
__duration_threshold_ms=100
__gpy_cmd_duration=5000
__gpy_test_ring_reload 2>"$doorbell_stderr" 1>/dev/null
if [[ -s "$doorbell_stderr" ]]; then
    echo "FAIL: doorbell handler wrote to stderr: $(<"$doorbell_stderr")"
    exit 1
fi
if [[ -e "$__gpy_shell_flag_base.reload" ]]; then
    echo "FAIL: doorbell left the reload flag behind"
    exit 1
fi
echo "PASS: doorbell consumes the reload flag"
PROMPT=$(__gpy_render_prompt 0)
if [[ "$PROMPT" == *"3.5s"* ]]; then
    echo "PASS: newly-enabled duration segment rendered after reload doorbell"
else
    echo "FAIL: newly-enabled duration segment missing after reload doorbell (PROMPT=$PROMPT)"
    exit 1
fi

# A bare doorbell (no flag) is a repaint: it must not reload the theme.
function __gpy_load_theme() {
    __enabled_segments=(directory)
}
TRAPURG >/dev/null 2>&1
if [[ "${__enabled_segments[*]}" == "duration" ]]; then
    echo "PASS: doorbell without a reload flag does not reload"
else
    echo "FAIL: doorbell without a reload flag reloaded (enabled=${__enabled_segments[*]})"
    exit 1
fi

function __gpy_load_theme() {
    __enabled_segments=(duration does-not-exist-segment)
}
__gpy_test_ring_reload 2>"$doorbell_stderr" 1>/dev/null
if [[ -s "$doorbell_stderr" ]]; then
    echo "FAIL: doorbell handler wrote to stderr on unknown segment: $(<"$doorbell_stderr")"
    exit 1
fi
echo "PASS: unknown segment in enabled list does not error the doorbell handler"
unset __duration_threshold_ms __gpy_cmd_duration doorbell_stderr

echo "=== Testing Git Worktree Detection ==="
exec 2>/dev/null
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
worktree_tree=$(git -C "$worktree_repo" write-tree)
worktree_commit=$(printf 'init\n' | git -C "$worktree_repo" commit-tree "$worktree_tree")
git -C "$worktree_repo" update-ref refs/heads/main "$worktree_commit"
git -c core.hooksPath="$worktree_hooks" -C "$worktree_repo" worktree add -q "$worktree_checkout"
exec 2>>"$stderr_file"
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

echo "=== Testing Zsh Agent Payloads ==="
register_payload=$(__gpy_build_register_payload)
if [[ "$register_payload" != *'"op":"register"'* || "$register_payload" != *'"cwd":'* || "$register_payload" != *'"shell":"zsh"'* || "$register_payload" != *'"shell_version":'* ]]; then
    echo "FAIL: Register payload missing required fields: $register_payload"
    exit 1
fi
workspace_payload=$(__gpy_build_workspace_payload)
if [[ "$workspace_payload" != *'"op":"workspace"'* || "$workspace_payload" != *'"cwd":'* ]]; then
    echo "FAIL: Workspace payload missing required fields: $workspace_payload"
    exit 1
fi
echo "PASS: Zsh agent payloads"

echo "=== Testing Last Segment Context ==="
# Dispatch passes is_last/is_first as "true"/"" directly to every segment --
# no per-segment conversion.
function __gpy_segment_alpha() { printf 'alpha:%s|' "$1" }
function __gpy_segment_beta() { printf 'beta:%s|' "$1" }
__enabled_segments=(alpha beta)
PROMPT=$(__gpy_render_prompt 0)
if [[ "$PROMPT" != *"alpha:|"* || "$PROMPT" != *"beta:true|"* ]]; then
    echo "FAIL: Last segment context not passed correctly: $PROMPT"
    exit 1
fi
function __gpy_request() {
    printf '%s:%s' "$1" "$4"
}
# __gpy_segment_git reads the instant cache directly and, on a cold
# miss with the agent enabled, makes a bounded synchronous IPC query
# through __gpy_sync_data_request instead of falling to oneshot, so that is the
# stub reached here -- with a guaranteed miss: point XDG_CACHE_HOME at an empty
# dir rather than relying on whatever real instant-prompt cache this
# machine's own gpy-agent may have already written for this repo.
function __gpy_sync_data_request() {
    printf '%s:%s' "$1" "$3"
}
function __gpy_register_with_agent() { :; }
local __gpy_last_segment_test_saved_xdg_cache_home=${XDG_CACHE_HOME:-}
export XDG_CACHE_HOME=$(mktemp -d)
git_last_output=$(__gpy_segment_git true)
git_last_status=$?
rm -rf "$XDG_CACHE_HOME"
if [[ -n "$__gpy_last_segment_test_saved_xdg_cache_home" ]]; then
    export XDG_CACHE_HOME="$__gpy_last_segment_test_saved_xdg_cache_home"
else
    unset XDG_CACHE_HOME
fi
if [[ "$git_last_output" != "git:true" ]]; then
    echo "FAIL: Git segment did not pass is_last=true: $git_last_output (status=$git_last_status)"
    exit 1
fi
unfunction __gpy_sync_data_request 2>/dev/null
echo "PASS: Last segment context"

echo "=== Testing Directory Display Modes ==="
temp_dir=$(mktemp -d)
mkdir -p "$temp_dir/alpha/beta/project"
pushd "$temp_dir/alpha/beta/project" >/dev/null || exit 1
HOME="$temp_dir"
GPY_UI_DIRECTORY_MAX_LENGTH=1000

# Directory is agent-rendered; the shell passes GPY_UI_DIRECTORY_DISPLAY and
# $PWD to the agent via __gpy_request. Mock __gpy_request to simulate the agent
# returning formatted path strings for each display mode so the assertions can
# verify delegation without requiring a live agent.
# Note: abbreviated uses the known test path structure (alpha/beta/project → a/b).
function __gpy_request() {
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

GPY_UI_DIRECTORY_DISPLAY=basename
output=$(__gpy_segment_directory)
if [[ "$output" != *" project "* ]]; then
    echo "FAIL: Basename mode did not render project name"
    popd >/dev/null
    rm -rf "$temp_dir"
    exit 1
fi

GPY_UI_DIRECTORY_DISPLAY=abbreviated
output=$(__gpy_segment_directory)
if [[ "$output" != *" ~/a/b/project "* ]]; then
    echo "FAIL: Abbreviated mode did not render abbreviated path"
    popd >/dev/null
    rm -rf "$temp_dir"
    exit 1
fi

GPY_UI_DIRECTORY_DISPLAY=full
output=$(__gpy_segment_directory)
if [[ "$output" != *" $temp_dir/alpha/beta/project "* ]]; then
    echo "FAIL: Full mode did not render full path"
    popd >/dev/null
    rm -rf "$temp_dir"
    exit 1
fi

unset GPY_UI_DIRECTORY_DISPLAY
unset GPY_UI_DIRECTORY_MAX_LENGTH
popd >/dev/null || exit 1
rm -rf "$temp_dir"
echo "PASS: Directory display modes"

echo "=== Testing Exit-Status Request Payload ==="

# What a shell-drawn segment shows once its non-printing wrappers and SGR
# sequences are removed (the segments are capped pills: the label sits
# between the delimiters, with no padding of its own).
visible_text() {
    printf '%s' "$1" | sed -e 's/%[{}]//g' -e $'s/\033\\[[0-9;]*m//g'
}

# is_first/is_last ride in the shared flags tail; the exit_status op is
# distinct from the agent health-check "status" op. The __gpy_send_json stub
# lives in the command-substitution subshell only.
output=$(function __gpy_send_json() { print -rn -- "$1"; }; __gpy_request_exit_status 1 true "" true)
if [[ "$output" != '{"op":"exit_status","success":true,"format":"zsh-prompt","is_last":true,"is_first":true}' ]]; then
    echo "FAIL: Exit-status request payload incorrect: $output"
    exit 1
fi
output=$(function __gpy_send_json() { print -rn -- "$1"; }; __gpy_request_exit_status 0 "" "blue" "")
if [[ "$output" != '{"op":"exit_status","success":false,"format":"zsh-prompt","prev_bg":"blue"}' ]]; then
    echo "FAIL: Exit-status failure request payload incorrect: $output"
    exit 1
fi
echo "PASS: is_first and exit_status request payloads"

# With a format set but no usable agent reply — agent unreachable (the send
# fails) or an older agent answering the unknown op with {"error":...} —
# status draws the local pill (one leading space) rather than vanishing or
# printing the JSON. Each case stubs __gpy_send_json in the subshell.
for gpy_test_reply in unreachable error; do
    output=$(
        if [[ "$gpy_test_reply" == error ]]; then
            function __gpy_send_json() { print -r -- '{"error":"Parse error: IPC communication failed: Unknown shell operation: exit_status"}'; }
        else
            function __gpy_send_json() { return 1; }
        fi
        __status_format=1 __gpy_last_status=0
        visible_text "$(__gpy_segment_status true "")"
    )
    if [[ "$output" != *" OK"* || "$output" == *error* ]]; then
        echo "FAIL: Agent $gpy_test_reply with a status format should draw the local status pill with one leading space; got: $output"
        exit 1
    fi
done
unset gpy_test_reply
echo "PASS: Status falls back to the local pill when the agent is unreachable or replies with an error"

echo "=== Testing Character/Directory Render Memoization ==="
# __gpy_request_character and __gpy_segment_directory both run inside the
# `$(...)` subshell __gpy_render_prompt executes in, which forks a subshell --
# an in-memory counter incremented inside a stub would not survive that
# subshell, so record calls to a file instead (same technique the "Render
# Loop prev_bg Threading" section above uses for the same reason).
memo_tmp_dir=$(mktemp -d)
memo_char_calls_file="$memo_tmp_dir/char_calls"
memo_dir_calls_file="$memo_tmp_dir/dir_calls"
: > "$memo_char_calls_file"
: > "$memo_dir_calls_file"

function __gpy_request_character() {
    printf 'x' >> "$memo_char_calls_file"
    printf 'CHAR'
}
function __gpy_segment_directory() {
    printf 'x' >> "$memo_dir_calls_file"
    printf 'DIR'
}
# Earlier sections in this file permanently redefine __gpy_load_theme; pin it
# here so the doorbell's reload is deterministic for this section regardless of
# what ran before it.
function __gpy_load_theme() {
    __enabled_segments=(directory)
}

function __gpy_memo_call_count() {
    wc -c < "$1" 2>/dev/null | tr -d ' '
}

# Mirrors __gpy_precmd's exact render + relay-consumption sequence --
# see the cache var declarations in zsh/core/init.zsh for why a cache-miss
# write inside __gpy_render_prompt's subshell has to be relayed through a
# file -- without its unrelated agent-registration/workspace-sync side
# effects, so this isolates the memoization mechanism under test.
function __gpy_memo_render() {
    local ret=$1
    PROMPT=$(__gpy_render_prompt $ret)
    __gpy_load_cache_relay "$__gpy_char_cache_relay_path" __gpy_char_cache_key __gpy_char_cache_val
    __gpy_load_cache_relay "$__gpy_dir_cache_relay_path" __gpy_dir_cache_key __gpy_dir_cache_val
}

function __gpy_memo_reset() {
    : > "$memo_char_calls_file"
    : > "$memo_dir_calls_file"
    __gpy_char_cache_key=""
    __gpy_char_cache_val=""
    __gpy_dir_cache_key=""
    __gpy_dir_cache_val=""
    rm -f "$__gpy_char_cache_relay_path" "$__gpy_dir_cache_relay_path"
}

__enabled_segments=(directory)
__color_directory_bg="blue"
__gpy_theme_name="testtheme"

memo_dir_a=$(mktemp -d)
memo_dir_b=$(mktemp -d)
cd "$memo_dir_a" || exit 1

# 1. Cache hit: identical status/PWD/theme -> zero additional IPC calls
__gpy_memo_reset
__gpy_memo_render 0 # baseline render (miss for both)
__gpy_memo_render 0 # identical render (must hit for both)
if [[ "$(__gpy_memo_call_count "$memo_char_calls_file")" == "1" && "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "1" ]]; then
    echo "PASS: identical render is a cache hit for character+directory (zero extra IPC)"
else
    echo "FAIL: identical render made extra IPC calls (char=$(__gpy_memo_call_count "$memo_char_calls_file"), dir=$(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

# 2. Exit status flip re-renders the character only
__gpy_memo_reset
__gpy_memo_render 0 # baseline (miss for both)
__gpy_memo_render 1 # status flipped
if [[ "$(__gpy_memo_call_count "$memo_char_calls_file")" == "2" && "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "1" ]]; then
    echo "PASS: exit status flip re-renders character only"
else
    echo "FAIL: exit status flip counts wrong (char=$(__gpy_memo_call_count "$memo_char_calls_file"), dir=$(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

# 3. cd re-renders the directory only
__gpy_memo_reset
cd "$memo_dir_a" || exit 1
__gpy_memo_render 0 # baseline (miss for both)
cd "$memo_dir_b" || exit 1
__gpy_memo_render 0 # PWD changed
if [[ "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "2" && "$(__gpy_memo_call_count "$memo_char_calls_file")" == "1" ]]; then
    echo "PASS: cd re-renders directory only"
else
    echo "FAIL: cd counts wrong (char=$(__gpy_memo_call_count "$memo_char_calls_file"), dir=$(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

# 4. Theme change (reload doorbell) re-renders both segments on the next render
__gpy_memo_reset
__gpy_theme_name="themeA"
__gpy_memo_render 0 # baseline (miss for both)
__gpy_theme_name="themeB"
__gpy_test_ring_reload >/dev/null 2>&1
__gpy_memo_render 0 # theme changed
if [[ "$(__gpy_memo_call_count "$memo_char_calls_file")" == "2" && "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "2" ]]; then
    echo "PASS: theme change re-renders character and directory"
else
    echo "FAIL: theme change counts wrong (char=$(__gpy_memo_call_count "$memo_char_calls_file"), dir=$(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

# 5. An empty/failed render is never cached (agent-down fallback must retry
# on the very next prompt, not get stuck serving nothing forever).
function __gpy_request_character() {
    printf 'x' >> "$memo_char_calls_file"
}
__gpy_memo_reset
__gpy_memo_render 0
__gpy_memo_render 0
if [[ "$(__gpy_memo_call_count "$memo_char_calls_file")" == "2" ]]; then
    echo "PASS: empty character render is never cached"
else
    echo "FAIL: empty character render count wrong ($(__gpy_memo_call_count "$memo_char_calls_file"))"
    exit 1
fi

function __gpy_segment_directory() {
    printf 'x' >> "$memo_dir_calls_file"
}
__gpy_memo_reset
__gpy_memo_render 0
__gpy_memo_render 0
if [[ "$(__gpy_memo_call_count "$memo_dir_calls_file")" == "2" ]]; then
    echo "PASS: empty directory render is never cached"
else
    echo "FAIL: empty directory render count wrong ($(__gpy_memo_call_count "$memo_dir_calls_file"))"
    exit 1
fi

cd "$ROOT" || exit 1
rm -rf "$memo_tmp_dir" "$memo_dir_a" "$memo_dir_b"
unset -f __gpy_memo_call_count __gpy_memo_reset __gpy_memo_render
echo "PASS: Character/directory render memoization"

echo "=== All Tests Passed ==="
