#!/usr/bin/env fish
# An empty or erased $HOME resolves the passwd home, exactly as the agent,
# Bash and Zsh do. Fish re-derives HOME at startup, but a session can later
# blank it (`set -gx HOME ''`, a wrapper, a test harness); resolving
# `/.cache/gpy` from __gpy_runtime_root while the agent uses the passwd home
# would leave the prompt watching a socket nobody has bound.
#
# The cross-shell path_parity matrix covers the other path keys with an empty
# HOME; its XDG_RUNTIME_DIR is always set so it never creates the real
# runtime directory. This test covers the runtime-root fallback itself, which
# only resolves from HOME when no XDG_* variable is set.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"

print_test_header "Fish empty-HOME fallback"

set -l pwhome (env -u HOME fish --no-config -c 'echo $HOME')
if test -z "$pwhome"
    test_skip "cannot determine the passwd home"
end

# resolve STATE FUNCTION: FUNCTION's answer with HOME blanked or erased
# and every XDG_* variable removed.
function resolve --argument-names state fn
    env -u XDG_RUNTIME_DIR -u XDG_CACHE_HOME -u GPY_AGENT_SOCKET_PATH fish --no-config -c '
        switch $argv[2]
            case empty
                set -gx HOME ""
            case unset
                set -e HOME
        end
        source $argv[1]/fish/core/ipc.fish
        $argv[3]' -- $repo_root $state $fn
end

for state in empty unset
    check "HOME $state: the runtime root is the passwd home's cache" "$pwhome/.cache/gpy" (resolve $state __gpy_runtime_root)
    check "HOME $state: the socket is under it" "$pwhome/.cache/gpy/gpy.sock" (resolve $state __gpy_ipc_endpoint)
    check "HOME $state: the instant-cache dir is under it" "$pwhome/.cache/gpy/instant-prompts" (resolve $state __gpy_instant_cache_dir)
end

# A relative HOME is no home at all, and with no usable XDG_* either there is
# no runtime root: GPY stays off for the session (no /tmp/gpy fallback), with
# at most one debug line under GPY_DEBUG=1.
set -l homeless_cwd (mktemp -d)
test_track_dir $homeless_cwd
for debug in 0 1
    set -l err $homeless_cwd.err
    set -l out (env -i PATH="$PATH" HOME=rel/home XDG_RUNTIME_DIR=run XDG_CACHE_HOME= GPY_DEBUG=$debug fish --no-config -i -c '
        cd $argv[2]
        source $argv[1]/fish/conf.d/gpy_init.fish
        functions -q __gpy_request_character; and echo loaded; or echo off
        set -q __gpy_load_failed; and echo agent-off
        __gpy_agent_enabled; and echo agent-on' $repo_root $homeless_cwd </dev/null 2>$err)
    check "GPY_DEBUG=$debug: nothing usable leaves GPY off" "off agent-off" "$out"
    set -l want_lines 0
    test $debug = 1; and set want_lines 1
    check "GPY_DEBUG=$debug: stderr carries $want_lines line(s)" $want_lines (count (cat $err))
    rm -f $err
end
check "nothing usable: nothing is created under the working directory" "" (ls -A $homeless_cwd | string collect)

test_finish "Fish empty-HOME fallback"
