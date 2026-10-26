#!/usr/bin/env fish
# tests/fish/prompt_dispatch_positions.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# fish_prompt's segment dispatch loop must pass every segment its
# is_last/is_first flags in the correct positional slots. Declaring them as
# `set -l is_last` with no value creates an EMPTY LIST (zero elements), not an
# empty string; Fish expands an empty list to zero arguments, so in
#
#   segment_{$segment}_render $is_last $is_first
#
# a first-but-not-last segment would silently drop the empty `is_last` slot
# and receive `is_first`'s value in the `is_last` position. This test drives
# the real fish_prompt dispatch loop (fish/functions/fish_prompt.fish, sourced
# after fish/core/init.fish exactly as other prompt-rendering tests do) with
# three stub segments and asserts each one receives is_last/is_first in the
# correct positions.

# Find repo root (mirrors other tests/fish/*.test.fish)
source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

# Isolate environment from user/dogfooding configuration (mirrors
# segments.test.fish / fresh_install.test.fish).
set -l saved_home $HOME
set -l saved_xdg_config $XDG_CONFIG_HOME
set -l temp_home (mktemp -d)
set -gx HOME $temp_home
set -gx XDG_CONFIG_HOME $temp_home/.config
mkdir -p $XDG_CONFIG_HOME/gpy

# Agent enabled but the supervisor disabled: exercises the full init path
# without spawning a persistent daemon, and fish_prompt degrades gracefully
# when no agent is reachable.
# Setting BOTH GPY_AGENT_ENABLED and GPY_AGENT_SUPERVISOR_ENABLED to 0 would
# trip init.fish's "completely disabled" early-exit and skip segment loading
# entirely, so only the supervisor is disabled here.
set -gx GPY_AGENT_ENABLED 1
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0

# Three always-on stub segments that record the raw positional arguments
# fish_prompt's dispatch loop actually calls them with.
set -g __stub_calls

function segment_stuba_detect
    return 0
end
function segment_stuba_render --argument-names is_last is_first
    set -g __stub_calls $__stub_calls "stuba:$is_last|$is_first"
end

function segment_stubb_detect
    return 0
end
function segment_stubb_render --argument-names is_last is_first
    set -g __stub_calls $__stub_calls "stubb:$is_last|$is_first"
end

function segment_stubc_detect
    return 0
end
function segment_stubc_render --argument-names is_last is_first
    set -g __stub_calls $__stub_calls "stubc:$is_last|$is_first"
end

# Force fish_prompt to render exactly these three stub segments, in order:
# first, middle, last. init.fish applies GPY_TEST_SEGMENTS as an override to
# __enabled_segments before loading segments (fish/core/init.fish).
set -gx GPY_TEST_SEGMENTS "stuba stubb stubc"

source fish/core/init.fish
# fish_prompt lives in an autoloaded function file, not init.fish -- source it
# explicitly, as other prompt-rendering tests do, instead of relying on the
# developer's installed copy.
source fish/functions/fish_prompt.fish

fish_prompt >/dev/null

if test (count $__stub_calls) -eq 3
    check "dispatch loop invoked all three stub segments" pass pass
else
    check "dispatch loop invoked all three stub segments (got: $__stub_calls)" pass fail
end

# The dispatch loop's is_last/is_first convention is "true" for set and ""
# for unset.
check "first segment gets is_first set, is_last empty" pass (test "$__stub_calls[1]" = "stuba:|true"; and echo pass; or echo fail)
check "middle segment gets both empty" pass (test "$__stub_calls[2]" = "stubb:|"; and echo pass; or echo fail)
check "last segment gets is_last set, is_first empty" pass (test "$__stub_calls[3]" = "stubc:true|"; and echo pass; or echo fail)

if test "$__stub_calls[1]" != "stuba:|true"
    echo "  first segment got: $__stub_calls[1] (expected stuba:|true)"
end
if test "$__stub_calls[3]" != "stubc:true|"
    echo "  last segment got: $__stub_calls[3] (expected stubc:true|)"
end

# Cleanup
rm -rf $temp_home
set -gx HOME $saved_home
if test -n "$saved_xdg_config"
    set -gx XDG_CONFIG_HOME $saved_xdg_config
else
    set -e XDG_CONFIG_HOME
end

test_finish "prompt dispatch position tests"
