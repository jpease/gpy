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

test_finish "Fish empty-HOME fallback"
