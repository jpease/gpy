#!/usr/bin/env fish
# tests/fish/git_segment_bg_from_cache.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The segment after git is drawn after the colour the agent recorded in
# `{key}.git_bg.none.ansi`; a missing file and every agent-free render use the
# clean colour.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/flags.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/segments/git.fish"

set -l tmp_dir (mktemp -d)
set -gx XDG_CACHE_HOME "$tmp_dir/cache"
set -l cache_dir "$XDG_CACHE_HOME/gpy/instant-prompts"
mkdir -p "$cache_dir"

set -l repo "$tmp_dir/myrepo"
mkdir -p "$repo/.git"
set -l resolved_root (__gpy_instant_cache_key_path git "$repo")
set -l cache_key (__gpy_chunk_cache_key (__gpy_path_to_cache_key "$resolved_root"))
set -l bg_file "$cache_dir/$cache_key."(__gpy_git_bg_file_tail ansi)

set -g __color_git_clean_bg green

# --- __gpy_git_segment_bg ---

check "missing file falls back to clean" green (__gpy_git_segment_bg "$repo")
printf '%s' red >"$bg_file"
check "recorded colour is returned" red (__gpy_git_segment_bg "$repo")
check "outside a repository falls back to clean" green (__gpy_git_segment_bg "$tmp_dir")

# --- segment_git_render, instant-cache hit ---

printf '%s' GIT >"$cache_dir/$cache_key.git_last.none.ansi"
set -g __gpy_prompt_now (__gpy_file_mtime "$cache_dir/$cache_key.git_last.none.ansi")
set -g GPY_AGENT_ENABLED 1
cd "$repo"
set -g __gpy_last_segment_bg ""
set -l out (segment_git_render true "" | string collect)
check "cache hit prints the cached render" GIT "$out"
check "cache hit leaves the recorded colour" red "$__gpy_last_segment_bg"

# --- segment_git_render, agent disabled: the stale file is ignored ---

rm -f "$cache_dir/$cache_key.git_last.none.ansi"
function __gpy_oneshot_fallback
    printf '%s' ONESHOT
end
set -g GPY_AGENT_ENABLED 0
set -g __gpy_last_segment_bg ""
set -l out (segment_git_render true "" | string collect)
check "agent-free render prints the oneshot" ONESHOT "$out"
check "agent-free render leaves the clean colour" green "$__gpy_last_segment_bg"

# --- no endpoint (no runtime root): never a socket ---

set -g GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SOCKET_PATH "$tmp_dir/agent.sock"
test_stub_listener "$tmp_dir/agent.sock" bind-only >/dev/null
if __gpy_git_socket_present
    pass "an existing socket file counts as present"
else
    fail "an existing socket file counts as present"
end
functions -c __gpy_ipc_endpoint __t_ipc_endpoint_real
function __gpy_ipc_endpoint
    return 1
end
if __gpy_git_socket_present
    fail "no endpoint is no socket (test -S with no argument is true)"
else
    pass "no endpoint is no socket (test -S with no argument is true)"
end
mkdir -p "$tmp_dir/cold/.git"
cd "$tmp_dir/cold"
if segment_git_omit
    pass "a cold miss with no endpoint omits the segment"
else
    fail "a cold miss with no endpoint omits the segment"
end
functions -e __gpy_ipc_endpoint
functions -c __t_ipc_endpoint_real __gpy_ipc_endpoint
set -e GPY_AGENT_SOCKET_PATH

rm -rf "$tmp_dir"
test_finish "git segment background from cache tests"
