#!/usr/bin/env fish
# tests/fish/install_dev_config_theme_preserve.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-dev.fish's _install_fish_files must never overwrite a user's
# ~/.config/gpy/config.toml (a divergent config gets a config.toml.new
# sibling instead). Fish wiring itself is gpy setup's (a stub gpy here).
#
# Runs the REAL install-dev.fish script with --files-only (no agent build,
# no cargo install, no network) against a sandboxed $HOME/$XDG_CONFIG_HOME,
# from the real repo root. PATH is scrubbed down to a minimal set of Unix
# tools so `gpy-agent` (installed on this machine) is not found on PATH --
# this keeps `_verify_installation`'s hot-reload branch (which would start
# a real `gpy-agent` background process and sleep 3s) from ever running.
# `fish --no-config` additionally prevents fish from re-deriving PATH from
# this user's universal `fish_user_paths`/`fish_variables`, which otherwise
# silently reintroduces `~/.local/bin` (where gpy-agent lives) even when
# the outer `env PATH=...` only lists safe directories.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

# Expose tools, not their installation directories: Homebrew can also contain
# the developer's live gpy-agent.
set -l safe_path (mktemp -d)
for tool in fish mkdir cp cmp rm readlink grep date mv ln cat
    set -l executable (command -s $tool)
    if test -z "$executable"
        fail "sandbox: required tool $tool is unavailable"
        rm -rf "$safe_path"
        exit 1
    end
    ln -s "$executable" "$safe_path/$tool"
end

function __gpy_cleanup_tool_path --on-event fish_exit --inherit-variable safe_path
    rm -rf "$safe_path"
end

function __gpy_run_install_dev --argument-names home_dir safe_path extra_flag
    mkdir -p "$home_dir/.local/bin"
    printf '#!/bin/sh\nexit 0\n' >"$home_dir/.local/bin/gpy"
    chmod +x "$home_dir/.local/bin/gpy"
    set -l flags --files-only
    if test -n "$extra_flag"
        set -a flags $extra_flag
    end
    env HOME=$home_dir XDG_CONFIG_HOME="$home_dir/.config" XDG_DATA_HOME="$home_dir/.local/share" PATH=$safe_path \
        fish --no-config install-dev.fish $flags 2>&1
end

# Sanity check: confirm gpy and gpy-agent are genuinely unreachable under the
# scrubbed environment before relying on that property below. If this ever
# fails, the hot-reload verification branch could start a real background
# agent process, so treat it as a hard test failure rather than silently
# proceeding.
set -l path_check (env PATH=$safe_path fish --no-config -c 'command -v gpy gpy-agent; or echo NOTFOUND' 2>&1)
if test "$path_check" = NOTFOUND
    pass "sandbox: gpy and gpy-agent are unreachable under scrubbed PATH/--no-config"
else
    fail "sandbox: installed gpy binary is still reachable ($path_check) -- refusing to run install-dev.fish"
    exit 1
end

# --- Test 1: an existing customized config.toml is left untouched, and a
# config.toml.new sibling with the shipped default appears instead. ---
set -l home1 (mktemp -d)
mkdir -p "$home1/.config/gpy"
set -l custom_config "$home1/.config/gpy/config.toml"
printf '%s\n' "# my custom config" "[custom]" "value = true" >"$custom_config"
cp "$custom_config" "$home1/original_config.toml"

set -l out1 (__gpy_run_install_dev "$home1" "$safe_path" "")
set -l status1 $status

if test $status1 -ne 0
    fail "install-dev.fish --files-only exited $status1: $out1"
else
    if diff -q "$home1/original_config.toml" "$custom_config" >/dev/null 2>&1
        pass "install-dev.fish: customized config.toml left untouched"
    else
        fail "install-dev.fish: customized config.toml was modified"
    end

    if test -f "$home1/.config/gpy/config.toml.new"
        if diff -q "$repo_root/config/config.example.toml" "$home1/.config/gpy/config.toml.new" >/dev/null 2>&1
            pass "install-dev.fish: config.toml.new matches shipped default"
        else
            fail "install-dev.fish: config.toml.new does not match shipped default"
        end
    else
        fail "install-dev.fish: config.toml.new was not created for a divergent config"
    end
