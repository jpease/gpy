#!/usr/bin/env zsh

# tests/zsh/integration.test.zsh
# Comprehensive integration test for all segments

# Get project root
ROOT=${0:a:h:h:h}
cd "$ROOT"

# Drop inherited repo-scoping git env (GIT_DIR/GIT_WORK_TREE/...) so the worktree
# fixture below targets its own throwaway repo, not this one, when run from a git
# hook — `git -C <tmp>` does not override these (#275).
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

# Hermetic XDG dirs (#632): __gpy_load_theme sources a theme-export cache from
# $XDG_CACHE_HOME/gpy when one exists (#614), which would replace the built-in
# defaults this file asserts on with the developer's own theme.
__gpy_integration_xdg_root=$(mktemp -d "${TMPDIR:-/tmp}/gpy-it-xdg.XXXXXX")
export XDG_CACHE_HOME="$__gpy_integration_xdg_root/cache"
export XDG_CONFIG_HOME="$__gpy_integration_xdg_root/config"
mkdir -p "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"
export GPY_AGENT_SOCKET_PATH="$ROOT/.gpy-test-missing.sock"
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
# 24-hour, which is what it did before the clock became agent-rendered and is
# why zsh showed 17:21:42 beside fish's 5:21 PM under the same theme.
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
delegated=$(__gpy_segment_clock "" "black" "true")
if [[ "$delegated" != *"AGENT-CLOCK"* ]]; then
    echo "FAIL: clock should delegate to the agent when it answers; got: $delegated"
    exit 1
fi
unfunction __gpy_request_clock
echo "PASS: clock delegates to the agent when available"

echo "=== Testing Duration Segment ==="
# Test with duration above threshold - agent-rendered (#199): mock the IPC call
# so the test validates delegation without requiring a live agent.
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
    echo "FAIL: Duration shown for 0.5s (should be hidden)"
    exit 1
fi
echo "PASS: Duration hidden for 0.5s"

# Language and git content are agent-rendered; this file runs with no agent
# and a socket that does not exist, so it cannot assert on either. The live
# renders (branch, dirty marker, registration, workspace sync, unregister on
# exit) are asserted against a real agent and an interactive zsh on a pty in
# tests/zsh/e2e_agent_autostart.test.zsh (#646).

echo "=== Testing Status Segment ==="
output=$(__gpy_segment_status 0)
if [[ -z "$output" ]]; then
    echo "FAIL: Status segment empty for success"
    exit 1
fi
# Check for green color (success) - Zsh format uses %F{green} or color code
if [[ ! "$output" =~ "green" ]] && [[ ! "$output" =~ "32m" ]]; then
    echo "FAIL: Status segment incorrect color for success"
    exit 1
fi
echo "PASS: Status success"

output=$(__gpy_segment_status 127)
if [[ -z "$output" ]]; then
    echo "FAIL: Status segment empty for failure"
    exit 1
fi
# Check for red color (failure)
if [[ ! "$output" =~ "red" ]] && [[ ! "$output" =~ "31m" ]]; then
    echo "FAIL: Status segment incorrect color for failure"
    exit 1
fi
echo "PASS: Status failure"

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
# __enabled_segments (via __gpy_load_theme) before the next render. This
# guards against a future regression to per-segment lazy sourcing that would
# reintroduce the gap Fish had (#296).
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
detected_root=$(__gpy_find_git_root "$worktree_checkout")
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
# #613: dispatch passes is_last/is_first as "true"/"" (not the old
# "last"/"first" literals) directly to every segment -- no per-segment
# conversion.
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
# __gpy_segment_git reads the instant cache directly (#614) and, on a cold
# miss with the agent enabled, makes a bounded synchronous IPC query (#843)
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

# Directory is agent-rendered (#199); the shell passes GPY_UI_DIRECTORY_DISPLAY and
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

echo "=== Testing Hostname Segment ==="

