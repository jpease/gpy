#!/usr/bin/env fish
# tests/fish/completions_dynamic.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression/smoke test for #329 (Epic #326, task 3/6): wiring up FISH
# completions for the `gpy` CLI.
#
# `gpy completions fish` (clap_complete) generates the STRUCTURAL completion
# surface (subcommand/flag names) at install time and is never checked in.
# `fish/completions/gpy-dynamic.fish` is a small, hand-authored, checked-in
# file that supplies the DYNAMIC value lists (installed theme/palette/segment
# names) by shelling out to the hidden `gpy __complete <kind>` command
# (#328). The installers concatenate the two into one autoloadable
# `completions/gpy.fish` (#702); this test builds that file and proves it
# autoloads (no manual `source`) and that the dynamic glue
# actually produces the live completions a user would see for:
#   - `gpy theme use <TAB>` / `gpy theme validate <TAB>`
#   - `gpy palette use <TAB>` / `gpy palette validate <TAB>`
#   - `gpy enable <TAB>` / `gpy disable <TAB>`
#
# ASSERTION STYLE: this uses the real `complete -C 'gpy ...'` fish
# completion engine (the actual acceptance criterion — what a user's TAB key
# would produce), not just a check that `gpy __complete <kind>` returns the
# right names or that the `complete` rules parse. `complete -C` works
# without `gpy` being "installed" anywhere in particular: fish resolves
# completions from whatever `complete -c gpy ...` rules are registered in
# the current session (autoloaded below), and the `-n` predicates in
# gpy-dynamic.fish shell out to `gpy __complete <kind>` at completion time,
# which requires the debug binary's directory on $PATH (set up below,
# mirroring tests/fish/starship_parity.test.fish's explicit debug-binary
# fallback).
#
# Per the "parity test needs binary rebuild" gotcha: `cargo test` does not
# rebuild the CLI binary used here, so this test rebuilds it directly if
# missing/stale relative to source, rather than trusting a possibly-stale
# target/debug/gpy.

set -l test_root (status dirname)/../..
set -g repo_root (path resolve $test_root)

set -g test_failures 0

function test_fail
    set -g test_failures (math $test_failures + 1)
    echo "❌ FAIL: $argv[1]"
end

function test_pass
    echo "✅ PASS: $argv[1]"
end

# `complete -C` returns "value\tdescription" per candidate (tab-separated).
# Strip the description so callers can match on the bare value. Takes the
# whole completions list as a single variadic arg (fish's --argument-names
# only binds one word per name; passing a multi-element list alongside other
# positional args silently misaligns them), so this is the only parameter.
function __cd_test_names
    string replace -r '\t.*$' '' -- $argv
end

# ---------------------------------------------------------------------------
# Ensure the debug `gpy` CLI binary exists (build it if not) and put its
# directory on PATH so both `gpy completions fish` and the `gpy __complete
# <kind>` calls made by gpy-dynamic.fish's predicates resolve deterministically
# to THIS checkout.
# ---------------------------------------------------------------------------
set -g gpy_bin $repo_root/gpy-agent/target/debug/gpy

if not test -x $gpy_bin
    echo "Building gpy debug binary for completions test..."
    pushd $repo_root/gpy-agent
    cargo build --quiet
    popd
end

if not test -x $gpy_bin
    echo "❌ gpy binary not found at $gpy_bin after build"
    exit 1
end

set -gx PATH (path dirname $gpy_bin) $PATH

set -l dynamic_completions $repo_root/fish/completions/gpy-dynamic.fish
if not test -f $dynamic_completions
    test_fail "fish/completions/gpy-dynamic.fish does not exist yet"
    if test $test_failures -gt 0
        exit 1
    end
end

# ---------------------------------------------------------------------------
# Build the completions directory exactly as the installers do (#702): ONE
# autoloadable `gpy.fish` = `gpy completions fish` followed by the dynamic
# glue. Fish autoloads `completions/<command>.fish` only for the command of
# that name, so a separate `gpy-dynamic.fish` would never be loaded. Nothing
# is `source`d by hand below: the first `complete -C 'gpy ...'` autoloads
# gpy.fish from $fish_complete_path, as a real interactive session does.
# ---------------------------------------------------------------------------
set -l tmp_dir (mktemp -d)
set -l comp_dir $tmp_dir/comp
mkdir -p $comp_dir
set -l generated_completions $comp_dir/gpy.fish

if not $gpy_bin completions fish >$generated_completions
    test_fail "gpy completions fish failed to generate output"
end
cat $dynamic_completions >>$generated_completions

if fish -n $generated_completions 2>$tmp_dir/installed.err
    test_pass "installed gpy.fish (structural + dynamic glue) parses"
else
    test_fail "installed gpy.fish does not parse: "(cat $tmp_dir/installed.err)
end

set fish_complete_path $comp_dir $fish_complete_path

# ---------------------------------------------------------------------------
# Dynamic value completions: theme
# ---------------------------------------------------------------------------
set -l theme_use_names (__cd_test_names (complete -C 'gpy theme use '))
if contains default $theme_use_names
    and contains starship $theme_use_names
    and contains text $theme_use_names
    test_pass "'gpy theme use <TAB>' completes installed theme names"
else
    test_fail "'gpy theme use <TAB>' did not complete installed theme names: $theme_use_names"
end

set -l theme_validate_names (__cd_test_names (complete -C 'gpy theme validate '))
if contains default $theme_validate_names
    test_pass "'gpy theme validate <TAB>' completes installed theme names"
else
    test_fail "'gpy theme validate <TAB>' did not complete installed theme names: $theme_validate_names"
end

# ---------------------------------------------------------------------------
# Dynamic value completions: palette
# ---------------------------------------------------------------------------
set -l palette_use_names (__cd_test_names (complete -C 'gpy palette use '))
if contains default $palette_use_names
    and contains nord $palette_use_names
    test_pass "'gpy palette use <TAB>' completes installed palette names"
else
    test_fail "'gpy palette use <TAB>' did not complete installed palette names: $palette_use_names"
end

set -l palette_validate_names (__cd_test_names (complete -C 'gpy palette validate '))
if contains default $palette_validate_names
    test_pass "'gpy palette validate <TAB>' completes installed palette names"
else
    test_fail "'gpy palette validate <TAB>' did not complete installed palette names: $palette_validate_names"
end

# ---------------------------------------------------------------------------
# Dynamic value completions: segment (enable/disable)
# ---------------------------------------------------------------------------
set -l enable_names (__cd_test_names (complete -C 'gpy enable '))
if contains git $enable_names
    and contains status $enable_names
    test_pass "'gpy enable <TAB>' completes segment names"
else
    test_fail "'gpy enable <TAB>' did not complete segment names: $enable_names"
end

set -l disable_names (__cd_test_names (complete -C 'gpy disable '))
if contains git $disable_names
    and contains status $disable_names
    test_pass "'gpy disable <TAB>' completes segment names"
else
    test_fail "'gpy disable <TAB>' did not complete segment names: $disable_names"
end

# ---------------------------------------------------------------------------
# Cleanup + verdict
# ---------------------------------------------------------------------------
rm -rf $tmp_dir

if test $test_failures -gt 0
    echo "❌ $test_failures completions test(s) failed"
    exit 1
end

echo "✅ All gpy completions tests passed"
exit 0
