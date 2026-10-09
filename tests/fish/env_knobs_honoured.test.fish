#!/usr/bin/env fish
# GPY_IPC_TIMEOUT_MS and the two instant-cache TTL knobs are honoured when set
# in the environment before the shell integration loads, as they already were
# in Bash and (for the TTLs) Zsh (#845). fish/core/constants.fish used to
# overwrite all three with its defaults at source time, so exporting one in
# config.fish did nothing.
#
# Each case runs a child fish with only the knob under test set, sources the
# constants the way init does, and reads the values back. A value that is not a
# non-negative integer falls back to the default instead of reaching `math`.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"

print_test_header "Env knobs are honoured (#845)"

set -g failures 0
function check --argument-names label ok detail
    if test "$ok" = 1
        print_test_result "$label" PASS
    else
        print_test_result "$label" FAIL "$detail"
        set -g failures (math $failures + 1)
    end
end

# constants_with VAR=VALUE...: "ipc git lang" as the child sees them.
function constants_with
    env -u GPY_IPC_TIMEOUT_MS -u GPY_GIT_INSTANT_CACHE_TTL_SECONDS -u GPY_LANGUAGE_CACHE_TTL_SECONDS $argv \
        fish --no-config -c 'source $argv[1]/fish/core/constants.fish; echo $GPY_IPC_TIMEOUT_MS $GPY_GIT_INSTANT_CACHE_TTL_SECONDS $GPY_LANGUAGE_CACHE_TTL_SECONDS' -- $repo_root
end

check "defaults when nothing is set" (test (constants_with) = "150 5 30"; and echo 1; or echo 0) "got: "(constants_with)
check "GPY_IPC_TIMEOUT_MS is honoured" (test (constants_with GPY_IPC_TIMEOUT_MS=750) = "750 5 30"; and echo 1; or echo 0) "got: "(constants_with GPY_IPC_TIMEOUT_MS=750)
check "GPY_GIT_INSTANT_CACHE_TTL_SECONDS is honoured" (test (constants_with GPY_GIT_INSTANT_CACHE_TTL_SECONDS=11) = "150 11 30"; and echo 1; or echo 0) "got: "(constants_with GPY_GIT_INSTANT_CACHE_TTL_SECONDS=11)
check "GPY_LANGUAGE_CACHE_TTL_SECONDS is honoured" (test (constants_with GPY_LANGUAGE_CACHE_TTL_SECONDS=22) = "150 5 22"; and echo 1; or echo 0) "got: "(constants_with GPY_LANGUAGE_CACHE_TTL_SECONDS=22)
check "a non-numeric GPY_IPC_TIMEOUT_MS falls back to the default" (test (constants_with GPY_IPC_TIMEOUT_MS=fast) = "150 5 30"; and echo 1; or echo 0) "got: "(constants_with GPY_IPC_TIMEOUT_MS=fast)
check "an empty TTL falls back to the default" (test (constants_with GPY_GIT_INSTANT_CACHE_TTL_SECONDS=) = "150 5 30"; and echo 1; or echo 0) "got: "(constants_with GPY_GIT_INSTANT_CACHE_TTL_SECONDS=)

if test $failures -gt 0
    print_test_footer "Env knobs are honoured" FAIL
    exit 1
end
print_test_footer "Env knobs are honoured" PASS
