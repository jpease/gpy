#!/usr/bin/env fish
# tests/fish/install_dev_config_theme_preserve.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #311: install-dev.fish's _install_fish_files
# unconditionally overwrote ~/.config/gpy/config.toml on every dev install
# (no backup), and used `rsync -a --delete` on config/themes/ under
# --clean, silently deleting a user's customized config and any
# user-authored custom theme file.
#
# Runs the REAL install-dev.fish script with --fish-only (no agent build,
# no cargo install, no network) against a sandboxed $HOME/$XDG_CONFIG_HOME,
# from the real repo root. PATH is scrubbed down to a minimal set of Unix
# tools so `gpy-agent` (installed on this machine) is not found on PATH --
# this keeps `_verify_installation`'s hot-reload branch (which would start
# a real `gpy-agent` background process and sleep 3s) from ever running.
# `fish --no-config` additionally prevents fish from re-deriving PATH from
# this user's universal `fish_user_paths`/`fish_variables`, which otherwise
# silently reintroduces `~/.local/bin` (where gpy-agent lives) even when
# the outer `env PATH=...` only lists safe directories.

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

set -l safe_path /usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin

function __gpy_run_install_dev --argument-names home_dir safe_path extra_flag
    set -l flags --fish-only
    if test -n "$extra_flag"
        set -a flags $extra_flag
    end
    env HOME=$home_dir XDG_CONFIG_HOME="$home_dir/.config" PATH=$safe_path \
        fish --no-config install-dev.fish $flags 2>&1
end

# Sanity check: confirm gpy-agent is genuinely unreachable under the
# scrubbed environment before relying on that property below. If this ever
# fails, the hot-reload verification branch could start a real background
# agent process, so treat it as a hard test failure rather than silently
# proceeding.
set -l path_check (env PATH=$safe_path fish --no-config -c 'command -v gpy-agent; or echo NOTFOUND' 2>&1)
if test "$path_check" = NOTFOUND
    __gpy_test_pass "sandbox: gpy-agent is unreachable under scrubbed PATH/--no-config"
else
    __gpy_test_fail "sandbox: gpy-agent is still reachable ($path_check) -- refusing to run install-dev.fish"
    exit 1
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

set -l out1 (__gpy_run_install_dev "$home1" "$safe_path" "")
set -l status1 $status

if test $status1 -ne 0
    __gpy_test_fail "install-dev.fish --fish-only exited $status1: $out1"
else
    if diff -q "$home1/original_config.toml" "$custom_config" >/dev/null 2>&1
        __gpy_test_pass "install-dev.fish: customized config.toml left untouched"
    else
        __gpy_test_fail "install-dev.fish: customized config.toml was modified"
    end

    if test -f "$home1/.config/gpy/config.toml.new"
        if diff -q "$repo_root/config/config.toml" "$home1/.config/gpy/config.toml.new" >/dev/null 2>&1
            __gpy_test_pass "install-dev.fish: config.toml.new matches shipped default"
        else
            __gpy_test_fail "install-dev.fish: config.toml.new does not match shipped default"
        end
    else
        __gpy_test_fail "install-dev.fish: config.toml.new was not created for a divergent config"
    end
end

# ===========================================================================
# Test 2: idempotency -- running install-dev.fish --fish-only twice in a
# row does not misbehave.
# ===========================================================================
__gpy_run_install_dev "$home1" "$safe_path" "" >/dev/null 2>&1
if test -f "$home1/.config/gpy/config.toml.new"
    if diff -q "$repo_root/config/config.toml" "$home1/.config/gpy/config.toml.new" >/dev/null 2>&1
        __gpy_test_pass "install-dev.fish: repeat run keeps a single correct config.toml.new"
    else
        __gpy_test_fail "install-dev.fish: repeat run corrupted config.toml.new"
    end
else
    __gpy_test_fail "install-dev.fish: repeat run removed config.toml.new"
end
if diff -q "$home1/original_config.toml" "$custom_config" >/dev/null 2>&1
    __gpy_test_pass "install-dev.fish: repeat run still leaves customized config.toml untouched"
else
    __gpy_test_fail "install-dev.fish: repeat run modified customized config.toml"
end

rm -rf "$home1"

# ===========================================================================
# Test 3: a user-created theme file survives the themes sync under --clean
# (the flavor that used `rsync -a --delete`), and shipped themes are still
# copied/updated correctly.
# ===========================================================================
set -l home2 (mktemp -d)
mkdir -p "$home2/.config/gpy/themes"
set -l custom_theme_path "$home2/.config/gpy/themes/my-custom-theme.toml"
printf '%s\n' "# my custom theme" "[colors]" 'foo = "bar"' >"$custom_theme_path"
cp "$custom_theme_path" "$home2/original_custom_theme.toml"

set -l out2 (__gpy_run_install_dev "$home2" "$safe_path" "--clean")
set -l status2 $status

if test $status2 -ne 0
    __gpy_test_fail "install-dev.fish --fish-only --clean exited $status2: $out2"
else
    if test -f "$custom_theme_path"
        if diff -q "$home2/original_custom_theme.toml" "$custom_theme_path" >/dev/null 2>&1
            __gpy_test_pass "install-dev.fish --clean: custom theme file survives sync"
        else
            __gpy_test_fail "install-dev.fish --clean: custom theme file content changed"
        end
    else
        __gpy_test_fail "install-dev.fish --clean: custom theme file was deleted"
    end

    set -l all_shipped_present 1
    for f in "$repo_root"/config/themes/*
        set -l base (path basename $f)
        if not diff -q "$f" "$home2/.config/gpy/themes/$base" >/dev/null 2>&1
            set all_shipped_present 0
            __gpy_test_fail "install-dev.fish --clean: shipped theme $base missing or stale in destination"
        end
    end
    if test $all_shipped_present -eq 1
        __gpy_test_pass "install-dev.fish --clean: shipped themes still copied/updated correctly"
    end
end

rm -rf "$home2"

# ===========================================================================
# Test 4: first-ever install (no existing config.toml) still creates
# config.toml with the shipped default, and does not create a spurious .new.
# ===========================================================================
set -l home3 (mktemp -d)

set -l out3 (__gpy_run_install_dev "$home3" "$safe_path" "")
set -l status3 $status

if test $status3 -ne 0
    __gpy_test_fail "install-dev.fish --fish-only exited $status3: $out3"
else
    if diff -q "$repo_root/config/config.toml" "$home3/.config/gpy/config.toml" >/dev/null 2>&1
        __gpy_test_pass "install-dev.fish: first install creates config.toml matching shipped default"
    else
        __gpy_test_fail "install-dev.fish: first install config.toml does not match shipped default"
    end
    if test -e "$home3/.config/gpy/config.toml.new"
        __gpy_test_fail "install-dev.fish: first install unexpectedly created config.toml.new"
    else
        __gpy_test_pass "install-dev.fish: first install does not create a spurious config.toml.new"
    end
end

rm -rf "$home3"

if test $failed -eq 1
    exit 1
end

echo "✅ install-dev.fish preserves customized config.toml and custom theme files"
