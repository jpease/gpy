#!/usr/bin/env fish
# An empty or erased $HOME resolves the passwd home, exactly as the agent,
# Bash and Zsh do (#845). Fish re-derives HOME at startup, but a session that
# later blanks it (`set -gx HOME ''`, a wrapper, a test harness) used to get
# `/.cache/gpy` from __gpy_runtime_root while the agent used the passwd home,
# so the prompt watched a socket nobody had bound.
#
# The cross-shell path_parity matrix covers the other path keys with an empty
# HOME; its XDG_RUNTIME_DIR is always set so it never creates the real
# runtime directory. This test covers the runtime-root fallback itself, which
# only resolves from HOME when no XDG_* variable is set.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"

print_test_header "Fish empty-HOME fallback (#845)"

set -g failures 0
function check --argument-names label ok detail
    if test "$ok" = 1
        print_test_result "$label" PASS
    else
        print_test_result "$label" FAIL "$detail"
        set -g failures (math $failures + 1)
    end
end

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
    check "HOME $state: the runtime root is the passwd home's cache" (test (resolve $state __gpy_runtime_root) = "$pwhome/.cache/gpy"; and echo 1; or echo 0) "got "(resolve $state __gpy_runtime_root)
    check "HOME $state: the socket is under it" (test (resolve $state __gpy_ipc_endpoint) = "$pwhome/.cache/gpy/gpy.sock"; and echo 1; or echo 0) "got "(resolve $state __gpy_ipc_endpoint)
    check "HOME $state: the instant-cache dir is under it" (test (resolve $state __gpy_instant_cache_dir) = "$pwhome/.cache/gpy/instant-prompts"; and echo 1; or echo 0) "got "(resolve $state __gpy_instant_cache_dir)
end

if test $failures -gt 0
    print_test_footer "Fish empty-HOME fallback" FAIL
    exit 1
end
print_test_footer "Fish empty-HOME fallback" PASS
