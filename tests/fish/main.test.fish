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

# Test 2: the config-path resolver returns a valid path structure.
#
# `__gpy_locate_config_path` is the resolver the prompt actually uses, and its
# candidate list mirrors `config::schema::get_config_paths` exactly.
set config_path (__gpy_locate_config_path)
if test -n "$config_path"
    pass "__gpy_locate_config_path returns non-empty path: $config_path"
else
    fail "__gpy_locate_config_path returned empty path"
    exit 1
end

# Every candidate is absolute: there is no project-local `.gpy.toml`,
# and a relative GPY_CONFIG_PATH is resolved against the current directory.
if string match -q "/*" -- $config_path
    pass "__gpy_locate_config_path returns an absolute path"
else
    fail "__gpy_locate_config_path returned a non-absolute path: $config_path"
    exit 1
end

for candidate in (__gpy_user_config_candidates)
    if not string match -q "/*" -- $candidate
        fail "__gpy_user_config_candidates offered a non-absolute candidate: $candidate"
        exit 1
    end
end
pass "every config candidate is absolute"

set -l relative_candidates (GPY_CONFIG_PATH=rel/custom.toml __gpy_user_config_candidates)
set -l relative_path $relative_candidates[1]
if test "$relative_path" = (pwd -P)/rel/custom.toml
    pass "a relative GPY_CONFIG_PATH resolves against the current directory"
else
    fail "relative GPY_CONFIG_PATH resolved to: $relative_path"
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
