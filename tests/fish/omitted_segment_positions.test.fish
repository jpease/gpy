#!/usr/bin/env fish
# tests/fish/omitted_segment_positions.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #766.
#
# fish_prompt assigns first/last positions from the detect pass, before
# anything renders. git and language pass detect on a .git root / project
# marker, but on a cold miss (no instant-cache entry) language always prints
# nothing, and git prints nothing while the agent is down. The segment still
# held its position, so the previous segment ended with a dangling separator
# instead of the closing cap, or the next one lost its first-segment form.
#
# Drives the real fish_prompt loop with no agent socket and an empty instant
# cache. segment_directory_render is replaced by a stub that prints the
# is_last/is_first it was called with.

set -g pass_count 0
set -g fail_count 0

function check --argument-names label result
    if test "$result" = pass
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label"
    end
end

set -l script_dir (dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

# Drop inherited repo-scoping git env so `git init` below targets the
# throwaway repo, not this one, when run from a git hook (#275).
set -e GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_PREFIX GIT_NAMESPACE

set -l saved_home $HOME
set -l saved_xdg_config $XDG_CONFIG_HOME
set -l saved_xdg_cache $XDG_CACHE_HOME
set -l saved_xdg_runtime $XDG_RUNTIME_DIR
set -l temp_home (mktemp -d)
set -gx HOME $temp_home
set -gx XDG_CONFIG_HOME $temp_home/.config
# Isolated cache and runtime roots: an empty instant cache and no agent socket.
set -gx XDG_CACHE_HOME $temp_home/.cache
set -gx XDG_RUNTIME_DIR $temp_home/run
set -e GPY_AGENT_SOCKET_PATH
mkdir -p $XDG_CONFIG_HOME/gpy $XDG_RUNTIME_DIR

# Supervisor disabled: full init path without spawning a daemon.
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
set -gx GPY_GIT_ENABLED 1
set -gx GPY_LANGUAGE_ENABLED 1
# Load the real git, language and status segments.
set -gx GPY_TEST_SEGMENTS "directory git language status"

source fish/core/init.fish
source fish/functions/fish_prompt.fish

function segment_directory_render --argument-names is_last is_first
    printf 'DIR[%s|%s]' "$is_last" "$is_first"
end

# Record every background refresh instead of sending it.
set -g __refresh_calls
function __gpy_maybe_refresh
    set -g __refresh_calls $__refresh_calls "$argv[1]"
end

set -g __segment_delim_first F
set -g __segment_delim_start S
set -g __segment_delim_end E
set -g __segment_delim_last L
set -g __icon_status_ok OK
set -g __gpy_add_newline 0
set -g __gpy_is_root 1

# Run fish_prompt with a clean exit status (it reads $status on entry) and
# return its output with ANSI escapes stripped.
function render_plain
    true
    fish_prompt | string collect | string replace -ra '\e\[[0-9;]*m' ''
end

set -l repo (mktemp -d)
git -C $repo init -q
set -l project (mktemp -d)
touch $project/Cargo.toml

# --- Git cold miss, agent down: git is omitted, directory closes the line ---
cd $repo
set -g __enabled_segments directory git
set -l out (render_plain | string collect)
check "git cold miss with agent down: directory is first and last" (string match -q '*DIR[true|true]*' -- $out; and echo pass; or echo fail)
string match -q '*DIR[true|true]*' -- $out; or echo "  got: "(string escape -- $out)

# --- Language cold miss: language is omitted, directory opens the line ---
cd $project
set -g __refresh_calls
set -g __enabled_segments language directory
set out (render_plain | string collect)
check "language cold miss: directory is first and last" (string match -q '*DIR[true|true]*' -- $out; and echo pass; or echo fail)
string match -q '*DIR[true|true]*' -- $out; or echo "  got: "(string escape -- $out)
check "language cold miss still requests one refresh" (test "$__refresh_calls" = lang; and echo pass; or echo fail)
test "$__refresh_calls" = lang; or echo "  refresh calls: $__refresh_calls"

# --- Omitted segment in the middle: neighbours keep their positions ---
set -g __enabled_segments directory language status
set out (render_plain | string collect)
check "omitted middle segment: directory stays first, not last" (string match -q 'DIR[|true]*' -- $out; and echo pass; or echo fail)
check "omitted middle segment: status follows directory with delim_start" (string match -q 'DIR[|true]SOK*' -- $out; and echo pass; or echo fail)
string match -q 'DIR[|true]SOK*' -- $out; or echo "  got: "(string escape -- $out)

# --- Every agent segment omitted: the shell-rendered status opens the line ---
cd $repo
set -g __enabled_segments language git status
set out (render_plain | string collect)
check "all agent segments omitted: status opens with delim_first" (string match -q 'FOK*' -- $out; and echo pass; or echo fail)
string match -q 'FOK*' -- $out; or echo "  got: "(string escape -- $out)

# --- Warm cache: git renders and keeps the last position ---
set -l cache_dir (__gpy_instant_cache_dir)
set -l cache_key (__gpy_path_to_cache_key (__gpy_find_git_root $repo))
mkdir -p $cache_dir
printf GITNONE >$cache_dir/$cache_key.git.none.ansi
printf GITWARM >$cache_dir/$cache_key.git_last.black.ansi
set -g __enabled_segments directory git
set out (render_plain | string collect)
check "warm git cache: directory is first, not last" (string match -q '*DIR[|true]*' -- $out; and echo pass; or echo fail)
check "warm git cache: git renders its exact variant" (string match -q '*DIR[|true]GITWARM*' -- $out; and echo pass; or echo fail)
string match -q '*DIR[|true]GITWARM*' -- $out; or echo "  got: "(string escape -- $out)

# --- The presence probe counts contextual-only entries (no `.none`) ---
# A cache populated only by requests that carried a prev_bg has no `.none`
# file; the probe must still see it rather than hiding a renderable segment.
set -l project_key (__gpy_path_to_cache_key (path resolve -- $project))
check "probe: no language entry before the cache is written" (__gpy_instant_cache_present lang $project; and echo fail; or echo pass)
printf LANG >$cache_dir/$project_key.lang_first.magenta.ansi
check "probe: a contextual-only language entry counts as present" (__gpy_instant_cache_present lang $project; and echo pass; or echo fail)

cd "$repo_root"
rm -rf $temp_home $repo $project
set -gx HOME $saved_home
if test -n "$saved_xdg_config"
    set -gx XDG_CONFIG_HOME $saved_xdg_config
else
    set -e XDG_CONFIG_HOME
end
if test -n "$saved_xdg_cache"
    set -gx XDG_CACHE_HOME $saved_xdg_cache
else
    set -e XDG_CACHE_HOME
end
if test -n "$saved_xdg_runtime"
    set -gx XDG_RUNTIME_DIR $saved_xdg_runtime
else
    set -e XDG_RUNTIME_DIR
end

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count omitted-segment position tests passed"
