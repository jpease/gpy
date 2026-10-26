#!/usr/bin/env fish
# GPY_IPC_TIMEOUT_MS, GPY_IPC_COLD_TIMEOUT_MS and the two instant-cache TTL knobs are honoured when set
# in the environment before the shell integration loads, as in Bash and (for
# the TTLs) Zsh: fish/core/constants.fish must not overwrite them with its
# defaults at source time, or exporting one in config.fish would do nothing.
#
# Each case runs a child fish with only the knob under test set, sources the
# constants the way init does, and reads the values back. A value that is not a
# non-negative integer falls back to the default instead of reaching `math`.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"

print_test_header "Env knobs are honoured"

# constants_with VAR=VALUE...: "ipc git lang" as the child sees them.
function constants_with
    env -u GPY_IPC_TIMEOUT_MS -u GPY_GIT_INSTANT_CACHE_TTL_SECONDS -u GPY_LANGUAGE_CACHE_TTL_SECONDS $argv \
        fish --no-config -c 'source $argv[1]/fish/core/constants.fish; echo $GPY_IPC_TIMEOUT_MS $GPY_GIT_INSTANT_CACHE_TTL_SECONDS $GPY_LANGUAGE_CACHE_TTL_SECONDS' -- $repo_root
end

check "defaults when nothing is set" "150 5 30" (constants_with)
check "GPY_IPC_TIMEOUT_MS is honoured" "750 5 30" (constants_with GPY_IPC_TIMEOUT_MS=750)
check "GPY_GIT_INSTANT_CACHE_TTL_SECONDS is honoured" "150 11 30" (constants_with GPY_GIT_INSTANT_CACHE_TTL_SECONDS=11)
check "GPY_LANGUAGE_CACHE_TTL_SECONDS is honoured" "150 5 22" (constants_with GPY_LANGUAGE_CACHE_TTL_SECONDS=22)
check "a non-numeric GPY_IPC_TIMEOUT_MS falls back to the default" "150 5 30" (constants_with GPY_IPC_TIMEOUT_MS=fast)
check "an empty TTL falls back to the default" "150 5 30" (constants_with GPY_GIT_INSTANT_CACHE_TTL_SECONDS=)

# The cold budget (nothing to show yet): 500 ms by default, honoured when set,
# never below GPY_IPC_TIMEOUT_MS, and the default when non-numeric.
function cold_with
    env -u GPY_IPC_TIMEOUT_MS -u GPY_IPC_COLD_TIMEOUT_MS $argv \
        fish --no-config -c 'source $argv[1]/fish/core/constants.fish; source $argv[1]/fish/core/util.fish; source $argv[1]/fish/core/ipc.fish; __gpy_ipc_cold_timeout_ms' -- $repo_root
end
check "the cold budget defaults to 500 ms" 500 (cold_with)
check "GPY_IPC_COLD_TIMEOUT_MS is honoured" 900 (cold_with GPY_IPC_COLD_TIMEOUT_MS=900)
check "the cold budget is never below GPY_IPC_TIMEOUT_MS" 2000 (cold_with GPY_IPC_TIMEOUT_MS=2000)
check "a non-numeric cold budget falls back to 500 ms" 500 (cold_with GPY_IPC_COLD_TIMEOUT_MS=slow)

test_finish "Env knobs are honoured"
