#!/usr/bin/env fish
# tests/fish/supervisor_deprecated_knobs.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The four Fish-only supervisor/autostart tunables are gone. Setting one in the
# environment prints one stderr line per shell, on the first prompt, and the
# value changes nothing.

set -l script_dir (path dirname (status --current-filename))
set -g repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/core/supervisor.fish"

print_test_header "Deprecated supervisor knobs warn once"

set -g tmp (mktemp -d)
test_track_dir $tmp
set -gx XDG_CACHE_HOME $tmp/cache
set -gx GPY_AGENT_SOCKET_PATH $tmp/agent.sock
set -gx GPY_AGENT_ENABLED 1

# Nothing below may reach a real agent or process.
function __gpy_retire_legacy_supervisor; end
function __gpy_ensure_agent_started; return 0; end
function __gpy_register_with_agent; return 0; end
function __gpy_supervisor_check; return 1; end

function __warnings_for_two_prompts
    set -e __gpy_supervisor_initialized
    __gpy_supervisor_on_prompt 2>$tmp/err
    __gpy_supervisor_on_prompt 2>>$tmp/err
    cat $tmp/err
end

set -l out (__warnings_for_two_prompts)
check "no deprecated name set prints nothing" 0 (count $out)

set -gx GPY_AGENT_AUTOSTART_MAX_ATTEMPTS 3
set out (__warnings_for_two_prompts)
check "one deprecated name prints one line over two prompts" 1 (count $out)
check "the line names the variable and the replacement" \
    "gpy: GPY_AGENT_AUTOSTART_MAX_ATTEMPTS is deprecated and has no effect; use [agent.supervisor] in config.toml" "$out[1]"

set -gx GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS 10
set -gx GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS 60
set -gx GPY_SUPERVISOR_LONG_BACKOFF_SECONDS 300
set out (__warnings_for_two_prompts)
check "each of the four names warns once" 4 (count $out)

test_finish "Deprecated supervisor knobs warn once"
