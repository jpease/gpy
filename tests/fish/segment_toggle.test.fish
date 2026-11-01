#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# The segment list overrides init.fish still honours:
#   - __gpy_test_segments (non-exported test hook) replaces the list;
#   - GPY_SHOW_LANGUAGES=0, GPY_MINIMAL_SEGMENTS and GPY_TEST_SEGMENTS are
#     deprecated: each still applies and warns once at the first prompt.
# Every case runs in a fresh Fish in a sandbox whose gpy-agent is a stub that
# records its calls and fails, so no export loads and the fallback list is used.

source (dirname (status --filename))/../lib/test_helpers.fish

set -g t_fish (command -s fish)
set -g t_sandbox (mktemp -d)
test_track_dir $t_sandbox
mkdir -p $t_sandbox/home $t_sandbox/cache $t_sandbox/config $t_sandbox/run $t_sandbox/bin
set -g t_calls $t_sandbox/agent-calls.log
printf '%s\n' '#!/bin/sh' 'echo "$*" >>"$STUB_AGENT_LOG"' 'exit 1' >$t_sandbox/bin/gpy-agent
chmod +x $t_sandbox/bin/gpy-agent

# Source GPY in a fresh Fish and print `segments=<list>`, then fire the first
# prompt twice and print what reached stderr. $argv[1] runs before init.fish;
# the rest are extra environment entries.
function run_case --argument-names pre_code
    set -l extra
    set -q argv[2]; and set extra $argv[2..-1]
    command env -i HOME=$t_sandbox/home XDG_CONFIG_HOME=$t_sandbox/config XDG_CACHE_HOME=$t_sandbox/cache \
        XDG_RUNTIME_DIR=$t_sandbox/run TMPDIR=$t_sandbox PATH=$t_sandbox/bin:/usr/bin:/bin TERM=dumb \
        GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_AUTOSTART_MAX_ATTEMPTS=0 \
        GPY_AGENT_SOCKET_PATH=$t_sandbox/run/gpy.sock STUB_AGENT_LOG=$t_calls GPY_T_ROOT=$__gpy_root \
        $extra $t_fish --no-config -c "$pre_code
source \$GPY_T_ROOT/fish/core/init.fish
echo segments=(string join , \$__enabled_segments)
emit fish_prompt
emit fish_prompt" 2>&1
end

function warnings_in
    string match '*is deprecated; use gpy enable or gpy disable instead' -- $argv
end

set -l default_list clock,duration,language,directory,git

echo "--- fallback list, no switch ---"
set -l out (run_case "")
check "no export: the fallback list is used" segments=$default_list "$out[1]"
check "no deprecation warning without a switch" 0 (count (warnings_in $out))
if grep -q 'theme export' $t_calls
    pass "the stub gpy-agent was asked for the export (and failed)"
else
    fail "the stub gpy-agent was never asked for a theme export"
end

echo "--- __gpy_test_segments ---"
set out (run_case "set -g __gpy_test_segments clock git")
check "__gpy_test_segments replaces the list" segments=clock,git "$out[1]"
check "__gpy_test_segments does not warn" 0 (count (warnings_in $out))

echo "--- deprecated switches ---"
set out (run_case "" GPY_MINIMAL_SEGMENTS=1)
check "GPY_MINIMAL_SEGMENTS still forces directory" segments=directory "$out[1]"
check "GPY_MINIMAL_SEGMENTS warns once across two prompts" \
    "gpy: GPY_MINIMAL_SEGMENTS is deprecated; use gpy enable or gpy disable instead" (warnings_in $out)

set out (run_case "" GPY_MINIMAL_SEGMENTS=)
check "an empty GPY_MINIMAL_SEGMENTS still forces directory" segments=directory "$out[1]"
check "an empty GPY_MINIMAL_SEGMENTS warns once" 1 (count (warnings_in $out))

set out (run_case "" "GPY_TEST_SEGMENTS=clock git")
check "GPY_TEST_SEGMENTS still replaces the list" segments=clock,git "$out[1]"
check "GPY_TEST_SEGMENTS warns once across two prompts" \
    "gpy: GPY_TEST_SEGMENTS is deprecated; use gpy enable or gpy disable instead" (warnings_in $out)

set out (run_case "" GPY_SHOW_LANGUAGES=0)
check "GPY_SHOW_LANGUAGES=0 still drops language" segments=clock,duration,directory,git "$out[1]"
check "GPY_SHOW_LANGUAGES=0 warns once across two prompts" \
    "gpy: GPY_SHOW_LANGUAGES is deprecated; use gpy enable or gpy disable instead" (warnings_in $out)

set out (run_case "" GPY_SHOW_LANGUAGES=1)
check "GPY_SHOW_LANGUAGES=1 changes nothing" segments=$default_list "$out[1]"
check "GPY_SHOW_LANGUAGES=1 does not warn" 0 (count (warnings_in $out))

set out (run_case "" GPY_SHOW_LANGUAGES=0 GPY_MINIMAL_SEGMENTS=1)
check "GPY_MINIMAL_SEGMENTS wins over GPY_SHOW_LANGUAGES" segments=directory "$out[1]"
check "each deprecated name present warns once" 2 (count (warnings_in $out))

test_finish "segment toggle tests"
