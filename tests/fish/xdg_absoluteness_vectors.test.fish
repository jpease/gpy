#!/usr/bin/env fish
# tests/fish/xdg_absoluteness_vectors.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #774: Fish's resolvers honour an XDG_* value iff it is absolute (starts with
# `/`) for every row of tests/fixtures/xdg_absoluteness_vectors.tsv (also read
# by the Rust, Bash and Zsh tests). See the fixture's header for the format.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/ipc.fish
source fish/core/util.fish

set -g pass_count 0
set -g fail_count 0

function check --argument-names label expected actual
    if test "$actual" = "$expected"
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label (expected [$expected], got [$actual])"
    end
end

# Undo the fixture's mini escape scheme (see its header): `\e` is the empty
# string, `\\` one backslash.
function unescape_value --argument-names raw
    if test "$raw" = '\e'
        return
    end
    string replace -a -- '\\\\' '\\' "$raw"
end

set -l fixture "$repo_root/tests/fixtures/xdg_absoluteness_vectors.tsv"
if not test -r "$fixture"
    echo "FAIL: cannot read $fixture"
    exit 1
end
set -l vector_count 0
set -gx HOME /home/fixture-home
set -e GPY_AGENT_SOCKET_PATH

while read -d \t -la cols
    test (count $cols) -eq 0; and continue
    set -l first_char (string sub -l 1 -- "$cols[1]")
    test "$first_char" = '#'; and continue
    test (count $cols) -lt 2; and continue

    set vector_count (math $vector_count + 1)
    set -l value_col $cols[1]
    set -l value (unescape_value "$value_col" | string collect -a)
    if not contains -- $cols[2] yes no
        echo "FAIL: vector $vector_count ($value_col): bad absolute column [$cols[2]]"
        set fail_count (math $fail_count + 1)
        continue
    end
    set -l expected_root "$HOME/.cache/gpy"
    test "$cols[2]" = yes; and set expected_root "$value/gpy"

    set -gx XDG_RUNTIME_DIR "$value"
    set -e XDG_CACHE_HOME
    check "vector $vector_count ($value_col) runtime root" "$expected_root" (__gpy_runtime_root)
    check "vector $vector_count ($value_col) ipc endpoint" "$expected_root/gpy.sock" (__gpy_ipc_endpoint)

    set -e XDG_RUNTIME_DIR
    set -gx XDG_CACHE_HOME "$value"
    check "vector $vector_count ($value_col) instant cache dir" "$expected_root/instant-prompts" (__gpy_instant_cache_dir)
    check "vector $vector_count ($value_col) theme export path" "$expected_root/theme-export.fish" (__gpy_theme_export_cache_path)
end <"$fixture"

if test $vector_count -eq 0
    echo "FAIL: no test vectors read from $fixture"
    exit 1
end

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count XDG absoluteness vector tests passed"