# Preserve ambient state so mutations here don't leak into later test sections
# (test hygiene note flagged by the reviewer on the bash equivalent of this task).
__gpy_hostname_test_had_host=${+HOST}
__gpy_hostname_test_saved_host=$HOST
__gpy_hostname_test_had_is_ssh=${+__gpy_is_ssh}
__gpy_hostname_test_saved_is_ssh=$__gpy_is_ssh
__gpy_hostname_test_had_show_always=${+__hostname_show_always}
__gpy_hostname_test_saved_show_always=$__hostname_show_always
__gpy_hostname_test_had_trim_at=${+__hostname_trim_at}
__gpy_hostname_test_saved_trim_at=$__hostname_trim_at
__gpy_hostname_test_had_icon=${+__icon_hostname}
__gpy_hostname_test_saved_icon=$__icon_hostname
__gpy_hostname_test_had_format=${+__hostname_format}
__gpy_hostname_test_saved_format=$__hostname_format

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

# Pure-zsh render path: trim at first delimiter, zero forks
unset __hostname_format
HOST="host.example.com"
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

# Icon prefixes the label over SSH when set, and is omitted when empty. #826:
# the icon is Starship's ssh_symbol, so a local session omits it too.
__gpy_is_ssh=1
__hostname_trim_at="."
__icon_hostname="@"
output=$(__gpy_segment_hostname)
if [[ "$output" != *" @ host "* ]]; then
    echo "FAIL: Hostname icon missing from SSH render: $output"
    exit 1
fi
echo "PASS: Hostname icon shown over SSH when set"

__gpy_is_ssh=0
output=$(__gpy_segment_hostname)
if [[ "$output" == *"@"* || "$output" != *" host "* ]]; then
    echo "FAIL: Hostname icon should be omitted in a local session: $output"
    exit 1
fi
echo "PASS: Hostname icon omitted in a local session"

__gpy_is_ssh=1
__icon_hostname=""
output=$(__gpy_segment_hostname)
if [[ "$output" == *"@ host"* ]]; then
    echo "FAIL: Hostname icon should be omitted when unset: $output"
    exit 1
fi
echo "PASS: Hostname icon omitted when unset"

# Confirm zsh prompt-escape structure (not raw ANSI) on the pure-zsh path.
if [[ "$output" != *'%K{'* || "$output" != *'%F{'* || "$output" != *'%f%k'* ]]; then
    echo "FAIL: Hostname pure-zsh render missing prompt escapes: $output"
    exit 1
fi
echo "PASS: Hostname uses zsh prompt escapes"

# #826: the real request builder appends "is_ssh":true only for is_ssh=1. The
# __gpy_send_json stub lives in the command-substitution subshell only.
output=$(function __gpy_send_json() { print -rn -- "$1"; }; __gpy_request_hostname h "" "" 1)
if [[ "$output" != '{"op":"hostname","hostname":"h","format":"zsh-prompt","is_ssh":true}' ]]; then
    echo "FAIL: Hostname request should carry is_ssh for an SSH session: $output"
    exit 1
fi
output=$(function __gpy_send_json() { print -rn -- "$1"; }; __gpy_request_hostname h "" "" 0)
if [[ "$output" != '{"op":"hostname","hostname":"h","format":"zsh-prompt"}' ]]; then
    echo "FAIL: Hostname request should omit is_ssh for a local session: $output"
    exit 1
fi
echo "PASS: Hostname request payload carries is_ssh only over SSH"

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
function __gpy_request_hostname() {
    printf 'AGENT[%s|%s|%s|%s|argc=%s]' "$1" "$2" "$3" "$4" "$#"
}
__hostname_format="{hostname}"
__gpy_is_ssh=0
output=$(__gpy_segment_hostname true "cyan")
if [[ "$output" != "AGENT[host.example.com|true|cyan|0|argc=4]" ]]; then
    echo "FAIL: Hostname agent-path args incorrect (want hostname|is_last|prev_bg|is_ssh): $output"
    exit 1
