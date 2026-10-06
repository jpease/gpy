#!/usr/bin/env fish
# Test (#785): segment_language_detect iterates __gpy_lang_marker_files, the
# list the agent's `theme export` generates from marker_file_names(). Sources
# a REAL export from the debug binary (in a temp XDG) and asserts the
# pre-filter accepts a non-git dir holding each marker (incl. setup.py,
# Package.swift, Rakefile), rejects an empty dir, and defers (returns 0) when
# the export has never been sourced.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/segments/language.fish"

set -l agent_bin "$repo_root/gpy-agent/target/debug/gpy-agent"
if not test -x "$agent_bin"
    echo "SKIP: $agent_bin not built (run `cargo build` in gpy-agent first)"
    test -n "$CI"; and exit 1
    exit 0
end

set -g pass_count 0
set -g fail_count 0

function assert_detect --argument-names label expected_status dir
    set -l old_pwd $PWD
    cd $dir
    segment_language_detect
    set -l got $status
    cd $old_pwd
    if test $got -eq $expected_status
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label (expected $expected_status, got $got)"
    end
end

set -l tmp (mktemp -d)

# Unset export: defer to the request path, never a hand-kept list.
set -e __gpy_lang_marker_files
mkdir "$tmp/plain"
assert_detect "unset export defers (returns 0)" 0 "$tmp/plain"

env HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/cfg" XDG_CACHE_HOME="$tmp/cache" \
    XDG_RUNTIME_DIR="$tmp/run" "$agent_bin" theme export --format fish >"$tmp/export.fish" 2>/dev/null
source "$tmp/export.fish"

if not set -q __gpy_lang_marker_files[1]
    echo "FAIL: export defines no __gpy_lang_marker_files"
    rm -rf "$tmp"
    exit 1
end

assert_detect "empty dir rejected" 1 "$tmp/plain"

for marker in $__gpy_lang_marker_files setup.py Package.swift Rakefile
    set -l dir "$tmp/m_$marker"
    mkdir -p "$dir"
    touch "$dir/$marker"
    assert_detect "marker $marker accepted" 0 "$dir"
end

mkdir -p "$tmp/m_setup.py/sub/deep"
assert_detect "marker found from a subdir" 0 "$tmp/m_setup.py/sub/deep"

rm -rf "$tmp"

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count language_marker_prefilter tests passed"