end

# --- Test 2: idempotency -- running install-dev.fish --files-only twice in a
# row does not misbehave. ---
__gpy_run_install_dev "$home1" "$safe_path" "" >/dev/null 2>&1
if test -f "$home1/.config/gpy/config.toml.new"
    if diff -q "$repo_root/config/config.example.toml" "$home1/.config/gpy/config.toml.new" >/dev/null 2>&1
        pass "install-dev.fish: repeat run keeps a single correct config.toml.new"
    else
        fail "install-dev.fish: repeat run corrupted config.toml.new"
    end
else
    fail "install-dev.fish: repeat run removed config.toml.new"
end
if diff -q "$home1/original_config.toml" "$custom_config" >/dev/null 2>&1
    pass "install-dev.fish: repeat run still leaves customized config.toml untouched"
else
    fail "install-dev.fish: repeat run modified customized config.toml"
end

rm -rf "$home1"

# --- Test 4: first-ever install (no existing config.toml) still creates
# config.toml with the shipped default, and does not create a spurious .new. ---
set -l home3 (mktemp -d)

set -l out3 (__gpy_run_install_dev "$home3" "$safe_path" "")
set -l status3 $status

if test $status3 -ne 0
    fail "install-dev.fish --files-only exited $status3: $out3"
else
    if diff -q "$repo_root/config/config.example.toml" "$home3/.config/gpy/config.toml" >/dev/null 2>&1
        pass "install-dev.fish: first install creates config.toml matching shipped default"
    else
        fail "install-dev.fish: first install config.toml does not match shipped default"
    end
    if test -e "$home3/.config/gpy/config.toml.new"
        fail "install-dev.fish: first install unexpectedly created config.toml.new"
    else
        pass "install-dev.fish: first install does not create a spurious config.toml.new"
    end
end

rm -rf "$home3"

# --- Test 4: a packaged gpy outside $HOME blocks the install unless --allow-shadow.
set -l home4 (mktemp -d)
set -l pkg_bin (mktemp -d)
printf '#!/bin/sh\necho "gpy 0.0.0"\n' >"$pkg_bin/gpy"
chmod +x "$pkg_bin/gpy"
mkdir -p "$home4/.local/bin"
set -l out4 (env HOME=$home4 XDG_CONFIG_HOME="$home4/.config" XDG_DATA_HOME="$home4/.local/share" PATH="$pkg_bin:$safe_path" \
    fish --no-config install-dev.fish --files-only 2>&1 | string collect)
set -l status4 $pipestatus[1]
if test $status4 -ne 0; and string match -q "*already installed at $pkg_bin/gpy*" -- "$out4"
    pass "install-dev.fish: refuses to shadow a packaged gpy"
else
    fail "install-dev.fish: did not refuse a packaged gpy (status $status4): $out4"
end
set -l out4b (env HOME=$home4 XDG_CONFIG_HOME="$home4/.config" XDG_DATA_HOME="$home4/.local/share" PATH="$pkg_bin:$safe_path" \
    fish --no-config install-dev.fish --files-only --allow-shadow 2>&1 | string collect)
if string match -q "*already installed at*" -- "$out4b"
    fail "install-dev.fish: --allow-shadow still refused: $out4b"
else
    pass "install-dev.fish: --allow-shadow bypasses the guard"
end
rm -rf "$home4" "$pkg_bin"

# --- Test 5: the old flag name is rejected, naming its replacement.
set -l out5 (fish --no-config install-dev.fish --fish-only 2>&1 | string collect)
set -l status5 $pipestatus[1]
if test $status5 -eq 1; and string match -q "*--files-only*" -- "$out5"
    pass "install-dev.fish: --fish-only exits 1 naming --files-only"
else
    fail "install-dev.fish: --fish-only was not rejected with --files-only (status $status5): $out5"
end

test_finish "install-dev.fish preserves customized config.toml"
