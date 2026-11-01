#!/usr/bin/env fish
# tests/fish/main.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later

# Basic integration tests for GPY prompt system

source (dirname (status --filename))/../lib/test_helpers.fish

echo "Testing GPY core functionality..."

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache.
set -gx XDG_CACHE_HOME (mktemp -d)

# Test 1: Init script loads
source fish/core/init.fish
if test $status -eq 0
    pass "Core init script loads without errors"
else
    fail "Core init script failed to load"
    exit 1
end

# Test 3: __gpy_json_escape properly escapes strings
source fish/core/ipc.fish

# Test basic escaping
set escaped (__gpy_json_escape "test/path")
if test -n "$escaped"
    pass "__gpy_json_escape returns non-empty for basic path"
else
    fail "__gpy_json_escape failed"
    exit 1
end

# Test escaping special characters (quotes)
set escaped_quotes (__gpy_json_escape 'path/with"quotes')
if string match -q '*\\"*' -- $escaped_quotes
    pass "__gpy_json_escape properly escapes quotes"
else
    fail "__gpy_json_escape failed to escape quotes: $escaped_quotes"
    exit 1
end

test_finish "core tests"
