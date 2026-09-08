#!/usr/bin/env fish
# tests/fish/main.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later

# Basic integration tests for GPY prompt system

echo "Testing GPY core functionality..."

# Isolate the theme-export cache dir so init.fish's fast path can't read a
# real, live dogfooding daemon's cache (#402).
set -gx XDG_CACHE_HOME (mktemp -d)

# Test 1: Init script loads
source fish/core/init.fish
if test $status -eq 0
    echo "✅ Core init script loads without errors"
else
    echo "❌ Core init script failed to load"
    exit 1
end

# Test 2: the config-path resolver returns a valid path structure.
#
# Was `gpy_config_path`, which named a `config.json` runtime snapshot nothing
# in the agent ever wrote; #626 deleted it. `__gpy_locate_config_path` is the
# resolver the prompt actually uses, and its candidate list mirrors
# `config::schema::get_config_paths` exactly.
set config_path (__gpy_locate_config_path)
if test -n "$config_path"
    echo "✅ __gpy_locate_config_path returns non-empty path: $config_path"
else
    echo "❌ __gpy_locate_config_path returned empty path"
    exit 1
end

# Every candidate but the last is absolute; the last is the project-local
# `.gpy.toml`, deliberately relative to the current directory.
if string match -q "/*" -- $config_path; or test "$config_path" = .gpy.toml
    echo "✅ __gpy_locate_config_path returns an absolute path or the project-local override"
else
    echo "❌ __gpy_locate_config_path returned invalid path: $config_path"
    exit 1
end

# Test 3: __gpy_json_escape properly escapes strings
source fish/core/ipc.fish

# Test basic escaping
set escaped (__gpy_json_escape "test/path")
if test -n "$escaped"
    echo "✅ __gpy_json_escape returns non-empty for basic path"
else
    echo "❌ __gpy_json_escape failed"
    exit 1
end

# Test escaping special characters (quotes)
set escaped_quotes (__gpy_json_escape 'path/with"quotes')
if string match -q '*\\"*' -- $escaped_quotes
    echo "✅ __gpy_json_escape properly escapes quotes"
else
    echo "❌ __gpy_json_escape failed to escape quotes: $escaped_quotes"
    exit 1
end

# Test 4: Essential logging functions exist
if functions -q __gpy_log_error
    and functions -q __gpy_log_warn
    echo "✅ Essential logging functions are defined"
else
    echo "❌ Essential logging functions missing"
    exit 1
end

echo "🎉 All core tests passed!"