fi
echo "PASS: Hostname dual-path routes to agent renderer with correctly-ordered args"

# #826: the 4th arg carries __gpy_is_ssh for an SSH session too.
__gpy_is_ssh=1
output=$(__gpy_segment_hostname true "cyan")
if [[ "$output" != "AGENT[host.example.com|true|cyan|1|argc=4]" ]]; then
    echo "FAIL: Hostname agent-path should forward __gpy_is_ssh=1 as the 4th arg: $output"
    exit 1
fi
echo "PASS: Hostname dual-path forwards __gpy_is_ssh as the 4th arg"

# is_last must stay the possibly-empty string (not "false") when this is not
# the last segment, and an empty prev_bg must still land in the 3rd slot (not
# shift left) — the bug class called out for this task. argc=4 proves the
# empty prev_bg was passed as a real (empty) positional arg, not omitted.
output=$(__gpy_segment_hostname "" "")
if [[ "$output" != "AGENT[host.example.com|||1|argc=4]" ]]; then
    echo "FAIL: Hostname agent-path args incorrect for not-last/empty-prev_bg: $output"
    exit 1
fi
echo "PASS: Hostname dual-path threads empty is_last and empty prev_bg correctly"

unset __hostname_format
unset -f __gpy_request_hostname

# Registration in __gpy_segment_bg: the render loop threads the hostname
# segment's background to the next segment's opening chevron.
__color_hostname_bg="magenta"
seg_bg=$(__gpy_segment_bg hostname)
if [[ "$seg_bg" != "magenta" ]]; then
    echo "FAIL: __gpy_segment_bg did not return hostname's background: $seg_bg"
    exit 1
fi
unset __color_hostname_bg
echo "PASS: Hostname registered in __gpy_segment_bg"

# Restore ambient state mutated above (test hygiene: avoid leaking
# HOST/__hostname_*/__gpy_is_ssh into later test sections or the parent shell).
if [[ $__gpy_hostname_test_had_host -eq 1 ]]; then
    HOST=$__gpy_hostname_test_saved_host
else
    unset HOST
fi
if [[ $__gpy_hostname_test_had_is_ssh -eq 1 ]]; then
    __gpy_is_ssh=$__gpy_hostname_test_saved_is_ssh
else
    unset __gpy_is_ssh
fi
if [[ $__gpy_hostname_test_had_show_always -eq 1 ]]; then
    __hostname_show_always=$__gpy_hostname_test_saved_show_always
else
    unset __hostname_show_always
fi
if [[ $__gpy_hostname_test_had_trim_at -eq 1 ]]; then
    __hostname_trim_at=$__gpy_hostname_test_saved_trim_at
else
    unset __hostname_trim_at
fi
if [[ $__gpy_hostname_test_had_icon -eq 1 ]]; then
    __icon_hostname=$__gpy_hostname_test_saved_icon
else
    unset __icon_hostname
fi
if [[ $__gpy_hostname_test_had_format -eq 1 ]]; then
    __hostname_format=$__gpy_hostname_test_saved_format
else
    unset __hostname_format
fi
unset __gpy_hostname_test_had_host __gpy_hostname_test_saved_host \
    __gpy_hostname_test_had_is_ssh __gpy_hostname_test_saved_is_ssh \
    __gpy_hostname_test_had_show_always __gpy_hostname_test_saved_show_always \
    __gpy_hostname_test_had_trim_at __gpy_hostname_test_saved_trim_at \
    __gpy_hostname_test_had_icon __gpy_hostname_test_saved_icon \
    __gpy_hostname_test_had_format __gpy_hostname_test_saved_format

echo "=== Testing Username Segment ==="

