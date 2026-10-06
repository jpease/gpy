#!/usr/bin/env fish
# tests/fish/update_config_theme_preserve.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #311: scripts/update.fish's _update_fish_scripts
# unconditionally overwrote ~/.config/gpy/config.toml on every update (no
# backup), and used `rsync --delete` (or an equally destructive `rm -rf` +
# `cp -r` fallback) on config/themes/, silently deleting a user's
# customized config and any user-authored custom theme file.
#
# Runs the REAL _update_fish_scripts function against a sandboxed
# $HOME/$XDG_CONFIG_HOME, from the real repo root so the shipped
# config/themes and fish/* source trees are used unmodified. The function
# is extracted from a stripped copy of scripts/update.fish with the
# trailing unconditional `update_gpy $argv` invocation removed, so no
# network call / cargo install / agent restart ever happens.

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -g failed 0

function __gpy_test_fail --argument-names msg
    echo "❌ $msg"
    set -g failed 1
end

function __gpy_test_pass --argument-names msg
    echo "✅ $msg"
end

# The stripped copy lives alongside the original update.fish so its
# relative `(dirname (status filename))/../fish/core/util.fish` source
# path still resolves to the real repo's fish/core/util.fish.
set -l scratch_copy "$repo_root/scripts/.__update_test_scratch.fish"
sed '$d' "$repo_root/scripts/update.fish" >"$scratch_copy"

function __gpy_run_update_fish_scripts --argument-names home_dir scratch_copy
    env HOME=$home_dir XDG_CONFIG_HOME="$home_dir/.config" fish -c '
        source $argv[1]
        _update_fish_scripts
    ' -- $scratch_copy 2>&1
end

# ===========================================================================
# Test 1: an existing customized config.toml is left untouched, and a
# config.toml.new sibling with the shipped default appears instead.
# ===========================================================================
set -l home1 (mktemp -d)
mkdir -p "$home1/.config/gpy"
set -l custom_config "$home1/.config/gpy/config.toml"
printf '%s\n' "# my custom config" "[custom]" "value = true" >"$custom_config"
cp "$custom_config" "$home1/original_config.toml"

set -l out1 (__gpy_run_update_fish_scripts "$home1" "$scratch_copy")
set -l status1 $status

if test $status1 -ne 0
    __gpy_test_fail "_update_fish_scripts exited $status1: $out1"
else
    if diff -q "$home1/original_config.toml" "$custom_config" >/dev/null 2>&1
        __gpy_test_pass "update.fish: customized config.toml left untouched"
    else
        __gpy_test_fail "update.fish: customized config.toml was modified"
    end

    if test -f "$home1/.config/gpy/config.toml.new"
        if diff -q "$repo_root/config/config.toml" "$home1/.config/gpy/config.toml.new" >/dev/null 2>&1
            __gpy_test_pass "update.fish: config.toml.new matches shipped default"
        else
            __gpy_test_fail "update.fish: config.toml.new does not match shipped default"
        end
    else
        __gpy_test_fail "update.fish: config.toml.new was not created for a divergent config"
    end
end

# ===========================================================================
# Test 2: idempotency -- running _update_fish_scripts twice in a row does
# not misbehave (no duplicate/corrupted .new, customized file still intact).
# ===========================================================================
__gpy_run_update_fish_scripts "$home1" "$scratch_copy" >/dev/null 2>&1
if test -f "$home1/.config/gpy/config.toml.new"
    if diff -q "$repo_root/config/config.toml" "$home1/.config/gpy/config.toml.new" >/dev/null 2>&1
        __gpy_test_pass "update.fish: repeat run keeps a single correct config.toml.new"
    else
        __gpy_test_fail "update.fish: repeat run corrupted config.toml.new"
    end
else
    __gpy_test_fail "update.fish: repeat run removed config.toml.new"
end
if diff -q "$home1/original_config.toml" "$custom_config" >/dev/null 2>&1
    __gpy_test_pass "update.fish: repeat run still leaves customized config.toml untouched"
else
    __gpy_test_fail "update.fish: repeat run modified customized config.toml"
end

rm -rf "$home1"

# ===========================================================================
# Test 3: a user-created theme file survives the themes sync, and shipped
# themes are still copied/updated correctly.
# ===========================================================================
set -l home2 (mktemp -d)
mkdir -p "$home2/.config/gpy/themes"
set -l custom_theme_path "$home2/.config/gpy/themes/my-custom-theme.toml"
printf '%s\n' "# my custom theme" "[colors]" 'foo = "bar"' >"$custom_theme_path"
cp "$custom_theme_path" "$home2/original_custom_theme.toml"

set -l out2 (__gpy_run_update_fish_scripts "$home2" "$scratch_copy")
set -l status2 $status

if test $status2 -ne 0
    __gpy_test_fail "_update_fish_scripts exited $status2: $out2"
else
    if test -f "$custom_theme_path"
        if diff -q "$home2/original_custom_theme.toml" "$custom_theme_path" >/dev/null 2>&1
            __gpy_test_pass "update.fish: custom theme file survives sync"
        else
            __gpy_test_fail "update.fish: custom theme file content changed"
        end
    else
        __gpy_test_fail "update.fish: custom theme file was deleted"
    end

    set -l all_shipped_present 1
    for f in "$repo_root"/config/themes/*
        set -l base (path basename $f)
        if not diff -q "$f" "$home2/.config/gpy/themes/$base" >/dev/null 2>&1
            set all_shipped_present 0
            __gpy_test_fail "update.fish: shipped theme $base missing or stale in destination"
        end
    end
    if test $all_shipped_present -eq 1
        __gpy_test_pass "update.fish: shipped themes still copied/updated correctly"
    end
end

rm -rf "$home2"

# ===========================================================================
# Test 4: first-ever install (no existing config.toml) still creates
# config.toml with the shipped default, and does not create a spurious .new.
# ===========================================================================
set -l home3 (mktemp -d)

set -l out3 (__gpy_run_update_fish_scripts "$home3" "$scratch_copy")
set -l status3 $status

if test $status3 -ne 0
    __gpy_test_fail "_update_fish_scripts exited $status3: $out3"
else
    if diff -q "$repo_root/config/config.toml" "$home3/.config/gpy/config.toml" >/dev/null 2>&1
        __gpy_test_pass "update.fish: first install creates config.toml matching shipped default"
    else
        __gpy_test_fail "update.fish: first install config.toml does not match shipped default"
    end
    if test -e "$home3/.config/gpy/config.toml.new"
        __gpy_test_fail "update.fish: first install unexpectedly created config.toml.new"
    else
        __gpy_test_pass "update.fish: first install does not create a spurious config.toml.new"
    end
end

rm -rf "$home3"
rm -f "$scratch_copy"

if test $failed -eq 1
    exit 1
end

echo "✅ update.fish preserves customized config.toml and custom theme files"