# Preserve ambient state so mutations here don't leak into later sections.
__gpy_username_test_had_user=${+USER}
__gpy_username_test_saved_user=$USER
__gpy_username_test_had_is_root=${+__gpy_is_root}
__gpy_username_test_saved_is_root=$__gpy_is_root
__gpy_username_test_had_is_sudo=${+__gpy_is_sudo}
__gpy_username_test_saved_is_sudo=$__gpy_is_sudo
__gpy_username_test_had_show_always=${+__username_show_always}
__gpy_username_test_saved_show_always=$__username_show_always
__gpy_username_test_had_icon=${+__icon_username}
__gpy_username_test_saved_icon=$__icon_username
__gpy_username_test_had_format=${+__username_format}
__gpy_username_test_saved_format=$__username_format

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

# Pure-zsh render path: renders $USER, no fork
unset __username_format
USER="root"
__icon_username=""
output=$(__gpy_segment_username)
if [[ "$output" != *" root "* ]]; then
    echo "FAIL: Username pure-zsh render missing \$USER: $output"
    exit 1
fi
echo "PASS: Username pure-zsh renders \$USER"

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

# Confirm zsh prompt-escape structure (not raw ANSI) on the pure-zsh path.
if [[ "$output" != *'%K{'* || "$output" != *'%F{'* || "$output" != *'%f%k'* ]]; then
    echo "FAIL: Username pure-zsh render missing prompt escapes: $output"
    exit 1
fi
echo "PASS: Username uses zsh prompt escapes"

# Dual-path: a non-empty __username_format routes to the agent renderer,
# quoting every positional arg so an empty prev_bg lands in the correct slot.
#
# #613: is_last is forwarded as-is ("true"/"") -- no more per-segment
# last/first-literal conversion to a "true"/"false" string.
function __gpy_request_username() {
    printf 'AGENT[%s|%s|%s|argc=%s]' "$1" "$2" "$3" "$#"
}
__username_format="{username}"
output=$(__gpy_segment_username true "cyan")
if [[ "$output" != "AGENT[root|true|cyan|argc=3]" ]]; then
    echo "FAIL: Username agent-path args incorrect (want username|is_last|prev_bg): $output"
    exit 1
fi
echo "PASS: Username dual-path routes to agent renderer with correctly-ordered args"

output=$(__gpy_segment_username "" "")
if [[ "$output" != "AGENT[root|||argc=3]" ]]; then
    echo "FAIL: Username agent-path args incorrect for not-last/empty-prev_bg: $output"
    exit 1
fi
echo "PASS: Username dual-path threads empty is_last and empty prev_bg correctly"

unset __username_format
unset -f __gpy_request_username

# Registration in __gpy_segment_bg: the render loop threads the username
# segment's background to the next segment's opening chevron.
__color_username_bg="magenta"
seg_bg=$(__gpy_segment_bg username)
if [[ "$seg_bg" != "magenta" ]]; then
    echo "FAIL: __gpy_segment_bg did not return username's background: $seg_bg"
    exit 1
fi
unset __color_username_bg
echo "PASS: Username registered in __gpy_segment_bg"

# Restore ambient state mutated above.
if [[ $__gpy_username_test_had_user -eq 1 ]]; then
    USER=$__gpy_username_test_saved_user
else
    unset USER
fi
if [[ $__gpy_username_test_had_is_root -eq 1 ]]; then
    __gpy_is_root=$__gpy_username_test_saved_is_root
else
    unset __gpy_is_root
fi
if [[ $__gpy_username_test_had_is_sudo -eq 1 ]]; then
    __gpy_is_sudo=$__gpy_username_test_saved_is_sudo
else
    unset __gpy_is_sudo
fi
if [[ $__gpy_username_test_had_show_always -eq 1 ]]; then
    __username_show_always=$__gpy_username_test_saved_show_always
else
    unset __username_show_always
fi
if [[ $__gpy_username_test_had_icon -eq 1 ]]; then
    __icon_username=$__gpy_username_test_saved_icon
else
    unset __icon_username
fi
if [[ $__gpy_username_test_had_format -eq 1 ]]; then
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

# Mirrors __gpy_precmd's exact render + relay-consumption sequence (#343) --
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
